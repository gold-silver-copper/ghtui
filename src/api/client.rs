//! The GitHub client: one HTTP client (reqwest) for GraphQL and REST, with
//! our own retry policy, rate-limit tracking, ETag revalidation and caching.

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::store::{Cached, HttpEntry, Store};
use cynic::{MutationBuilder, QueryBuilder};
use http::{HeaderMap, HeaderValue, StatusCode, header};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::auth::Token;
use crate::api::browse;
use crate::api::model::{
    Capped, ChecksState, NewThread, NodeId, PatchFile, PendingReview, PrDetail, PrRef, RepoId,
    ReviewEvent, ReviewThread, ViewedFiles,
};
use crate::api::notifications;
use crate::api::queries::{self, nodes};
use crate::api::rate_limit::{RateLimits, retry_after};
use crate::api::raw;
use crate::api::timeline;
use crate::host::Host;
use crate::lock;

const MAX_ATTEMPTS: u32 = 3;
/// About when GitHub gives up on a request (it allows 10 seconds).
const GITHUB_TIMEOUT: Duration = Duration::from_secs(9);
/// Rate-limit waits longer than this are reported instead of slept through.
const MAX_INLINE_WAIT_SECS: u64 = 10;

#[derive(Debug, Clone, thiserror::Error)]
pub enum ApiError {
    #[error(
        "GitHub rejected the token: {}.",
        crate::api::auth::sign_in_hint(crate::host::current())
    )]
    Unauthorized,
    #[error("rate limited by GitHub; retry in {0}s")]
    RateLimited(u64),
    #[error("not found: {0}")]
    NotFound(String),
    /// GitHub withholds it for a legal reason (a DMCA takedown, say), and
    /// says which, with a link to the notice.
    #[error("{0}")]
    Blocked(String),
    /// It was there, and GitHub has since dropped it.
    #[error("{0}")]
    Gone(String),
    /// GitHub refuses it to this token or this account, said as what's
    /// missing and how to add it.
    #[error("{0}")]
    Denied(String),
    /// Reading through git (a wiki) failed.
    #[error("git: {0}")]
    Git(String),
    #[error("GitHub returned {status}: {message}")]
    Http { status: u16, message: String },
    #[error("GraphQL: {}", .0.join("; "))]
    GraphQl(Vec<String>),
    #[error("network error: {0}")]
    Network(String),
    /// GitHub couldn't be reached, and nothing was sent
    /// ([`crate::api::reach`]).
    #[error("GitHub can't be reached ({0})")]
    Offline(String),
    #[error("unexpected response from GitHub: {0}")]
    Decode(String),
    #[error("could not start HTTP client: {0}")]
    Setup(String),
    /// A bug in ghtui, not a problem with GitHub.
    #[error("internal error: {0}")]
    Internal(String),
    /// The Enterprise Server doesn't have it, said as what and which.
    #[error("{0}")]
    Unsupported(String),
}

impl From<serde_json::Error> for ApiError {
    fn from(err: serde_json::Error) -> Self {
        ApiError::Decode(err.to_string())
    }
}

/// What polling a REST path found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Polled {
    /// The same as the answer tagged with the ETag asked with.
    Same,
    /// A new answer, and its ETag (empty if GitHub gave none).
    Changed { etag: String, body: String },
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
    recorder: Option<Arc<crate::api::corpus::Recorder>>,
    /// What an Enterprise Server is known to lack: its version's (asked
    /// once), what it said it lacks, and what of it was said.
    enterprise: Arc<Enterprise>,
    /// Whether GitHub can be reached ([`crate::api::reach`]).
    reach: crate::api::reach::Reach,
    /// Doubled before each retry: the first waits twice this.
    backoff: Duration,
}

/// An Enterprise Server's version, and what it's known to lack.
#[derive(Default)]
struct Enterprise {
    server: tokio::sync::OnceCell<crate::api::enterprise::Server>,
    learned: Mutex<Vec<crate::api::enterprise::Lack>>,
    /// What pages go without, said once each: what was, and what's yet
    /// to be.
    said: Mutex<(Vec<String>, Vec<String>)>,
}

struct Http {
    token: Token,
    /// Its website links are this GitHub's.
    host: Host,
    /// The REST API's base (see [`Host::rest_api`]), or a local server in
    /// tests.
    rest: String,
    /// The GraphQL endpoint.
    graphql: String,
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

/// A REST answer, whatever its status ([`GitHub::rest_answer`]).
pub(crate) struct Answer(Response);

impl Answer {
    pub(crate) fn status(&self) -> u16 {
        self.0.status.as_u16()
    }

    pub(crate) fn body(&self) -> &str {
        &self.0.body
    }

    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.0.headers.get(name).and_then(|v| v.to_str().ok())
    }

    /// The error its status means, as any other request's would be.
    pub(crate) fn checked(&self, what: impl std::fmt::Display) -> Result<(), ApiError> {
        check_status(&self.0, what)
    }

    /// The cursor for the next page in its `Link` header: `after=…` or
    /// `page=…`, as a query parameter to send.
    pub(crate) fn next_page(&self) -> Option<String> {
        next_query(self.header(header::LINK.as_str())?)?
            .split('&')
            .find(|kv| kv.starts_with("after=") || kv.starts_with("page="))
            .map(str::to_owned)
    }
}

enum Request<'a> {
    /// With the cached copy, answered 304 if it hasn't changed since.
    Get {
        path: &'a str,
        cached: Option<&'a HttpEntry>,
    },
    /// A GraphQL request.
    Post {
        body: &'a serde_json::Value,
        /// Safe to send twice: a query, not a mutation.
        idempotent: bool,
        /// A preview's media type, for fields GitHub serves only under
        /// one (the dependency graph's).
        accept: Option<&'static str>,
    },
    /// A REST change (a re-run, marking a thread read), with a JSON body
    /// if it takes one.
    Write {
        method: http::Method,
        path: &'a str,
        body: Option<&'a serde_json::Value>,
    },
}

impl Request<'_> {
    fn idempotent(&self) -> bool {
        match self {
            Request::Get { .. } => true,
            Request::Post { idempotent, .. } => *idempotent,
            Request::Write { .. } => false,
        }
    }
}

impl GitHub {
    /// A client for `host`'s API.
    pub fn new(token: Token, store: Store, host: Host) -> Self {
        Self::with_bases(token, store, host, host.rest_api(), host.graphql_api())
    }

    /// For tests: point the client at a local server, its GraphQL at
    /// `/graphql`, retrying at once.
    pub fn with_base_uri(token: Token, store: Store, base_uri: &str) -> Self {
        Self {
            backoff: Duration::ZERO,
            ..Self::enterprise_at(token, store, Host::GITHUB_COM, base_uri)
        }
    }

    fn with_bases(token: Token, store: Store, host: Host, rest: String, graphql: String) -> Self {
        Self {
            http: Arc::new(Http {
                token,
                host,
                rest,
                graphql,
                client: tokio::sync::OnceCell::new(),
            }),
            store,
            limits: Arc::default(),
            rejected: Arc::default(),
            left_out: Arc::default(),
            doubts: Arc::default(),
            recorder: None,
            enterprise: Arc::default(),
            reach: crate::api::reach::Reach::default(),
            backoff: Duration::from_millis(250),
        }
    }

    /// Whether GitHub can be reached, to follow.
    pub fn reach(&self) -> &crate::api::reach::Reach {
        &self.reach
    }

    /// One GET of the rate limit (which costs nothing), whatever the
    /// reach: any answer means GitHub is there.
    pub(crate) async fn probe_once(&self) -> Result<(), ApiError> {
        let client = self.client().await?;
        let url = format!("{}/rate_limit", self.http.rest);
        match client.get(url).send().await {
            Ok(_) => {
                self.reach.found();
                Ok(())
            }
            Err(err) => {
                if err.is_connect() {
                    self.reach.lost();
                }
                Err(crate::api::reach::offline(&err))
            }
        }
    }

    /// For tests: an Enterprise Server `host`, served by a local server
    /// at `base_uri` (its REST there, its GraphQL at `/graphql`).
    pub fn enterprise_at(token: Token, store: Store, host: Host, base_uri: &str) -> Self {
        let rest = base_uri.trim_end_matches('/');
        let graphql = format!("{rest}/graphql");
        Self::with_bases(token, store, host, rest.to_owned(), graphql)
    }

    /// The Enterprise Server this is, with its version, asked once; on
    /// github.com, `None`.
    async fn server(&self) -> Option<crate::api::enterprise::Server> {
        use crate::api::enterprise::{Server, Version};
        if !self.http.host.is_enterprise() {
            return None;
        }
        let server = self.enterprise.server.get_or_init(|| async {
            #[derive(serde::Deserialize)]
            struct Meta {
                installed_version: Option<String>,
            }
            let request = Request::Get {
                path: META,
                cached: None,
            };
            let meta = (self.send_as_is(&request).await)
                .ok()
                .filter(|r| r.status.is_success());
            let version = meta
                .and_then(|r| serde_json::from_str::<Meta>(&r.body).ok())
                .and_then(|m| Version::parse(&m.installed_version?));
            tracing::info!(?version, "the Enterprise Server's version");
            Server { version }
        });
        Some(*server.await)
    }

    /// Says, once each, what pages go without on `server`.
    fn missing(&self, server: crate::api::enterprise::Server, about: &[String]) {
        let mut said = lock(&self.enterprise.said);
        let (all, new) = &mut *said;
        let fresh: Vec<String> = (about.iter())
            .filter(|a| !all.contains(a))
            .cloned()
            .collect();
        if fresh.is_empty() {
            return;
        }
        all.extend(fresh.iter().cloned());
        new.push(format!(
            "{} doesn't have {}: ghtui goes without",
            server.name(),
            fresh.join(", ")
        ));
    }

