//! The GitHub client: one HTTP client (reqwest) for GraphQL and REST, with
//! our own retry policy, rate-limit tracking, ETag revalidation and caching.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cynic::{MutationBuilder, QueryBuilder};
use ghtui_store::{Cached, HttpEntry, Store};
use http::{HeaderMap, HeaderValue, StatusCode, header};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::auth::Token;
use crate::browse;
use crate::model::{
    Inbox, NewThread, NodeId, PatchFile, PrDetail, PrRef, PrSummary, RepoId, ReviewEvent,
    ReviewThread, ViewedFiles,
};
use crate::queries::{self, nodes};
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
    /// A bug in ghtui, not a problem with GitHub.
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<serde_json::Error> for ApiError {
    fn from(err: serde_json::Error) -> Self {
        ApiError::Decode(err.to_string())
    }
}

/// Cheap to clone. The HTTP client is built lazily, on a blocking thread, by
/// the first request: loading the platform's root certificates takes over
/// 100ms on macOS and must not delay the first paint.
#[derive(Clone)]
pub struct GitHub {
    http: Arc<Http>,
    store: Store,
    limits: Arc<Mutex<RateLimits>>,
    /// GitHub's last answer was 401, and whether that was reported.
    rejected: Arc<(AtomicBool, AtomicBool)>,
}

struct Http {
    token: Token,
    /// `https://api.github.com`, or a local server in tests.
    base_uri: String,
    client: tokio::sync::OnceCell<reqwest::Client>,
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
        /// Safe to send twice: a query, not a mutation.
        idempotent: bool,
    },
}

