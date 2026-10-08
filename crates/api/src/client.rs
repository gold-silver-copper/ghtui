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
use serde_json::Value;

use crate::auth::Token;
use crate::browse;
use crate::model::{
    Capped, Inbox, NewThread, NodeId, PatchFile, PrDetail, PrRef, PrSummary, RepoId, ReviewEvent,
    ReviewThread, ViewedFiles,
};
use crate::queries::{self, nodes};
use crate::rate_limit::{RateLimits, retry_after};
use crate::raw;

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
    /// It was there, and GitHub has since dropped it.
    #[error("{0}")]
    Gone(String),
    /// Reading through git (a wiki) failed.
    #[error("git: {0}")]
    Git(String),
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
    /// What partial GraphQL results left out (GitHub's errors beside the
    /// data), not yet reported.
    left_out: Arc<Mutex<Vec<String>>>,
    /// Where GitHub's answers didn't add up, not yet reported.
    doubts: Arc<Mutex<Vec<String>>>,
    /// Where responses are recorded, for the corpus (never by the app).
    recorder: Option<Arc<crate::corpus::Recorder>>,
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
            left_out: Arc::default(),
            doubts: Arc::default(),
            recorder: None,
        }
    }

    /// Records every response into `dir`, for the corpus. The token isn't
    /// recorded: only the request's method, path and body, and the
    /// response's status, `Link` header and body.
    #[must_use]
    pub fn recording(mut self, dir: std::path::PathBuf) -> Self {
        self.recorder = Some(Arc::new(crate::corpus::Recorder(dir)));
        self
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

    /// What GitHub left out of partial results since this was last asked:
    /// its errors for data the token can't see (an organization's SSO,
    /// a fine-grained token's permissions), which come back as nulls.
    pub fn take_left_out(&self) -> Vec<String> {
        std::mem::take(
            &mut *self
                .left_out
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// Where GitHub's answers didn't add up since this was last asked: a
    /// comparison whose last commit isn't its head, more items than the
    /// total, … (see [`Self::doubt`]).
    pub fn take_doubts(&self) -> Vec<String> {
        std::mem::take(
            &mut *self
                .doubts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// Notes that an answer doesn't add up: logged, and reported. What's
    /// shown is still GitHub's answer; this says not to trust it.
    fn doubt(&self, what: String) {
        tracing::warn!(what, "GitHub's answer doesn't add up");
        let mut doubts = self
            .doubts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !doubts.contains(&what) {
            doubts.push(what);
        }
    }

    /// Checks a page of a list adds up: no more items than GitHub counts.
    fn check_page<T: browse::Page>(&self, list: &T, what: impl std::fmt::Display) {
        let (items, total) = list.counts();
        if items as u64 > total {
            self.doubt(format!("{what}: a page of {items}, of {total} in all"));
        }
    }

    /// A list from its `first` page and those `fetch` gets after it, `max`
    /// pages at most. It stops at a page with no next, at a cursor seen
    /// before (doubted), or at `max`; a cut the total doesn't count is noted
    /// as left out, and counted.
    async fn more_pages<T, F: Future<Output = Result<browse::Results<T>, ApiError>>>(
        &self,
        what: impl std::fmt::Display,
        first: browse::Results<T>,
        max: u32,
        mut fetch: impl FnMut(Option<String>) -> F,
    ) -> Result<Capped<T>, ApiError> {
        let mut list = first;
        let mut seen = std::collections::HashSet::new();
        for _ in 1..max {
            let Some(after) = list.next.take() else {
                break;
            };
            if !seen.insert(after.clone()) {
                self.doubt(format!("{what}: GitHub gave the cursor {after} twice"));
                break;
            }
            let page = fetch(Some(after)).await?;
            list.total = list.total.max(page.total);
            list.items.extend(page.items);
            list.next = page.next;
        }
        let fetched = list.items.len() as u64;
        if list.next.is_some() && list.total <= fetched {
            self.leave_out(vec![format!("{what}: only the first {fetched}")]);
            list.total = fetched + 1;
        }
        Ok(Capped::new(list.items, list.total))
    }

    /// Checks a GraphQL list's page adds up: a short page is the last
    /// (nulls count: they're items the token can't see).
    fn check_connection<T>(
        &self,
        page: &browse::wire::Connection<T>,
        size: usize,
        what: impl std::fmt::Display,
    ) {
        let n = page.nodes.len();
        if page.page_info.has_next_page && n < size {
            self.doubt(format!(
                "{what}: a page of {n} (of {size}) says there are more"
            ));
        }
        if n as u64 > page.total_count {
            self.doubt(format!(
                "{what}: a page of {n}, of {} in all",
                page.total_count
            ));
        }
    }

    /// Notes a REST search that timed out, so found only some results.
    fn note_incomplete(&self, incomplete: bool) {
        if incomplete {
            self.leave_out(vec![
                "the search took too long and found only some results".to_owned(),
            ]);
        }
    }

    /// Notes a partial result's errors to report.
    fn leave_out(&self, errors: Vec<String>) {
        if errors.is_empty() {
            return;
        }
        tracing::warn!(?errors, "partial GraphQL result");
        let mut left_out = self
            .left_out
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for error in errors {
            if !left_out.contains(&error) {
                left_out.push(error);
            }
        }
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
            if let Some(recorder) = &self.recorder {
                let (request, sent) = match request {
                    Request::Get { path, .. } => (format!("GET {path}"), None),
                    Request::Post { path, body, .. } => (format!("POST {path}"), Some(*body)),
                };
                let link = headers
                    .get(header::LINK)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned);
                let recorded = crate::corpus::Recorded {
                    request,
                    body: sent.cloned(),
                    status: status.as_u16(),
                    link,
                    response: body.clone(),
                };
                recorder.record(&recorded, sent);
            }
            return Ok(Response {
                status,
                headers,
                body,
            });
        }
    }

    /// Runs a query whose root is never null, taking its data whole.
    /// Partial results are accepted (and their errors kept to report, see
    /// [`Self::take_left_out`]); a response without data is an error.
    async fn graphql<Q: DeserializeOwned + cynic::QueryFragment + Unrooted, V: Serialize>(
        &self,
        op: cynic::Operation<Q, V>,
    ) -> Result<Q, ApiError> {
        self.find(op, "", Some).await
    }

    /// Runs a query for the one thing `pick`ed from its data. Nothing picked
    /// is `subject` not found when GitHub's only errors (if any) said what
    /// wasn't there, and GitHub's other errors otherwise.
    async fn find<Q: DeserializeOwned + cynic::QueryFragment, V: Serialize, T>(
        &self,
        op: cynic::Operation<Q, V>,
        subject: impl std::fmt::Display,
        pick: impl FnOnce(Q) -> Option<T>,
    ) -> Result<T, ApiError> {
        let reply = self.ask(&serde_json::to_value(&op)?, true).await?;
        reply.picked(subject, pick, |errors| self.leave_out(errors))
    }

    /// Runs a GraphQL mutation. Any error fails it, with GitHub's messages:
    /// a rejected mutation comes back as data with a null field plus errors.
    pub(crate) async fn mutate<Q: DeserializeOwned, V: Serialize>(
        &self,
        op: cynic::Operation<Q, V>,
    ) -> Result<Q, ApiError> {
        self.ask(&serde_json::to_value(&op)?, false)
            .await?
            .mutation()
    }

    /// Runs a query given as text (built at run time, or read as JSON) for
    /// the value at `at`, a JSON pointer into its data; a null there is
    /// missing, which [`Reply::found`] explains. Point at the deepest value
    /// you need. Public for the contract tests.
    pub async fn graphql_json<T: DeserializeOwned>(
        &self,
        query: &str,
        variables: Value,
        subject: impl std::fmt::Display,
        at: &str,
    ) -> Result<T, ApiError> {
        let body = serde_json::json!({ "query": query, "variables": variables });
        let reply = self.ask(&body, true).await?;
        reply.at(subject, at, |errors| self.leave_out(errors))
    }

    /// Posts a GraphQL request: the one place that reads GitHub's errors.
    /// A query's NOT_FOUND that explains a null in the data is dropped (the
    /// null says it); a mutation (not `idempotent`) keeps every error. A
    /// response without data is an error.
    async fn ask(&self, body: &Value, idempotent: bool) -> Result<Reply, ApiError> {
        #[derive(serde::Deserialize)]
        struct Wire {
            #[serde(default)]
            data: Value,
            errors: Option<Vec<GqlError>>,
        }
        let response = self
            .send(&Request::Post {
                path: "/graphql",
                body,
                idempotent,
            })
            .await?;
        check_status(&response, "/graphql")?;
        let wire: Wire = serde_json::from_str(&response.body)?;
        let errors = wire.errors.unwrap_or_default();
        if wire.data.is_null() {
            return Err(match messages(errors) {
                errors if errors.is_empty() => ApiError::Decode("no data".into()),
                errors => ApiError::GraphQl(errors),
            });
        }
        let errors = errors
            .into_iter()
            .filter(|e| !(idempotent && e.explains_null(&wire.data)));
        let errors = messages(errors);
        Ok(Reply::new(wire.data, errors))
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
        check_status(&response, path)?;
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

    /// POSTs to a REST path with an empty body, for an action (re-run,
    /// cancel) that only has to succeed. Never sent twice.
    pub(crate) async fn rest_post(&self, path: &str) -> Result<(), ApiError> {
        let body = serde_json::json!({});
        let response = self
            .send(&Request::Post {
                path,
                body: &body,
                idempotent: false,
            })
            .await?;
        check_status(&response, path)
    }

    /// GETs a REST path and decodes its JSON.
    async fn rest_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        Ok(serde_json::from_str(&self.rest_get(path).await?)?)
    }

    /// A page of a list GitHub pages by cursor (`after=`, in the `Link`
    /// header), and the cursor for the next page, if there is one.
    async fn rest_cursor_page<T: DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<(T, Option<String>), ApiError> {
        let response = self.send(&Request::Get { path, etag: None }).await?;
        check_status(&response, path)?;
        let next = response
            .headers
            .get(header::LINK)
            .and_then(|v| v.to_str().ok())
            .and_then(next_after);
        Ok((serde_json::from_str(&response.body)?, next))
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
            authored,
            review_requested: requested,
        };
        Ok(self.kept(INBOX_KEY, inbox).await)
    }

    async fn search_prs(&self, query: &str) -> Result<Capped<PrSummary>, ApiError> {
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
        let detail = self
            .find(op, pr, |q| PrDetail::from_wire(q.repository?))
            .await?;
        Ok(self.kept(&pr_key(pr), detail).await)
    }

    /// Viewed state of every file in the PR (paginated, 100 per page).
    pub async fn viewed_files(&self, pr: &PrRef) -> Result<ViewedFiles, ApiError> {
        let page = move |after| async move {
            let op = queries::PrFilesQuery::build(page_vars(pr, after)?);
            let files = self.find(op, pr, |q| q.repository?.pull_request).await?;
            let states = files.files.map(|page| {
                let states = nodes(page.nodes).map(|f| (f.path, f.viewer_viewed_state));
                browse::Results::uncounted(states.collect(), page.page_info.next())
            });
            Ok::<_, ApiError>((files.id.into(), states.unwrap_or_default()))
        };
        let (pull_request_id, first) = page(None).await?;
        let states = self.more_pages(
            format!("{pr}'s files"),
            first,
            FILE_PAGES,
            |after| async move { Ok(page(after).await?.1) },
        );
        Ok(ViewedFiles {
            pull_request_id,
            states: states.await?.items.into_iter().collect(),
        })
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
        let page = move |after| async move {
            let op = queries::ThreadsQuery::build(page_vars(pr, after)?);
            let page = self
                .find(op, pr, |q| q.repository?.pull_request)
                .await?
                .review_threads;
            let threads = nodes(page.nodes).map(ReviewThread::from_wire).collect();
            Ok(browse::Results::uncounted(threads, page.page_info.next()))
        };
        let what = format!("{pr}'s review threads");
        let threads = self.more_pages(what, page(None).await?, FILE_PAGES, page);
        Ok(threads.await?.items)
    }

    /// GitHub's per-file patches, for commentable ranges. Paginated (100 per
    /// page) and capped by GitHub at 3000 files.
    pub async fn pr_patches(&self, pr: &PrRef) -> Result<Vec<PatchFile>, ApiError> {
        let page = move |after: Option<String>| async move {
            let page = rest_page(after.as_deref());
            let path = format!(
                "/repos/{}/{}/pulls/{}/files?per_page=100&page={page}",
                pr.repo.owner, pr.repo.name, pr.number
            );
            let batch: Vec<PatchFile> = self.rest_json(&path).await?;
            let next = (batch.len() == 100).then(|| page.saturating_add(1).to_string());
            Ok(browse::Results::uncounted(batch, next))
        };
        let what = format!("{pr}'s patches");
        let files = self.more_pages(what, page(None).await?, FILE_PAGES, page);
        Ok(files.await?.items)
    }

    /// The PR's node ID and the viewer's pending review on it, if any.
    /// The PR's node ID, and your pending review on it, if any, with the
    /// commit that review is on.
    pub async fn pending_review(
        &self,
        pr: &PrRef,
    ) -> Result<(NodeId, Option<(NodeId, Option<String>)>), ApiError> {
        let op = queries::PendingReviewQuery::build(number_vars(pr)?);
        let pr_node = self.find(op, pr, |q| q.repository?.pull_request).await?;
        let review = nodes(pr_node.reviews.and_then(|r| r.nodes))
            .next()
            .map(|r| (NodeId::from(r.id), r.commit.map(|c| c.oid.0)));
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
            .find(op, pr, |q| q.repository?.pull_request)
            .await?
            .reviews
            .and_then(|r| r.nodes)
            .unwrap_or_default();
        Ok(reviews
            .into_iter()
            .flatten()
            .rev()
            .find(|r| r.state != crate::model::ReviewState::Pending)
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

    /// A repository's overview: stats, root directory, last commit.
    pub async fn repo(&self, repo: &RepoId) -> Result<browse::RepoOverview, ApiError> {
        let op = browse::RepoQuery::build(browse::RepoVariables {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
            expression: "HEAD:".into(),
        });
        let overview = self
            .find(op, repo, |q| q.repository?.into_overview())
            .await?;
        Ok(self.kept(&browse::keys::repo(repo), overview).await)
    }

    /// The README GitHub shows for the repository root, if it has one.
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
            Err(ApiError::NotFound(_)) => {
                return Ok(self.kept(&browse::keys::readme(repo), None).await);
            }
            Err(err) => return Err(err),
        };
        let compact: String = wire.content.split_whitespace().collect();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(compact)
            .map_err(|e| ApiError::Decode(e.to_string()))?;
        let readme = browse::Readme {
            path: wire.path,
            text: String::from_utf8_lossy(&bytes).into_owned(),
        };
        Ok(self.kept(&browse::keys::readme(repo), Some(readme)).await)
    }

    /// A directory at `rev:path`.
    pub async fn tree(
        &self,
        repo: &RepoId,
        rev: &str,
        path: &str,
    ) -> Result<Vec<browse::TreeEntry>, ApiError> {
        match self.object(repo, rev, path).await? {
            browse::GitObject::Tree(t) => {
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
            browse::GitObject::Blob(b) => Ok(browse::Blob {
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
    ) -> Result<browse::GitObject, ApiError> {
        let op = browse::ObjectQuery::build(browse::RepoVariables {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
            expression: format!("{rev}:{path}"),
        });
        self.find(op, format!("{repo}/{path}"), |q| q.repository?.object)
            .await
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
            let vars = serde_json::json!({ "q": query, "after": after });
            let mut search: Value =
                (self.graphql_json(raw::DISCUSSION_SEARCH, vars, query, "/search")).await?;
            // A search's nodes may be other kinds, which come as `{}`.
            if let Some(serde_json::Value::Array(nodes)) = search.get_mut("nodes") {
                nodes.retain(|n| n.get("number").is_some());
            }
            let page: browse::wire::Connection<browse::wire_discussions::Hit> =
                serde_json::from_value(search)?;
            return Ok(SearchResults::Discussions(
                page.filter_results(browse::wire_discussions::Hit::into_hit),
            ));
        }
        // GitHub refuses an empty commit or code search (422).
        if query.trim().is_empty() {
            let empty = Results::default();
            return Ok(if kind == Kind::Commits {
                SearchResults::Commits(empty)
            } else {
                SearchResults::Code(Results::default())
            });
        }
        let number = rest_page(after.as_deref());
        let what = if kind == Kind::Commits {
            "commits"
        } else {
            "code"
        };
        let path = format!(
            "/search/{what}?q={}&per_page={REST_PAGE}&page={number}",
            encode_query(query)
        );
        let next = |total: u64| next_page(number, REST_PAGE, total.min(SEARCH_CAP));
        if kind == Kind::Commits {
            let page: browse::rest_search::Page<browse::rest_search::Commit> =
                self.rest_json(&path).await?;
            self.note_incomplete(page.incomplete_results);
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
        self.note_incomplete(page.incomplete_results);
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
        let pick = |q: browse::IssueQuery| match q.repository?.issue_or_pull_request? {
            browse::IssueOrPr::Issue(issue) => issue.into_detail().map(Some),
            browse::IssueOrPr::PullRequest(_) => Some(None),
            browse::IssueOrPr::Other => None,
        };
        let Some(issue) = self.find(op, format!("{repo}#{number}"), pick).await? else {
            return Ok(None);
        };
        Ok(Some(
            self.kept(&browse::keys::issue(repo, number), issue).await,
        ))
    }

    /// Comments, reviews and commits of a pull request.
    pub async fn pr_activity(&self, pr: &PrRef) -> Result<browse::PrActivity, ApiError> {
        let op = browse::PrActivityQuery::build(number_vars(pr)?);
        let activity = self
            .find(op, pr, |q| {
                Some(q.repository?.pull_request?.into_activity())
            })
            .await?;
        Ok(self.kept(&browse::keys::pr_activity(pr), activity).await)
    }

    /// A user's or organization's profile.
    pub async fn profile(&self, login: &str) -> Result<browse::Profile, ApiError> {
        let op = browse::ProfileQuery::build(browse::ProfileVariables {
            login: login.to_owned(),
        });
        let profile = self
            .find(op, login, browse::ProfileQuery::into_profile)
            .await?;
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
        let names = names.get(..browse::LAST_COMMIT_ENTRIES).unwrap_or(names);
        let mut out = std::collections::HashMap::new();
        // 100 `history` fields a query.
        for names in names.chunks(100) {
            self.last_commits_of(repo, rev, dir, names, &mut out)
                .await?;
        }
        Ok(self
            .kept(&browse::keys::last_commits(repo, rev, dir), out)
            .await)
    }

    async fn last_commits_of(
        &self,
        repo: &RepoId,
        rev: &str,
        dir: &str,
        names: &[String],
        out: &mut std::collections::HashMap<String, browse::CommitInfo>,
    ) -> Result<(), ApiError> {
        let paths: Vec<String> = names
            .iter()
            .map(|name| {
                if dir.is_empty() {
                    name.clone()
                } else {
                    format!("{dir}/{name}")
                }
            })
            .collect();
        let query = raw::last_commits(&paths);
        let vars = serde_json::json!({ "owner": repo.owner, "name": repo.name, "rev": rev });
        let commit: Value =
            (self.graphql_json(&query, vars, format!("{repo}@{rev}"), "/repository/object"))
                .await?;
        for (i, name) in names.iter().enumerate() {
            let node = commit.pointer(&format!("/e{i}/nodes/0"));
            let Some(node) = node.filter(|n| !n.is_null()) else {
                continue;
            };
            match <browse::wire::Commit as serde::Deserialize>::deserialize(node) {
                Ok(commit) => {
                    out.insert(name.clone(), commit.into_info());
                }
                Err(err) => tracing::warn!(%err, %name, "a last commit didn't decode"),
            }
        }
        Ok(())
    }

    /// A commit, by a revision GitHub can resolve (a short or full SHA).
    pub async fn commit(&self, repo: &RepoId, rev: &str) -> Result<browse::CommitDetail, ApiError> {
        let op = browse::CommitQuery::build(browse::RepoVariables {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
            expression: rev.to_owned(),
        });
        let pick = |q: browse::CommitQuery| match q.repository?.object? {
            browse::CommitObject::Commit(c) => Some(c.into_detail()),
            browse::CommitObject::Other => None,
        };
        let commit = self.find(op, format!("{repo}@{rev}"), pick).await?;
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
        let pick = |q: browse::HistoryQuery| match q.repository?.object? {
            browse::HistoryObject::Commit(c) => Some(c.history.into_results()),
            browse::HistoryObject::Other => None,
        };
        let history = self.find(op, format!("{repo}@{rev}"), pick).await?;
        Ok(self
            .kept_page(first, &browse::keys::history(repo, rev, path), history)
            .await)
    }

    /// The checks on a pull request's head commit.
    pub async fn pr_checks(&self, pr: &PrRef) -> Result<browse::Checks, ApiError> {
        let op = browse::PrChecksQuery::build(number_vars(pr)?);
        let pr_checks = self.find(op, pr, |q| q.repository?.pull_request).await?;
        let want = pr_checks.head_ref_oid.0;
        let head = nodes(pr_checks.commits.nodes)
            .next()
            .ok_or_else(|| ApiError::NotFound(pr.to_string()))?;
        let checks = self.all_checks(&pr.repo, head.commit).await?;
        if checks.oid != want {
            self.doubt(format!(
                "{pr}'s checks are for {}, but its head is {}",
                text_short(&checks.oid),
                text_short(&want)
            ));
        }
        Ok(self.kept(&browse::keys::pr_checks(pr), checks).await)
    }

    /// A commit's checks with every page of them (up to [`PAGES`]): a
    /// check run again can have its newest run on a later page.
    async fn all_checks(
        &self,
        repo: &RepoId,
        commit: browse::ChecksCommit,
    ) -> Result<browse::Checks, ApiError> {
        use cynic::QueryBuilder as _;
        let oid = commit.oid.0;
        let first = commit
            .status_check_rollup
            .map(|r| r.contexts.into_results())
            .unwrap_or_default();
        let what = format!("{}'s checks", text_short(&oid));
        let expression = &oid;
        let page = move |after| async move {
            let op = browse::ContextsQuery::build(browse::ContextsVariables {
                owner: repo.owner.clone(),
                name: repo.name.clone(),
                expression: expression.clone(),
                after,
            });
            let pick = |q: browse::ContextsQuery| match q.repository?.object? {
                browse::ContextsTarget::Commit(c) => Some(c.status_check_rollup),
                browse::ContextsTarget::Other => None,
            };
            let rollup = self.find(op, repo, pick).await?;
            Ok(rollup
                .map(|r| r.contexts.into_results())
                .unwrap_or_default())
        };
        let contexts = self.more_pages(what, first, PAGES, page).await?;
        Ok(browse::checks(oid, contexts))
    }

    /// The checks on a repository's default branch.
    pub async fn branch_checks(&self, repo: &RepoId) -> Result<browse::Checks, ApiError> {
        let op = browse::BranchChecksQuery::build(browse::BranchesVariables {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
        });
        let pick = |q: browse::BranchChecksQuery| match q.repository?.default_branch_ref?.target? {
            browse::ChecksTarget::Commit(commit) => Some(commit),
            browse::ChecksTarget::Other => None,
        };
        let commit = self.find(op, repo, pick).await?;
        let checks = self.all_checks(repo, commit).await?;
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
        let (root, field, vars, who) = match list {
            L::Stargazers(repo) | L::Watchers(repo) => (
                "repository(owner: $owner, name: $name)",
                if matches!(list, L::Stargazers(_)) {
                    "stargazers"
                } else {
                    "watchers"
                },
                serde_json::json!({ "owner": repo.owner, "name": repo.name, "after": after }),
                repo.to_string(),
            ),
            L::Followers(login) | L::Following(login) => (
                "user(login: $login)",
                if matches!(list, L::Followers(_)) {
                    "followers"
                } else {
                    "following"
                },
                serde_json::json!({ "login": login, "after": after }),
                login.clone(),
            ),
            L::People(login) => (
                "organization(login: $login)",
                "membersWithRole",
                serde_json::json!({ "login": login, "after": after }),
                login.clone(),
            ),
        };
        let query = raw::users(root, field);
        let wire: browse::wire::Connection<browse::wire::Person> =
            self.graphql_json(&query, vars, &who, "/node/list").await?;
        let people = field.trim_end_matches("WithRole");
        self.check_connection(&wire, 30, format!("{who}'s {people}"));
        let results = wire.into_results(browse::wire::Person::into_summary);
        Ok(self
            .kept_page(first, &browse::keys::users(list), results)
            .await)
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
            .find(op, login, |q| q.repository_owner)
            .await?
            .repositories
            .into_results();
        Ok(self
            .kept_page(first, &browse::keys::owner_repos(login, sort), repos)
            .await)
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
            .find(op, login, |q| q.user)
            .await?
            .starred_repositories
            .into_results();
        Ok(self
            .kept_page(first, &browse::keys::starred(login), stars)
            .await)
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
            .find(op, repo, |q| q.repository)
            .await?
            .forks
            .into_results();
        Ok(self
            .kept_page(first, &browse::keys::forks(repo), forks)
            .await)
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
            .find(op, repo, |q| q.repository)
            .await?
            .releases
            .into_results();
        Ok(self
            .kept_page(first, &browse::keys::releases(repo), releases)
            .await)
    }

    /// A release by its tag, with its notes and assets.
    pub async fn release(&self, repo: &RepoId, tag: &str) -> Result<browse::Release, ApiError> {
        let op = browse::ReleaseQuery::build(browse::ReleaseVariables {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
            tag: tag.to_owned(),
        });
        let release = self
            .find(op, format!("{repo} release {tag}"), |q| {
                q.repository?.release
            })
            .await?
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
            .find(op, repo, |q| q.repository?.refs)
            .await?
            .into_results();
        Ok(self.kept_page(first, &browse::keys::tags(repo), tags).await)
    }

    /// Branches by name, 30 at a time from `after`, each with its latest
    /// commit and pull request. (GitHub's commit-date order doesn't sort
    /// branches by date.)
    pub async fn branches(
        &self,
        repo: &RepoId,
        after: Option<String>,
    ) -> Result<browse::Results<browse::BranchInfo>, ApiError> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire {
            default_branch_ref: Option<browse::wire::Name>,
            refs: browse::wire::Connection<browse::wire_branches::Branch>,
        }
        let first = after.is_none();
        let vars = serde_json::json!({ "owner": repo.owner, "name": repo.name, "after": after });
        let wire: Wire = self
            .graphql_json(raw::BRANCHES, vars, repo, "/repository")
            .await?;
        self.check_connection(&wire.refs, 30, format!("{repo}'s branches"));
        let default = wire.default_branch_ref.map(|r| r.name);
        let branches = (wire.refs).into_results(|b| b.into_info(repo, default.as_deref()));
        Ok(self
            .kept_page(first, &browse::keys::branches(repo), branches)
            .await)
    }

    /// Open or closed milestones, soonest due first, 25 at a time from
    /// `after`, with how many there are of each.
    pub async fn milestones(
        &self,
        repo: &RepoId,
        closed: bool,
        after: Option<String>,
    ) -> Result<browse::MilestoneList, ApiError> {
        use browse::wire::{Connection, Count};
        #[derive(serde::Deserialize)]
        struct Wire {
            open: Count,
            closed: Count,
            milestones: Connection<browse::wire_milestones::Milestone>,
        }
        let first = after.is_none();
        let query = raw::milestones(closed);
        let vars = serde_json::json!({ "owner": repo.owner, "name": repo.name, "after": after });
        let wire: Wire = self.graphql_json(&query, vars, repo, "/repository").await?;
        self.check_connection(&wire.milestones, 25, format!("{repo}'s milestones"));
        let list = browse::MilestoneList {
            open: wire.open.total_count,
            closed: wire.closed.total_count,
            results: (wire.milestones).into_results(browse::wire_milestones::Milestone::into_info),
        };
        Ok(self
            .kept_page(first, &browse::keys::milestones(repo, closed), list)
            .await)
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
        let query = raw::milestone();
        let vars = serde_json::json!({ "owner": repo.owner, "name": repo.name, "number": number });
        let subject = format!("{repo} milestone {number}");
        let wire: browse::wire_milestones::Milestone =
            (self.graphql_json(&query, vars, subject, "/repository/milestone")).await?;
        let info = wire.into_info();
        // GitHub's search can't match a title with quotes in it.
        let unsearchable = info.title.contains('"');
        let items = if unsearchable {
            browse::Results::default()
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
                _ => browse::Results::default(),
            }
        };
        let detail = browse::MilestoneDetail {
            info,
            items,
            unsearchable,
        };
        Ok(self
            .kept_page(first, &browse::keys::milestone(repo, number), detail)
            .await)
    }
    /// Deployments, newest first, 25 at a time from `after`, to one
    /// environment if given; with the environments.
    pub async fn deployments(
        &self,
        repo: &RepoId,
        environment: Option<&str>,
        after: Option<String>,
    ) -> Result<browse::DeploymentList, ApiError> {
        use browse::wire::{Connection, Counted, Name};
        #[derive(serde::Deserialize)]
        struct Wire {
            deployments: Connection<browse::wire_deployments::Deployment>,
            environments: Counted<Name>,
        }
        let first = after.is_none();
        let environments = environment.map(|e| vec![e]);
        let vars = serde_json::json!({ "owner": repo.owner, "name": repo.name, "after": after, "envs": environments });
        let wire: Wire = self
            .graphql_json(raw::DEPLOYMENTS, vars, repo, "/repository")
            .await?;
        self.check_connection(&wire.deployments, 25, format!("{repo}'s deployments"));
        let list = browse::DeploymentList {
            environments: wire.environments.into_capped(|n| Some(n.name)),
            results: (wire.deployments)
                .into_results(browse::wire_deployments::Deployment::into_info),
        };
        Ok(self
            .kept_page(first, &browse::keys::deployments(repo, environment), list)
            .await)
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
        let head = wire
            .permalink_url
            .rsplit_once("...")
            .and_then(|(_, head)| head.rsplit(':').next())
            .filter(|head| !head.is_empty())
            .map(str::to_owned);
        let comparison = wire.into_comparison(direct);
        // The head is the newest commit listed (or, behind, the merge base).
        if let Some(head) = head
            && !comparison.to.starts_with(&head)
        {
            self.doubt(format!(
                "comparing {spec}, the newest commit listed is {}, but the head is {head}",
                text_short(&comparison.to)
            ));
        }
        let listed = comparison.commits.len() as u64;
        let expected = comparison.commits.total().min(browse::COMPARE_COMMITS);
        if listed != expected {
            self.doubt(format!(
                "comparing {spec}, {listed} commits are listed of {}, not {expected}",
                comparison.commits.total()
            ));
        }
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
        let vars =
            serde_json::json!({ "owner": repo.owner, "name": repo.name, "rev": rev, "path": path });
        let subject = format!("{repo}:{rev}:{path}");
        let wire: browse::wire_blame::Blame =
            (self.graphql_json(raw::BLAME, vars, subject, "/repository/object/blame")).await?;
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
        let vars = serde_json::json!({ "login": login, "after": after });
        let wire: browse::wire::Connection<browse::wire_gists::Gist> =
            (self.graphql_json(raw::GISTS, vars, format!("{login}'s gists"), "/user/gists"))
                .await?;
        self.check_connection(&wire, 30, format!("{login}'s gists"));
        let gists = wire.into_results(browse::wire_gists::Gist::into_summary);
        Ok(self
            .kept_page(first, &browse::keys::gists(login), gists)
            .await)
    }
    /// The teams in an organization you can see (its members see them),
    /// by name, 30 at a time from `after`.
    pub async fn teams(
        &self,
        org: &str,
        after: Option<String>,
    ) -> Result<browse::Results<browse::TeamSummary>, ApiError> {
        let first = after.is_none();
        let vars = serde_json::json!({ "org": org, "after": after });
        let wire: browse::wire::Connection<browse::wire_teams::Team> =
            (self.graphql_json(&raw::teams(), vars, org, "/organization/teams")).await?;
        self.check_connection(&wire, 30, format!("{org}'s teams"));
        let teams = wire.into_results(browse::wire_teams::Team::into_summary);
        Ok(self
            .kept_page(first, &browse::keys::teams(org), teams)
            .await)
    }

    /// A team: its members, repositories and child teams.
    pub async fn team(&self, org: &str, slug: &str) -> Result<browse::TeamDetail, ApiError> {
        let vars = serde_json::json!({ "org": org, "slug": slug });
        let wire: browse::wire_teams::Detail = (self.graphql_json(
            &raw::team(),
            vars,
            format!("{org}/{slug}"),
            "/organization/team",
        ))
        .await?;
        let team = wire.into_detail();
        Ok(self.kept(&browse::keys::team(org, slug), team).await)
    }
    /// A repository's published security advisories, or GitHub's newest
    /// reviewed ones, up to 100. REST: repositories' advisories aren't in
    /// GraphQL, and one shape serves both.
    pub async fn advisories(
        &self,
        repo: Option<&RepoId>,
        after: Option<String>,
    ) -> Result<browse::Results<browse::Advisory>, ApiError> {
        let first = after.is_none();
        let mut path = match repo {
            Some(r) => format!(
                "/repos/{}/{}/security-advisories?state=published&per_page={REST_PAGE}",
                r.owner, r.name
            ),
            None => format!("/advisories?type=reviewed&per_page={REST_PAGE}"),
        };
        if let Some(after) = &after {
            path.push_str(&format!("&after={}", encode_query(after)));
        }
        let (wire, next): (Vec<browse::rest_advisories::Advisory>, _) =
            self.rest_cursor_page(&path).await?;
        let items: Vec<browse::Advisory> = wire
            .into_iter()
            .map(browse::rest_advisories::Advisory::into_advisory)
            .collect();
        // GitHub doesn't say how many there are.
        let list = browse::Results {
            total: items.len() as u64,
            items,
            next,
        };
        Ok(self
            .kept_page(first, &browse::keys::advisories(repo), list)
            .await)
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
        let jobs_path = &format!("{path}/jobs?per_page=100");
        let (wire, jobs) = tokio::join!(
            self.rest_json::<browse::rest_actions::Run>(&path),
            self.rest_json::<browse::rest_actions::Jobs>(jobs_path)
        );
        // Big matrices have more than a page of jobs; an empty page is the last.
        let results = |page: u64, jobs: browse::rest_actions::Jobs| browse::Results {
            next: next_page(page, 100, jobs.total_count).filter(|_| !jobs.jobs.is_empty()),
            total: jobs.total_count,
            items: jobs.jobs,
        };
        let page = move |after: Option<String>| async move {
            let page = rest_page(after.as_deref());
            let jobs = self.rest_json(&format!("{jobs_path}&page={page}")).await?;
            Ok(results(page, jobs))
        };
        let what = format!("run {run}'s jobs");
        let jobs = self
            .more_pages(what, results(1, jobs?), PAGES, page)
            .await?;
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

    /// A job's log (see [`browse::JobLog`]), never cached: logs are big.
    /// A running job has none yet (GitHub writes it when the job ends), and
    /// one past the repository's retention has expired.
    pub async fn job_log(&self, repo: &RepoId, job: u64) -> Result<browse::JobLog, ApiError> {
        if self.job(repo, job).await?.outcome == browse::CheckOutcome::Pending {
            return Ok(browse::JobLog {
                running: true,
                ..browse::JobLog::default()
            });
        }
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
        if response.status == StatusCode::GONE {
            return Err(ApiError::Gone(
                "this log has expired (GitHub keeps logs for the repository's retention period, 90 days by default)".into(),
            ));
        }
        check_status(&response, &path)?;
        let (cut, text) = browse::log::cut(&response.body);
        Ok(browse::JobLog {
            text: text.to_owned(),
            cut,
            running: false,
        })
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
            disabled: Some(wire.state)
                .filter(|s| s != "active")
                .map(|s| s.replace('_', " ")),
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
        let page = rest_page(after.as_deref());
        let path = format!(
            "/repos/{}/{}/actions/workflows/{}/runs?per_page={REST_PAGE}&page={page}",
            repo.owner,
            repo.name,
            encode_path(file)
        );
        let wire: browse::rest_actions::Runs = self.rest_json(&path).await?;
        let runs = browse::Results {
            total: wire.total_count,
            items: wire
                .workflow_runs
                .into_iter()
                .map(browse::rest_actions::Run::into_summary)
                .collect(),
            next: next_page(page, REST_PAGE, wire.total_count),
        };
        Ok(self
            .kept_page(page == 1, &browse::keys::workflow_runs(repo, file), runs)
            .await)
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
        let pick = |q: browse::CommitChecksQuery| match q.repository?.object? {
            browse::ChecksTarget::Commit(commit) => Some(commit),
            browse::ChecksTarget::Other => None,
        };
        let commit = self.find(op, format!("{repo}@{rev}"), pick).await?;
        let checks = self.all_checks(repo, commit).await?;
        // A full commit ID names the commit checked.
        if rev.len() == 40 && rev.bytes().all(|b| b.is_ascii_hexdigit()) && checks.oid != rev {
            self.doubt(format!(
                "the checks asked for {} are for {}",
                text_short(rev),
                text_short(&checks.oid)
            ));
        }
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
        let prefix = &format!(
            "https://github.com/orgs/{}/discussions/",
            org.to_lowercase()
        );
        // The organization's discussions are among its repositories', in
        // best-match order: look through the search's pages for them.
        let page = move |after| async move {
            let vars = serde_json::json!({ "q": format!("org:{org}"), "after": after });
            let subject = format!("{org}'s discussions");
            let search: browse::wire::Connection<browse::wire_discussions::Found> =
                (self.graphql_json(raw::DISCUSSION_URLS, vars, subject, "/search")).await?;
            let mut found = search.filter_results(|n| {
                (n.url.to_lowercase().starts_with(prefix))
                    .then(|| RepoId::parse(&n.repository?.name_with_owner))?
            });
            found.next = found.next.filter(|_| found.items.is_empty());
            Ok(found)
        };
        let what = format!("the search for {org}'s discussions");
        let found = self.more_pages(what, page(None).await?, PAGES, page);
        (found.await?.items.into_iter().next())
            .ok_or_else(|| ApiError::NotFound(format!("{org}'s discussions")))
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
        let at = "/repository/discussionCategories";
        let categories: browse::wire::Counted<w::Category> =
            (self.graphql_json(raw::DISCUSSION_CATEGORIES, vars, &repo, at)).await?;
        let categories = categories.into_capped(Some);
        let category_id = match category {
            Some(slug) => match categories.iter().find(|c| c.slug == slug) {
                Some(c) => Some(c.id.clone()),
                None if categories.left_out() > 0 => {
                    return Err(ApiError::NotFound(format!(
                        "{repo}'s category {slug} (or it's past the first {} of {})",
                        categories.len(),
                        categories.total()
                    )));
                }
                None => return Err(ApiError::NotFound(format!("{repo}'s category {slug}"))),
            },
            None => None,
        };
        let vars = serde_json::json!({ "owner": repo.owner, "name": repo.name, "after": after, "category": category_id });
        let page: browse::wire::Connection<w::Summary> =
            (self.graphql_json(raw::DISCUSSIONS, vars, &repo, "/repository/discussions")).await?;
        self.check_connection(&page, 25, format!("{repo}'s discussions"));
        let list = browse::DiscussionList {
            categories: categories.map(|c| browse::DiscussionCategory {
                name: c.name,
                slug: c.slug,
            }),
            results: page.into_results(w::Summary::into_summary),
        };
        Ok(self
            .kept_page(first, &browse::keys::discussions(of, category), list)
            .await)
    }

    /// A discussion, its comments and their replies.
    pub async fn discussion(
        &self,
        of: &browse::DiscussionsOf,
        number: u64,
    ) -> Result<browse::DiscussionDetail, ApiError> {
        let repo = self.discussions_repo(of).await?;
        let vars = serde_json::json!({ "owner": repo.owner, "name": repo.name, "number": number });
        let subject = format!("{repo} discussion {number}");
        let wire: browse::wire_discussions::Detail =
            (self.graphql_json(raw::DISCUSSION, vars, subject, "/repository/discussion")).await?;
        let detail = wire.into_detail(repo);
        Ok(self
            .kept(&browse::keys::discussion(of, number), detail)
            .await)
    }

    /// Branches by name, up to [`PAGES`] pages of 100, and the newest 100
    /// tags; with how many there are of each. (GitHub's commit-date order
    /// sorts branches by name, backwards, so it isn't used for them.)
    pub async fn refs(&self, repo: &RepoId) -> Result<browse::Refs, ApiError> {
        use browse::wire::{Connection, Name};
        /// The first page, which has the tags too.
        #[derive(serde::Deserialize)]
        struct Wire {
            heads: Connection<Name>,
            tags: Connection<Name>,
        }
        let what = &format!("{repo}'s branches");
        let vars = |after: &Option<String>| serde_json::json!({ "owner": repo.owner, "name": repo.name, "after": after, "tags": after.is_none() });
        let heads = |heads: Connection<Name>| {
            self.check_connection(&heads, 100, what);
            heads.into_results(|n| n.name)
        };
        let first: Wire = self
            .graphql_json(raw::REFS, vars(&None), repo, "/repository")
            .await?;
        self.check_connection(&first.tags, 100, format!("{repo}'s tags"));
        let tags = first.tags.into_results(|n| n.name);
        let tags = Capped::new(tags.items, tags.total);
        let more = |after| async move {
            let page = self.graphql_json(raw::REFS, vars(&after), repo, "/repository/heads");
            Ok(heads(page.await?))
        };
        let branches = self
            .more_pages(what, heads(first.heads), PAGES, more)
            .await?;
        let refs = browse::Refs { branches, tags };
        Ok(self.kept(&browse::keys::refs(repo), refs).await)
    }

    /// Saves a small value in the cache (e.g. recently visited pages).
    pub async fn remember<T: Serialize + Send + 'static>(&self, key: &str, value: T) {
        let store = self.store.clone();
        let key = key.to_owned();
        spawn_store(move || store.query_put(&key, &value)).await;
    }

    /// [`Self::kept`] for a list's first page; later pages aren't cached.
    /// A list's page, checked it adds up, then kept if it's the first.
    async fn kept_page<T: Serialize + Clone + Send + browse::Page + 'static>(
        &self,
        first: bool,
        key: &str,
        value: T,
    ) -> T {
        self.check_page(&value, key);
        self.kept_if(first, key, value).await
    }

    async fn kept_if<T: Serialize + Clone + Send + 'static>(
        &self,
        first: bool,
        key: &str,
        value: T,
    ) -> T {
        if first {
            self.kept(key, value).await
        } else {
            value
        }
    }

    /// [`Self::remember`]s `value` and hands it back.
    async fn kept<T: Serialize + Clone + Send + 'static>(&self, key: &str, value: T) -> T {
        self.remember(key, value.clone()).await;
        value
    }

    /// Repositories you own or contribute to, most recently pushed first.
    pub async fn viewer_repos(&self) -> Result<Capped<browse::RepoSummary>, ApiError> {
        let data = self.graphql(browse::ViewerReposQuery::build(())).await?;
        let repos = browse::repo_list(data.viewer.repositories);
        Ok(self.kept(browse::keys::VIEWER_REPOS, repos).await)
    }
}

