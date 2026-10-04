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
use crate::browse;
use crate::model::{
    Inbox, NewThread, PatchFile, PrDetail, PrRef, PrSummary, RepoId, ReviewEvent, ReviewThread,
    Side, ViewedFiles, ViewedState,
};
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

    /// Runs a GraphQL query. Partial results are accepted (and their errors
    /// logged); a response without data is an error.
    pub async fn graphql<Q, V>(&self, op: cynic::Operation<Q, V>) -> Result<Q, ApiError>
    where
        Q: DeserializeOwned + 'static,
        V: Serialize,
    {
        let (data, errors) = self.graphql_raw(op).await?;
        match data {
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

    /// Runs a GraphQL mutation. Any error fails it, with GitHub's messages:
    /// a rejected mutation comes back as data with a null field plus errors.
    async fn mutate<Q, V>(&self, op: cynic::Operation<Q, V>) -> Result<Q, ApiError>
    where
        Q: DeserializeOwned + 'static,
        V: Serialize,
    {
        match self.graphql_raw(op).await? {
            (_, errors) if !errors.is_empty() => Err(ApiError::GraphQl(errors)),
            (Some(data), _) => Ok(data),
            (None, _) => Err(ApiError::Decode("no data".into())),
        }
    }

    async fn graphql_raw<Q, V>(
        &self,
        op: cynic::Operation<Q, V>,
    ) -> Result<(Option<Q>, Vec<String>), ApiError>
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
        let errors = parsed
            .errors
            .unwrap_or_default()
            .into_iter()
            .map(|e| e.message)
            .collect();
        Ok((parsed.data, errors))
    }

    /// Runs a query built at runtime (fields that depend on data), as JSON.
    async fn graphql_json(
        &self,
        query: &str,
        variables: serde_json::Value,
    ) -> Result<serde_json::Value, ApiError> {
        let body = serde_json::json!({ "query": query, "variables": variables });
        let response = self
            .send(&Request::Post {
                path: "/graphql",
                body: &body,
            })
            .await?;
        check_status(&response)?;
        let mut parsed: serde_json::Value =
            serde_json::from_str(&response.body).map_err(|e| ApiError::Decode(e.to_string()))?;
        match parsed.get_mut("data").map(serde_json::Value::take) {
            Some(data) if !data.is_null() => Ok(data),
            _ => Err(ApiError::GraphQl(
                parsed["errors"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| e["message"].as_str().map(str::to_owned))
                    .collect(),
            )),
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
            self.mutate(queries::MarkFileAsViewed::build(vars)).await?;
        } else {
            self.mutate(queries::UnmarkFileAsViewed::build(vars))
                .await?;
        }
        Ok(())
    }

    /// All review threads of a PR, with up to 100 comments each.
    pub async fn review_threads(&self, pr: &PrRef) -> Result<Vec<ReviewThread>, ApiError> {
        use cynic::QueryBuilder;
        let number = pr_number(pr)?;
        let mut after = None;
        let mut threads = Vec::new();
        loop {
            let op = queries::ThreadsQuery::build(queries::ThreadsVariables {
                owner: pr.repo.owner.clone(),
                name: pr.repo.name.clone(),
                number,
                after: after.take(),
            });
            let data = self.graphql(op).await?;
            let page = data
                .repository
                .and_then(|r| r.pull_request)
                .ok_or_else(|| ApiError::NotFound(pr.to_string()))?
                .review_threads;
            threads.extend(
                page.nodes
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(ReviewThread::from_wire),
            );
            match page.page_info.end_cursor {
                Some(cursor) if page.page_info.has_next_page => after = Some(cursor),
                _ => break,
            }
        }
        Ok(threads)
    }

    /// GitHub's per-file patches, for commentable ranges. Paginated (100 per
    /// page) and capped by GitHub at 3000 files.
    pub async fn pr_patches(&self, pr: &PrRef) -> Result<Vec<PatchFile>, ApiError> {
        let mut files = Vec::new();
        for page in 1..=30 {
            let path = format!(
                "/repos/{}/{}/pulls/{}/files?per_page=100&page={page}",
                pr.repo.owner, pr.repo.name, pr.number
            );
            let body = self.rest_get(&path).await?;
            let batch: Vec<PatchFile> =
                serde_json::from_str(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
            let done = batch.len() < 100;
            files.extend(batch);
            if done {
                break;
            }
        }
        Ok(files)
    }

    /// The PR's node ID and the viewer's pending review on it, if any.
    pub async fn pending_review(&self, pr: &PrRef) -> Result<(String, Option<String>), ApiError> {
        use cynic::QueryBuilder;
        let op = queries::PendingReviewQuery::build(queries::PendingReviewVariables {
            owner: pr.repo.owner.clone(),
            name: pr.repo.name.clone(),
            number: pr_number(pr)?,
        });
        let pr_node = self
            .graphql(op)
            .await?
            .repository
            .and_then(|r| r.pull_request)
            .ok_or_else(|| ApiError::NotFound(pr.to_string()))?;
        let review = pr_node
            .reviews
            .and_then(|r| r.nodes)
            .into_iter()
            .flatten()
            .flatten()
            .next()
            .map(|r| r.id.into_inner());
        Ok((pr_node.id.into_inner(), review))
    }

    /// The head commit of `login`'s latest submitted review, if any.
    pub async fn last_review_commit(
        &self,
        pr: &PrRef,
        login: &str,
    ) -> Result<Option<String>, ApiError> {
        use cynic::QueryBuilder;
        let op = queries::LastReviewQuery::build(queries::LastReviewVariables {
            owner: pr.repo.owner.clone(),
            name: pr.repo.name.clone(),
            number: pr_number(pr)?,
            login: login.to_owned(),
        });
        let reviews = self
            .graphql(op)
            .await?
            .repository
            .and_then(|r| r.pull_request)
            .and_then(|p| p.reviews)
            .and_then(|r| r.nodes)
            .unwrap_or_default();
        Ok(reviews
            .into_iter()
            .flatten()
            .rev()
            .find(|r| r.state != queries::ReviewState::Pending)
            .and_then(|r| r.commit)
            .map(|c| c.oid.0))
    }

    /// Starts a pending review on `commit`.
    pub async fn start_review(
        &self,
        pull_request_id: &str,
        commit: &str,
    ) -> Result<String, ApiError> {
        use cynic::MutationBuilder;
        let op = queries::StartReview::build(queries::StartReviewVariables {
            pull_request_id: cynic::Id::new(pull_request_id),
            commit: queries::GitObjectId(commit.to_owned()),
        });
        self.mutate(op)
            .await?
            .add_pull_request_review
            .and_then(|p| p.pull_request_review)
            .map(|r| r.id.into_inner())
            .ok_or_else(|| ApiError::Decode("no review in response".into()))
    }

    /// Adds a thread to a pending review. Fails (with GitHub's message) if
    /// the anchor isn't part of GitHub's diff.
    pub async fn add_review_thread(
        &self,
        review_id: &str,
        thread: &NewThread,
    ) -> Result<String, ApiError> {
        use cynic::MutationBuilder;
        let side = |s: Side| match s {
            Side::Left => queries::DiffSide::Left,
            Side::Right => queries::DiffSide::Right,
        };
        let line = |n: Option<u32>| n.and_then(|n| i32::try_from(n).ok());
        let input = queries::AddThreadInput {
            pull_request_review_id: Some(cynic::Id::new(review_id)),
            path: Some(thread.path.clone()),
            body: thread.body.clone(),
            line: line(thread.line),
            side: thread.line.map(|_| side(thread.side)),
            start_line: line(thread.start_line),
            start_side: thread.start_line.and(thread.start_side).map(side),
            subject_type: Some(if thread.line.is_some() {
                queries::ThreadSubjectType::Line
            } else {
                queries::ThreadSubjectType::File
            }),
        };
        self.mutate(queries::AddThread::build(queries::AddThreadVariables {
            input,
        }))
        .await?
        .add_pull_request_review_thread
        .and_then(|p| p.thread)
        .map(|t| t.id.into_inner())
        .ok_or_else(|| ApiError::Decode("no thread in response".into()))
    }

    pub async fn submit_review(
        &self,
        review_id: &str,
        event: ReviewEvent,
        body: &str,
    ) -> Result<(), ApiError> {
        use cynic::MutationBuilder;
        let event = match event {
            ReviewEvent::Comment => queries::ReviewEvent::Comment,
            ReviewEvent::Approve => queries::ReviewEvent::Approve,
            ReviewEvent::RequestChanges => queries::ReviewEvent::RequestChanges,
        };
        let op = queries::SubmitReview::build(queries::SubmitReviewVariables {
            review_id: cynic::Id::new(review_id),
            event,
            body: (!body.trim().is_empty()).then(|| body.to_owned()),
        });
        self.mutate(op).await.map(drop)
    }

    /// Replies to a thread right away (outside any pending review).
    pub async fn reply(&self, thread_id: &str, body: &str) -> Result<(), ApiError> {
        use cynic::MutationBuilder;
        let op = queries::Reply::build(queries::ReplyVariables {
            thread_id: cynic::Id::new(thread_id),
            body: body.to_owned(),
        });
        self.mutate(op).await.map(drop)
    }

    pub async fn set_resolved(&self, thread_id: &str, resolved: bool) -> Result<(), ApiError> {
        use cynic::MutationBuilder;
        let vars = queries::ThreadIdVariables {
            thread_id: cynic::Id::new(thread_id),
        };
        if resolved {
            self.mutate(queries::Resolve::build(vars)).await.map(drop)
        } else {
            self.mutate(queries::Unresolve::build(vars)).await.map(drop)
        }
    }

    // ---- browsing ----------------------------------------------------------

    /// A cached page, by one of the keys in [`crate::browse::keys`].
    pub fn cached<T: DeserializeOwned>(&self, key: &str) -> Option<ghtui_store::Cached<T>> {
        self.store.query_get(key)
    }

    /// A repository's overview: stats, root directory, last commit, README.
    pub async fn repo(&self, repo: &RepoId) -> Result<browse::RepoOverview, ApiError> {
        use cynic::QueryBuilder;
        let op = browse::RepoQuery::build(browse::RepoVariables {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
            expression: "HEAD:".into(),
        });
        let (data, readme) = tokio::join!(self.graphql(op), self.readme(repo));
        let readme = readme.unwrap_or_else(|err| {
            tracing::debug!(%repo, %err, "no readme");
            None
        });
        let overview = data?
            .repository
            .and_then(|r| r.into_overview(readme))
            .ok_or_else(|| ApiError::NotFound(repo.to_string()))?;
        self.put_query(browse::keys::repo(repo), overview.clone())
            .await;
        Ok(overview)
    }

    /// The README GitHub shows for the repository root.
    pub async fn readme(&self, repo: &RepoId) -> Result<Option<browse::Readme>, ApiError> {
        use base64::Engine;
        #[derive(serde::Deserialize)]
        struct Wire {
            path: String,
            content: String,
        }
        let body = match self
            .rest_get(&format!("/repos/{}/{}/readme", repo.owner, repo.name))
            .await
        {
            Ok(body) => body,
            Err(ApiError::NotFound(_)) => return Ok(None),
            Err(err) => return Err(err),
        };
        let wire: Wire =
            serde_json::from_str(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let compact: String = wire.content.split_whitespace().collect();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(compact)
            .map_err(|e| ApiError::Decode(e.to_string()))?;
        Ok(Some(browse::Readme {
            path: wire.path,
            text: String::from_utf8_lossy(&bytes).into_owned(),
        }))
    }

    /// A directory at `rev:path`.
    pub async fn tree(
        &self,
        repo: &RepoId,
        rev: &str,
        path: &str,
    ) -> Result<Vec<browse::TreeEntry>, ApiError> {
        use cynic::QueryBuilder;
        let op = browse::ObjectQuery::build(browse::RepoVariables {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
            expression: format!("{rev}:{path}"),
        });
        match self.graphql(op).await?.repository.and_then(|r| r.object) {
            Some(browse::GitObject::Tree(t)) => {
                let entries = browse::entries(t);
                self.put_query(browse::keys::tree(repo, rev, path), entries.clone())
                    .await;
                Ok(entries)
            }
            _ => Err(ApiError::NotFound(format!("{repo}/{path}"))),
        }
    }

    /// A file at `rev:path`.
    pub async fn blob(
        &self,
        repo: &RepoId,
        rev: &str,
        path: &str,
    ) -> Result<browse::Blob, ApiError> {
        use cynic::QueryBuilder;
        let op = browse::ObjectQuery::build(browse::RepoVariables {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
            expression: format!("{rev}:{path}"),
        });
        match self.graphql(op).await?.repository.and_then(|r| r.object) {
            Some(browse::GitObject::Blob(b)) => Ok(browse::Blob {
                path: path.to_owned(),
                text: if b.is_binary.unwrap_or(false) {
                    None
                } else {
                    b.text
                },
                size: u64::try_from(b.byte_size).unwrap_or(0),
                truncated: b.is_truncated,
            }),
            _ => Err(ApiError::NotFound(format!("{repo}/{path}"))),
        }
    }

    /// One page of search results (30 per page).
    pub async fn search(
        &self,
        kind: browse::SearchKind,
        query: &str,
        after: Option<String>,
    ) -> Result<browse::SearchResults, ApiError> {
        use cynic::QueryBuilder;
        let first_page = after.is_none();
        // GitHub's issue search covers both; the tabs separate them.
        let typed = |is: &str| {
            if query.contains("is:issue") || query.contains("is:pr") {
                query.to_owned()
            } else {
                format!("{query} {is}")
            }
        };
        let api_query = match kind {
            browse::SearchKind::Issues => typed("is:issue"),
            browse::SearchKind::Pulls => typed("is:pr"),
            _ => query.to_owned(),
        };
        let op = browse::BrowseSearch::build(browse::BrowseSearchVariables {
            query: api_query,
            kind: match kind {
                browse::SearchKind::Repos => browse::SearchType::Repository,
                browse::SearchKind::Issues | browse::SearchKind::Pulls => browse::SearchType::Issue,
                browse::SearchKind::Users => browse::SearchType::User,
            },
            first: 30,
            after,
        });
        let conn = self.graphql(op).await?.search;
        let next = conn
            .page_info
            .end_cursor
            .filter(|_| conn.page_info.has_next_page);
        let items = conn.nodes.into_iter().flatten().flatten();
        let count = |n: i32| u64::try_from(n).unwrap_or(0);
        let results = match kind {
            browse::SearchKind::Repos => browse::SearchResults::Repos(browse::Results {
                total: count(conn.repository_count),
                items: items
                    .filter_map(|i| match i {
                        browse::BrowseItem::Repository(r) => r.into_summary(),
                        _ => None,
                    })
                    .collect(),
                next,
            }),
            browse::SearchKind::Issues | browse::SearchKind::Pulls => {
                browse::SearchResults::Issues(browse::Results {
                    total: count(conn.issue_count),
                    items: items.filter_map(browse::BrowseItem::into_issue).collect(),
                    next,
                })
            }
            browse::SearchKind::Users => browse::SearchResults::Users(browse::Results {
                total: count(conn.user_count),
                items: items.filter_map(browse::BrowseItem::into_user).collect(),
                next,
            }),
        };
        if first_page {
            self.put_query(browse::keys::search(kind, query), results.clone())
                .await;
        }
        Ok(results)
    }

    /// An issue; `None` when the number is a pull request (GitHub redirects
    /// those).
    pub async fn issue(
        &self,
        repo: &RepoId,
        number: u64,
    ) -> Result<Option<browse::IssueDetail>, ApiError> {
        use cynic::QueryBuilder;
        let op = browse::IssueQuery::build(browse::IssueVariables {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
            number: i32::try_from(number)
                .map_err(|_| ApiError::NotFound(format!("{repo}#{number}")))?,
        });
        let issue = match self
            .graphql(op)
            .await?
            .repository
            .and_then(|r| r.issue_or_pull_request)
        {
            Some(browse::IssueOrPr::Issue(issue)) => issue.into_detail(),
            Some(browse::IssueOrPr::PullRequest(_)) => return Ok(None),
            _ => None,
        }
        .ok_or_else(|| ApiError::NotFound(format!("{repo}#{number}")))?;
        self.put_query(browse::keys::issue(repo, number), issue.clone())
            .await;
        Ok(Some(issue))
    }

    /// Comments, reviews and commits of a pull request.
    pub async fn pr_activity(&self, pr: &PrRef) -> Result<browse::PrActivity, ApiError> {
        use cynic::QueryBuilder;
        let op = browse::PrActivityQuery::build(browse::IssueVariables {
            owner: pr.repo.owner.clone(),
            name: pr.repo.name.clone(),
            number: pr_number(pr)?,
        });
        let activity = self
            .graphql(op)
            .await?
            .repository
            .and_then(|r| r.pull_request)
            .map(browse::WirePrActivity::into_activity)
            .ok_or_else(|| ApiError::NotFound(pr.to_string()))?;
        self.put_query(browse::keys::pr_activity(pr), activity.clone())
            .await;
        Ok(activity)
    }

    /// A user's or organization's profile.
    pub async fn profile(&self, login: &str) -> Result<browse::Profile, ApiError> {
        use cynic::QueryBuilder;
        let op = browse::ProfileQuery::build(browse::ProfileVariables {
            login: login.to_owned(),
        });
        let profile = self
            .graphql(op)
            .await?
            .into_profile()
            .ok_or_else(|| ApiError::NotFound(login.to_owned()))?;
        self.put_query(browse::keys::profile(login), profile.clone())
            .await;
        Ok(profile)
    }

    /// Every file path at `rev` (GitHub's "Go to file"), and whether GitHub
    /// cut the list short.
    pub async fn file_list(
        &self,
        repo: &RepoId,
        rev: &str,
    ) -> Result<(Vec<String>, bool), ApiError> {
        #[derive(serde::Deserialize)]
        struct Entry {
            path: String,
            #[serde(rename = "type")]
            kind: String,
        }
        #[derive(serde::Deserialize)]
        struct Wire {
            tree: Vec<Entry>,
            #[serde(default)]
            truncated: bool,
        }
        let body = self
            .rest_get(&format!(
                "/repos/{}/{}/git/trees/{}?recursive=1",
                repo.owner,
                repo.name,
                encode_path(rev)
            ))
            .await?;
        let wire: Wire =
            serde_json::from_str(&body).map_err(|e| ApiError::Decode(e.to_string()))?;
        let files: Vec<String> = wire
            .tree
            .into_iter()
            .filter(|e| e.kind == "blob")
            .map(|e| e.path)
            .collect();
        Ok((files, wire.truncated))
    }

    /// The latest commit touching each of `names` in `dir` at `rev`, as
    /// GitHub's file list shows: one query, one `history` per entry.
    pub async fn last_commits(
        &self,
        repo: &RepoId,
        rev: &str,
        dir: &str,
        names: &[String],
    ) -> Result<std::collections::HashMap<String, browse::CommitInfo>, ApiError> {
        let quote = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let mut fields = String::new();
        for (i, name) in names.iter().take(100).enumerate() {
            let path = if dir.is_empty() {
                name.clone()
            } else {
                format!("{dir}/{name}")
            };
            fields.push_str(&format!(
                "e{i}: history(first: 1, path: \"{}\") {{ nodes {{ oid messageHeadline committedDate author {{ name user {{ login }} }} }} }}\n",
                quote(&path)
            ));
        }
        let query = format!(
            "query($owner: String!, $name: String!, $rev: String!) {{ repository(owner: $owner, name: $name) {{ object(expression: $rev) {{ ... on Commit {{ {fields} }} }} }} }}"
        );
        let data = self
            .graphql_json(
                &query,
                serde_json::json!({ "owner": repo.owner, "name": repo.name, "rev": rev }),
            )
            .await?;
        let commit = &data["repository"]["object"];
        let mut out = std::collections::HashMap::new();
        for (i, name) in names.iter().take(100).enumerate() {
            let node = &commit[format!("e{i}")]["nodes"][0];
            let (Some(oid), Some(headline), Some(date)) = (
                node["oid"].as_str(),
                node["messageHeadline"].as_str(),
                node["committedDate"].as_str(),
            ) else {
                continue;
            };
            let author = node["author"]["user"]["login"]
                .as_str()
                .or_else(|| node["author"]["name"].as_str())
                .unwrap_or("someone");
            out.insert(
                name.clone(),
                browse::CommitInfo {
                    oid: oid.to_owned(),
                    headline: headline.to_owned(),
                    author: author.to_owned(),
                    date: date.to_owned(),
                },
            );
        }
        self.put_query(browse::keys::last_commits(repo, rev, dir), out.clone())
            .await;
        Ok(out)
    }

    /// Branches and tags, most recently committed first.
    pub async fn refs(&self, repo: &RepoId) -> Result<browse::Refs, ApiError> {
        use cynic::QueryBuilder;
        let op = browse::BranchesQuery::build(browse::BranchesVariables {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
        });
        let refs = self
            .graphql(op)
            .await?
            .repository
            .map(browse::RepoBranches::into_refs)
            .ok_or_else(|| ApiError::NotFound(repo.to_string()))?;
        self.put_query(browse::keys::refs(repo), refs.clone()).await;
        Ok(refs)
    }

    /// Saves a small value in the cache (e.g. recently visited pages).
    pub async fn remember<T: Serialize + Send + 'static>(&self, key: &str, value: T) {
        self.put_query(key.to_owned(), value).await;
    }

    /// Repositories you own or contribute to, most recently pushed first.
    pub async fn viewer_repos(&self) -> Result<Vec<browse::RepoSummary>, ApiError> {
        use cynic::QueryBuilder;
        let data = self.graphql(browse::ViewerReposQuery::build(())).await?;
        let (repos, _) = browse::repo_list(data.viewer.repositories);
        self.put_query(browse::keys::VIEWER_REPOS.to_owned(), repos.clone())
            .await;
        Ok(repos)
    }

    /// Comments on an issue or pull request (by node ID).
    pub async fn add_comment(&self, subject_id: &str, body: &str) -> Result<(), ApiError> {
        use cynic::MutationBuilder;
        let op = browse::AddComment::build(browse::AddCommentVariables {
            subject: cynic::Id::new(subject_id),
            body: body.to_owned(),
        });
        self.mutate(op).await.map(drop)
    }

    pub async fn set_starred(&self, repo_id: &str, starred: bool) -> Result<(), ApiError> {
        use cynic::MutationBuilder;
        let vars = browse::StarVariables {
            starrable: cynic::Id::new(repo_id),
        };
        if starred {
            self.mutate(browse::AddStar::build(vars)).await.map(drop)
        } else {
            self.mutate(browse::RemoveStar::build(vars)).await.map(drop)
        }
    }

    async fn put_query<T: Serialize + Send + 'static>(&self, key: String, value: T) {
        let store = self.store.clone();
        spawn_store(move || store.query_put(&key, &value)).await;
    }
}

/// Percent-encodes a ref for a URL path (branch names may contain `/`).
fn encode_path(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
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

fn pr_number(pr: &PrRef) -> Result<i32, ApiError> {
    i32::try_from(pr.number).map_err(|_| ApiError::NotFound(pr.to_string()))
}

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
