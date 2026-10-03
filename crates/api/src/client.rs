//! The GitHub client: one octocrab HTTP stack for GraphQL and REST, with our
//! own retry policy, rate-limit tracking, ETag revalidation and caching.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ghtui_store::{Cached, HttpEntry, Store};
use http::{HeaderMap, HeaderValue, StatusCode, header};
use octocrab::Octocrab;
use octocrab::service::middleware::retry::RetryConfig;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::auth::Token;
use crate::model::{Inbox, PrDetail, PrRef, PrSummary, ViewedFiles, ViewedState};
use crate::queries;
use crate::rate_limit::{RateLimits, retry_after};

const MAX_ATTEMPTS: u32 = 3;
/// Rate-limit waits longer than this are reported instead of slept through.
const MAX_INLINE_WAIT_SECS: u64 = 10;

#[derive(Debug, Clone, thiserror::Error)]
pub enum ApiError {
    #[error("GitHub rejected the token. Run `gh auth login` or check GH_TOKEN.")]
    Unauthorized,
    #[error("rate limited by GitHub; retry in {0}s")]
    RateLimited(u64),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("GitHub returned {status}: {message}")]
    Http { status: u16, message: String },
    #[error("GraphQL: {}", .0.join("; "))]
    GraphQl(Vec<String>),
    #[error("network error: {0}")]
    Network(String),
    #[error("unexpected response from GitHub: {0}")]
    Decode(String),
    #[error("could not start HTTP client: {0}")]
    Setup(String),
}

/// Installs rustls' `ring` provider as the process default. Must run before
/// any TLS client is built; safe to call repeatedly.
pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Cheap to clone. The HTTP client is built lazily, on a blocking thread, by
/// the first request: loading the platform's root certificates takes over
/// 100ms on macOS and must not delay the first paint.
#[derive(Clone)]
pub struct GitHub {
    http: Arc<Http>,
    store: Store,
    limits: Arc<Mutex<RateLimits>>,
}

struct Http {
    token: Token,
    base_uri: Option<String>,
    client: tokio::sync::OnceCell<Octocrab>,
}

impl std::fmt::Debug for GitHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHub")
            .field("store", &self.store)
            .finish()
    }
}

struct Response {
    status: StatusCode,
    headers: HeaderMap,
    body: String,
}

enum Request<'a> {
    Get {
        path: &'a str,
        etag: Option<&'a str>,
    },
    Post {
        path: &'a str,
        body: &'a serde_json::Value,
    },
}

impl GitHub {
    pub fn new(token: Token, store: Store) -> Self {
        Self::with_base_uri(token, store, None)
    }

    /// For tests: point the client at a local server.
    pub fn with_base_uri(token: Token, store: Store, base_uri: Option<&str>) -> Self {
        Self {
            http: Arc::new(Http {
                token,
                base_uri: base_uri.map(str::to_owned),
                client: tokio::sync::OnceCell::new(),
            }),
            store,
            limits: Arc::default(),
        }
    }

