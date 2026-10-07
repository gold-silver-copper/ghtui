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

/// A team's fields, as [`browse::wire_teams::Team`] reads them.
const TEAM: &str =
    "slug name description privacy members { totalCount } repositories { totalCount }";

/// A milestone's fields, as [`browse::wire_milestones::Milestone`] reads them.
const MILESTONE: &str = "number title description dueOn closed closedAt updatedAt openIssues: issues(states: OPEN) { totalCount } doneIssues: issues(states: CLOSED) { totalCount } openPrs: pullRequests(states: OPEN) { totalCount } donePrs: pullRequests(states: [CLOSED, MERGED]) { totalCount }";
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
        use browse::{SearchKind as Kind, SearchType};
        let first_page = after.is_none();
        // GitHub's issue search covers both; the tabs separate them.
        let (search_type, is) = match kind {
            Kind::Repos => (SearchType::Repository, ""),
            Kind::Issues => (SearchType::Issue, " is:issue"),
            Kind::Pulls => (SearchType::Issue, " is:pr"),
            Kind::Users => (SearchType::User, ""),
            Kind::Discussions | Kind::Commits | Kind::Code => {
                let results = self.search_other(kind, query, after).await?;
                if first_page {
                    self.remember(&browse::keys::search(kind, query), results.clone())
                        .await;
                }
                return Ok(results);
            }
        };
        let api_query = if query.contains("is:issue") || query.contains("is:pr") {
            query.to_owned()
        } else {
            format!("{query}{is}")
        };
        let results = self.search_as(kind, search_type, api_query, after).await?;
        if first_page {
            self.remember(&browse::keys::search(kind, query), results.clone())
                .await;
        }
        Ok(results)
    }

    /// A page of a discussion search (GraphQL), or of a commit or code
    /// search (REST only), 30 at a time; REST pages by number.
    async fn search_other(
        &self,
        kind: browse::SearchKind,
        query: &str,
        after: Option<String>,
    ) -> Result<browse::SearchResults, ApiError> {
        use browse::{Results, SearchKind as Kind, SearchResults};
        if kind == Kind::Discussions {
            let data = self
                .graphql_json(
                    "query($q: String!, $after: String) { search(type: DISCUSSION, query: $q, first: 30, after: $after) { discussionCount pageInfo { hasNextPage endCursor } nodes { ... on Discussion { repository { nameWithOwner } number title author { login } category { name } comments { totalCount } isAnswered upvoteCount updatedAt } } } }",
                    serde_json::json!({ "q": query, "after": after }),
                )
                .await?;
            let search = data.get("search").cloned().unwrap_or_default();
            let repos: Vec<Option<RepoId>> = search
                .pointer("/nodes")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter(|n| n.get("number").is_some())
                .map(|n| {
                    n.pointer("/repository/nameWithOwner")?
                        .as_str()
                        .and_then(RepoId::parse)
                })
                .collect();
            let mut wire = search;
            if let Some(obj) = wire.as_object_mut() {
                let count = obj.remove("discussionCount").unwrap_or_default();
                obj.insert("totalCount".into(), count);
                if let Some(serde_json::Value::Array(nodes)) = obj.get_mut("nodes") {
                    nodes.retain(|n| n.get("number").is_some());
                }
            }
            let page: browse::wire_discussions::Summaries = serde_json::from_value(wire)?;
            let page = page.into_results();
            return Ok(SearchResults::Discussions(Results {
                total: page.total,
                items: page
                    .items
                    .into_iter()
                    .zip(repos)
                    .filter_map(|(summary, repo)| {
                        Some(browse::DiscussionHit {
                            repo: repo?,
                            summary,
                        })
                    })
                    .collect(),
                next: page.next,
            }));
        }
        // GitHub refuses an empty commit or code search (422).
        if query.trim().is_empty() {
            let empty = Results {
                total: 0,
                items: Vec::new(),
                next: None,
            };
            return Ok(if kind == Kind::Commits {
                SearchResults::Commits(empty)
            } else {
                SearchResults::Code(Results {
                    total: 0,
                    items: Vec::new(),
                    next: None,
                })
            });
        }
        let number: u64 = after.as_deref().and_then(|a| a.parse().ok()).unwrap_or(1);
        let what = if kind == Kind::Commits {
            "commits"
        } else {
            "code"
        };
        let path = format!(
            "/search/{what}?q={}&per_page=30&page={number}",
            encode_query(query)
        );
        // GitHub serves the first 1000 results.
        let next = |total: u64| (number * 30 < total.min(1000)).then(|| (number + 1).to_string());
        if kind == Kind::Commits {
            let page: browse::rest_search::Page<browse::rest_search::Commit> =
                self.rest_json(&path).await?;
            return Ok(SearchResults::Commits(Results {
                next: next(page.total_count),
                total: page.total_count,
                items: page
                    .items
                    .into_iter()
                    .filter_map(browse::rest_search::Commit::into_hit)
                    .collect(),
            }));
        }
        let page: browse::rest_search::Page<browse::rest_search::Code> =
            self.rest_json(&path).await?;
        Ok(SearchResults::Code(Results {
            next: next(page.total_count),
            total: page.total_count,
            items: page
                .items
                .into_iter()
                .filter_map(browse::rest_search::Code::into_hit)
                .collect(),
        }))
    }

    /// A page of a search, as GitHub's API takes it.
    async fn search_as(
        &self,
        kind: browse::SearchKind,
        search_type: browse::SearchType,
        query: String,
        after: Option<String>,
    ) -> Result<browse::SearchResults, ApiError> {
        use browse::{BrowseItem, Results, SearchKind as Kind, SearchResults};
        let op = browse::BrowseSearch::build(browse::BrowseSearchVariables {
            query,
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
            Kind::Users | Kind::Discussions | Kind::Commits | Kind::Code => {
                SearchResults::Users(Results {
                    total: count(conn.user_count),
                    items: items.filter_map(BrowseItem::into_user).collect(),
                    next,
                })
            }
        };
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

    /// Branches by name, 30 at a time from `after`, each with its latest
    /// commit and pull request. (GitHub's commit-date order doesn't sort
    /// branches by date.)
    pub async fn branches(
        &self,
        repo: &RepoId,
        after: Option<String>,
    ) -> Result<browse::Results<browse::BranchInfo>, ApiError> {
        let first = after.is_none();
        let data = self
            .graphql_json(
                "query($owner: String!, $name: String!, $after: String) { repository(owner: $owner, name: $name) { defaultBranchRef { name } refs(refPrefix: \"refs/heads/\", first: 30, after: $after, orderBy: {field: ALPHABETICAL, direction: ASC}) { totalCount pageInfo { hasNextPage endCursor } nodes { name target { ... on Commit { oid messageHeadline committedDate author { name user { login } } } } associatedPullRequests(first: 1, orderBy: {field: CREATED_AT, direction: DESC}) { nodes { number state } } } } } }",
                serde_json::json!({ "owner": repo.owner, "name": repo.name, "after": after }),
            )
            .await?;
        let default = data
            .pointer("/repository/defaultBranchRef/name")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let wire: browse::wire_branches::Branches = serde_json::from_value(
            data.pointer("/repository/refs")
                .filter(|r| !r.is_null())
                .cloned()
                .ok_or_else(|| ApiError::NotFound(repo.to_string()))?,
        )?;
        let branches = wire.into_results(default.as_deref());
        if first {
            return Ok(self.kept(&browse::keys::branches(repo), branches).await);
        }
        Ok(branches)
    }

    /// Open or closed milestones, soonest due first, 25 at a time from
    /// `after`, with how many there are of each.
    pub async fn milestones(
        &self,
        repo: &RepoId,
        closed: bool,
        after: Option<String>,
    ) -> Result<browse::MilestoneList, ApiError> {
        let first = after.is_none();
        let state = if closed { "CLOSED" } else { "OPEN" };
        let query = format!(
            "query($owner: String!, $name: String!, $after: String) {{ repository(owner: $owner, name: $name) {{ open: milestones(states: OPEN) {{ totalCount }} closed: milestones(states: CLOSED) {{ totalCount }} milestones(first: 25, after: $after, states: {state}, orderBy: {{field: DUE_DATE, direction: {}}}) {{ totalCount pageInfo {{ hasNextPage endCursor }} nodes {{ {MILESTONE} }} }} }} }}",
            if closed { "DESC" } else { "ASC" }
        );
        let data = self
            .graphql_json(
                &query,
                serde_json::json!({ "owner": repo.owner, "name": repo.name, "after": after }),
            )
            .await?;
        let count = |which: &str| {
            data.pointer(&format!("/repository/{which}/totalCount"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default()
        };
        let (open, closed_count) = (count("open"), count("closed"));
        let wire: browse::wire_milestones::Milestones = serde_json::from_value(
            data.pointer("/repository/milestones")
                .filter(|m| !m.is_null())
                .cloned()
                .ok_or_else(|| ApiError::NotFound(repo.to_string()))?,
        )?;
        let list = browse::MilestoneList {
            open,
            closed: closed_count,
            results: wire.into_results(),
        };
        if first {
            return Ok(self
                .kept(&browse::keys::milestones(repo, closed), list)
                .await);
        }
        Ok(list)
    }

    /// A milestone, and 30 of its issues and pull requests from `after`,
    /// most recently updated first.
    pub async fn milestone(
        &self,
        repo: &RepoId,
        number: u64,
        after: Option<String>,
    ) -> Result<browse::MilestoneDetail, ApiError> {
        let first = after.is_none();
        let query = format!(
            "query($owner: String!, $name: String!, $number: Int!) {{ repository(owner: $owner, name: $name) {{ milestone(number: $number) {{ {MILESTONE} }} }} }}"
        );
        let data = self
            .graphql_json(
                &query,
                serde_json::json!({ "owner": repo.owner, "name": repo.name, "number": number }),
            )
            .await?;
        let wire: browse::wire_milestones::Milestone = serde_json::from_value(
            data.pointer("/repository/milestone")
                .filter(|m| !m.is_null())
                .cloned()
                .ok_or_else(|| ApiError::NotFound(format!("{repo} milestone {number}")))?,
        )?;
        let info = wire.into_info();
        // GitHub's search can't match a title with quotes in it.
        let unsearchable = info.title.contains('"');
        let items = if unsearchable {
            browse::Results {
                total: 0,
                items: Vec::new(),
                next: None,
            }
        } else {
            let search = format!("repo:{repo} milestone:\"{}\" sort:updated-desc", info.title);
            match self
                .search_as(
                    browse::SearchKind::Issues,
                    browse::SearchType::Issue,
                    search,
                    after,
                )
                .await?
            {
                browse::SearchResults::Issues(items) => items,
                _ => browse::Results {
                    total: 0,
                    items: Vec::new(),
                    next: None,
                },
            }
        };
        let detail = browse::MilestoneDetail {
            info,
            items,
            unsearchable,
        };
        if first {
            return Ok(self
                .kept(&browse::keys::milestone(repo, number), detail)
                .await);
        }
        Ok(detail)
    }
    /// Deployments, newest first, 25 at a time from `after`, to one
    /// environment if given; with the environments.
    pub async fn deployments(
        &self,
        repo: &RepoId,
        environment: Option<&str>,
        after: Option<String>,
    ) -> Result<browse::DeploymentList, ApiError> {
        let first = after.is_none();
        let environments = environment.map(|e| vec![e]);
        let data = self
            .graphql_json(
                "query($owner: String!, $name: String!, $after: String, $envs: [String!]) { repository(owner: $owner, name: $name) { environments(first: 50) { nodes { name } } deployments(first: 25, after: $after, environments: $envs, orderBy: {field: CREATED_AT, direction: DESC}) { totalCount pageInfo { hasNextPage endCursor } nodes { environment state createdAt creator { login } ref { name } commitOid latestStatus { logUrl environmentUrl } } } } }",
                serde_json::json!({ "owner": repo.owner, "name": repo.name, "after": after, "envs": environments }),
            )
            .await?;
        let wire: browse::wire_deployments::Deployments = serde_json::from_value(
            data.pointer("/repository/deployments")
                .filter(|d| !d.is_null())
                .cloned()
                .ok_or_else(|| ApiError::NotFound(repo.to_string()))?,
        )?;
        let list = browse::DeploymentList {
            environments: data
                .pointer("/repository/environments/nodes")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|n| Some(n.get("name")?.as_str()?.to_owned()))
                .collect(),
            results: wire.into_results(),
        };
        if first {
            return Ok(self
                .kept(&browse::keys::deployments(repo, environment), list)
                .await);
        }
        Ok(list)
    }
    /// Compares two revisions, as a compare URL names them (`a...b`,
    /// `a..b`, `owner:branch`, `owner:repo:branch`): the newest 250 commits
    /// (GitHub's default, ending at the head; asking for a page size pages
    /// from the oldest instead) and up to 300 files.
    pub async fn compare(&self, repo: &RepoId, spec: &str) -> Result<browse::Comparison, ApiError> {
        let (base, head, direct) = match spec.split_once("...") {
            Some((base, head)) => (base, head, false),
            None => match spec.split_once("..") {
                Some((base, head)) => (base, head, true),
                // One revision: against the default branch, as on GitHub.
                None => ("HEAD", spec, false),
            },
        };
        let path = format!(
            "/repos/{}/{}/compare/{}...{}",
            repo.owner,
            repo.name,
            encode_path(base),
            encode_path(head)
        );
        let wire: browse::rest_compare::Compare = self.rest_json(&path).await?;
        let comparison = wire.into_comparison(direct);
        Ok(self
            .kept(&browse::keys::compare(repo, spec), comparison)
            .await)
    }
    /// Which commit last changed each line of a file at a revision.
    pub async fn blame(
        &self,
        repo: &RepoId,
        rev: &str,
        path: &str,
    ) -> Result<browse::Blame, ApiError> {
        let data = self
            .graphql_json(
                "query($owner: String!, $name: String!, $rev: String!, $path: String!) { repository(owner: $owner, name: $name) { object(expression: $rev) { ... on Commit { blame(path: $path) { ranges { startingLine endingLine age commit { oid messageHeadline committedDate author { name user { login } } } } } } } } }",
                serde_json::json!({ "owner": repo.owner, "name": repo.name, "rev": rev, "path": path }),
            )
            .await?;
        let wire: browse::wire_blame::Blame = serde_json::from_value(
            data.pointer("/repository/object/blame")
                .filter(|b| !b.is_null())
                .cloned()
                .ok_or_else(|| ApiError::NotFound(format!("{repo}:{rev}:{path}")))?,
        )?;
        let blame = wire.into_blame();
        Ok(self
            .kept(&browse::keys::blame(repo, rev, path), blame)
            .await)
    }
    /// A gist and its files. REST, since GraphQL finds gists only through
    /// their owners, and a gist's URL may not name its owner.
    pub async fn gist(&self, id: &str) -> Result<browse::Gist, ApiError> {
        let wire: browse::rest_gists::Gist = self
            .rest_json(&format!("/gists/{}", encode_path(id)))
            .await?;
        let gist = wire.into_gist();
        Ok(self.kept(&browse::keys::gist(id), gist).await)
    }

    /// Someone's public gists, most recently updated first, 30 at a time
    /// from `after`.
    pub async fn gists(
        &self,
        login: &str,
        after: Option<String>,
    ) -> Result<browse::Results<browse::GistSummary>, ApiError> {
        let first = after.is_none();
        let data = self
            .graphql_json(
                "query($login: String!, $after: String) { user(login: $login) { gists(first: 30, after: $after, privacy: PUBLIC, orderBy: {field: UPDATED_AT, direction: DESC}) { totalCount pageInfo { hasNextPage endCursor } nodes { name description updatedAt stargazerCount files(limit: 5) { name } comments { totalCount } } } } }",
                serde_json::json!({ "login": login, "after": after }),
            )
            .await?;
        let wire: browse::wire_gists::Gists = serde_json::from_value(
            data.pointer("/user/gists")
                .filter(|g| !g.is_null())
                .cloned()
                .ok_or_else(|| ApiError::NotFound(format!("{login}'s gists")))?,
        )?;
        let gists = wire.into_results();
        if first {
            return Ok(self.kept(&browse::keys::gists(login), gists).await);
        }
        Ok(gists)
    }
    /// The teams in an organization you can see (its members see them),
    /// by name, 30 at a time from `after`.
    pub async fn teams(
        &self,
        org: &str,
        after: Option<String>,
    ) -> Result<browse::Results<browse::TeamSummary>, ApiError> {
        let first = after.is_none();
        let data = self
            .graphql_json(
                &format!("query($org: String!, $after: String) {{ organization(login: $org) {{ teams(first: 30, after: $after, orderBy: {{field: NAME, direction: ASC}}) {{ totalCount pageInfo {{ hasNextPage endCursor }} nodes {{ {TEAM} }} }} }} }}"),
                serde_json::json!({ "org": org, "after": after }),
            )
            .await?;
        let wire: browse::wire_teams::Teams = serde_json::from_value(
            data.pointer("/organization/teams")
                .filter(|t| !t.is_null())
                .cloned()
                .ok_or_else(|| ApiError::NotFound(org.to_owned()))?,
        )?;
        let teams = wire.into_results();
        if first {
            return Ok(self.kept(&browse::keys::teams(org), teams).await);
        }
        Ok(teams)
    }

    /// A team: its members, repositories and child teams.
    pub async fn team(&self, org: &str, slug: &str) -> Result<browse::TeamDetail, ApiError> {
        let data = self
            .graphql_json(
                &format!("query($org: String!, $slug: String!) {{ organization(login: $org) {{ team(slug: $slug) {{ {TEAM} parentTeam {{ {TEAM} }} memberList: members(first: 50) {{ nodes {{ login name }} }} repoList: repositories(first: 50) {{ nodes {{ nameWithOwner description stargazerCount }} }} childTeams(first: 50) {{ nodes {{ {TEAM} }} }} }} }} }}"),
                serde_json::json!({ "org": org, "slug": slug }),
            )
            .await?;
        let wire: browse::wire_teams::Detail = serde_json::from_value(
            data.pointer("/organization/team")
                .filter(|t| !t.is_null())
                .cloned()
                .ok_or_else(|| ApiError::NotFound(format!("{org}/{slug}")))?,
        )?;
        let team = wire.into_detail();
        Ok(self.kept(&browse::keys::team(org, slug), team).await)
    }
    /// A repository's published security advisories, or GitHub's newest
    /// reviewed ones, up to 100. REST: repositories' advisories aren't in
    /// GraphQL, and one shape serves both.
    pub async fn advisories(
        &self,
        repo: Option<&RepoId>,
    ) -> Result<Vec<browse::Advisory>, ApiError> {
        let path = match repo {
            Some(r) => format!(
                "/repos/{}/{}/security-advisories?state=published&per_page=100",
                r.owner, r.name
            ),
            None => "/advisories?type=reviewed&per_page=50".to_owned(),
        };
        let wire: Vec<browse::rest_advisories::Advisory> = self.rest_json(&path).await?;
        let list: Vec<browse::Advisory> = wire
            .into_iter()
            .map(browse::rest_advisories::Advisory::into_advisory)
            .collect();
        Ok(self.kept(&browse::keys::advisories(repo), list).await)
    }

    /// A security advisory: a repository's, or from GitHub's database.
    pub async fn advisory(
        &self,
        repo: Option<&RepoId>,
        ghsa: &str,
    ) -> Result<browse::Advisory, ApiError> {
        let id = encode_path(ghsa);
        let path = match repo {
            Some(r) => format!("/repos/{}/{}/security-advisories/{id}", r.owner, r.name),
            None => format!("/advisories/{id}"),
        };
        let wire: browse::rest_advisories::Advisory = self.rest_json(&path).await?;
        let advisory = wire.into_advisory();
        Ok(self
            .kept(&browse::keys::advisory(repo, ghsa), advisory)
            .await)
    }
    /// A workflow run (one attempt of it, or the latest) and its jobs.
    pub async fn workflow_run(
        &self,
        repo: &RepoId,
        run: u64,
        attempt: Option<u64>,
    ) -> Result<browse::WorkflowRun, ApiError> {
        let base = format!("/repos/{}/{}/actions/runs/{run}", repo.owner, repo.name);
        let path = match attempt {
            Some(n) => format!("{base}/attempts/{n}"),
            None => base,
        };
        let jobs_path = format!("{path}/jobs?per_page=100");
        let (wire, jobs) = tokio::join!(
            self.rest_json::<browse::rest_actions::Run>(&path),
            self.rest_json::<browse::rest_actions::Jobs>(&jobs_path)
        );
        let browse::rest_actions::Jobs {
            mut jobs,
            total_count,
        } = jobs?;
        // Big matrices have more than a page of jobs (up to 1,000 here).
        let mut page = 2;
        while (jobs.len() as u64) < total_count && page <= 10 {
            let more: browse::rest_actions::Jobs =
                self.rest_json(&format!("{jobs_path}&page={page}")).await?;
            if more.jobs.is_empty() {
                break;
            }
            jobs.extend(more.jobs);
            page += 1;
        }
        let run = wire?.into_run(jobs);
        Ok(self
            .kept(&browse::keys::run(repo, run.id, attempt), run)
            .await)
    }

    /// A job and its steps.
    pub async fn job(&self, repo: &RepoId, job: u64) -> Result<browse::Job, ApiError> {
        let path = format!("/repos/{}/{}/actions/jobs/{job}", repo.owner, repo.name);
        let job_wire: browse::rest_actions::Job = self.rest_json(&path).await?;
        let job = job_wire.into_job();
        Ok(self.kept(&browse::keys::job(repo, job.id), job).await)
    }

    /// A job's log, as plain text: its last 2 MB (where failures are), and
    /// never cached (logs are big and don't change once a job is done).
    pub async fn job_log(&self, repo: &RepoId, job: u64) -> Result<String, ApiError> {
        const KEEP: usize = 2 << 20;
        let path = format!(
            "/repos/{}/{}/actions/jobs/{job}/logs",
            repo.owner, repo.name
        );
        let response = self
            .send(&Request::Get {
                path: &path,
                etag: None,
            })
            .await?;
        check_status(&response)?;
        let log = response.body;
        let start = log.len().saturating_sub(KEEP);
        let start = (start..log.len())
            .find(|&i| log.is_char_boundary(i))
            .unwrap_or(0);
        Ok(log.get(start..).unwrap_or_default().to_owned())
    }

    /// A workflow, by its file name (`ci.yml`) or ID.
    pub async fn workflow(&self, repo: &RepoId, file: &str) -> Result<browse::Workflow, ApiError> {
        let path = format!(
            "/repos/{}/{}/actions/workflows/{}",
            repo.owner,
            repo.name,
            encode_path(file)
        );
        let wire: browse::rest_actions::Workflow = self.rest_json(&path).await?;
        let workflow = browse::Workflow {
            name: wire.name,
            path: wire.path,
            state: wire.state,
        };
        Ok(self
            .kept(&browse::keys::workflow(repo, file), workflow)
            .await)
    }

    /// A workflow's runs, newest first, 30 at a time; `after` is the page
    /// number to fetch (REST pages by number). The first page is cached.
    pub async fn workflow_runs(
        &self,
        repo: &RepoId,
        file: &str,
        after: Option<String>,
    ) -> Result<browse::Results<browse::RunSummary>, ApiError> {
        let page: u64 = after.as_deref().and_then(|a| a.parse().ok()).unwrap_or(1);
        let path = format!(
            "/repos/{}/{}/actions/workflows/{}/runs?per_page=30&page={page}",
            repo.owner,
            repo.name,
            encode_path(file)
        );
        let wire: browse::rest_actions::Runs = self.rest_json(&path).await?;
        let shown = page * 30;
        let runs = browse::Results {
            total: wire.total_count,
            items: wire
                .workflow_runs
                .into_iter()
                .map(browse::rest_actions::Run::into_summary)
                .collect(),
            next: (shown < wire.total_count).then(|| (page + 1).to_string()),
        };
        if page == 1 {
            return Ok(self
                .kept(&browse::keys::workflow_runs(repo, file), runs)
                .await);
        }
        Ok(runs)
    }

    /// The checks on a commit, by a revision GitHub can resolve.
    pub async fn commit_checks(
        &self,
        repo: &RepoId,
        rev: &str,
    ) -> Result<browse::Checks, ApiError> {
        let op = browse::CommitChecksQuery::build(browse::RepoVariables {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
            expression: rev.to_owned(),
        });
        let object = self.graphql(op).await?.repository.and_then(|r| r.object);
        let Some(browse::ChecksTarget::Commit(commit)) = object else {
            return Err(ApiError::NotFound(format!("{repo}@{rev}")));
        };
        let checks = commit.into_checks();
        Ok(self
            .kept(&browse::keys::commit_checks(repo, rev), checks)
            .await)
    }

    /// The repository a discussion list or discussion is in: an
    /// organization's are in one of its repositories, which a search for
    /// them names.
    async fn discussions_repo(&self, of: &browse::DiscussionsOf) -> Result<RepoId, ApiError> {
        let org = match of {
            browse::DiscussionsOf::Repo(repo) => return Ok(repo.clone()),
            browse::DiscussionsOf::Org(org) => org,
        };
        let prefix = format!(
            "https://github.com/orgs/{}/discussions/",
            org.to_lowercase()
        );
        // The organization's discussions are among its repositories', in
        // best-match order: look through up to five pages.
        let mut after: Option<String> = None;
        for _ in 0..5 {
            let data = self
                .graphql_json(
                    "query($q: String!, $after: String) { search(type: DISCUSSION, query: $q, first: 50, after: $after) { pageInfo { hasNextPage endCursor } nodes { ... on Discussion { url repository { nameWithOwner } } } } }",
                    serde_json::json!({ "q": format!("org:{org}"), "after": after }),
                )
                .await?;
            let found = data
                .pointer("/search/nodes")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .find(|n| {
                    n.get("url")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|u| u.to_lowercase().starts_with(&prefix))
                })
                .and_then(|n| n.pointer("/repository/nameWithOwner")?.as_str())
                .and_then(RepoId::parse);
            if let Some(repo) = found {
                return Ok(repo);
            }
            let more = data.pointer("/search/pageInfo/hasNextPage")
                == Some(&serde_json::Value::Bool(true));
            after = data
                .pointer("/search/pageInfo/endCursor")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            if !more || after.is_none() {
                break;
            }
        }
        Err(ApiError::NotFound(format!("{org}'s discussions")))
    }

    /// Discussions, most recently updated first, 25 at a time from
    /// `after`, in a category (by its slug) if given; with the categories.
    pub async fn discussions(
        &self,
        of: &browse::DiscussionsOf,
        category: Option<&str>,
        after: Option<String>,
    ) -> Result<browse::DiscussionList, ApiError> {
        use browse::wire_discussions as w;
        let first = after.is_none();
        let repo = self.discussions_repo(of).await?;
        let vars = serde_json::json!({ "owner": repo.owner, "name": repo.name });
        let data = self
            .graphql_json(
                "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { discussionCategories(first: 50) { nodes { id name slug } } } }",
                vars,
            )
            .await?;
        let categories: w::Categories = serde_json::from_value(
            data.pointer("/repository/discussionCategories")
                .cloned()
                .unwrap_or_default(),
        )?;
        let categories: Vec<w::Category> = categories.nodes.into_iter().flatten().collect();
        let category_id = match category {
            Some(slug) => match categories.iter().find(|c| c.slug == slug) {
                Some(c) => Some(c.id.clone()),
                None => return Err(ApiError::NotFound(format!("{repo}'s category {slug}"))),
            },
            None => None,
        };
        let data = self
            .graphql_json(
                "query($owner: String!, $name: String!, $after: String, $category: ID) { repository(owner: $owner, name: $name) { discussions(first: 25, after: $after, categoryId: $category, orderBy: {field: UPDATED_AT, direction: DESC}) { totalCount pageInfo { hasNextPage endCursor } nodes { number title author { login } category { name } comments { totalCount } isAnswered upvoteCount updatedAt } } } }",
                serde_json::json!({ "owner": repo.owner, "name": repo.name, "after": after, "category": category_id }),
            )
            .await?;
        let page: w::Summaries = serde_json::from_value(
            data.pointer("/repository/discussions")
                .cloned()
                .unwrap_or_default(),
        )?;
        let list = browse::DiscussionList {
            categories: categories
                .into_iter()
                .map(|c| browse::DiscussionCategory {
                    name: c.name,
                    slug: c.slug,
                })
                .collect(),
            results: page.into_results(),
        };
        if first {
            return Ok(self
                .kept(&browse::keys::discussions(of, category), list)
                .await);
        }
        Ok(list)
    }

    /// A discussion, its comments and their replies.
    pub async fn discussion(
        &self,
        of: &browse::DiscussionsOf,
        number: u64,
    ) -> Result<browse::DiscussionDetail, ApiError> {
        let repo = self.discussions_repo(of).await?;
        let data = self
            .graphql_json(
                "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { discussion(number: $number) { number title body author { login } createdAt category { name } isAnswered upvoteCount comments(first: 50) { totalCount nodes { databaseId author { login } body createdAt isAnswer upvoteCount replies(first: 30) { totalCount nodes { databaseId author { login } body createdAt } } } } } } }",
                serde_json::json!({ "owner": repo.owner, "name": repo.name, "number": number }),
            )
            .await?;
        let wire: browse::wire_discussions::Detail = serde_json::from_value(
            data.pointer("/repository/discussion")
                .filter(|d| !d.is_null())
                .cloned()
                .ok_or_else(|| ApiError::NotFound(format!("{repo} discussion {number}")))?,
        )?;
        let detail = wire.into_detail(repo);
        Ok(self
            .kept(&browse::keys::discussion(of, number), detail)
            .await)
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

/// Percent-encodes a query string's value.
fn encode_query(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(b).to_string()
            }
            b' ' => "+".to_owned(),
            b => format!("%{b:02X}"),
        })
        .collect()
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