/// A query whose root GitHub never answers with null (`search`,
/// `viewer`), so [`GitHub::graphql`] may take its data whole. A claim
/// about GitHub's schema: a query for one thing goes through
/// [`GitHub::find`] instead.
trait Unrooted {}
impl Unrooted for queries::SearchQuery {}
impl Unrooted for browse::BrowseSearch {}
impl Unrooted for browse::ViewerReposQuery {}

use reply::Reply;
mod reply {
    use serde::de::DeserializeOwned;
    use serde_json::Value;

    use super::ApiError;

    /// A GraphQL response with data, and GitHub's errors beside it that the
    /// data doesn't already say.
    pub(super) struct Reply {
        data: Value,
        errors: Vec<String>,
    }

    impl Reply {
        pub(super) fn new(data: Value, errors: Vec<String>) -> Self {
            Self { data, errors }
        }

        /// What a typed query's data yields to `pick`; see [`Self::found`].
        /// `Q` is a query's own type, so the whole data can't be taken as a
        /// `Value` and its nulls explained by hand.
        pub(super) fn picked<Q: DeserializeOwned + cynic::QueryFragment, T>(
            self,
            subject: impl std::fmt::Display,
            pick: impl FnOnce(Q) -> Option<T>,
            leave_out: impl FnOnce(Vec<String>),
        ) -> Result<T, ApiError> {
            self.found(subject, |d| serde_json::from_value(d).map(pick), leave_out)
        }

