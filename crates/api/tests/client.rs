//! Client behavior against a scripted local HTTP server: ETag revalidation,
//! retries, rate limits, error mapping and GraphQL decoding.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "the scripted server isn't a #[test] function, but a panic in it is still a test failure"
)]

use std::sync::{Arc, Mutex};

use ghtui_api::auth::Token;
use ghtui_api::model::{NodeId, PrRef, RepoId};
use ghtui_api::{ApiError, GitHub};
use ghtui_store::Store;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Clone)]
struct Reply {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: String,
    /// Only for a request whose line contains this (for requests made
    /// concurrently, which arrive in any order).
    path: Option<&'static str>,
}

impl Reply {
    fn new(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into(),
            path: None,
        }
    }

    fn on(mut self, path: &'static str) -> Self {
        self.path = Some(path);
        self
    }

    fn header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }
}

#[derive(Debug, Clone)]
struct Seen {
    request_line: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap()
    }

    /// The GraphQL document sent.
    fn query(&self) -> String {
        self.json()["query"].as_str().unwrap().to_owned()
    }
}

/// Serves `replies` in order (one per request, across connections) and
/// records every request.
async fn serve(replies: Vec<Reply>) -> (String, Arc<Mutex<Vec<Seen>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let queue = Arc::new(Mutex::new(replies));
    let seen_task = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let seen = seen_task.clone();
            let queue = queue.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                loop {
                    let (request, len) = loop {
                        if let Some(request) = parse_request(&buf) {
                            break request;
                        }
                        let mut chunk = [0u8; 4096];
                        match socket.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                    };
                    buf.drain(..len);
                    let line = request.request_line.clone();
                    seen.lock().unwrap().push(request);

                    let reply = {
                        let mut queue = queue.lock().unwrap();
                        let next = queue
                            .iter()
                            .position(|r| r.path.is_none_or(|p| line.contains(p)));
                        next.map_or_else(
                            || Reply::new(500, "script exhausted"),
                            |i| queue.remove(i),
                        )
                    };
                    let mut out = format!(
                        "HTTP/1.1 {} X\r\ncontent-length: {}\r\n",
                        reply.status,
                        reply.body.len()
                    );
                    for (k, v) in &reply.headers {
                        out.push_str(&format!("{k}: {v}\r\n"));
                    }
                    out.push_str("\r\n");
                    out.push_str(&reply.body);
                    if socket.write_all(out.as_bytes()).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    (format!("http://{addr}"), seen)
}

/// The first whole request in `buf` (headers, then Content-Length bytes),
/// and its length.
fn parse_request(buf: &[u8]) -> Option<(Seen, usize)> {
    let header_end = buf.windows(4).position(|w| w == b"\r\n\r\n")? + 4;
    let head = String::from_utf8_lossy(&buf[..header_end]);
    let mut lines = head.split("\r\n");
    let mut seen = Seen {
        request_line: lines.next().unwrap_or_default().to_owned(),
        headers: lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
            .collect(),
        body: String::new(),
    };
    let len = seen
        .header("content-length")
        .map_or(0, |v| v.parse().unwrap_or(0));
    seen.body = String::from_utf8_lossy(buf.get(header_end..header_end + len)?).into_owned();
    Some((seen, header_end + len))
}

fn client(base: &str, store: Store) -> GitHub {
    GitHub::with_base_uri(Token::new("test-token"), store, Some(base))
}

/// A client without a cache, against a server playing `replies`.
async fn github(replies: Vec<Reply>) -> (GitHub, Arc<Mutex<Vec<Seen>>>) {
    let (base, seen) = serve(replies).await;
    (client(&base, Store::disabled()), seen)
}

#[tokio::test]
async fn rest_get_revalidates_with_etag() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("cache.redb"));
    let (base, seen) = serve(vec![
        Reply::new(200, r#"{"login":"octocat"}"#).header("etag", "\"v1\""),
        Reply::new(304, ""),
    ])
    .await;
    let gh = client(&base, store);

    assert_eq!(gh.viewer_login().await.unwrap(), "octocat");
    assert_eq!(gh.viewer_login().await.unwrap(), "octocat");

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].header("if-none-match"), None);
    assert_eq!(seen[1].header("if-none-match"), Some("\"v1\""));
    assert!(seen[0].request_line.starts_with("GET /user "));
}