    async fn octo(&self) -> Result<&Octocrab, ApiError> {
        self.http
            .client
            .get_or_try_init(|| async {
                let token = self.http.token.expose().to_owned();
                let base_uri = self.http.base_uri.clone();
                tokio::task::spawn_blocking(move || build_client(token, base_uri.as_deref()))
                    .await
                    .map_err(|e| ApiError::Setup(e.to_string()))?
            })
            .await
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn rate_limits(&self) -> RateLimits {
        *self.limits.lock().unwrap_or_else(|e| e.into_inner())
    }

    async fn send(&self, request: &Request<'_>) -> Result<Response, ApiError> {
        let octo = self.octo().await?;
        let mut attempt = 0;
        loop {
            attempt += 1;
            let last = attempt >= MAX_ATTEMPTS;
            let result = match request {
                Request::Get { path, etag } => {
                    let mut headers = HeaderMap::new();
                    if let Some(etag) = etag
                        && let Ok(value) = HeaderValue::from_str(etag)
                    {
                        headers.insert(header::IF_NONE_MATCH, value);
                    }
                    octo._get_with_headers(*path, Some(headers)).await
                }
                Request::Post { path, body } => octo._post(*path, Some(*body)).await,
            };
            let response = match result {
                Ok(response) => response,
                Err(err) if !last => {
                    tracing::debug!(%err, attempt, "request failed; retrying");
                    backoff(attempt).await;
                    continue;
                }
                Err(err) => return Err(ApiError::Network(err.to_string())),
            };
            let status = response.status();
            let headers = response.headers().clone();
            self.limits
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .update(&headers);

            if let Some(wait) = retry_after(status, &headers, ghtui_store::now()) {
                if last || wait > MAX_INLINE_WAIT_SECS {
                    return Err(ApiError::RateLimited(wait));
                }
                tracing::info!(wait, "rate limited; waiting");
                tokio::time::sleep(Duration::from_secs(wait.max(1))).await;
                continue;
            }
            if status.is_server_error() && !last {
                tracing::debug!(%status, attempt, "server error; retrying");
                backoff(attempt).await;
                continue;
            }
            let body = octo
                .body_to_string(response)
                .await
                .map_err(|e| ApiError::Network(e.to_string()))?;
            return Ok(Response {
                status,
                headers,
                body,
            });
        }
    }

    /// Runs a GraphQL operation. Partial results are accepted (and their
    /// errors logged); a response without data is an error.
    pub async fn graphql<Q, V>(&self, op: cynic::Operation<Q, V>) -> Result<Q, ApiError>
    where
        Q: DeserializeOwned + 'static,
        V: Serialize,
    {
        let body = serde_json::to_value(&op).map_err(|e| ApiError::Decode(e.to_string()))?;
        let response = self
            .send(&Request::Post {
                path: "/graphql",
                body: &body,
            })
            .await?;
        check_status(&response)?;
        let parsed: cynic::GraphQlResponse<Q> =
            serde_json::from_str(&response.body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let errors: Vec<String> = parsed
            .errors
            .unwrap_or_default()
            .into_iter()
            .map(|e| e.message)
            .collect();
        match parsed.data {
            Some(data) => {
                if !errors.is_empty() {
                    tracing::warn!(?errors, "partial GraphQL result");
                }
                Ok(data)
            }
            None if errors.is_empty() => Err(ApiError::Decode("no data".into())),
            None => Err(ApiError::GraphQl(errors)),
        }
    }

    /// GETs a REST path, revalidating with the cached ETag. A 304 serves the
    /// cached body (and doesn't count against the rate limit).
    pub async fn rest_get(&self, path: &str) -> Result<String, ApiError> {
        let cached = self.store.http_get(path);
        let response = self
            .send(&Request::Get {
                path,
                etag: cached.as_ref().map(|c| c.etag.as_str()),
            })
            .await?;
        if response.status == StatusCode::NOT_MODIFIED
            && let Some(cached) = cached
        {
            return Ok(cached.body);
        }
        check_status(&response)?;
        if let Some(etag) = response
            .headers
            .get(header::ETAG)
            .and_then(|v| v.to_str().ok())
        {
            let entry = HttpEntry {
                etag: etag.to_owned(),
                body: response.body.clone(),
                fetched_at: ghtui_store::now(),
            };
            let store = self.store.clone();
            let key = path.to_owned();
            spawn_store(move || store.http_put(&key, &entry)).await;
        }
        Ok(response.body)
    }

    pub async fn viewer_login(&self) -> Result<String, ApiError> {
        #[derive(serde::Deserialize)]
        struct User {
            login: String,
        }
        let body = self.rest_get("/user").await?;
        let user: User =
            serde_json::from_str(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        Ok(user.login)
    }

    pub fn cached_inbox(&self) -> Option<Cached<Inbox>> {
        self.store.query_get(INBOX_KEY)
    }

    /// My open PRs and the open PRs where my review is requested, fetched
    /// as two concurrent searches.
    pub async fn inbox(&self) -> Result<Inbox, ApiError> {
        let (authored, requested) = tokio::try_join!(
            self.search_prs("is:open is:pr author:@me archived:false sort:updated-desc"),
            self.search_prs("is:open is:pr review-requested:@me archived:false sort:updated-desc"),
        )?;
        let inbox = Inbox {
            authored: authored.0,
            authored_total: authored.1,
            review_requested: requested.0,
            review_requested_total: requested.1,
        };
        self.put_query(INBOX_KEY.to_owned(), inbox.clone()).await;
        Ok(inbox)
    }

    async fn search_prs(&self, query: &str) -> Result<(Vec<PrSummary>, u64), ApiError> {
        use cynic::QueryBuilder;
        let op = queries::SearchQuery::build(queries::SearchVariables {
            query: query.to_owned(),
            first: queries::INBOX_PAGE,
        });
        let data = self.graphql(op).await?;
        Ok(crate::model::search_results(&data.search))
    }

    pub fn cached_pull_request(&self, pr: &PrRef) -> Option<Cached<PrDetail>> {
        self.store.query_get(&pr_key(pr))
    }

    pub async fn pull_request(&self, pr: &PrRef) -> Result<PrDetail, ApiError> {
        use cynic::QueryBuilder;
        let number = i32::try_from(pr.number).map_err(|_| ApiError::NotFound(pr.to_string()))?;
        let op = queries::PullRequestQuery::build(queries::PullRequestVariables {
            owner: pr.repo.owner.clone(),
            name: pr.repo.name.clone(),
            number,
        });
        let data = self.graphql(op).await?;
        let detail = data
            .repository
            .and_then(|r| r.pull_request)
            .and_then(|p| PrDetail::from_wire(&p))
            .ok_or_else(|| ApiError::NotFound(pr.to_string()))?;
        self.put_query(pr_key(pr), detail.clone()).await;
        Ok(detail)
    }

    /// Viewed state of every file in the PR (paginated, 100 per page).
    pub async fn viewed_files(&self, pr: &PrRef) -> Result<ViewedFiles, ApiError> {
        use cynic::QueryBuilder;
        let number = i32::try_from(pr.number).map_err(|_| ApiError::NotFound(pr.to_string()))?;
        let mut after = None;
        let mut out = ViewedFiles {
            pull_request_id: String::new(),
            states: std::collections::HashMap::new(),
        };
        loop {
            let op = queries::PrFilesQuery::build(queries::PrFilesVariables {
                owner: pr.repo.owner.clone(),
                name: pr.repo.name.clone(),
                number,
                after: after.take(),
            });
            let data = self.graphql(op).await?;
            let files = data
                .repository
                .and_then(|r| r.pull_request)
                .ok_or_else(|| ApiError::NotFound(pr.to_string()))?;
            out.pull_request_id = files.id.into_inner();
            let Some(page) = files.files else { break };
            for file in page.nodes.into_iter().flatten().flatten() {
                let state = match file.viewer_viewed_state {
                    queries::FileViewedState::Viewed => ViewedState::Viewed,
                    queries::FileViewedState::Dismissed => ViewedState::Dismissed,
                    queries::FileViewedState::Unviewed => ViewedState::Unviewed,
                };
                out.states.insert(file.path, state);
            }
            match page.page_info.end_cursor {
                Some(cursor) if page.page_info.has_next_page => after = Some(cursor),
                _ => break,
            }
        }
        Ok(out)
    }

    /// Marks (or unmarks) a file as viewed on GitHub.
    pub async fn set_viewed(
        &self,
        pull_request_id: &str,
        path: &str,
        viewed: bool,
    ) -> Result<(), ApiError> {
        use cynic::MutationBuilder;
        let vars = queries::ViewedVariables {
            pull_request_id: cynic::Id::new(pull_request_id),
            path: path.to_owned(),
        };
        if viewed {
            self.graphql(queries::MarkFileAsViewed::build(vars)).await?;
        } else {
            self.graphql(queries::UnmarkFileAsViewed::build(vars))
                .await?;
        }
        Ok(())
    }

    async fn put_query<T: Serialize + Send + 'static>(&self, key: String, value: T) {
        let store = self.store.clone();
        spawn_store(move || store.query_put(&key, &value)).await;
    }
}

fn build_client(token: String, base_uri: Option<&str>) -> Result<Octocrab, ApiError> {
    install_crypto_provider();
    let mut builder = Octocrab::builder()
        .personal_token(token)
        .add_retry_config(RetryConfig::None)
        .set_connect_timeout(Some(Duration::from_secs(10)))
        .set_read_timeout(Some(Duration::from_secs(30)));
    if let Some(uri) = base_uri {
        builder = builder
            .base_uri(uri)
            .map_err(|e| ApiError::Setup(e.to_string()))?;
    }
    builder.build().map_err(|e| ApiError::Setup(e.to_string()))
}

const INBOX_KEY: &str = "inbox";

fn pr_key(pr: &PrRef) -> String {
    format!("pr:{pr}")
}

/// redb writes fsync; keep them off the async worker threads.
async fn spawn_store(f: impl FnOnce() + Send + 'static) {
    if let Err(err) = tokio::task::spawn_blocking(f).await {
        tracing::warn!(%err, "cache write task failed");
    }
}

async fn backoff(attempt: u32) {
    tokio::time::sleep(Duration::from_millis(250 << attempt)).await;
}

fn check_status(response: &Response) -> Result<(), ApiError> {
    let status = response.status;
    if status.is_success() {
        return Ok(());
    }
    if status == StatusCode::UNAUTHORIZED {
        return Err(ApiError::Unauthorized);
    }
    #[derive(serde::Deserialize)]
    struct Message {
        message: String,
    }
    let message = serde_json::from_str::<Message>(&response.body)
        .map(|m| m.message)
        .unwrap_or_else(|_| response.body.chars().take(200).collect());
    if status == StatusCode::NOT_FOUND {
        return Err(ApiError::NotFound(message));
    }
    Err(ApiError::Http {
        status: status.as_u16(),
        message,
    })
}