    /// What an Enterprise Server lacks that pages go without, not yet
    /// said.
    pub fn take_missing(&self) -> Vec<String> {
        let mut said = lock(&self.enterprise.said);
        std::mem::take(&mut said.1)
    }

    /// A GraphQL request `body` as `server` can take it: without what it
    /// lacks, and what its answer reads in place of each field left out.
    fn for_server<'a>(
        &self,
        server: crate::api::enterprise::Server,
        body: &'a Value,
    ) -> (Cow<'a, Value>, Vec<(Vec<String>, Value)>) {
        use crate::api::enterprise::{lacks, rewrite};
        let mut lacks = lacks(server);
        lacks.extend(lock(&self.enterprise.learned).iter().cloned());
        let query = body
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(done) = rewrite(query, &lacks) else {
            return (Cow::Borrowed(body), Vec::new());
        };
        self.missing(server, &done.about);
        let mut sent = body.clone();
        if let Some(map) = sent.as_object_mut() {
            map.insert("query".into(), Value::String(done.query));
            if let Some(Value::Object(vars)) = map.get_mut("variables") {
                for name in &done.unused {
                    vars.remove(name);
                }
            }
        }
        (Cow::Owned(sent), done.fills)
    }

    /// Learns what a server's refusal says it lacks; whether it learned
    /// anything new.
    fn learn(&self, messages: &[String]) -> bool {
        let mut learned = lock(&self.enterprise.learned);
        let mut new = false;
        for lack in messages
            .iter()
            .filter_map(|m| crate::api::enterprise::learn(m))
        {
            if !learned.iter().any(|l| l.missing == lack.missing) {
                tracing::info!(?lack, "the Enterprise Server lacks it");
                learned.push(lack);
                new = true;
            }
        }
        new
    }

    /// Records every response into `dir`, for the corpus. The token isn't
    /// recorded: only the request's method, path and body, and the
    /// response's status, `Link` header and body.
    #[must_use]
    pub fn recording(mut self, dir: std::path::PathBuf) -> Self {
        self.recorder = Some(Arc::new(crate::api::corpus::Recorder(dir)));
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
        std::mem::take(&mut *lock(&self.left_out))
    }

    /// Where GitHub's answers didn't add up since this was last asked: a
    /// comparison whose last commit isn't its head, more items than the
    /// total, … (see [`Self::doubt`]).
    pub fn take_doubts(&self) -> Vec<String> {
        std::mem::take(&mut *lock(&self.doubts))
    }

    /// Notes that an answer doesn't add up: logged, and reported. What's
    /// shown is still GitHub's answer; this says not to trust it.
    fn doubt(&self, what: String) {
        tracing::warn!(what, "GitHub's answer doesn't add up");
        note(&self.doubts, [what]);
    }

    /// A page of a list, checked it adds up: no more items than GitHub
    /// counts.
    pub(crate) fn checked<T: browse::Page>(&self, list: T, what: impl std::fmt::Display) -> T {
        let (items, total) = list.counts();
        if items as u64 > total {
            self.doubt(format!("{what}: a page of {items}, of {total} in all"));
        }
        list
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
    pub(crate) fn check_connection<T>(
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
        note(&self.left_out, errors);
    }

    /// Whether GitHub started (`Some(true)`) or stopped rejecting the token
    /// since this was last asked.
    pub fn token_rejected_change(&self) -> Option<bool> {
        let now = self.rejected.0.load(Ordering::Relaxed);
        let was = self.rejected.1.swap(now, Ordering::Relaxed);
        (now != was).then_some(now)
    }

    pub fn rate_limits(&self) -> RateLimits {
        *lock(&self.limits)
    }

    async fn send(&self, request: &Request<'_>) -> Result<Response, ApiError> {
        // A REST endpoint an Enterprise Server lacks isn't asked.
        if let Request::Get { path, .. } | Request::Write { path, .. } = request
            && *path != META
            && let Some(server) = self.server().await
            && let Some(about) = crate::api::enterprise::rest_lack(server, path)
        {
            self.missing(server, &[about.to_owned()]);
            return Err(ApiError::Unsupported(format!(
                "{} doesn't have {about}",
                server.name()
            )));
        }
        self.send_as_is(request).await
    }

    /// [`Self::send`], whatever the server lacks.
    async fn send_as_is(&self, request: &Request<'_>) -> Result<Response, ApiError> {
        // Unreachable: the cache answers, and only a probe asks.
        if self.reach.offline_since().is_some() {
            return Err(ApiError::Offline("not sent while offline".into()));
        }
        let client = self.client().await?;
        let mut attempt = 0;
        loop {
            attempt += 1;
            let last = attempt >= MAX_ATTEMPTS;
            let sent = std::time::Instant::now();
            let result = match request {
                Request::Get { path, cached } => {
                    let mut get = client.get(format!("{}{path}", self.http.rest));
                    // Unchanged since the cached copy, GitHub answers 304,
                    // which doesn't count against the rate limit.
                    let validators = cached.iter().flat_map(|c| {
                        [
                            (header::IF_NONE_MATCH, Some(c.etag.as_str())),
                            (header::IF_MODIFIED_SINCE, c.last_modified.as_deref()),
                        ]
                    });
                    for (name, value) in validators {
                        if let Some(value) = value.filter(|v| !v.is_empty())
                            && let Ok(value) = HeaderValue::from_str(value)
                        {
                            get = get.header(name, value);
                        }
                    }
                    get.send().await
                }
                Request::Post { body, accept, .. } => {
                    let post = client.post(&self.http.graphql).json(body);
                    match accept {
                        Some(accept) => post.header(header::ACCEPT, *accept).send().await,
                        None => post.send().await,
                    }
                }
                Request::Write { method, path, body } => {
                    let url = format!("{}{path}", self.http.rest);
                    let write = client.request(method.clone(), url);
                    match body {
                        Some(body) => write.json(body).send().await,
                        None => write.send().await,
                    }
                }
            };
            // A mutation that may have reached GitHub is never sent again:
            // it could post a comment twice. A failed connect never left.
            let response = match result {
                Ok(response) => response,
                Err(err) if !last && (request.idempotent() || err.is_connect()) => {
                    tracing::debug!(%err, attempt, "request failed; retrying");
                    tokio::time::sleep(self.backoff * (1 << attempt)).await;
                    continue;
                }
                // It never left: GitHub can't be reached.
                Err(err) if err.is_connect() => {
                    self.reach.lost();
                    return Err(crate::api::reach::offline(&err));
                }
                Err(err) => return Err(ApiError::Network(err.to_string())),
            };
            self.reach.found();
            let status = response.status();
            self.rejected
                .0
                .store(status == StatusCode::UNAUTHORIZED, Ordering::Relaxed);
            let headers = response.headers().clone();
            lock(&self.limits).update(&headers);

            if let Some(wait) = retry_after(status, &headers, crate::store::now()) {
                if last || wait > MAX_INLINE_WAIT_SECS {
                    return Err(ApiError::RateLimited(wait));
                }
                tracing::info!(wait, "rate limited; waiting");
                tokio::time::sleep(Duration::from_secs(wait.max(1))).await;
                continue;
            }
            // GitHub stops a request after 10 seconds: one that ran out of
            // them would only run out again.
            let timed_out = sent.elapsed() >= GITHUB_TIMEOUT;
            if status.is_server_error() && !last && request.idempotent() && !timed_out {
                tracing::debug!(%status, attempt, "server error; retrying");
                tokio::time::sleep(self.backoff * (1 << attempt)).await;
                continue;
            }
            let body = response
                .text()
                .await
                .map_err(|e| ApiError::Network(e.to_string()))?;
            if let Some(recorder) = &self.recorder {
                let (request, sent) = match request {
                    Request::Get { path, .. } => (format!("GET {path}"), None),
                    Request::Post { body, .. } => ("POST /graphql".to_owned(), Some(*body)),
                    Request::Write { method, path, body } => (format!("{method} {path}"), *body),
                };
                let link = headers
                    .get(header::LINK)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned);
                let recorded = crate::api::corpus::Recorded {
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
    pub(crate) async fn find<Q: DeserializeOwned + cynic::QueryFragment, V: Serialize, T>(
        &self,
        op: cynic::Operation<Q, V>,
        subject: impl std::fmt::Display,
        pick: impl FnOnce(Q) -> Option<T>,
    ) -> Result<T, ApiError> {
        let reply = self.ask(&serde_json::to_value(&op)?, true, None).await?;
        reply.picked(subject, pick, |errors| self.leave_out(errors))
    }

    /// Runs a GraphQL mutation. Any error fails it, with GitHub's messages:
    /// a rejected mutation comes back as data with a null field plus errors.
    pub(crate) async fn mutate<Q: DeserializeOwned, V: Serialize>(
        &self,
        op: cynic::Operation<Q, V>,
    ) -> Result<Q, ApiError> {
        (self.ask(&serde_json::to_value(&op)?, false, None).await?).mutation()
    }

    /// Runs a mutation given as text ([`crate::api::raw`]), which only has
    /// to succeed: any error fails it, with GitHub's messages.
    pub(crate) async fn mutate_json(&self, query: &str, variables: Value) -> Result<(), ApiError> {
        (self.graphql_data::<Value>(query, variables, false).await).map(drop)
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
        let reply = self.ask(&gql_body(query, variables), true, None).await?;
        reply.at(subject, at, |errors| self.leave_out(errors))
    }

    /// A query (or, not `idempotent`, a mutation) given as text, for its
    /// data whole: for a query of several roots, or what a mutation made.
    /// Any error fails it, with GitHub's messages.
    pub(crate) async fn graphql_data<T: DeserializeOwned>(
        &self,
        query: &str,
        variables: Value,
        idempotent: bool,
    ) -> Result<T, ApiError> {
        (self
            .ask(&gql_body(query, variables), idempotent, None)
            .await?)
            .mutation()
    }

    /// [`Self::graphql_json`], under a preview's media type `accept`.
    pub(crate) async fn graphql_preview<T: DeserializeOwned>(
        &self,
        query: &str,
        variables: Value,
        subject: impl std::fmt::Display,
        at: &str,
        accept: &'static str,
    ) -> Result<T, ApiError> {
        let reply = self
            .ask(&gql_body(query, variables), true, Some(accept))
            .await?;
        reply.at(subject, at, |errors| self.leave_out(errors))
    }

    /// Posts a GraphQL request: the one place that reads GitHub's errors.
    /// A query's NOT_FOUND that explains a null in the data is dropped (the
    /// null says it); a mutation (not `idempotent`) keeps every error. A
    /// response without data is an error. `accept` is a preview's media
    /// type, for fields GitHub serves only under one.
    async fn ask(
        &self,
        body: &Value,
        idempotent: bool,
        accept: Option<&'static str>,
    ) -> Result<Reply, ApiError> {
        #[derive(serde::Deserialize)]
        struct Wire {
            #[serde(default)]
            data: Value,
            errors: Option<Vec<GqlError>>,
        }
        let server = self.server().await;
        let mut tries = 0;
        let (mut wire, fills) = loop {
            let (sent, fills) = match server {
                Some(server) => self.for_server(server, body),
                None => (Cow::Borrowed(body), Vec::new()),
            };
            let request = Request::Post {
                body: &sent,
                idempotent,
                accept,
            };
            let response = self.send(&request).await?;
            check_status(&response, "GitHub's GraphQL API")?;
            let wire: Wire = serde_json::from_str(&response.body)?;
            // An Enterprise Server refusing a query for what it lacks
            // has run none of it: asked again without that, it may answer.
            tries += 1;
            if server.is_some() && wire.data.is_null() && tries < 4 {
                let said = (wire.errors.iter().flatten())
                    .filter_map(|e| e.message.clone())
                    .collect::<Vec<_>>();
                if self.learn(&said) {
                    continue;
                }
            }
            break (wire, fills);
        };
        crate::api::enterprise::patch(&mut wire.data, &fills);
        let errors = wire.errors.take().unwrap_or_default();
        let missing_root = errors
            .iter()
            .any(|e| e.is_root() && e.explains_null(&wire.data));
        if idempotent
            && missing_root
            && let Some(blocked) = self.blocked(body).await
        {
            return Err(blocked);
        }
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

    /// Why GitHub's GraphQL API found nothing for a query of a repository
    /// (its `owner` and `name`), when it isn't that the repository is
    /// missing: GraphQL calls a blocked one missing; only REST says it's
    /// blocked, and why.
    async fn blocked(&self, body: &Value) -> Option<ApiError> {
        let vars = body.get("variables")?;
        let (owner, name) = (vars.get("owner")?.as_str()?, vars.get("name")?.as_str()?);
        let path = format!("/repos/{}/{}", encode_path(owner), encode_path(name));
        let response = self
            .send(&Request::Get {
                path: &path,
                cached: None,
            })
            .await;
        let response = response.ok()?;
        (response.status == StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS)
            .then(|| check_status(&response, "").err())
            .flatten()
    }

    /// GETs a REST path, revalidating with the cached copy's `ETag` and
    /// `Last-Modified`. A 304 serves the cached body (and doesn't count
    /// against the rate limit). `what` is
    /// what's asked for, to name if it isn't there.
    pub async fn rest_get(
        &self,
        path: &str,
        what: impl std::fmt::Display,
    ) -> Result<String, ApiError> {
        Ok(self.rest_fetch(path, what).await?.body)
    }

    /// [`Self::rest_get`], with the `Link` header that says where the next
    /// page is (kept with the cached copy, for a 304), and how soon GitHub
    /// asks to be asked again.
    async fn rest_fetch(
        &self,
        path: &str,
        what: impl std::fmt::Display,
    ) -> Result<HttpEntry, ApiError> {
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
                cached: cached.as_ref(),
            })
            .await?;
        let named = |name: &str| {
            let value = response.headers.get(name)?.to_str().ok();
            value.map(str::to_owned)
        };
        let poll_interval = named("x-poll-interval").and_then(|s| s.trim().parse().ok());
        if response.status == StatusCode::NOT_MODIFIED
            && let Some(cached) = cached
        {
            return Ok(HttpEntry {
                poll_interval,
                ..cached
            });
        }
        check_status(&response, what)?;
        let entry = HttpEntry {
            etag: named(header::ETAG.as_str()).unwrap_or_default(),
            last_modified: named(header::LAST_MODIFIED.as_str()),
            link: named(header::LINK.as_str()),
            body: response.body,
            poll_interval,
        };
        if !entry.etag.is_empty() || entry.last_modified.is_some() {
            self.store.http_put(path, &entry);
        }
        Ok(entry)
    }

    /// Asks whether a REST path's answer changed since the one tagged
    /// `etag`: GitHub answers 304 for free (it doesn't count against the
    /// rate limit) when it hasn't. Without an ETag, the answer is new.
    pub async fn poll(&self, path: &str, etag: Option<&str>) -> Result<Polled, ApiError> {
        let cached = etag.map(|etag| HttpEntry {
            etag: etag.to_owned(),
            ..HttpEntry::default()
        });
        let response = self
            .send(&Request::Get {
                path,
                cached: cached.as_ref(),
            })
            .await?;
        if response.status == StatusCode::NOT_MODIFIED && etag.is_some() {
            return Ok(Polled::Same);
        }
        check_status(&response, short_path(path))?;
        let etag = (response.headers.get(header::ETAG))
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        Ok(Polled::Changed {
            etag,
            body: response.body,
        })
    }

    /// Sends a REST change (a re-run, a cancel, marking a thread read),
    /// which only has to succeed. Never sent twice.
    pub(crate) async fn rest_write(
        &self,
        method: http::Method,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<(), ApiError> {
        (self.rest_write_answer(method, path, body).await).map(drop)
    }

    /// GETs a REST path and hands back GitHub's answer whatever its
    /// status, for a caller that reads a refusal itself (what a token's
    /// scopes lack). Not cached.
    pub(crate) async fn rest_answer(&self, path: &str) -> Result<Answer, ApiError> {
        Ok(Answer(
            self.send(&Request::Get { path, cached: None }).await?,
        ))
    }

    /// GETs a REST path: GitHub's answer, or, when it refused, what
    /// `refused` makes of the refusal from its status, GitHub's message
    /// and the answer (its headers say a token's scopes). A refusal it
    /// makes nothing of is the error its status means.
    pub(crate) async fn rest_refusable<E>(
        &self,
        path: &str,
        what: impl std::fmt::Display,
        refused: impl FnOnce(u16, &str, &Answer) -> Option<E>,
    ) -> Result<Result<Answer, E>, ApiError> {
        #[derive(Deserialize)]
        struct Message {
            message: Option<String>,
        }
        let answer = self.rest_answer(path).await?;
        let status = answer.status();
        if (200..300).contains(&status) {
            return Ok(Ok(answer));
        }
        let message = serde_json::from_str::<Message>(answer.body())
            .ok()
            .and_then(|m| m.message)
            .unwrap_or_default();
        if let Some(why) = refused(status, &message, &answer) {
            return Ok(Err(why));
        }
        answer.checked(&what)?;
        Err(ApiError::Decode(format!("{what}: status {status}")))
    }

    /// [`Self::rest_write`], for a change whose answer says what it made
    /// (a release, a gist).
    pub(crate) async fn rest_write_json<T: DeserializeOwned>(
        &self,
        method: http::Method,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<T, ApiError> {
        let answer = self.rest_write_answer(method, path, body).await?;
        Ok(serde_json::from_str(&answer)?)
    }

    /// [`Self::rest_write`], for a change GitHub answers with what it
    /// made: its answer (empty for 204).
    pub(crate) async fn rest_write_answer(
        &self,
        method: http::Method,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<String, ApiError> {
        let response = (self.send(&Request::Write { method, path, body })).await?;
        check_status(&response, short_path(path))?;
        Ok(response.body)
    }

    /// Saves what a REST path answers (following GitHub's redirect to its
    /// storage, without the token) to `dest`, as it arrives, through a
    /// temporary file beside it: an existing `dest` is replaced only if
    /// `replace`. Its size, in bytes.
    pub(crate) async fn download(
        &self,
        path: &str,
        dest: &std::path::Path,
        replace: bool,
    ) -> Result<u64, ApiError> {
        self.download_as(path, "application/vnd.github+json", dest, replace)
            .await
    }

    /// [`Self::download`], asking for the answer as `accept` (a release
    /// asset's bytes are `application/octet-stream`).
    pub(crate) async fn download_as(
        &self,
        path: &str,
        accept: &str,
        dest: &std::path::Path,
        replace: bool,
    ) -> Result<u64, ApiError> {
        let failed =
            |e: &dyn std::fmt::Display| ApiError::Network(format!("{}: {e}", dest.display()));
        let client = self.client().await?;
        let mut response = client
            .get(format!("{}{path}", self.http.rest))
            .header(header::ACCEPT, accept)
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let headers = HeaderMap::new();
            let answer = Response {
                status,
                headers,
                body,
            };
            check_status(&answer, short_path(path))?;
            return Err(ApiError::Network(format!("{path}: {status}")));
        }
        let dir = dest.parent().filter(|d| !d.as_os_str().is_empty());
        let dir = dir.unwrap_or_else(|| std::path::Path::new("."));
        let mut part = tempfile::NamedTempFile::new_in(dir).map_err(|e| failed(&e))?;
        let mut size = 0u64;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?
        {
            std::io::Write::write_all(&mut part, &chunk).map_err(|e| failed(&e))?;
            size += chunk.len() as u64;
        }
        // A temporary file is private; what's saved reads like any download.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let open = std::fs::Permissions::from_mode(0o644);
            part.as_file()
                .set_permissions(open)
                .map_err(|e| failed(&e))?;
        }
        let kept = if replace {
            part.persist(dest).map(drop)
        } else {
            part.persist_noclobber(dest).map(drop)
        };
        kept.map_err(|e| failed(&e.error))?;
        Ok(size)
    }

    /// Sends the file at `file` to `url` (a release's upload URL, on
    /// GitHub's uploads host), as it's read, as `content_type`: GitHub's
    /// answer. Never sent twice.
    pub(crate) async fn upload(
        &self,
        url: &str,
        content_type: &str,
        file: &std::path::Path,
    ) -> Result<String, ApiError> {
        let failed =
            |e: &dyn std::fmt::Display| ApiError::Network(format!("{}: {e}", file.display()));
        let client = self.client().await?;
        let opened = tokio::fs::File::open(file).await.map_err(|e| failed(&e))?;
        let size = opened.metadata().await.map_err(|e| failed(&e))?.len();
        let response = client
            .post(url)
            .header(header::CONTENT_TYPE, content_type)
            .header(header::CONTENT_LENGTH, size)
            .body(reqwest::Body::from(opened))
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        let answer = Response {
            status: response.status(),
            headers: response.headers().clone(),
            body: response
                .text()
                .await
                .map_err(|e| ApiError::Network(e.to_string()))?,
        };
        check_status(&answer, "the release's assets")?;
        Ok(answer.body)
    }

    /// GETs a REST path and decodes its JSON.
    async fn rest_json<T: DeserializeOwned>(
        &self,
        path: &str,
        what: impl std::fmt::Display,
    ) -> Result<T, ApiError> {
        Ok(serde_json::from_str(&self.rest_get(path, what).await?)?)
    }

    /// A page of a list GitHub pages by cursor (`after=`, in the `Link`
    /// header), and the cursor for the next page, if there is one.
    async fn rest_cursor_page<T: DeserializeOwned>(
        &self,
        path: &str,
        what: impl std::fmt::Display,
    ) -> Result<(T, Option<String>), ApiError> {
        let response = self.send(&Request::Get { path, cached: None }).await?;
        check_status(&response, what)?;
        let next = response
            .headers
            .get(header::LINK)
            .and_then(|v| v.to_str().ok())
            .and_then(next_after);
        Ok((serde_json::from_str(&response.body)?, next))
    }

    pub async fn viewer_login(&self) -> Result<String, ApiError> {
        let user: browse::UserLogin = self.rest_json("/user", "your account").await?;
        // Now the cache (and drafts) can be this account's.
        self.store.signed_in(&user.login);
        Ok(user.login)
    }

    /// The pull request, how far behind its base it is included.
    pub async fn pull_request(&self, pr: &PrRef) -> Result<PrDetail, ApiError> {
        let detail = self.pull_request_alone(pr).await?;
        Ok(self.with_behind(pr, detail).await)
    }

    /// The pull request, without how far behind its base it is: that's
    /// a second request ([`Self::with_behind`]).
    pub async fn pull_request_alone(&self, pr: &PrRef) -> Result<PrDetail, ApiError> {
        let op = queries::PullRequestQuery::build(number_vars(pr)?);
        self.find(op, pr, |q| PrDetail::from_wire(q.repository?))
            .await
    }

    /// `detail` with how far behind its base it is, if it's open.
    pub async fn with_behind(&self, pr: &PrRef, mut detail: PrDetail) -> PrDetail {
        // GitHub says a branch is behind only when the repository requires
        // it up to date; it may be behind anyway.
        if detail.state.is_open() {
            detail.behind_by = self.behind_by(pr, &detail).await;
        }
        detail
    }

    /// How many commits on its base `detail`'s branch doesn't have, if
    /// GitHub can compare them (its branch may be gone).
    async fn behind_by(&self, pr: &PrRef, detail: &PrDetail) -> Option<u64> {
        let op = queries::BehindQuery::build(queries::BehindVariables {
            owner: pr.repo.owner.clone(),
            name: pr.repo.name.clone(),
            base: format!("refs/heads/{}", detail.base_ref),
            head: detail.head_oid.clone(),
        });
        let behind = self.find(op, pr, |q| q.repository?.base?.compare).await;
        match behind {
            Ok(b) => u64::try_from(b.behind_by).ok(),
            Err(err) => {
                tracing::debug!(%pr, %err, "couldn't compare the branch with its base");
                None
            }
        }
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
                "/repos/{}/pulls/{}/files?per_page=100&page={page}",
                pr.repo, pr.number
            );
            let batch: Vec<PatchFile> = self.rest_json(&path, format!("{pr}'s files")).await?;
            let next = (batch.len() == 100).then(|| page.saturating_add(1).to_string());
            Ok(browse::Results::uncounted(batch, next))
        };
        let what = format!("{pr}'s patches");
        let files = self.more_pages(what, page(None).await?, FILE_PAGES, page);
        Ok(files.await?.items)
    }

    /// The PR's node ID, and your pending review on it, if any.
    pub async fn pending_review(
        &self,
        pr: &PrRef,
    ) -> Result<(NodeId, Option<PendingReview>), ApiError> {
        let op = queries::PendingReviewQuery::build(number_vars(pr)?);
        let pr_node = self.find(op, pr, |q| q.repository?.pull_request).await?;
        let review = nodes(pr_node.reviews.and_then(|r| r.nodes))
            .next()
            .map(|r| PendingReview {
                id: r.id.into(),
                commit: r.commit.map(|c| c.oid.0),
                comments: crate::api::model::count(r.comments.total_count),
            });
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
            .find(|r| r.state != crate::api::model::ReviewState::Pending)
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

    /// A value [`Self::remember`]ed under `key`.
    pub fn cached<T: DeserializeOwned>(&self, key: &str) -> Option<Cached<T>> {
        self.store.query_get(key)
    }

    /// A repository's overview: stats, root directory, last commit.
    pub async fn repo(&self, repo: &RepoId) -> Result<browse::RepoOverview, ApiError> {
        let op = browse::RepoQuery::build(browse::RepoVariables::at(repo, "HEAD:"));
        self.find(op, repo, |q| q.repository?.into_overview()).await
    }

    /// The README GitHub shows for the repository root, if it has one.
    pub async fn readme(&self, repo: &RepoId) -> Result<Option<browse::Readme>, ApiError> {
        use base64::Engine;
        #[derive(serde::Deserialize)]
        struct Wire {
            path: String,
            content: String,
        }
        let path = format!("/repos/{repo}/readme");
        let wire: Wire = match self.rest_json(&path, format!("{repo}'s README")).await {
            Err(ApiError::NotFound(_)) => return Ok(None),
            wire => wire?,
        };
        let compact: String = wire.content.split_whitespace().collect();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(compact)
            .map_err(|e| ApiError::Decode(e.to_string()))?;
        let readme = browse::Readme {
            path: wire.path,
            text: String::from_utf8_lossy(&bytes).into_owned(),
        };
        Ok(Some(readme))
    }

    /// A directory at `rev:path`.
    pub async fn tree(
        &self,
        repo: &RepoId,
        rev: &str,
        path: &str,
    ) -> Result<Vec<browse::TreeEntry>, ApiError> {
        match self.object(repo, rev, path).await? {
            browse::GitObject::Tree(t) => Ok(browse::entries(t)),
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
                size: crate::api::model::count(b.byte_size),
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
        let op =
            browse::ObjectQuery::build(browse::RepoVariables::at(repo, format!("{rev}:{path}")));
        self.find(op, format!("{repo}/{path}"), |q| q.repository?.object)
            .await
    }

    /// One page of search results: [`Self::search_showing`], whole.
    pub async fn search(
        &self,
        kind: browse::SearchKind,
        query: &str,
        after: Option<String>,
    ) -> Result<browse::SearchResults, ApiError> {
        self.search_showing(kind, query, after, |_| {}).await
    }

    /// How many issues or pull requests `query` finds open and closed, in
    /// one request.
    pub async fn state_counts(&self, query: &str) -> Result<browse::StateCounts, ApiError> {
        let op = browse::StateCountsQuery::build(browse::StateCountsVariables {
            open: format!("{query} is:open"),
            closed: format!("{query} is:closed"),
        });
        let found = self.graphql(op).await?;
        let count = crate::api::model::count;
        Ok(browse::StateCounts {
            open: count(found.open.issue_count),
            closed: count(found.closed.issue_count),
        })
    }

    /// One page of search results (30 per page). GitHub gives a request
    /// 10 seconds and answers 502 past them, and a busy account's search
    /// that also reads each pull request's checks runs out of them (25
    /// with checks takes 5–6s across an organization like microsoft's, 50
    /// times out). So the search reads none, `found` sees its page at
    /// once, and the checks of the pull requests on it come from a second
    /// request by ID, which takes 1–4s for 30. A search GitHub finds
    /// nothing for is asked of REST too, which says why when it's a
    /// mistake (a user or repository that isn't there) where GraphQL
    /// quietly finds nothing.
    pub async fn search_showing(
        &self,
        kind: browse::SearchKind,
        query: &str,
        after: Option<String>,
        found: impl FnOnce(&browse::SearchResults) + Send,
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
                return Ok(results);
            }
        };
        let api_query = if query.contains("is:issue") || query.contains("is:pr") {
            query.to_owned()
        } else {
            format!("{query}{is}")
        };
        let mut results = self.search_as(kind, search_type, &api_query, after).await?;
        if first_page
            && results.counts().0 == 0
            && kind != Kind::Users
            && let Some(refused) = self.refusal(kind, &api_query).await
        {
            return Err(refused);
        }
        let prs: Vec<_> = match &results {
            browse::SearchResults::Issues(r) => (r.items.iter())
                .filter_map(|i| match &i.kind {
                    browse::IssueKind::Pr { id, .. } => Some(id.clone()),
                    browse::IssueKind::Issue(_) => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        if !prs.is_empty() {
            found(&results);
            let found = self.checks_of(&prs).await;
            if let browse::SearchResults::Issues(r) = &mut results {
                for item in &mut r.items {
                    if let browse::IssueKind::Pr { id, checks, .. } = &mut item.kind {
                        *checks = checks.or_else(|| found.get(id.as_str()).copied());
                    }
                }
            }
        }
        Ok(results)
    }

    /// The checks of these pull requests, by ID. Checks are a decoration:
    /// without them, the rows still show, so a failure is only logged.
    async fn checks_of(&self, ids: &[NodeId]) -> std::collections::HashMap<String, ChecksState> {
        let ids = ids.iter().map(NodeId::gql).collect();
        let op = queries::RollupsQuery::build(queries::NodesVariables { ids });
        match self.graphql(op).await {
            Ok(q) => (q.nodes.into_iter().flatten())
                .filter_map(|node| match node {
                    queries::RollupNode::PullRequest(pr) => {
                        Some((pr.id.into_inner(), crate::api::model::checks(&pr.commits)?))
                    }
                    queries::RollupNode::Other => None,
                })
                .collect(),
            Err(err) => {
                tracing::warn!(%err, "couldn't read the found pull requests' checks");
                std::collections::HashMap::new()
            }
        }
    }

    /// GitHub's reason for refusing `query`, which GraphQL doesn't give:
    /// asked of REST's search, which answers 422 with it.
    async fn refusal(&self, kind: browse::SearchKind, query: &str) -> Option<ApiError> {
        let what = match kind {
            browse::SearchKind::Repos => "repositories",
            _ => "issues",
        };
        let path = format!("/search/{what}?q={}&per_page=1", encode_query(query));
        match self.rest_get(&path, "the search").await {
            Err(err @ ApiError::Http { status: 422, .. }) => Some(err),
            Err(err) => {
                tracing::debug!(%err, "couldn't ask REST why a search found nothing");
                None
            }
            Ok(_) => None,
        }
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
            return Ok(if kind == Kind::Commits {
                SearchResults::Commits(Results::default())
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
        use browse::rest_search::{Code, Commit};
        Ok(if kind == Kind::Commits {
            SearchResults::Commits(self.rest_hits(&path, next, Commit::into_hit).await?)
        } else {
            SearchResults::Code(self.rest_hits(&path, next, Code::into_hit).await?)
        })
    }

    /// A page of a REST search at `path`, each item made a hit by `hit`.
    async fn rest_hits<W: DeserializeOwned, T>(
        &self,
        path: &str,
        next: impl Fn(u64) -> Option<String>,
        hit: fn(W) -> Option<T>,
    ) -> Result<browse::Results<T>, ApiError> {
        let page: browse::rest_search::Page<W> = self.rest_json(path, "the search").await?;
        self.note_incomplete(page.incomplete_results);
        Ok(browse::Results {
            next: next(page.total_count),
            total: page.total_count,
            items: page.items.into_iter().filter_map(hit).collect(),
        })
    }

    /// A page of a search, as GitHub's API takes it, with each item's pull
    /// request ID (for issues and pull requests).
    async fn search_as(
        &self,
        kind: browse::SearchKind,
        search_type: browse::SearchType,
        query: &str,
        after: Option<String>,
    ) -> Result<browse::SearchResults, ApiError> {
        use browse::{BrowseItem, Results, SearchKind as Kind, SearchResults};
        let op = browse::BrowseSearch::build(browse::BrowseSearchVariables {
            query: query.to_owned(),
            kind: search_type,
            first: 30,
            after,
        });
        let conn = self.graphql(op).await?.search;
        let next = conn.page_info.next();
        let items = nodes(conn.nodes);
        let count = crate::api::model::count;
        Ok(match kind {
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
        })
    }

    /// An issue; `None` when the number is a pull request (GitHub redirects
    /// those).
    pub async fn issue(
        &self,
        repo: &RepoId,
        number: u64,
    ) -> Result<Option<browse::IssueDetail>, ApiError> {
        let at = PrRef {
            repo: repo.clone(),
            number,
        };
        let op = browse::IssueQuery::build(number_vars(&at)?);
        let pick = |q: browse::IssueQuery| match q.repository?.issue_or_pull_request? {
            browse::IssueOrPr::Issue(issue) => issue.into_detail().map(Some),
            browse::IssueOrPr::PullRequest(_) => Some(None),
            browse::IssueOrPr::Other => None,
        };
        self.find(op, at, pick).await
    }

    /// A pull request's timeline (comments, reviews, events), commits and
    /// review threads: the threads apart, in pages, at the same time.
    pub async fn pr_activity(&self, pr: &PrRef) -> Result<browse::PrActivity, ApiError> {
        let op = browse::PrActivityQuery::build(number_vars(pr)?);
        let activity = self.find(op, pr, |q| {
            Some(q.repository?.pull_request?.into_activity())
        });
        let (activity, threads) = tokio::try_join!(activity, self.review_threads(pr))?;
        Ok(browse::PrActivity {
            threads,
            ..activity
        })
    }

    /// All of a pull request's reviews, oldest first.
    pub async fn reviews(&self, pr: &PrRef) -> Result<Capped<timeline::Review>, ApiError> {
        let page = move |after| async move {
            let op = timeline::ReviewsQuery::build(page_vars(pr, after)?);
            let page = self
                .find(op, pr, |q| q.repository?.pull_request?.reviews)
                .await?;
            let reviews = nodes(page.nodes).map(timeline::WireReview::into_review);
            let reviews = reviews.collect();
            Ok(browse::Results::uncounted(reviews, page.page_info.next()))
        };
        let what = format!("{pr}'s reviews");
        let reviews = self.more_pages(what, page(None).await?, FILE_PAGES, page);
        reviews.await
    }

    /// A user's or organization's profile.
    pub async fn profile(&self, login: &str) -> Result<browse::Profile, ApiError> {
        let op = browse::ProfileQuery::build(browse::ProfileVariables {
            login: login.to_owned(),
        });
        self.find(op, login, browse::ProfileQuery::into_profile)
            .await
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
            .rest_json(
                &format!("/repos/{repo}/git/trees/{}?recursive=1", encode_path(rev)),
                format!("{repo}@{rev}"),
            )
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
        Ok(out)
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
        let op = browse::CommitQuery::build(browse::RepoVariables::at(repo, rev));
        let pick = |q: browse::CommitQuery| match q.repository?.object? {
            browse::CommitObject::Commit(c) => Some(c.into_detail()),
            browse::CommitObject::Other => None,
        };
        self.find(op, format!("{repo}@{rev}"), pick).await
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
        Ok(self.checked(history, format!("{repo}@{rev}'s history")))
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
        Ok(checks)
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
        let first = browse::rollup_contexts(commit.status_check_rollup);
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

    /// A page of people, 30 at a time from `after`; the first is cached.
    pub async fn users(
        &self,
        list: &browse::UserList,
        after: Option<String>,
    ) -> Result<browse::Results<browse::UserSummary>, ApiError> {
        use browse::UserList as L;
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
        Ok(wire.into_results(browse::wire::Person::into_summary))
    }

    /// A user's or organization's own repositories in `sort` order, 30 at a
    /// time from `after`; the first page is cached.
    pub async fn owner_repos(
        &self,
        login: &str,
        sort: browse::RepoSort,
        after: Option<String>,
    ) -> Result<browse::Results<browse::RepoSummary>, ApiError> {
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
        Ok(self.checked(repos, format!("{login}'s repositories")))
    }

    /// What a user starred, most recently first, 30 at a time from
    /// `after`; the first page is cached.
    pub async fn starred(
        &self,
        login: &str,
        after: Option<String>,
    ) -> Result<browse::Results<browse::RepoSummary>, ApiError> {
        let op = browse::StarredQuery::build(browse::LoginPageVariables {
            login: login.to_owned(),
            after,
        });
        let stars = self
            .find(op, login, |q| q.user)
            .await?
            .starred_repositories
            .into_results();
        Ok(self.checked(stars, format!("{login}'s stars")))
    }

    /// A repository's forks, most starred first, 30 at a time from
    /// `after`; the first page is cached.
    pub async fn forks(
        &self,
        repo: &RepoId,
        after: Option<String>,
    ) -> Result<browse::Results<browse::RepoSummary>, ApiError> {
        let op = browse::ForksQuery::build(list_vars(repo, after));
        let forks = self
            .find(op, repo, |q| q.repository)
            .await?
            .forks
            .into_results();
        Ok(self.checked(forks, format!("{repo}'s forks")))
    }

    /// A repository's releases, newest first, 20 at a time from `after`;
    /// the first page is cached.
    pub async fn releases(
        &self,
        repo: &RepoId,
        after: Option<String>,
    ) -> Result<browse::Results<browse::Release>, ApiError> {
        let op = browse::ReleasesQuery::build(list_vars(repo, after));
        let releases = self
            .find(op, repo, |q| q.repository)
            .await?
            .releases
            .into_results();
        Ok(self.checked(releases, format!("{repo}'s releases")))
    }

    /// A release by its tag, with its notes and assets.
    pub async fn release(&self, repo: &RepoId, tag: &str) -> Result<browse::Release, ApiError> {
        let op = browse::ReleaseQuery::build(browse::ReleaseVariables {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
            tag: tag.to_owned(),
        });
        let release = self.find(op, format!("{repo} release {tag}"), |q| {
            q.repository?.release
        });
        Ok(release.await?.into_release())
    }

    /// A repository's tags, 30 at a time from `after`; the first page is
    /// cached.
    pub async fn tags(
        &self,
        repo: &RepoId,
        after: Option<String>,
    ) -> Result<browse::Results<browse::TagInfo>, ApiError> {
        let op = browse::TagsQuery::build(list_vars(repo, after));
        let tags = self
            .find(op, repo, |q| q.repository?.refs)
            .await?
            .into_results();
        Ok(self.checked(tags, format!("{repo}'s tags")))
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
        let vars = serde_json::json!({ "owner": repo.owner, "name": repo.name, "after": after });
        let wire: Wire = self
            .graphql_json(raw::BRANCHES, vars, repo, "/repository")
            .await?;
        self.check_connection(&wire.refs, 30, format!("{repo}'s branches"));
        let default = wire.default_branch_ref.map(|r| r.name);
        Ok((wire.refs).into_results(|b| b.into_info(repo, default.as_deref())))
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
        let query = raw::milestones(closed);
        let vars = serde_json::json!({ "owner": repo.owner, "name": repo.name, "after": after });
        let wire: Wire = self.graphql_json(&query, vars, repo, "/repository").await?;
        self.check_connection(&wire.milestones, 25, format!("{repo}'s milestones"));
        let list = browse::MilestoneList {
            open: wire.open.total_count,
            closed: wire.closed.total_count,
            results: (wire.milestones).into_results(browse::wire_milestones::Milestone::into_info),
        };
        Ok(self.checked(list, format!("{repo}'s milestones")))
    }

    /// A milestone, and 30 of its issues and pull requests from `after`,
    /// most recently updated first.
    pub async fn milestone(
        &self,
        repo: &RepoId,
        number: u64,
        after: Option<String>,
    ) -> Result<browse::MilestoneDetail, ApiError> {
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
                    &search,
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
        Ok(self.checked(detail, format!("{repo}'s milestone {number}")))
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
        Ok(self.checked(list, format!("{repo}'s deployments")))
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
            "/repos/{repo}/compare/{}...{}",
            encode_path(base),
            encode_path(head)
        );
        let what = format!("{repo}'s comparison {spec}");
        let wire: browse::rest_compare::Compare = self.rest_json(&path, what).await?;
        let head = wire
            .permalink_url
            .rsplit_once("...")
            .and_then(|(_, head)| head.rsplit(':').next())
            .filter(|head| !head.is_empty())
            .map(str::to_owned);
        // Counted on the wire: the comparison's list holds no more than its total.
        let (listed, total) = (wire.commits.len() as u64, wire.total_commits);
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
        let expected = total.min(browse::COMPARE_COMMITS);
        if listed != expected {
            self.doubt(format!(
                "comparing {spec}, {listed} commits are listed of {total}, not {expected}"
            ));
        }
        Ok(comparison)
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
        Ok(wire.into_blame())
    }
    /// A gist and its files. REST, since GraphQL finds gists only through
    /// their owners, and a gist's URL may not name its owner.
    pub async fn gist(&self, id: &str) -> Result<browse::Gist, ApiError> {
        let wire: browse::rest_gists::Gist = self
            .rest_json(&format!("/gists/{}", encode_path(id)), format!("gist {id}"))
            .await?;
        Ok(wire.into_gist())
    }

    /// Someone's public gists, most recently updated first, 30 at a time
    /// from `after`.
    pub async fn gists(
        &self,
        login: &str,
        after: Option<String>,
    ) -> Result<browse::Results<browse::GistSummary>, ApiError> {
        let vars = serde_json::json!({ "login": login, "after": after });
        let wire: browse::wire::Connection<browse::wire_gists::Gist> =
            (self.graphql_json(raw::GISTS, vars, format!("{login}'s gists"), "/user/gists"))
                .await?;
        self.check_connection(&wire, 30, format!("{login}'s gists"));
        Ok(wire.into_results(browse::wire_gists::Gist::into_summary))
    }
    /// The teams in an organization you can see (its members see them),
    /// by name, 30 at a time from `after`.
    pub async fn teams(
        &self,
        org: &str,
        after: Option<String>,
    ) -> Result<browse::Results<browse::TeamSummary>, ApiError> {
        let vars = serde_json::json!({ "org": org, "after": after });
        let wire: browse::wire::Connection<browse::wire_teams::Team> =
            (self.graphql_json(&raw::teams(), vars, org, "/organization/teams")).await?;
        self.check_connection(&wire, 30, format!("{org}'s teams"));
        Ok(wire.into_results(browse::wire_teams::Team::into_summary))
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
        Ok(wire.into_detail())
    }
    /// A repository's published security advisories, or GitHub's newest
    /// reviewed ones, up to 100. REST: repositories' advisories aren't in
    /// GraphQL, and one shape serves both.
    pub async fn advisories(
        &self,
        repo: Option<&RepoId>,
        after: Option<String>,
    ) -> Result<browse::Results<browse::Advisory>, ApiError> {
        let mut path = match repo {
            Some(r) => {
                format!("/repos/{r}/security-advisories?state=published&per_page={REST_PAGE}")
            }
            None => format!("/advisories?type=reviewed&per_page={REST_PAGE}"),
        };
        if let Some(after) = &after {
            path.push_str(&format!("&after={}", encode_query(after)));
        }
        let (wire, next): (Vec<browse::rest_advisories::Advisory>, _) =
            self.rest_cursor_page(&path, "the advisories").await?;
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
        Ok(self.checked(list, "the advisories"))
    }

    /// A security advisory: a repository's, or from GitHub's database.
    pub async fn advisory(
        &self,
        repo: Option<&RepoId>,
        ghsa: &str,
    ) -> Result<browse::Advisory, ApiError> {
        let id = encode_path(ghsa);
        let path = match repo {
            Some(r) => format!("/repos/{r}/security-advisories/{id}"),
            None => format!("/advisories/{id}"),
        };
        let wire: browse::rest_advisories::Advisory = self.rest_json(&path, ghsa).await?;
        Ok(wire.into_advisory())
    }
    /// A workflow run (one attempt of it, or the latest) and its jobs.
    pub async fn workflow_run(
        &self,
        repo: &RepoId,
        run: u64,
        attempt: Option<u64>,
    ) -> Result<browse::WorkflowRun, ApiError> {
        let base = format!("/repos/{repo}/actions/runs/{run}");
        let path = match attempt {
            Some(n) => format!("{base}/attempts/{n}"),
            None => base,
        };
        let jobs_path = &format!("{path}/jobs?per_page=100");
        let (asked, jobs_asked) = (format!("{repo}'s run {run}"), format!("run {run}'s jobs"));
        let (wire, jobs) = tokio::join!(
            self.rest_json::<browse::rest_actions::Run>(&path, &asked),
            self.rest_json::<browse::rest_actions::Jobs>(jobs_path, &jobs_asked)
        );
        let wire = wire?;
        // A run just started has no jobs, nor artifacts, for GitHub to
        // find for a moment: none yet.
        let starting =
            browse::CheckOutcome::of(wire.status, wire.conclusion) == browse::CheckOutcome::Pending;
        let jobs = match jobs {
            Err(ApiError::NotFound(_)) if starting => Ok(browse::rest_actions::Jobs {
                jobs: Vec::new(),
                total_count: 0,
            }),
            jobs => jobs,
        };
        // Big matrices have more than a page of jobs; an empty page is the last.
        let results = |page: u64, jobs: browse::rest_actions::Jobs| browse::Results {
            next: next_page(page, 100, jobs.total_count).filter(|_| !jobs.jobs.is_empty()),
            total: jobs.total_count,
            items: jobs.jobs,
        };
        let page = move |after: Option<String>| async move {
            let page = rest_page(after.as_deref());
            let jobs_page = format!("{jobs_path}&page={page}");
            let jobs = self
                .rest_json(&jobs_page, format!("run {run}'s jobs"))
                .await?;
            Ok(results(page, jobs))
        };
        let what = format!("run {run}'s jobs");
        let (jobs, artifacts) = tokio::join!(
            self.more_pages(what, results(1, jobs?), PAGES, page),
            self.artifacts(repo, run)
        );
        let mut run = wire.into_run(jobs?);
        run.artifacts = match artifacts {
            Err(ApiError::NotFound(_)) if starting => Capped::default(),
            artifacts => artifacts?,
        };
        Ok(run)
    }

    /// A job, its steps, and its run as it is now.
    pub async fn job(&self, repo: &RepoId, job: u64) -> Result<browse::Job, ApiError> {
        let wire = self.job_wire(repo, job).await?;
        let base = format!("/repos/{repo}/actions/runs/{}", wire.run_id);
        let run: browse::rest_actions::Run = self
            .rest_json(&base, format!("{repo}'s run {}", wire.run_id))
            .await?;
        let (attempt, latest) = (wire.run_attempt.unwrap_or(1), run.run_attempt.unwrap_or(1));
        // Re-run, it's a new job with the same name in a later attempt.
        let rerun = if latest > attempt {
            let path = format!("{base}/attempts/{latest}/jobs?per_page=100");
            let jobs: browse::rest_actions::Jobs = self
                .rest_json(&path, format!("{repo}'s run {}'s jobs", wire.run_id))
                .await?;
            (jobs.jobs.into_iter())
                .find(|j| j.name == wire.name && j.id != wire.id)
                .map(|j| j.id)
        } else {
            None
        };
        let mut job = wire.into_job(run.into_job_run(rerun));
        // A running job's annotations come with its steps' ends: asked once
        // it's done.
        if job.outcome != browse::CheckOutcome::Pending {
            job.annotations = self.job_annotations(repo, job.id).await?;
        }
        Ok(job)
    }

    async fn job_wire(
        &self,
        repo: &RepoId,
        job: u64,
    ) -> Result<browse::rest_actions::Job, ApiError> {
        let path = format!("/repos/{repo}/actions/jobs/{job}");
        self.rest_json(&path, format!("{repo}'s job {job}")).await
    }

    /// A job's log (see [`browse::JobLog`]), never cached: logs are big.
    /// A running job has none yet (GitHub writes it when the job ends), and
    /// one past the repository's retention has expired.
    pub async fn job_log(&self, repo: &RepoId, job: u64) -> Result<browse::JobLog, ApiError> {
        if self.job_wire(repo, job).await?.outcome() == browse::CheckOutcome::Pending {
            return Ok(browse::JobLog {
                running: true,
                ..browse::JobLog::default()
            });
        }
        let path = format!("/repos/{repo}/actions/jobs/{job}/logs");
        let response = self
            .send(&Request::Get {
                path: &path,
                cached: None,
            })
            .await?;
        if response.status == StatusCode::GONE {
            return Err(ApiError::Gone(
                "this log has expired (GitHub keeps logs for the repository's retention period, 90 days by default)".into(),
            ));
        }
        check_status(&response, format!("{repo}'s job {job}'s log"))?;
        let (cut, text) = browse::log::cut(&response.body);
        Ok(browse::JobLog {
            text: text.to_owned(),
            cut,
            running: false,
        })
    }

    /// A workflow, by its file name (`ci.yml`) or ID.
    pub async fn workflow(&self, repo: &RepoId, file: &str) -> Result<browse::Workflow, ApiError> {
        let path = format!("/repos/{repo}/actions/workflows/{}", encode_path(file));
        let what = format!("{repo}'s workflow {file}");
        let wire: browse::rest_actions::Workflow = self.rest_json(&path, what).await?;
        let workflow = browse::Workflow {
            name: wire.name,
            path: wire.path,
            disabled: Some(wire.state)
                .filter(|s| s != "active")
                .map(|s| s.replace('_', " ")),
        };
        Ok(workflow)
    }

    /// A repository's workflow runs that match `query` (see
    /// [`browse::RunFilter`]; the words it can't use are left out), newest
    /// first, 30 at a time; `after` is the page number to fetch (REST pages
    /// by number). The first page is cached.
    pub async fn runs(
        &self,
        repo: &RepoId,
        query: &str,
        after: Option<String>,
    ) -> Result<browse::Results<browse::RunSummary>, ApiError> {
        let page = rest_page(after.as_deref());
        let (path, what) = self.runs_path(repo, query, page).await?;
        let wire: browse::rest_actions::Runs = self.rest_json(&path, what).await?;
        let runs = browse::Results {
            total: wire.total_count,
            items: wire
                .workflow_runs
                .into_iter()
                .map(browse::rest_actions::Run::into_summary)
                .collect(),
            next: next_page(page, REST_PAGE, wire.total_count),
        };
        Ok(self.checked(runs, format!("{repo}'s runs")))
    }

    /// Where page `page` of the runs `query` filters is (see
    /// [`browse::RunFilter`]), and what it's called: a workflow named by
    /// its name is looked up first.
    pub async fn runs_path(
        &self,
        repo: &RepoId,
        query: &str,
        page: u64,
    ) -> Result<(String, String), ApiError> {
        let filter = browse::RunFilter::parse(query);
        let base = format!("/repos/{repo}/actions");
        let (list, what) = match &filter.workflow {
            Some(w) => {
                let id = encode_path(&self.workflow_id(repo, w).await?);
                (
                    format!("{base}/workflows/{id}/runs"),
                    format!("{repo}'s workflow {w}"),
                )
            }
            None => (format!("{base}/runs"), format!("{repo}'s workflow runs")),
        };
        let mut params = url::form_urlencoded::Serializer::new(String::new());
        params.extend_pairs(&filter.params);
        params.append_pair("per_page", &REST_PAGE.to_string());
        params.append_pair("page", &page.to_string());
        Ok((format!("{list}?{}", params.finish()), what))
    }

    /// Asks whether a discussion changed since the answer `seen` (its
    /// title, text, answer, and its comments' count and newest, as
    /// [`raw::DISCUSSION_PROBE`] has them). GraphQL has no ETags: the
    /// answer is its own tag, and each ask costs a point of the rate limit.
    pub async fn poll_discussion(
        &self,
        repo: &RepoId,
        number: u64,
        seen: Option<&str>,
    ) -> Result<Polled, ApiError> {
        let vars = serde_json::json!({ "owner": repo.owner, "name": repo.name, "number": number });
        let subject = format!("{repo} discussion {number}");
        let node: Value = self
            .graphql_json(
                raw::DISCUSSION_PROBE,
                vars,
                subject,
                "/repository/discussion",
            )
            .await?;
        let body = node.to_string();
        Ok(if seen == Some(body.as_str()) {
            Polled::Same
        } else {
            Polled::Changed {
                etag: body.clone(),
                body,
            }
        })
    }

    /// Your notifications that `query` keeps (see
    /// [`notifications::InboxFilter`]), most recently updated first, a page
    /// at a time; `after` is the page number (REST pages by number). GitHub
    /// doesn't count them: a page says only whether another follows. A
    /// filter applied here can leave a page nearly empty, so pages are
    /// fetched until one fills or the inbox ends, [`INBOX_PAGES`] at most.
    /// The first page is cached.
    pub async fn notifications(
        &self,
        query: &str,
        after: Option<String>,
    ) -> Result<notifications::Inbox, ApiError> {
        let filter = notifications::InboxFilter::parse(query);
        let first = rest_page(after.as_deref());
        let what = match &filter.repo {
            Some(repo) => format!("your notifications from {repo}"),
            None => "your notifications".to_owned(),
        };
        let mut done = self.done_threads().await;
        let (mut items, mut searched, mut page, mut more) = (Vec::new(), 0, first, false);
        let mut forgot = false;
        for _ in 0..INBOX_PAGES {
            let fetched = self
                .rest_fetch(&filter.path(page, REST_PAGE), &what)
                .await?;
            let wire: Vec<notifications::rest::Thread> = serde_json::from_str(&fetched.body)?;
            searched += wire.len() as u64;
            let threads: Vec<notifications::Notification> = (wire.into_iter())
                .filter_map(notifications::rest::Thread::into_notification)
                .collect();
            // A thread marked done that GitHub updated since is back.
            forgot |= done.forget_updated(&threads);
            items.extend((threads.into_iter()).filter(|n| filter.keeps(n) && !done.hides(n)));
            more = fetched.link.as_deref().is_some_and(has_next);
            page = page.saturating_add(1);
            let full = items.len() as u64 >= REST_PAGE;
            if !more || full || !filter.after_fetching() {
                break;
            }
        }
        if forgot {
            self.remember(notifications::Done::KEY, &done);
        }
        let cut = more && filter.after_fetching() && (items.len() as u64) < REST_PAGE;
        let inbox = notifications::Inbox {
            list: browse::Results {
                total: items.len() as u64,
                items,
                next: more.then(|| page.to_string()),
            },
            searched,
            cut,
        };
        Ok(self.checked(inbox, "your notifications"))
    }

    /// The threads this account marked done (see
    /// [`notifications::Done`]).
    async fn done_threads(&self) -> notifications::Done {
        let store = self.store.clone();
        let read = move || store.query_get::<notifications::Done>(notifications::Done::KEY);
        let found = tokio::task::spawn_blocking(read).await.ok().flatten();
        found.map(|c| c.value).unwrap_or_default()
    }

    /// Keeps that `threads` are marked done, so the inbox leaves them out
    /// until GitHub updates them again.
    pub(crate) async fn remember_done(&self, threads: &[notifications::ThreadAt]) {
        if threads.is_empty() {
            return;
        }
        let mut done = self.done_threads().await;
        done.mark(threads);
        self.remember(notifications::Done::KEY, &done);
    }

    /// How many of your notifications are unread, up to a page of them.
    /// Asked only if the inbox changed since it was last asked, so polling
    /// an unchanged one costs nothing; GitHub says how often to poll.
    pub async fn unread(&self) -> Result<notifications::Unread, ApiError> {
        let path = format!("/notifications?per_page={UNREAD_PAGE}");
        let fetched = self.rest_fetch(&path, "your notifications").await?;
        let threads: Vec<serde::de::IgnoredAny> = serde_json::from_str(&fetched.body)?;
        Ok(notifications::Unread {
            count: threads.len() as u64,
            more: fetched.link.as_deref().is_some_and(has_next),
            poll_secs: fetched.poll_interval.unwrap_or(UNREAD_POLL_SECS),
        })
    }

    /// A workflow's ID for REST: its file name or ID as it is, else found
    /// by its name.
    async fn workflow_id(&self, repo: &RepoId, workflow: &str) -> Result<String, ApiError> {
        let file = workflow.ends_with(".yml") || workflow.ends_with(".yaml");
        if file || workflow.bytes().all(|b| b.is_ascii_digit()) {
            return Ok(workflow.to_owned());
        }
        let path = format!("/repos/{repo}/actions/workflows?per_page=100");
        let list: browse::rest_actions::Workflows =
            self.rest_json(&path, format!("{repo}'s workflows")).await?;
        let shown = list.workflows.len() as u64;
        let found = (list.workflows.into_iter()).find(|w| w.name.eq_ignore_ascii_case(workflow));
        match found {
            Some(w) => Ok(w.id.to_string()),
            None if shown < list.total_count => Err(ApiError::NotFound(format!(
                "a workflow named “{workflow}” in {repo}'s first {shown} of {}",
                list.total_count
            ))),
            None => Err(ApiError::NotFound(format!(
                "a workflow named “{workflow}” in {repo}"
            ))),
        }
    }

    /// The checks on a commit, by a revision GitHub can resolve.
    pub async fn commit_checks(
        &self,
        repo: &RepoId,
        rev: &str,
    ) -> Result<browse::Checks, ApiError> {
        let op = browse::CommitChecksQuery::build(browse::RepoVariables::at(repo, rev));
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
        Ok(checks)
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
            "{}/orgs/{}/discussions/",
            self.http.host.web(),
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
        Ok(self.checked(list, "the discussions"))
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
            (self.graphql_json(&raw::discussion(), vars, subject, "/repository/discussion"))
                .await?;
        Ok(wire.into_detail(repo))
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
        Ok(browse::Refs { branches, tags })
    }

    /// The ref a plain GitHub URL's `spot` (`feature/x/src/lib.rs`)
    /// starts with, as GitHub reads it: the longest leading run of
    /// segments that names a branch or tag, in one query. `None` when no
    /// run of two or more does: the ref is the first segment (a one-word
    /// branch, a SHA). Refs deeper than [`REF_DEPTH`] segments aren't
    /// looked for.
    pub async fn ref_in(&self, repo: &RepoId, spot: &str) -> Result<Option<String>, ApiError> {
        let segments: Vec<&str> = spot.split('/').collect();
        let names: Vec<String> = (2..=segments.len().min(REF_DEPTH))
            .filter_map(|n| segments.get(..n).map(|s| s.join("/")))
            .collect();
        if names.is_empty() {
            return Ok(None);
        }
        let mut vars = serde_json::Map::new();
        vars.insert("owner".into(), Value::from(repo.owner.as_str()));
        vars.insert("name".into(), Value::from(repo.name.as_str()));
        for (i, name) in names.iter().enumerate() {
            vars.insert(format!("r{i}"), Value::from(name.as_str()));
        }
        let found: std::collections::HashMap<String, Option<browse::wire::Name>> = self
            .graphql_json(
                &raw::refs_named(names.len()),
                vars.into(),
                repo,
                "/repository",
            )
            .await?;
        Ok(names
            .iter()
            .enumerate()
            .rev()
            .find(|(i, _)| found.get(&format!("r{i}")).is_some_and(Option::is_some))
            .map(|(_, name)| name.clone()))
    }

    /// The cache this client keeps its answers in.
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Saves a small value in the cache (e.g. recently visited pages).
    pub fn remember<T: Serialize>(&self, key: &str, value: &T) {
        self.store.query_put(key, value);
    }
}

/// A query whose root GitHub never answers with null (`search`,
/// `viewer`), so [`GitHub::graphql`] may take its data whole. A claim
/// about GitHub's schema: a query for one thing goes through
/// [`GitHub::find`] instead.
trait Unrooted {}
impl Unrooted for queries::RollupsQuery {}
impl Unrooted for browse::BrowseSearch {}
impl Unrooted for browse::StateCountsQuery {}

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
    /// Whether it's about one of the query's roots (`repository`, or an
    /// alias of it).
    fn is_root(&self) -> bool {
        self.path.as_ref().is_some_and(|p| p.len() == 1)
    }

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

/// Where an Enterprise Server says its version.
const META: &str = "/meta";

/// How many items a REST list page holds here.
const REST_PAGE: u64 = 30;
/// GitHub's search serves its first thousand results.
const SEARCH_CAP: u64 = 1000;
/// Pages of 100 read of a list GitHub can make long (checks, branches,
/// jobs, search results): a thousand.
const PAGES: u32 = 10;

/// How many segments deep [`GitHub::ref_in`] looks for a ref.
const REF_DEPTH: usize = 20;
/// Pages of 100 of a PR's files (GitHub's own cap is 3000) or review
/// threads.
const FILE_PAGES: u32 = 30;

/// Pages of your notifications fetched at most to fill one, when the
/// inbox's filter applies after fetching.
const INBOX_PAGES: u32 = 10;
/// The unread notifications counted (GitHub's largest page of them).
const UNREAD_PAGE: u64 = 50;
/// How often the unread count is polled when GitHub doesn't say.
const UNREAD_POLL_SECS: u64 = 60;

/// Whether a `Link` header says there's a next page.
fn has_next(link: &str) -> bool {
    link.split(',').any(|l| l.contains("rel=\"next\""))
}

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

/// The query string of a `Link` header's `rel="next"` URL.
fn next_query(link: &str) -> Option<&str> {
    let next = link.split(',').find(|l| l.contains("rel=\"next\""))?;
    let url = next.split(['<', '>']).nth(1)?;
    Some(url.split_once('?')?.1)
}

/// The `after` cursor in a `Link` header's `rel="next"` URL.
fn next_after(link: &str) -> Option<String> {
    let after = next_query(link)?
        .split('&')
        .find_map(|kv| kv.strip_prefix("after="))?;
    Some(
        url::form_urlencoded::parse(format!("a={after}").as_bytes())
            .next()?
            .1
            .into_owned(),
    )
}

/// A GraphQL request's body.
fn gql_body(query: &str, variables: Value) -> Value {
    serde_json::json!({ "query": query, "variables": variables })
}

/// A REST path as an error names it: without `/repos/`.
fn short_path(path: &str) -> &str {
    path.strip_prefix("/repos/").unwrap_or(path)
}

/// Percent-encodes a query string's value (spaces as `+`).
fn encode_query(s: &str) -> String {
    escape(s, true)
}

/// Percent-encodes a ref for a URL path (branch names may contain `/`).
pub(crate) fn encode_path(s: &str) -> String {
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
/// Adds what `list` doesn't have yet.
fn note(list: &Mutex<Vec<String>>, said: impl IntoIterator<Item = String>) {
    let mut list = lock(list);
    for what in said {
        if !list.contains(&what) {
            list.push(what);
        }
    }
}

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

/// A failed response's error. A 404 names `what` was asked for: GitHub's
/// message only says "Not Found". A 451 says why GitHub blocks it, and
/// where its notice is.
fn check_status(response: &Response, what: impl std::fmt::Display) -> Result<(), ApiError> {
    #[derive(serde::Deserialize)]
    struct Message {
        message: String,
        block: Option<Block>,
        /// What a 422 found wrong ("Validation Failed" says only that).
        #[serde(default)]
        errors: Vec<Detail>,
    }
    /// One thing a 422 found wrong: its message, or what and why
    /// (`tag_name`, `already_exists`).
    #[derive(serde::Deserialize)]
    struct Detail {
        message: Option<String>,
        field: Option<String>,
        code: Option<String>,
    }
    impl Detail {
        fn text(self) -> Option<String> {
            self.message.or_else(|| {
                let code = self.code?.replace('_', " ");
                Some(match self.field {
                    Some(field) => format!("{field} {code}"),
                    None => code,
                })
            })
        }
    }
    #[derive(serde::Deserialize)]
    struct Block {
        reason: Option<String>,
        html_url: Option<String>,
    }
    let status = response.status;
    if status.is_success() {
        return Ok(());
    }
    if status == StatusCode::UNAUTHORIZED {
        return Err(ApiError::Unauthorized);
    }
    if status == StatusCode::NOT_FOUND {
        return Err(ApiError::NotFound(what.to_string()));
    }
    let wire = serde_json::from_str::<Message>(&response.body).ok();
    if status == StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS {
        let block = wire.as_ref().and_then(|m| m.block.as_ref());
        return Err(ApiError::Blocked(blocked_text(block.and_then(|b| {
            Some((b.reason.as_deref()?, b.html_url.as_deref()))
        }))));
    }
    let message = match wire {
        Some(m) => {
            let details: Vec<String> = m.errors.into_iter().filter_map(Detail::text).collect();
            if details.is_empty() {
                m.message
            } else {
                details.join("; ")
            }
        }
        // GitHub's gateway gave up: a page, not an answer.
        None if matches!(status.as_u16(), 502 | 504) => {
            "GitHub gave up on it (it stops after 10 seconds)".to_owned()
        }
        None => response.body.chars().take(200).collect(),
    };
    Err(ApiError::Http {
        status: status.as_u16(),
        message,
    })
}

/// What a 451 says: GitHub blocks the repository, for `reason` (GitHub's
/// word for it: `dmca`, `tos`, …), with a link to the notice.
fn blocked_text(block: Option<(&str, Option<&str>)>) -> String {
    let Some((reason, notice)) = block else {
        return "GitHub blocks access to this repository for legal reasons".into();
    };
    let why = match reason {
        "dmca" => "a DMCA takedown notice".to_owned(),
        "tos" => "a violation of its Terms of Service".to_owned(),
        other => other.replace('_', " "),
    };
    match notice {
        Some(url) => format!("GitHub blocks access to this repository because of {why} ({url})"),
        None => format!("GitHub blocks access to this repository because of {why}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_enterprise_server_is_asked_at_its_own_api() {
        let ghe = Host::parse("ghe.example.com").unwrap();
        let gh = GitHub::new(Token::new("t"), Store::disabled(), ghe);
        assert_eq!(gh.http.rest, "https://ghe.example.com/api/v3");
        assert_eq!(gh.http.graphql, "https://ghe.example.com/api/graphql");
        let gh = GitHub::new(Token::new("t"), Store::disabled(), Host::GITHUB_COM);
        assert_eq!(gh.http.rest, "https://api.github.com");
        assert_eq!(gh.http.graphql, "https://api.github.com/graphql");
    }

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