        /// The non-null value at `at`, a JSON pointer below the data's root
        /// (never the whole data); see [`Self::found`].
        pub(super) fn at<T: DeserializeOwned>(
            self,
            subject: impl std::fmt::Display,
            at: &str,
            leave_out: impl FnOnce(Vec<String>),
        ) -> Result<T, ApiError> {
            if at.len() < 2 || !at.starts_with('/') {
                return Err(ApiError::Internal(format!("{at:?} is not below the root")));
            }
            let read = |mut data: Value| {
                let value = data.pointer_mut(at).map(Value::take);
                value
                    .filter(|v| !v.is_null())
                    .map(T::deserialize)
                    .transpose()
            };
            self.found(subject, read, leave_out)
        }

        /// What a query `read`s from the data: the one place that decides
        /// why a value is missing. Found, GitHub's errors are what it left
        /// out; missing beside errors, they say why; missing without any,
        /// `subject` is not found.
        fn found<T>(
            self,
            subject: impl std::fmt::Display,
            read: impl FnOnce(Value) -> Result<Option<T>, serde_json::Error>,
            leave_out: impl FnOnce(Vec<String>),
        ) -> Result<T, ApiError> {
            match read(self.data) {
                Ok(Some(found)) => {
                    leave_out(self.errors);
                    Ok(found)
                }
                _ if !self.errors.is_empty() => Err(ApiError::GraphQl(self.errors)),
                Ok(None) => Err(ApiError::NotFound(subject.to_string())),
                Err(err) => Err(err.into()),
            }
        }