#[tokio::test]
async fn retries_server_errors() {
    let (gh, seen) = github(vec![
        Reply::new(502, "bad gateway"),
        Reply::new(200, r#"{"login":"octocat"}"#),
    ])
    .await;
    assert_eq!(gh.viewer_login().await.unwrap(), "octocat");
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn gives_up_after_three_attempts() {
    let (gh, seen) = github(vec![
        Reply::new(503, "x"),
        Reply::new(503, "x"),
        Reply::new(503, r#"{"message":"unavailable"}"#),
    ])
    .await;
    match gh.viewer_login().await {
        Err(ApiError::Http {
            status: 503,
            message,
        }) => assert_eq!(message, "unavailable"),
        other => panic!("{other:?}"),
    }
    assert_eq!(seen.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn maps_unauthorized() {
    let (gh, _) = github(vec![
        Reply::new(401, r#"{"message":"Bad credentials"}"#),
        Reply::new(401, r#"{"message":"Bad credentials"}"#),
        Reply::new(200, r#"{"login":"octocat"}"#),
    ])
    .await;
    assert!(matches!(
        gh.viewer_login().await,
        Err(ApiError::Unauthorized)
    ));
    // Reported once when it starts, once when it stops.
    assert_eq!(gh.token_rejected_change(), Some(true));
    let _ = gh.viewer_login().await;
    assert_eq!(gh.token_rejected_change(), None);
    let _ = gh.viewer_login().await;
    assert_eq!(gh.token_rejected_change(), Some(false));
}

#[tokio::test]
async fn long_rate_limit_waits_are_reported_not_slept() {
    let (gh, seen) = github(vec![
        Reply::new(403, r#"{"message":"secondary rate limit"}"#).header("retry-after", "60"),
    ])
    .await;
    assert!(matches!(
        gh.viewer_login().await,
        Err(ApiError::RateLimited(60))
    ));
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn tracks_rate_limit_headers() {
    let (gh, _) = github(vec![
        Reply::new(200, r#"{"login":"octocat"}"#)
            .header("x-ratelimit-resource", "core")
            .header("x-ratelimit-remaining", "4321")
            .header("x-ratelimit-limit", "5000")
            .header("x-ratelimit-reset", "1700000000"),
    ])
    .await;
    gh.viewer_login().await.unwrap();
    assert_eq!(gh.rate_limits().rest.unwrap().remaining, 4321);
}

const PR_RESPONSE: &str = r#"{"data":{"repository":{"pullRequest":{
  "number": 7, "title": "Add thing", "body": "Body text", "url": "https://github.com/o/r/pull/7",
  "isDraft": false, "state": "OPEN", "createdAt": "2026-01-01T00:00:00Z", "updatedAt": "2026-01-02T00:00:00Z",
  "author": {"login": "alice"}, "repository": {"nameWithOwner": "o/r"},
  "baseRefName": "main", "headRefName": "feature", "baseRefOid": "aaa", "headRefOid": "bbb",
  "headRepository": {"nameWithOwner": "alice/r"},
  "additions": 10, "deletions": 2, "changedFiles": 3, "mergeable": "MERGEABLE",
  "reviewDecision": "APPROVED",
  "labels": {"nodes": [{"name": "bug", "color": "d73a4a"}]},
  "milestone": {"number": 3, "title": "v1"},
  "commits": {"nodes": [{"commit": {"statusCheckRollup": {"state": "FAILURE"}}}]},
  "comments": {"totalCount": 4}
}}, "rateLimit": {"cost": 1, "limit": 5000, "remaining": 4999, "resetAt": "2026-01-01T01:00:00Z"}}}"#;

#[tokio::test]
async fn fetches_and_caches_pull_request() {
    use ghtui_api::model::{ChecksState, Mergeable, PrState, ReviewDecision};

    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("cache.redb"));
    let (base, seen) = serve(vec![Reply::new(200, PR_RESPONSE)]).await;
    let gh = client(&base, store);
    let pr = PrRef::parse("o/r#7").unwrap();

    assert!(gh.cached_pull_request(&pr).is_none());
    let detail = gh.pull_request(&pr).await.unwrap();
    assert_eq!(detail.summary.title, "Add thing");
    assert_eq!(detail.summary.state, PrState::Open);
    assert_eq!(detail.summary.review, Some(ReviewDecision::Approved));
    assert_eq!(detail.summary.checks, Some(ChecksState::Failing));
    assert_eq!(detail.mergeable, Mergeable::Yes);
    assert_eq!(detail.labels[0].name, "bug");
    assert_eq!(detail.head_oid, "bbb");
    assert_eq!(detail.milestone.as_ref().map(|m| m.number), Some(3));

    let cached = gh.cached_pull_request(&pr).unwrap();
    assert_eq!(cached.value, detail);

    let seen = seen.lock().unwrap();
    assert!(seen[0].request_line.starts_with("POST /graphql "));
    let body = seen[0].json();
    assert_eq!(body["variables"]["number"], 7);
    assert_eq!(body["variables"]["owner"], "o");
}

#[tokio::test]
async fn missing_pull_request_is_not_found() {
    let (gh, _) = github(vec![Reply::new(
        200,
        r#"{"data":{"repository":{"pullRequest":null},"rateLimit":null},
            "errors":[{"message":"Could not resolve to a PullRequest with the number of 9."}]}"#,
    )])
    .await;
    let pr = PrRef::parse("o/r#9").unwrap();
    assert!(matches!(
        gh.pull_request(&pr).await,
        Err(ApiError::NotFound(_))
    ));
}

#[tokio::test]
async fn graphql_errors_without_data_are_errors() {
    let (gh, _) = github(vec![Reply::new(
        200,
        r#"{"errors":[{"message":"Something went wrong"}]}"#,
    )])
    .await;
    match gh.inbox().await {
        Err(ApiError::GraphQl(errors)) => assert_eq!(errors, ["Something went wrong"]),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn viewed_files_paginate() {
    use ghtui_api::model::ViewedState;
    let page = |nodes: &str, next: Option<&str>| {
        format!(
            r#"{{"data":{{"repository":{{"pullRequest":{{"id":"PR_kw1","files":{{
                "pageInfo":{{"hasNextPage":{},"endCursor":{}}},
                "nodes":[{nodes}]}}}}}}}}}}"#,
            next.is_some(),
            next.map_or("null".to_owned(), |c| format!("\"{c}\""))
        )
    };
    let (gh, seen) = github(vec![
        Reply::new(
            200,
            page(
                r#"{"path":"a.rs","viewerViewedState":"VIEWED"},{"path":"b.rs","viewerViewedState":"UNVIEWED"}"#,
                Some("c1"),
            ),
        ),
        Reply::new(
            200,
            page(r#"{"path":"c.rs","viewerViewedState":"DISMISSED"}"#, None),
        ),
    ]).await;
    let viewed = gh
        .viewed_files(&PrRef::parse("o/r#3").unwrap())
        .await
        .unwrap();
    assert_eq!(viewed.pull_request_id.as_str(), "PR_kw1");
    assert_eq!(viewed.states["a.rs"], ViewedState::Viewed);
    assert_eq!(viewed.states["b.rs"], ViewedState::Unviewed);
    assert_eq!(viewed.states["c.rs"], ViewedState::Dismissed);
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    let second = seen[1].json();
    assert_eq!(second["variables"]["after"], "c1");
}