impl Request<'_> {
    fn idempotent(&self) -> bool {
        match self {
            Request::Get { .. } => true,
            Request::Post { idempotent, .. } => *idempotent,
        }
    }
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
                base_uri: base_uri
                    .unwrap_or("https://api.github.com")
                    .trim_end_matches('/')
                    .to_owned(),
                client: tokio::sync::OnceCell::new(),
            }),
            store,
            limits: Arc::default(),
            rejected: Arc::default(),
        }
    }

    async fn client(&self) -> Result<&reqwest::Client, ApiError> {
        self.http
            .client
            .get_or_try_init(|| async {
                let token = self.http.token.expose().to_owned();
                tokio::task::spawn_blocking(move || build_client(&token))
                    .await
                    .map_err(|e| ApiError::Setup(e.to_string()))?
            })
            .await
    }

    /// Whether GitHub started (`Some(true)`) or stopped rejecting the token
    /// since this was last asked.
    pub fn token_rejected_change(&self) -> Option<bool> {
        let now = self.rejected.0.load(Ordering::Relaxed);
        let was = self.rejected.1.swap(now, Ordering::Relaxed);
        (now != was).then_some(now)
    }

    pub fn rate_limits(&self) -> RateLimits {
        *self
            .limits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    async fn send(&self, request: &Request<'_>) -> Result<Response, ApiError> {
        let client = self.client().await?;
        let mut attempt = 0;
        loop {
            attempt += 1;
            let last = attempt >= MAX_ATTEMPTS;
            let result = match request {
                Request::Get { path, etag } => {
                    let mut get = client.get(format!("{}{path}", self.http.base_uri));
                    if let Some(etag) = etag
                        && let Ok(value) = HeaderValue::from_str(etag)
                    {
                        get = get.header(header::IF_NONE_MATCH, value);
                    }
                    get.send().await
                }
                Request::Post { path, body, .. } => {
                    let url = format!("{}{path}", self.http.base_uri);
                    client.post(url).json(body).send().await
                }
            };
            // A mutation that may have reached GitHub is never sent again:
            // it could post a comment twice. A failed connect never left.
            let response = match result {
                Ok(response) => response,
                Err(err) if !last && (request.idempotent() || err.is_connect()) => {
                    tracing::debug!(%err, attempt, "request failed; retrying");
                    backoff(attempt).await;
                    continue;
                }
                Err(err) => return Err(ApiError::Network(err.to_string())),
            };
            let status = response.status();
            self.rejected
                .0
                .store(status == StatusCode::UNAUTHORIZED, Ordering::Relaxed);
            let headers = response.headers().clone();
            self.limits
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .update(&headers);

            if let Some(wait) = retry_after(status, &headers, ghtui_store::now()) {
                if last || wait > MAX_INLINE_WAIT_SECS {
                    return Err(ApiError::RateLimited(wait));
                }
                tracing::info!(wait, "rate limited; waiting");
                tokio::time::sleep(Duration::from_secs(wait.max(1))).await;
                continue;
            }
            if status.is_server_error() && !last && request.idempotent() {
                tracing::debug!(%status, attempt, "server error; retrying");
                backoff(attempt).await;
                continue;
            }
            let body = response
                .text()
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
        let idempotent = op.query.trim_start().starts_with("query");
        let parsed: cynic::GraphQlResponse<Q> = self
            .post_graphql(&serde_json::to_value(&op)?, idempotent)
            .await?;
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
        let mut parsed: serde_json::Value = self.post_graphql(&body, true).await?;
        match parsed.get_mut("data").map(serde_json::Value::take) {
            Some(data) if !data.is_null() => Ok(data),
            _ => Err(ApiError::GraphQl(
                parsed
                    .get("errors")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|e| e.get("message")?.as_str().map(str::to_owned))
                    .collect(),
            )),
        }
    }

    async fn post_graphql<T: DeserializeOwned>(
        &self,
        body: &serde_json::Value,
        idempotent: bool,
    ) -> Result<T, ApiError> {
        let response = self
            .send(&Request::Post {
                path: "/graphql",
                body,
                idempotent,
            })
            .await?;
        check_status(&response)?;
        Ok(serde_json::from_str(&response.body)?)
    }

    /// GETs a REST path, revalidating with the cached ETag. A 304 serves the
    /// cached body (and doesn't count against the rate limit).
    pub async fn rest_get(&self, path: &str) -> Result<String, ApiError> {
        let cached = {
            let (store, key) = (self.store.clone(), path.to_owned());
            tokio::task::spawn_blocking(move || store.http_get(&key))
                .await
                .unwrap_or_else(|err| {
                    tracing::warn!(%err, "cache read task failed");
                    None
                })
        };
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
            };
            let store = self.store.clone();
            let key = path.to_owned();
            spawn_store(move || store.http_put(&key, &entry)).await;
        }
        Ok(response.body)
    }

    /// GETs a REST path and decodes its JSON.
    async fn rest_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        Ok(serde_json::from_str(&self.rest_get(path).await?)?)
    }

    pub async fn viewer_login(&self) -> Result<String, ApiError> {
        #[derive(serde::Deserialize)]
        struct User {
            login: String,
        }
        let user: User = self.rest_json("/user").await?;
        Ok(user.login)
    }

    pub fn cached_inbox(&self) -> Option<Cached<Inbox>> {
        self.cached(INBOX_KEY)
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
        Ok(self.kept(INBOX_KEY, inbox).await)
    }

    async fn search_prs(&self, query: &str) -> Result<(Vec<PrSummary>, u64), ApiError> {
        let op = queries::SearchQuery::build(queries::SearchVariables {
            query: query.to_owned(),
            first: queries::INBOX_PAGE,
        });
        let data = self.graphql(op).await?;
        Ok(crate::model::search_results(data.search))
    }

    pub fn cached_pull_request(&self, pr: &PrRef) -> Option<Cached<PrDetail>> {
        self.cached(&pr_key(pr))
    }

    pub async fn pull_request(&self, pr: &PrRef) -> Result<PrDetail, ApiError> {
        let op = queries::PullRequestQuery::build(number_vars(pr)?);
        let data = self.graphql(op).await?;
        let detail = data
            .repository
            .and_then(|r| r.pull_request)
            .and_then(PrDetail::from_wire)
            .ok_or_else(|| ApiError::NotFound(pr.to_string()))?;
        Ok(self.kept(&pr_key(pr), detail).await)
    }

    /// Viewed state of every file in the PR (paginated, 100 per page).
    pub async fn viewed_files(&self, pr: &PrRef) -> Result<ViewedFiles, ApiError> {
        let mut out = ViewedFiles {
            pull_request_id: NodeId::default(),
            states: std::collections::HashMap::new(),
        };
        let mut after = None;
        loop {
            let op = queries::PrFilesQuery::build(page_vars(pr, after)?);
            let files = self
                .graphql(op)
                .await?
                .repository
                .and_then(|r| r.pull_request)
                .ok_or_else(|| ApiError::NotFound(pr.to_string()))?;
            out.pull_request_id = files.id.into();
            let Some(page) = files.files else { break };
            out.states
                .extend(nodes(page.nodes).map(|f| (f.path, f.viewer_viewed_state)));
            after = page.page_info.next();
            if after.is_none() {
                break;
            }
        }
        Ok(out)
    }

    /// Marks (or unmarks) a file as viewed on GitHub.
    pub async fn set_viewed(
        &self,
        pull_request_id: &NodeId,
        path: &str,
        viewed: bool,
    ) -> Result<(), ApiError> {
        let vars = queries::ViewedVariables {
            pull_request_id: pull_request_id.gql(),
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
        let mut threads = Vec::new();
        let mut after = None;
        loop {
            let op = queries::ThreadsQuery::build(page_vars(pr, after)?);
            let page = self
                .graphql(op)
                .await?
                .repository
                .and_then(|r| r.pull_request)
                .ok_or_else(|| ApiError::NotFound(pr.to_string()))?
                .review_threads;
            threads.extend(nodes(page.nodes).map(ReviewThread::from_wire));
            after = page.page_info.next();
            if after.is_none() {
                break;
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
            let batch: Vec<PatchFile> = self.rest_json(&path).await?;
            let done = batch.len() < 100;
            files.extend(batch);
            if done {
                break;
            }
        }
        Ok(files)
    }

    /// The PR's node ID and the viewer's pending review on it, if any.
    pub async fn pending_review(&self, pr: &PrRef) -> Result<(NodeId, Option<NodeId>), ApiError> {
        let op = queries::PendingReviewQuery::build(number_vars(pr)?);
        let pr_node = self
            .graphql(op)
            .await?
            .repository
            .and_then(|r| r.pull_request)
            .ok_or_else(|| ApiError::NotFound(pr.to_string()))?;
        let review = nodes(pr_node.reviews.and_then(|r| r.nodes))
            .next()
            .map(|r| NodeId::from(r.id));
        Ok((pr_node.id.into(), review))
    }

    /// The head commit of `login`'s latest submitted review, if any.
    pub async fn last_review_commit(
        &self,
        pr: &PrRef,
        login: &str,
    ) -> Result<Option<String>, ApiError> {
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
        pull_request_id: &NodeId,
        commit: &str,
    ) -> Result<NodeId, ApiError> {
        let op = queries::StartReview::build(queries::StartReviewVariables {
            pull_request_id: pull_request_id.gql(),
            commit: queries::GitObjectId(commit.to_owned()),
        });
        self.mutate(op)
            .await?
            .add_pull_request_review
            .and_then(|p| p.pull_request_review)
            .map(|r| NodeId::from(r.id))
            .ok_or_else(|| ApiError::Decode("no review in response".into()))
    }

    /// Adds a thread to a pending review. Fails (with GitHub's message) if
    /// the anchor isn't part of GitHub's diff.
    pub async fn add_review_thread(
        &self,
        review_id: &NodeId,
        thread: &NewThread,
    ) -> Result<NodeId, ApiError> {
        let line = |n: Option<u32>| n.and_then(|n| i32::try_from(n).ok());
        let input = queries::AddThreadInput {
            pull_request_review_id: Some(review_id.gql()),
            path: Some(thread.path.clone()),
            body: thread.body.clone(),
            line: line(thread.line),
            side: thread.line.map(|_| thread.side),
            start_line: line(thread.start_line),
            start_side: thread.start_line.and(thread.start_side),
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
        .map(|t| NodeId::from(t.id))
        .ok_or_else(|| ApiError::Decode("no thread in response".into()))
    }

    pub async fn submit_review(
        &self,
        review_id: &NodeId,
        event: ReviewEvent,
        body: &str,
    ) -> Result<(), ApiError> {
        let event = match event {
            ReviewEvent::Comment => queries::ReviewEvent::Comment,
            ReviewEvent::Approve => queries::ReviewEvent::Approve,
            ReviewEvent::RequestChanges => queries::ReviewEvent::RequestChanges,
        };
        let op = queries::SubmitReview::build(queries::SubmitReviewVariables {
            review_id: review_id.gql(),
            event,
            body: (!body.trim().is_empty()).then(|| body.to_owned()),
        });
        self.mutate(op).await.map(drop)
    }

    /// Replies to a thread right away (outside any pending review).
    pub async fn reply(&self, thread_id: &NodeId, body: &str) -> Result<(), ApiError> {
        let op = queries::Reply::build(queries::ReplyVariables {
            thread_id: thread_id.gql(),
            body: body.to_owned(),
        });
        self.mutate(op).await.map(drop)
    }

    pub async fn set_resolved(&self, thread_id: &NodeId, resolved: bool) -> Result<(), ApiError> {
        let vars = queries::ThreadIdVariables {
            thread_id: thread_id.gql(),
        };
        if resolved {
            self.mutate(queries::Resolve::build(vars)).await.map(drop)
        } else {
            self.mutate(queries::Unresolve::build(vars)).await.map(drop)
        }
    }

    // ---- browsing ----------------------------------------------------------

    /// A cached page, by one of the keys in [`crate::browse::keys`].
    pub fn cached<T: DeserializeOwned>(&self, key: &str) -> Option<Cached<T>> {
        self.store.query_get(key)
    }

    /// A repository's overview: stats, root directory, last commit, README.
    pub async fn repo(&self, repo: &RepoId) -> Result<browse::RepoOverview, ApiError> {
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
        Ok(self.kept(&browse::keys::repo(repo), overview).await)
    }

    /// The README GitHub shows for the repository root.
    pub async fn readme(&self, repo: &RepoId) -> Result<Option<browse::Readme>, ApiError> {
        use base64::Engine;
        #[derive(serde::Deserialize)]
        struct Wire {
            path: String,
            content: String,
        }
        let wire: Wire = match self
            .rest_json(&format!("/repos/{}/{}/readme", repo.owner, repo.name))
            .await
        {
            Ok(wire) => wire,
            Err(ApiError::NotFound(_)) => return Ok(None),
            Err(err) => return Err(err),
        };
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
        match self.object(repo, rev, path).await? {
            Some(browse::GitObject::Tree(t)) => {
                let key = browse::keys::tree(repo, rev, path);
                Ok(self.kept(&key, browse::entries(t)).await)
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
        match self.object(repo, rev, path).await? {
            Some(browse::GitObject::Blob(b)) => Ok(browse::Blob {
                path: path.to_owned(),
                text: b.text.filter(|_| b.is_binary != Some(true)),
                size: crate::model::count(b.byte_size),
                truncated: b.is_truncated,
            }),
            _ => Err(ApiError::NotFound(format!("{repo}/{path}"))),
        }
    }

    async fn object(
        &self,
        repo: &RepoId,
        rev: &str,
        path: &str,
    ) -> Result<Option<browse::GitObject>, ApiError> {
        let op = browse::ObjectQuery::build(browse::RepoVariables {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
            expression: format!("{rev}:{path}"),
        });
        Ok(self.graphql(op).await?.repository.and_then(|r| r.object))
    }

    /// One page of search results (30 per page).
    pub async fn search(
        &self,
        kind: browse::SearchKind,
        query: &str,
        after: Option<String>,
    ) -> Result<browse::SearchResults, ApiError> {
        use browse::{BrowseItem, Results, SearchKind as Kind, SearchResults, SearchType};
        let first_page = after.is_none();
        // GitHub's issue search covers both; the tabs separate them.
        let (search_type, is) = match kind {
            Kind::Repos => (SearchType::Repository, ""),
            Kind::Issues => (SearchType::Issue, " is:issue"),
            Kind::Pulls => (SearchType::Issue, " is:pr"),
            Kind::Users => (SearchType::User, ""),
        };
        let api_query = if query.contains("is:issue") || query.contains("is:pr") {
            query.to_owned()
        } else {
            format!("{query}{is}")
        };
        let op = browse::BrowseSearch::build(browse::BrowseSearchVariables {
            query: api_query,
            kind: search_type,
            first: 30,
            after,
        });
        let conn = self.graphql(op).await?.search;
        let next = conn.page_info.next();
        let items = nodes(conn.nodes);
        let count = crate::model::count;
        let results = match kind {
            Kind::Repos => SearchResults::Repos(Results {
                total: count(conn.repository_count),
                items: items
                    .filter_map(|i| match i {
                        BrowseItem::Repository(r) => r.into_summary(),
                        _ => None,
                    })
                    .collect(),
                next,
            }),
            Kind::Issues | Kind::Pulls => SearchResults::Issues(Results {
                total: count(conn.issue_count),
                items: items.filter_map(BrowseItem::into_issue).collect(),
                next,
            }),
            Kind::Users => SearchResults::Users(Results {
                total: count(conn.user_count),
                items: items.filter_map(BrowseItem::into_user).collect(),
                next,
            }),
        };
        if first_page {
            self.remember(&browse::keys::search(kind, query), results.clone())
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
        let op = browse::IssueQuery::build(queries::NumberVariables {
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
        Ok(Some(
            self.kept(&browse::keys::issue(repo, number), issue).await,
        ))
    }

    /// Comments, reviews and commits of a pull request.
    pub async fn pr_activity(&self, pr: &PrRef) -> Result<browse::PrActivity, ApiError> {
        let op = browse::PrActivityQuery::build(number_vars(pr)?);
        let activity = self
            .graphql(op)
            .await?
            .repository
            .and_then(|r| r.pull_request)
            .map(browse::WirePrActivity::into_activity)
            .ok_or_else(|| ApiError::NotFound(pr.to_string()))?;
        Ok(self.kept(&browse::keys::pr_activity(pr), activity).await)
    }

    /// A user's or organization's profile.
    pub async fn profile(&self, login: &str) -> Result<browse::Profile, ApiError> {
        let op = browse::ProfileQuery::build(browse::ProfileVariables {
            login: login.to_owned(),
        });
        let profile = self
            .graphql(op)
            .await?
            .into_profile()
            .ok_or_else(|| ApiError::NotFound(login.to_owned()))?;
        Ok(self.kept(&browse::keys::profile(login), profile).await)
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
        let wire: Wire = self
            .rest_json(&format!(
                "/repos/{}/{}/git/trees/{}?recursive=1",
                repo.owner,
                repo.name,
                encode_path(rev)
            ))
            .await?;
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
        let names = names.get(..100).unwrap_or(names);
        let fields: String = names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let path = if dir.is_empty() { name.clone() } else { format!("{dir}/{name}") };
                format!(
                    "e{i}: history(first: 1, path: \"{}\") {{ nodes {{ oid messageHeadline committedDate author {{ name user {{ login }} }} }} }}\n",
                    quote(&path)
                )
            })
            .collect();
        let query = format!(
            "query($owner: String!, $name: String!, $rev: String!) {{ repository(owner: $owner, name: $name) {{ object(expression: $rev) {{ ... on Commit {{ {fields} }} }} }} }}"
        );
        let data = self
            .graphql_json(
                &query,
                serde_json::json!({ "owner": repo.owner, "name": repo.name, "rev": rev }),
            )
            .await?;
        let mut out = std::collections::HashMap::new();
        for (i, name) in names.iter().enumerate() {
            let field = |path: &str| {
                data.pointer(&format!("/repository/object/e{i}/nodes/0/{path}"))
                    .and_then(serde_json::Value::as_str)
            };
            let (Some(oid), Some(headline), Some(date)) = (
                field("oid"),
                field("messageHeadline"),
                field("committedDate"),
            ) else {
                continue;
            };
            let author = field("author/user/login")
                .or_else(|| field("author/name"))
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
        Ok(self
            .kept(&browse::keys::last_commits(repo, rev, dir), out)
            .await)
    }

    /// A commit, by a revision GitHub can resolve (a short or full SHA).
    pub async fn commit(&self, repo: &RepoId, rev: &str) -> Result<browse::CommitDetail, ApiError> {
        let op = browse::CommitQuery::build(browse::RepoVariables {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
            expression: rev.to_owned(),
        });
        let commit = match self.graphql(op).await?.repository.and_then(|r| r.object) {
            Some(browse::CommitObject::Commit(c)) => c.into_detail(),
            _ => return Err(ApiError::NotFound(format!("{repo}@{rev}"))),
        };
        Ok(self.kept(&browse::keys::commit(repo, rev), commit).await)
    }

    /// A revision's commits, newest first, touching `path` if it isn't
    /// empty; 30 at a time, from `after`. The first page is cached.
    pub async fn history(
        &self,
        repo: &RepoId,
        rev: &str,
        path: &str,
        after: Option<String>,
    ) -> Result<browse::Results<browse::CommitInfo>, ApiError> {
        let first = after.is_none();
        let op = browse::HistoryQuery::build(browse::HistoryVariables {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
            expression: rev.to_owned(),
            path: (!path.is_empty()).then(|| path.to_owned()),
            after,
        });
        let history = match self.graphql(op).await?.repository.and_then(|r| r.object) {
            Some(browse::HistoryObject::Commit(c)) => c.history.into_results(),
            _ => return Err(ApiError::NotFound(format!("{repo}@{rev}"))),
        };
        if first {
            return Ok(self
                .kept(&browse::keys::history(repo, rev, path), history)
                .await);
        }
        Ok(history)
    }

    /// The checks on a pull request's head commit.
    pub async fn pr_checks(&self, pr: &PrRef) -> Result<browse::Checks, ApiError> {
        let op = browse::PrChecksQuery::build(number_vars(pr)?);
        let head = self
            .graphql(op)
            .await?
            .repository
            .and_then(|r| r.pull_request)
            .and_then(|p| nodes(p.commits.nodes).next())
            .ok_or_else(|| ApiError::NotFound(pr.to_string()))?;
        let checks = head.commit.into_checks();
        Ok(self.kept(&browse::keys::pr_checks(pr), checks).await)
    }

    /// The checks on a repository's default branch.
    pub async fn branch_checks(&self, repo: &RepoId) -> Result<browse::Checks, ApiError> {
        let op = browse::BranchChecksQuery::build(browse::BranchesVariables {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
        });
        let target = self
            .graphql(op)
            .await?
            .repository
            .and_then(|r| r.default_branch_ref)
            .and_then(|r| r.target);
        let Some(browse::ChecksTarget::Commit(commit)) = target else {
            return Err(ApiError::NotFound(repo.to_string()));
        };
        let checks = commit.into_checks();
        Ok(self.kept(&browse::keys::branch_checks(repo), checks).await)
    }

    /// A page of people, 30 at a time from `after`; the first is cached.
    pub async fn users(
        &self,
        list: &browse::UserList,
        after: Option<String>,
    ) -> Result<browse::Results<browse::UserSummary>, ApiError> {
        use browse::UserList as L;
        let first = after.is_none();
        let (root, field, vars) = match list {
            L::Stargazers(repo) | L::Watchers(repo) => (
                "repository(owner: $owner, name: $name)",
                if matches!(list, L::Stargazers(_)) {
                    "stargazers"
                } else {
                    "watchers"
                },
                serde_json::json!({ "owner": repo.owner, "name": repo.name, "after": after }),
            ),
            L::Followers(login) | L::Following(login) => (
                "user(login: $login)",
                if matches!(list, L::Followers(_)) {
                    "followers"
                } else {
                    "following"
                },
                serde_json::json!({ "login": login, "after": after }),
            ),
            L::People(login) => (
                "organization(login: $login)",
                "membersWithRole",
                serde_json::json!({ "login": login, "after": after }),
            ),
        };
        let params = if root.starts_with("repository") {
            "$owner: String!, $name: String!"
        } else {
            "$login: String!"
        };
        let query = format!(
            "query({params}, $after: String) {{ node: {root} {{ list: {field}(first: 30, after: $after) {{ totalCount pageInfo {{ hasNextPage endCursor }} nodes {{ login name bio }} }} }} }}"
        );
        let data = self.graphql_json(&query, vars).await?;
        let list_json = data
            .pointer("/node/list")
            .ok_or_else(|| ApiError::NotFound(format!("{list:?}")))?;
        let text = |v: &serde_json::Value, key: &str| {
            v.get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        let items = list_json
            .get("nodes")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|u| {
                Some(browse::UserSummary {
                    login: text(u, "login")?,
                    name: text(u, "name"),
                    bio: text(u, "bio"),
                    is_org: false,
                })
            })
            .collect();
        let page_info = list_json.get("pageInfo");
        let more = page_info
            .and_then(|p| p.get("hasNextPage"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let results = browse::Results {
            total: list_json
                .get("totalCount")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            items,
            next: page_info
                .and_then(|p| text(p, "endCursor"))
                .filter(|_| more),
        };
        if first {
            return Ok(self.kept(&browse::keys::users(list), results).await);
        }
        Ok(results)
    }

    /// A user's or organization's own repositories in `sort` order, 30 at a
    /// time from `after`; the first page is cached.
    pub async fn owner_repos(
        &self,
        login: &str,
        sort: browse::RepoSort,
        after: Option<String>,
    ) -> Result<browse::Results<browse::RepoSummary>, ApiError> {
        let first = after.is_none();
        let op = browse::OwnerReposQuery::build(browse::OwnerReposVariables {
            login: login.to_owned(),
            after,
            order: sort.into(),
        });
        let repos = self
            .graphql(op)
            .await?
            .repository_owner
            .ok_or_else(|| ApiError::NotFound(login.to_owned()))?
            .repositories
            .into_results();
        if first {
            return Ok(self
                .kept(&browse::keys::owner_repos(login, sort), repos)
                .await);
        }
        Ok(repos)
    }

    /// What a user starred, most recently first, 30 at a time from
    /// `after`; the first page is cached.
    pub async fn starred(
        &self,
        login: &str,
        after: Option<String>,
    ) -> Result<browse::Results<browse::RepoSummary>, ApiError> {
        let first = after.is_none();
        let op = browse::StarredQuery::build(browse::LoginPageVariables {
            login: login.to_owned(),
            after,
        });
        let stars = self
            .graphql(op)
            .await?
            .user
            .ok_or_else(|| ApiError::NotFound(login.to_owned()))?
            .starred_repositories
            .into_results();
        if first {
            return Ok(self.kept(&browse::keys::starred(login), stars).await);
        }
        Ok(stars)
    }

    /// A repository's forks, most starred first, 30 at a time from
    /// `after`; the first page is cached.
    pub async fn forks(
        &self,
        repo: &RepoId,
        after: Option<String>,
    ) -> Result<browse::Results<browse::RepoSummary>, ApiError> {
        let first = after.is_none();
        let op = browse::ForksQuery::build(list_vars(repo, after));
        let forks = self
            .graphql(op)
            .await?
            .repository
            .ok_or_else(|| ApiError::NotFound(repo.to_string()))?
            .forks
            .into_results();
        if first {
            return Ok(self.kept(&browse::keys::forks(repo), forks).await);
        }
        Ok(forks)
    }

    /// A repository's releases, newest first, 20 at a time from `after`;
    /// the first page is cached.
    pub async fn releases(
        &self,
        repo: &RepoId,
        after: Option<String>,
    ) -> Result<browse::Results<browse::Release>, ApiError> {
        let first = after.is_none();
        let op = browse::ReleasesQuery::build(list_vars(repo, after));
        let releases = self
            .graphql(op)
            .await?
            .repository
            .ok_or_else(|| ApiError::NotFound(repo.to_string()))?
            .releases
            .into_results();
        if first {
            return Ok(self.kept(&browse::keys::releases(repo), releases).await);
        }
        Ok(releases)
    }

    /// A release by its tag, with its notes and assets.
    pub async fn release(&self, repo: &RepoId, tag: &str) -> Result<browse::Release, ApiError> {
        let op = browse::ReleaseQuery::build(browse::ReleaseVariables {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
            tag: tag.to_owned(),
        });
        let release = self
            .graphql(op)
            .await?
            .repository
            .and_then(|r| r.release)
            .ok_or_else(|| ApiError::NotFound(format!("{repo} release {tag}")))?
            .into_release();
        Ok(self.kept(&browse::keys::release(repo, tag), release).await)
    }

    /// A repository's tags, 30 at a time from `after`; the first page is
    /// cached.
    pub async fn tags(
        &self,
        repo: &RepoId,
        after: Option<String>,
    ) -> Result<browse::Results<browse::TagInfo>, ApiError> {
        let first = after.is_none();
        let op = browse::TagsQuery::build(list_vars(repo, after));
        let tags = self
            .graphql(op)
            .await?
            .repository
            .and_then(|r| r.refs)
            .ok_or_else(|| ApiError::NotFound(repo.to_string()))?
            .into_results();
        if first {
            return Ok(self.kept(&browse::keys::tags(repo), tags).await);
        }
        Ok(tags)
    }

    /// Branches and tags, most recently committed first.
    pub async fn refs(&self, repo: &RepoId) -> Result<browse::Refs, ApiError> {
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
        Ok(self.kept(&browse::keys::refs(repo), refs).await)
    }

    /// Saves a small value in the cache (e.g. recently visited pages).
    pub async fn remember<T: Serialize + Send + 'static>(&self, key: &str, value: T) {
        let store = self.store.clone();
        let key = key.to_owned();
        spawn_store(move || store.query_put(&key, &value)).await;
    }

    /// [`Self::remember`]s `value` and hands it back.
    async fn kept<T: Serialize + Clone + Send + 'static>(&self, key: &str, value: T) -> T {
        self.remember(key, value.clone()).await;
        value
    }

    /// Repositories you own or contribute to, most recently pushed first.
    pub async fn viewer_repos(&self) -> Result<Vec<browse::RepoSummary>, ApiError> {
        let data = self.graphql(browse::ViewerReposQuery::build(())).await?;
        let (repos, _) = browse::repo_list(data.viewer.repositories);
        Ok(self.kept(browse::keys::VIEWER_REPOS, repos).await)
    }

    /// Comments on an issue or pull request (by node ID).
    pub async fn add_comment(&self, subject_id: &NodeId, body: &str) -> Result<(), ApiError> {
        let op = browse::AddComment::build(browse::AddCommentVariables {
            subject: subject_id.gql(),
            body: body.to_owned(),
        });
        self.mutate(op).await.map(drop)
    }

    pub async fn set_starred(&self, repo_id: &NodeId, starred: bool) -> Result<(), ApiError> {
        let vars = browse::StarVariables {
            starrable: repo_id.gql(),
        };
        if starred {
            self.mutate(browse::AddStar::build(vars)).await.map(drop)
        } else {
            self.mutate(browse::RemoveStar::build(vars)).await.map(drop)
        }
    }
}

/// Percent-encodes a ref for a URL path (branch names may contain `/`).
fn encode_path(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(b).to_string()
            }
            b => format!("%{b:02X}"),
        })
        .collect()
}

/// The HTTP client: the token on every request, rustls with the platform's
/// roots (loading those is why this runs on a blocking thread), GitHub's
/// JSON media type, and no retries of its own.
fn build_client(token: &str) -> Result<reqwest::Client, ApiError> {
    // rustls needs a process-wide crypto provider; a second install is a no-op.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let setup = |e: &dyn std::fmt::Display| ApiError::Setup(e.to_string());
    let mut auth = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|e| setup(&e))?;
    auth.set_sensitive(true);
    let headers = HeaderMap::from_iter([
        (header::AUTHORIZATION, auth),
        (
            header::ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        ),
        (
            http::HeaderName::from_static("x-github-api-version"),
            HeaderValue::from_static("2022-11-28"),
        ),
    ]);
    reqwest::Client::builder()
        .user_agent(concat!("ghtui/", env!("CARGO_PKG_VERSION")))
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| setup(&e))
}

const INBOX_KEY: &str = "inbox";

fn pr_number(pr: &PrRef) -> Result<i32, ApiError> {
    i32::try_from(pr.number).map_err(|_| ApiError::NotFound(pr.to_string()))
}

fn list_vars(repo: &RepoId, after: Option<String>) -> browse::ListVariables {
    browse::ListVariables {
        owner: repo.owner.clone(),
        name: repo.name.clone(),
        after,
    }
}

fn number_vars(pr: &PrRef) -> Result<queries::NumberVariables, ApiError> {
    Ok(queries::NumberVariables {
        owner: pr.repo.owner.clone(),
        name: pr.repo.name.clone(),
        number: pr_number(pr)?,
    })
}

fn page_vars(pr: &PrRef, after: Option<String>) -> Result<queries::PageVariables, ApiError> {
    Ok(queries::PageVariables {
        owner: pr.repo.owner.clone(),
        name: pr.repo.name.clone(),
        number: pr_number(pr)?,
        after,
    })
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
    #[derive(serde::Deserialize)]
    struct Message {
        message: String,
    }
    let status = response.status;
    if status.is_success() {
        return Ok(());
    }
    if status == StatusCode::UNAUTHORIZED {
        return Err(ApiError::Unauthorized);
    }
    let message = serde_json::from_str::<Message>(&response.body)
        .map_or_else(|_| response.body.chars().take(200).collect(), |m| m.message);
    if status == StatusCode::NOT_FOUND {
        return Err(ApiError::NotFound(message));
    }
    Err(ApiError::Http {
        status: status.as_u16(),
        message,
    })
}