        /// A mutation's data, or GitHub's errors if it has any.
        pub(super) fn mutation<Q: DeserializeOwned>(self) -> Result<Q, ApiError> {
            if !self.errors.is_empty() {
                return Err(ApiError::GraphQl(self.errors));
            }
            Ok(serde_json::from_value(self.data)?)
        }
    }
}

/// One of GitHub's GraphQL errors: what it says, its type (`NOT_FOUND`,
/// `FORBIDDEN`, …) and where in the query it is.
#[derive(Debug, serde::Deserialize)]
struct GqlError {
    message: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    path: Option<Vec<Value>>,
}

impl GqlError {
    /// Whether this is GitHub's NOT_FOUND for what `data` holds as null (or
    /// what's under a null): the reason it's absent.
    fn explains_null(&self, data: &Value) -> bool {
        self.kind.as_deref() == Some("NOT_FOUND")
            && self
                .path
                .iter()
                .flatten()
                .try_fold(data, |at, step| match step {
                    _ if at.is_null() => Some(at),
                    Value::String(key) => at.get(key),
                    Value::Number(i) => at.get(usize::try_from(i.as_u64()?).ok()?),
                    _ => None,
                })
                .is_some_and(Value::is_null)
    }
}

/// What GitHub's errors say: an error without a message by its type, so
/// it still counts.
fn messages(errors: impl IntoIterator<Item = GqlError>) -> Vec<String> {
    let said = |e: GqlError| {
        e.message
            .or(e.kind)
            .unwrap_or_else(|| "an unexplained error".into())
    };
    errors.into_iter().map(said).collect()
}