#[tokio::test]
async fn set_viewed_sends_the_right_mutation() {
    let (gh, seen) = github(vec![
        Reply::new(
            200,
            r#"{"data":{"markFileAsViewed":{"clientMutationId":null}}}"#,
        ),
        Reply::new(
            200,
            r#"{"data":{"unmarkFileAsViewed":{"clientMutationId":null}}}"#,
        ),
        Reply::new(
            200,
            r#"{"errors":[{"message":"Could not resolve to a node"}]}"#,
        ),
    ])
    .await;
    gh.set_viewed(&NodeId::new("PR_kw1"), "src/a.rs", true)
        .await
        .unwrap();
    gh.set_viewed(&NodeId::new("PR_kw1"), "src/a.rs", false)
        .await
        .unwrap();
    assert!(matches!(
        gh.set_viewed(&NodeId::new("bad"), "x", true).await,
        Err(ApiError::GraphQl(_))
    ));
    let seen = seen.lock().unwrap();
    let first = seen[0].json();
    assert!(seen[0].query().contains("markFileAsViewed"));
    assert_eq!(first["variables"]["pullRequestId"], "PR_kw1");
    assert_eq!(first["variables"]["path"], "src/a.rs");
    assert!(seen[1].query().contains("unmarkFileAsViewed"));
}