/// How many items a REST list page holds here.
const REST_PAGE: u64 = 30;
/// GitHub's search serves its first thousand results.
const SEARCH_CAP: u64 = 1000;
/// Pages of 100 read of a list GitHub can make long (checks, branches,
/// jobs, search results): a thousand.
const PAGES: u32 = 10;
/// Pages of 100 of a PR's files (GitHub's own cap is 3000) or review
/// threads.
const FILE_PAGES: u32 = 30;

/// A REST list's page number from its cursor (pages count from 1).
fn rest_page(after: Option<&str>) -> u64 {
    after.and_then(|a| a.parse().ok()).unwrap_or(1)
}

/// The cursor for the page after `page` of `total` items, `per_page` a
/// page, if there is one.
fn next_page(page: u64, per_page: u64, total: u64) -> Option<String> {
    (page.saturating_mul(per_page) < total).then(|| page.saturating_add(1).to_string())
}

/// A commit ID shortened, as GitHub shows it.
fn text_short(oid: &str) -> &str {
    oid.get(..7).unwrap_or(oid)
}

/// The `after` cursor in a `Link` header's `rel="next"` URL.
fn next_after(link: &str) -> Option<String> {
    let next = link.split(',').find(|l| l.contains("rel=\"next\""))?;
    let url = next.split(['<', '>']).nth(1)?;
    let query = url.split_once('?')?.1;
    let after = query.split('&').find_map(|kv| kv.strip_prefix("after="))?;
    Some(
        url::form_urlencoded::parse(format!("a={after}").as_bytes())
            .next()?
            .1
            .into_owned(),
    )
}

/// Percent-encodes a query string's value (spaces as `+`).
fn encode_query(s: &str) -> String {
    escape(s, true)
}

/// Percent-encodes a ref for a URL path (branch names may contain `/`).
fn encode_path(s: &str) -> String {
    escape(s, false)
}

/// `s` with every byte but unreserved ones (`A-Z a-z 0-9 - _ . ~`)
/// percent-encoded, and spaces as `+` when `plus`.
fn escape(s: &str, plus: bool) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(b));
            }
            b' ' if plus => out.push('+'),
            b => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
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

/// A failed response's error. A 404 names what was asked for at `path`
/// (`o/r/actions/jobs/2`): GitHub's message only says "Not Found".
fn check_status(response: &Response, path: &str) -> Result<(), ApiError> {
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
    if status == StatusCode::NOT_FOUND {
        let path = path.split('?').next().unwrap_or(path);
        let asked = path.strip_prefix("/repos/").or(path.strip_prefix('/'));
        return Err(ApiError::NotFound(asked.unwrap_or(path).to_owned()));
    }
    let message = serde_json::from_str::<Message>(&response.body)
        .map_or_else(|_| response.body.chars().take(200).collect(), |m| m.message);
    Err(ApiError::Http {
        status: status.as_u16(),
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A REST list has another page while the pages so far hold fewer than
    /// its total.
    #[test]
    fn rest_lists_page_to_their_total() {
        assert_eq!(next_page(1, REST_PAGE, 0), None);
        assert_eq!(next_page(1, REST_PAGE, REST_PAGE), None);
        assert_eq!(next_page(1, REST_PAGE, REST_PAGE + 1).as_deref(), Some("2"));
        assert_eq!(next_page(2, REST_PAGE, 2 * REST_PAGE), None);
        assert_eq!(
            next_page(2, REST_PAGE, 2 * REST_PAGE + 1).as_deref(),
            Some("3")
        );
    }

    /// A NOT_FOUND explains a null (or missing) value at its path, through
    /// keys and list indexes; one at the root, or without a type, doesn't.
    #[test]
    fn a_not_found_explains_only_the_null_it_points_at() {
        let data = serde_json::json!({"a": [{"b": 1}, null], "c": null});
        let error = |kind: Option<&str>, path: serde_json::Value| GqlError {
            message: None,
            kind: kind.map(str::to_owned),
            path: serde_json::from_value(path).ok(),
        };
        let found = Some("NOT_FOUND");
        assert!(error(found, serde_json::json!(["a", 1])).explains_null(&data));
        assert!(error(found, serde_json::json!(["c"])).explains_null(&data));
        assert!(error(found, serde_json::json!(["c", "e", 0])).explains_null(&data));
        assert!(!error(found, serde_json::json!(["d"])).explains_null(&data));
        assert!(!error(found, serde_json::json!(["a", 0, "b", "e"])).explains_null(&data));
        assert!(!error(found, serde_json::json!(["a", 0])).explains_null(&data));
        assert!(!error(found, serde_json::json!([])).explains_null(&data));
        assert!(!error(None, serde_json::json!(["c"])).explains_null(&data));
        assert!(!error(Some("FORBIDDEN"), serde_json::json!(["c"])).explains_null(&data));
    }
}