const THREADS: &str = r#"{"data":{"repository":{"pullRequest":{"reviewThreads":{
  "pageInfo":{"hasNextPage":false,"endCursor":null},
  "nodes":[
    {"id":"T1","path":"src/a.rs","diffSide":"RIGHT","startDiffSide":"RIGHT","line":12,"startLine":10,
     "originalLine":12,"originalStartLine":10,"isOutdated":false,"isResolved":false,"subjectType":"LINE",
     "viewerCanReply":true,"viewerCanResolve":true,"viewerCanUnresolve":false,
     "comments":{"nodes":[
       {"id":"C1","author":{"login":"alice"},"body":"Why?","createdAt":"2026-10-01T00:00:00Z",
        "url":"https://github.com/o/r/pull/7#discussion_r1","originalCommit":{"oid":"abc"},"state":"SUBMITTED"},
       {"id":"C2","author":null,"body":"Because.","createdAt":"2026-10-02T00:00:00Z",
        "url":"https://github.com/o/r/pull/7#discussion_r2","originalCommit":{"oid":"abc"},"state":"PENDING"}]}},
    {"id":"T2","path":"src/b.rs","diffSide":"LEFT","startDiffSide":null,"line":null,"startLine":null,
     "originalLine":5,"originalStartLine":null,"isOutdated":true,"isResolved":true,"subjectType":"LINE",
     "viewerCanReply":true,"viewerCanResolve":false,"viewerCanUnresolve":true,"comments":{"nodes":[]}},
    {"id":"T3","path":"README.md","diffSide":"RIGHT","startDiffSide":null,"line":null,"startLine":null,
     "originalLine":null,"originalStartLine":null,"isOutdated":false,"isResolved":false,"subjectType":"FILE",
     "viewerCanReply":true,"viewerCanResolve":true,"viewerCanUnresolve":false,"comments":{"nodes":[]}}
  ]}}}}}"#;

#[tokio::test]
async fn decodes_review_threads() {
    use ghtui_api::model::Side;
    let (gh, _) = github(vec![Reply::new(200, THREADS)]).await;
    let threads = gh
        .review_threads(&PrRef::parse("o/r#7").unwrap())
        .await
        .unwrap();
    assert_eq!(threads.len(), 3);
    let t1 = &threads[0];
    assert_eq!(
        (t1.side, t1.line, t1.start_line),
        (Side::Right, Some(12), Some(10))
    );
    assert_eq!(t1.comments.len(), 2);
    assert_eq!(t1.comments[1].author, "ghost");
    assert!(t1.comments[1].pending);
    assert_eq!(t1.comments[0].original_commit.as_deref(), Some("abc"));
    let t2 = &threads[1];
    assert!(t2.outdated && t2.resolved);
    assert_eq!(
        (t2.side, t2.line, t2.original_line),
        (Side::Left, None, Some(5))
    );
    assert!(threads[2].file_level);
}

#[tokio::test]
async fn patches_paginate_until_a_short_page() {
    let full: Vec<String> = (0..100)
        .map(|i| format!(r#"{{"filename":"f{i}.rs","patch":"@@ -1 +1 @@\n-a\n+b"}}"#))
        .collect();
    let (gh, seen) = github(vec![
        Reply::new(200, format!("[{}]", full.join(","))),
        Reply::new(
            200,
            r#"[{"filename":"img.png"},{"filename":"new.rs","previous_filename":"old.rs","patch":"@@ -1,2 +1,2 @@"}]"#,
        ),
    ]).await;
    let files = gh
        .pr_patches(&PrRef::parse("o/r#7").unwrap())
        .await
        .unwrap();
    assert_eq!(files.len(), 102);
    assert_eq!(files[100].patch, None, "binary files have no patch");
    assert_eq!(files[101].previous_filename.as_deref(), Some("old.rs"));
    let seen = seen.lock().unwrap();
    assert!(
        seen[0]
            .request_line
            .contains("/repos/o/r/pulls/7/files?per_page=100&page=1")
    );
    assert!(seen[1].request_line.contains("page=2"));
}

/// A run's jobs come a page at a time until there are as many as GitHub
/// counts.
#[tokio::test]
async fn a_runs_jobs_paginate() {
    let job = |id: u64| {
        format!(
            r#"{{"id":{id},"run_id":9,"name":"j{id}","status":"completed","conclusion":"success"}}"#
        )
    };
    let page = |ids: std::ops::Range<u64>| {
        let jobs: Vec<String> = ids.map(job).collect();
        format!(r#"{{"total_count":102,"jobs":[{}]}}"#, jobs.join(","))
    };
    let (gh, seen) = github(vec![
        Reply::new(200, r#"{"id":9,"run_number":1,"event":"push","head_sha":"abc","status":"completed","conclusion":"success"}"#)
            .on("/runs/9 "),
        Reply::new(200, page(0..100)).on("jobs?per_page=100 "),
        Reply::new(200, page(100..102)).on("page=2"),
    ])
    .await;
    let run = gh
        .workflow_run(&RepoId::new("o", "r"), 9, None)
        .await
        .unwrap();
    assert_eq!(run.jobs.len(), 102);
    assert_eq!(seen.lock().unwrap().len(), 3);
}

/// An organization's discussions are found in whichever of its
/// repositories holds them, past the first page of a search.
#[tokio::test]
async fn an_organizations_discussions_are_found_past_the_first_page() {
    let (gh, seen) = github(vec![
        Reply::new(
            200,
            r#"{"data":{"search":{"pageInfo":{"hasNextPage":true,"endCursor":"c1"},"nodes":[{"url":"https://github.com/acme/app/discussions/1","repository":{"nameWithOwner":"acme/app"}}]}}}"#,
        ),
        Reply::new(
            200,
            r#"{"data":{"search":{"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[{"url":"https://github.com/orgs/acme/discussions/9","repository":{"nameWithOwner":"acme/community"}}]}}}"#,
        ),
        Reply::new(200, r#"{"data":{"repository":null}}"#),
    ])
    .await;
    let of = ghtui_api::browse::DiscussionsOf::Org("acme".into());
    let result = gh.discussion(&of, 9).await;
    assert!(
        matches!(result, Err(ApiError::NotFound(ref m)) if m.contains("acme/community")),
        "{result:?}"
    );
    assert_eq!(seen.lock().unwrap().len(), 3);
}

/// An empty commit or code search, which GitHub refuses, finds nothing
/// without asking.
#[tokio::test]
async fn an_empty_code_search_asks_nothing() {
    let (gh, seen) = github(Vec::new()).await;
    let results = gh
        .search(ghtui_api::browse::SearchKind::Code, " ", None)
        .await
        .unwrap();
    assert_eq!(results.counts(), (0, 0));
    assert!(seen.lock().unwrap().is_empty());
}

/// A milestone whose title has quotes isn't searched for, which GitHub
/// can't do, and says so.
#[tokio::test]
async fn a_quoted_milestone_title_is_not_searched() {
    let (gh, seen) = github(vec![Reply::new(
        200,
        r#"{"data":{"repository":{"milestone":{"number":3,"title":"Say \"hi\"","description":null,"dueOn":null,"closed":false,"closedAt":null,"updatedAt":"2026-10-01T00:00:00Z","openIssues":{"totalCount":2},"doneIssues":{"totalCount":0},"openPrs":{"totalCount":0},"donePrs":{"totalCount":0}}}}}"#,
    )])
    .await;
    let m = gh.milestone(&RepoId::new("o", "r"), 3, None).await.unwrap();
    assert!(m.unsearchable);
    assert_eq!(seen.lock().unwrap().len(), 1, "no search");
}

/// A discussion search reads GitHub's count of discussions and leaves out
/// nodes that aren't discussions.
#[tokio::test]
async fn discussion_search_reads_its_count_and_hits() {
    let (gh, _) = github(vec![Reply::new(
        200,
        r#"{"data":{"search":{"discussionCount":7,"pageInfo":{"hasNextPage":true,"endCursor":"c"},"nodes":[{},{"repository":{"nameWithOwner":"o/r"},"number":4,"title":"t","author":null,"category":{"name":"Q&A"},"comments":{"totalCount":2},"isAnswered":true,"upvoteCount":1,"updatedAt":"2026-10-01T00:00:00Z"}]}}}"#,
    )])
    .await;
    let results = gh
        .search(ghtui_api::browse::SearchKind::Discussions, "q", None)
        .await
        .unwrap();
    let ghtui_api::browse::SearchResults::Discussions(r) = results else {
        panic!("not discussions");
    };
    assert_eq!(
        (r.total, r.items.len(), r.next.as_deref()),
        (7, 1, Some("c"))
    );
    assert_eq!(r.items[0].repo, RepoId::new("o", "r"));
    assert!(r.items[0].summary.answered);
}

/// Branches come by name, page after page, and tags newest first, with
/// how many there are of each.
#[tokio::test]
async fn branches_page_by_name() {
    let page = |names: &[&str], next: Option<&str>, tags: bool| {
        let nodes: Vec<String> = names
            .iter()
            .map(|n| format!(r#"{{"name":"{n}"}}"#))
            .collect();
        let heads = format!(
            r#"{{"totalCount":3,"pageInfo":{{"hasNextPage":{},"endCursor":{}}},"nodes":[{}]}}"#,
            next.is_some(),
            next.map_or("null".to_owned(), |c| format!("\"{c}\"")),
            nodes.join(",")
        );
        let tags = if tags {
            r#","tags":{"totalCount":7,"pageInfo":{"hasNextPage":true,"endCursor":"t"},"nodes":[{"name":"v2"},{"name":"v1"}]}"#
        } else {
            ""
        };
        format!(r#"{{"data":{{"repository":{{"heads":{heads}{tags}}}}}}}"#)
    };
    let (gh, seen) = github(vec![
        Reply::new(200, page(&["a", "b"], Some("c1"), true)),
        Reply::new(200, page(&["c"], None, false)),
    ])
    .await;
    let refs = gh.refs(&RepoId::new("o", "r")).await.unwrap();
    assert_eq!(refs.branches, ["a", "b", "c"]);
    assert_eq!(
        (refs.branch_total, refs.tags.len(), refs.tag_total),
        (3, 2, 7)
    );
    let seen = seen.lock().unwrap();
    assert!(seen[0].query().contains("ALPHABETICAL"));
    assert_eq!(seen[1].json()["variables"]["after"], "c1");
}

/// A comparison of one revision is against the default branch.
#[tokio::test]
async fn one_revision_compares_with_the_default_branch() {
    let (gh, seen) = github(vec![Reply::new(
        200,
        r#"{"status":"ahead","ahead_by":1,"behind_by":0,"total_commits":1,"base_commit":{"sha":"b"},"merge_base_commit":{"sha":"m"},"commits":[{"sha":"h","commit":{"message":"m"}}]}"#,
    )])
    .await;
    let c = gh
        .compare(&RepoId::new("o", "r"), "feature/x")
        .await
        .unwrap();
    assert_eq!((c.from.as_str(), c.to.as_str()), ("m", "h"));
    assert!(
        seen.lock().unwrap()[0]
            .request_line
            .contains("/repos/o/r/compare/HEAD...feature%2Fx ")
    );
}

/// A category the repository doesn't have is not found, not every
/// discussion under its name.
#[tokio::test]
async fn an_unknown_discussion_category_is_not_found() {
    let (gh, _) = github(vec![Reply::new(
        200,
        r#"{"data":{"repository":{"discussionCategories":{"nodes":[{"id":"C1","name":"Ideas","slug":"ideas"}]}}}}"#,
    )])
    .await;
    let of = ghtui_api::browse::DiscussionsOf::Repo(RepoId::new("o", "r"));
    let result = gh.discussions(&of, Some("nope"), None).await;
    assert!(matches!(result, Err(ApiError::NotFound(_))), "{result:?}");
}

#[tokio::test]
async fn review_submission_calls() {
    use ghtui_api::model::{NewThread, ReviewEvent, Side};
    let (gh, seen) = github(vec![
        Reply::new(200, r#"{"data":{"repository":{"pullRequest":{"id":"PR_1","reviews":{"nodes":[]}}}}}"#),
        Reply::new(200, r#"{"data":{"addPullRequestReview":{"pullRequestReview":{"id":"R_1"}}}}"#),
        Reply::new(200, r#"{"data":{"addPullRequestReviewThread":{"thread":{"id":"T_9"}}}}"#),
        Reply::new(200, r#"{"data":{"addPullRequestReviewThread":{"thread":{"id":"T_10"}}}}"#),
        Reply::new(
            200,
            r#"{"data":{"addPullRequestReviewThread":null},"errors":[{"message":"pull_request_review_thread.line must be part of the diff"}]}"#,
        ),
        Reply::new(200, r#"{"data":{"submitPullRequestReview":{"pullRequestReview":{"id":"R_1"}}}}"#),
        Reply::new(200, r#"{"data":{"addPullRequestReviewThreadReply":{"comment":{"id":"C_1"}}}}"#),
        Reply::new(200, r#"{"data":{"resolveReviewThread":{"thread":{"id":"T_9"}}}}"#),
    ]).await;
    let pr = PrRef::parse("o/r#7").unwrap();
    let (pr_id, pending) = gh.pending_review(&pr).await.unwrap();
    assert_eq!((pr_id.as_str(), pending), ("PR_1", None));
    let review = gh.start_review(&pr_id, "deadbeef").await.unwrap();
    assert_eq!(review.as_str(), "R_1");

    let range = NewThread {
        path: "src/a.rs".into(),
        body: "Rename this".into(),
        line: Some(12),
        side: Side::Right,
        start_line: Some(10),
        start_side: Some(Side::Right),
    };
    assert_eq!(
        gh.add_review_thread(&review, &range)
            .await
            .unwrap()
            .as_str(),
        "T_9"
    );
    let file_level = NewThread {
        line: None,
        start_line: None,
        start_side: None,
        ..range.clone()
    };
    assert_eq!(
        gh.add_review_thread(&review, &file_level).await.unwrap(),
        NodeId::new("T_10")
    );
    match gh.add_review_thread(&review, &range).await {
        Err(ApiError::GraphQl(errors)) => assert!(errors[0].contains("part of the diff")),
        other => panic!("{other:?}"),
    }
    gh.submit_review(&review, ReviewEvent::RequestChanges, "Needs work")
        .await
        .unwrap();
    gh.reply(&NodeId::new("T_9"), "Done").await.unwrap();
    gh.set_resolved(&NodeId::new("T_9"), true).await.unwrap();

    let seen = seen.lock().unwrap();
    assert_eq!(seen[1].json()["variables"]["commit"], "deadbeef");
    let line = &seen[2].json()["variables"]["input"];
    assert_eq!(line["line"], 12);
    assert_eq!(line["startLine"], 10);
    assert_eq!(line["side"], "RIGHT");
    assert_eq!(line["subjectType"], "LINE");
    let file = &seen[3].json()["variables"]["input"];
    assert_eq!(file["subjectType"], "FILE");
    assert!(
        file.get("line").is_none(),
        "file comments have no line: {file}"
    );
    assert_eq!(seen[5].json()["variables"]["event"], "REQUEST_CHANGES");
    assert_eq!(seen[5].json()["variables"]["body"], "Needs work");
    assert!(seen[6].query().contains("addPullRequestReviewThreadReply"));
    assert!(seen[7].query().contains("resolveReviewThread"));
}

#[tokio::test]
async fn last_review_commit_skips_pending_reviews() {
    let (gh, seen) = github(vec![Reply::new(
        200,
        r#"{"data":{"repository":{"pullRequest":{"reviews":{"nodes":[
            {"state":"COMMENTED","commit":{"oid":"old"}},
            {"state":"APPROVED","commit":{"oid":"newer"}},
            {"state":"PENDING","commit":{"oid":"draft"}}]}}}}}"#,
    )])
    .await;
    let commit = gh
        .last_review_commit(&PrRef::parse("o/r#7").unwrap(), "me")
        .await
        .unwrap();
    assert_eq!(commit.as_deref(), Some("newer"));
    let seen = seen.lock().unwrap();
    let body = seen[0].json();
    assert_eq!(body["variables"]["login"], "me");
}

/// A 502 after a mutation may mean GitHub applied it: sending it again
/// could post the reply twice.
#[tokio::test]
async fn mutations_are_not_retried_after_server_errors() {
    let (gh, seen) = github(vec![
        Reply::new(502, "bad gateway"),
        Reply::new(
            200,
            r#"{"data":{"addPullRequestReviewThreadReply":{"comment":{"id":"C_1"}}}}"#,
        ),
    ])
    .await;
    assert!(matches!(
        gh.reply(&NodeId::new("T_9"), "Done").await,
        Err(ApiError::Http { status: 502, .. })
    ));
    assert_eq!(seen.lock().unwrap().len(), 1);
}

/// Queries also go out as POSTs, but they're safe to repeat.
#[tokio::test]
async fn graphql_queries_are_retried() {
    let (gh, seen) = github(vec![
        Reply::new(502, "bad gateway"),
        Reply::new(
            200,
            r#"{"data":{"repository":{"pullRequest":{"reviews":{"nodes":[]}}}}}"#,
        ),
    ])
    .await;
    let commit = gh
        .last_review_commit(&PrRef::parse("o/r#7").unwrap(), "me")
        .await
        .unwrap();
    assert_eq!(commit, None);
    assert_eq!(seen.lock().unwrap().len(), 2);
}

/// A refused connection never reached GitHub, so even a mutation retries.
#[tokio::test]
async fn refused_connections_are_retried_even_for_mutations() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let gh = client(&format!("http://{addr}"), Store::disabled());
    let started = std::time::Instant::now();
    assert!(matches!(
        gh.reply(&NodeId::new("T_9"), "Done").await,
        Err(ApiError::Network(_))
    ));
    // Three attempts means two backoffs (500ms + 1s).
    assert!(started.elapsed() >= std::time::Duration::from_millis(1400));
}

/// Every request carries the token, GitHub's media type and API version,
/// and says who's asking.
#[tokio::test]
async fn requests_carry_the_token_and_github_headers() {
    let (gh, seen) = github(vec![Reply::new(200, r#"{"login":"octocat"}"#)]).await;
    gh.viewer_login().await.unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].header("authorization"), Some("Bearer test-token"));
    assert_eq!(
        seen[0].header("accept"),
        Some("application/vnd.github+json")
    );
    assert_eq!(seen[0].header("x-github-api-version"), Some("2022-11-28"));
    assert!(seen[0].header("user-agent").unwrap().starts_with("ghtui/"));
}
