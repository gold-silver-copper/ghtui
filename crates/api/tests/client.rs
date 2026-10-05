//! Client behavior against a scripted local HTTP server: ETag revalidation,
//! retries, rate limits, error mapping and GraphQL decoding.

// The scripted server isn't a `#[test]` function, but a panic in it is still
// a test failure.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::sync::{Arc, Mutex};

use ghtui_api::auth::Token;
use ghtui_api::model::PrRef;
use ghtui_api::{ApiError, GitHub};
use ghtui_store::Store;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Clone)]
struct Reply {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: String,
}

impl Reply {
    fn new(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into(),
        }
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
}

/// Serves `replies` in order (one per request, across connections) and
/// records every request.
async fn serve(replies: Vec<Reply>) -> (String, Arc<Mutex<Vec<Seen>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let queue = Arc::new(Mutex::new(replies.into_iter()));
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
                    // Read one request: headers, then Content-Length bytes.
                    let header_end = loop {
                        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break pos + 4;
                        }
                        let mut chunk = [0u8; 4096];
                        match socket.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                    };
                    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
                    let mut lines = head.split("\r\n");
                    let request_line = lines.next().unwrap_or_default().to_owned();
                    let headers: Vec<(String, String)> = lines
                        .filter_map(|l| l.split_once(':'))
                        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
                        .collect();
                    let len: usize = headers
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                        .and_then(|(_, v)| v.parse().ok())
                        .unwrap_or(0);
                    while buf.len() < header_end + len {
                        let mut chunk = [0u8; 4096];
                        match socket.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                    }
                    let body =
                        String::from_utf8_lossy(&buf[header_end..header_end + len]).to_string();
                    buf.drain(..header_end + len);
                    seen.lock().unwrap().push(Seen {
                        request_line,
                        headers,
                        body,
                    });

                    let reply = queue
                        .lock()
                        .unwrap()
                        .next()
                        .unwrap_or_else(|| Reply::new(500, "script exhausted"));
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

fn client(base: &str, store: Store) -> GitHub {
    GitHub::with_base_uri(Token::new("test-token"), store, Some(base))
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
    let (base, seen) = serve(vec![
        Reply::new(502, "bad gateway"),
        Reply::new(200, r#"{"login":"octocat"}"#),
    ])
    .await;
    let gh = client(&base, Store::disabled());
    assert_eq!(gh.viewer_login().await.unwrap(), "octocat");
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn gives_up_after_three_attempts() {
    let (base, seen) = serve(vec![
        Reply::new(503, "x"),
        Reply::new(503, "x"),
        Reply::new(503, r#"{"message":"unavailable"}"#),
    ])
    .await;
    let gh = client(&base, Store::disabled());
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
    let (base, _) = serve(vec![Reply::new(401, r#"{"message":"Bad credentials"}"#)]).await;
    let gh = client(&base, Store::disabled());
    assert!(matches!(
        gh.viewer_login().await,
        Err(ApiError::Unauthorized)
    ));
}

#[tokio::test]
async fn long_rate_limit_waits_are_reported_not_slept() {
    let (base, seen) = serve(vec![
        Reply::new(403, r#"{"message":"secondary rate limit"}"#).header("retry-after", "60"),
    ])
    .await;
    let gh = client(&base, Store::disabled());
    assert!(matches!(
        gh.viewer_login().await,
        Err(ApiError::RateLimited(60))
    ));
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn tracks_rate_limit_headers() {
    let (base, _) = serve(vec![
        Reply::new(200, r#"{"login":"octocat"}"#)
            .header("x-ratelimit-resource", "core")
            .header("x-ratelimit-remaining", "4321")
            .header("x-ratelimit-limit", "5000")
            .header("x-ratelimit-reset", "1700000000"),
    ])
    .await;
    let gh = client(&base, Store::disabled());
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

    let cached = gh.cached_pull_request(&pr).unwrap();
    assert_eq!(cached.value, detail);

    let seen = seen.lock().unwrap();
    assert!(seen[0].request_line.starts_with("POST /graphql "));
    let body: serde_json::Value = serde_json::from_str(&seen[0].body).unwrap();
    assert_eq!(body["variables"]["number"], 7);
    assert_eq!(body["variables"]["owner"], "o");
}

#[tokio::test]
async fn missing_pull_request_is_not_found() {
    let (base, _) = serve(vec![Reply::new(
        200,
        r#"{"data":{"repository":{"pullRequest":null},"rateLimit":null},
            "errors":[{"message":"Could not resolve to a PullRequest with the number of 9."}]}"#,
    )])
    .await;
    let gh = client(&base, Store::disabled());
    let pr = PrRef::parse("o/r#9").unwrap();
    assert!(matches!(
        gh.pull_request(&pr).await,
        Err(ApiError::NotFound(_))
    ));
}

#[tokio::test]
async fn graphql_errors_without_data_are_errors() {
    let (base, _) = serve(vec![Reply::new(
        200,
        r#"{"errors":[{"message":"Something went wrong"}]}"#,
    )])
    .await;
    let gh = client(&base, Store::disabled());
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
    let (base, seen) = serve(vec![
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
    ])
    .await;
    let gh = client(&base, Store::disabled());
    let viewed = gh
        .viewed_files(&PrRef::parse("o/r#3").unwrap())
        .await
        .unwrap();
    assert_eq!(viewed.pull_request_id, "PR_kw1");
    assert_eq!(viewed.states["a.rs"], ViewedState::Viewed);
    assert_eq!(viewed.states["b.rs"], ViewedState::Unviewed);
    assert_eq!(viewed.states["c.rs"], ViewedState::Dismissed);
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    let second: serde_json::Value = serde_json::from_str(&seen[1].body).unwrap();
    assert_eq!(second["variables"]["after"], "c1");
}

#[tokio::test]
async fn set_viewed_sends_the_right_mutation() {
    let (base, seen) = serve(vec![
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
    let gh = client(&base, Store::disabled());
    gh.set_viewed("PR_kw1", "src/a.rs", true).await.unwrap();
    gh.set_viewed("PR_kw1", "src/a.rs", false).await.unwrap();
    assert!(matches!(
        gh.set_viewed("bad", "x", true).await,
        Err(ApiError::GraphQl(_))
    ));
    let seen = seen.lock().unwrap();
    let first: serde_json::Value = serde_json::from_str(&seen[0].body).unwrap();
    assert!(
        first["query"]
            .as_str()
            .unwrap()
            .contains("markFileAsViewed")
    );
    assert_eq!(first["variables"]["pullRequestId"], "PR_kw1");
    assert_eq!(first["variables"]["path"], "src/a.rs");
    let second: serde_json::Value = serde_json::from_str(&seen[1].body).unwrap();
    assert!(
        second["query"]
            .as_str()
            .unwrap()
            .contains("unmarkFileAsViewed")
    );
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
    let (base, _) = serve(vec![Reply::new(200, THREADS)]).await;
    let gh = client(&base, Store::disabled());
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
    let (base, seen) = serve(vec![
        Reply::new(200, format!("[{}]", full.join(","))),
        Reply::new(
            200,
            r#"[{"filename":"img.png"},{"filename":"new.rs","previous_filename":"old.rs","patch":"@@ -1,2 +1,2 @@"}]"#,
        ),
    ])
    .await;
    let gh = client(&base, Store::disabled());
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

#[tokio::test]
async fn review_submission_calls() {
    use ghtui_api::model::{NewThread, ReviewEvent, Side};
    let (base, seen) = serve(vec![
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
    ])
    .await;
    let gh = client(&base, Store::disabled());
    let pr = PrRef::parse("o/r#7").unwrap();
    let (pr_id, pending) = gh.pending_review(&pr).await.unwrap();
    assert_eq!((pr_id.as_str(), pending), ("PR_1", None));
    let review = gh.start_review(&pr_id, "deadbeef").await.unwrap();
    assert_eq!(review, "R_1");

    let range = NewThread {
        path: "src/a.rs".into(),
        body: "Rename this".into(),
        line: Some(12),
        side: Side::Right,
        start_line: Some(10),
        start_side: Some(Side::Right),
    };
    assert_eq!(gh.add_review_thread(&review, &range).await.unwrap(), "T_9");
    let file_level = NewThread {
        line: None,
        start_line: None,
        start_side: None,
        ..range.clone()
    };
    assert_eq!(
        gh.add_review_thread(&review, &file_level).await.unwrap(),
        "T_10"
    );
    match gh.add_review_thread(&review, &range).await {
        Err(ApiError::GraphQl(errors)) => assert!(errors[0].contains("part of the diff")),
        other => panic!("{other:?}"),
    }
    gh.submit_review(&review, ReviewEvent::RequestChanges, "Needs work")
        .await
        .unwrap();
    gh.reply("T_9", "Done").await.unwrap();
    gh.set_resolved("T_9", true).await.unwrap();

    let seen = seen.lock().unwrap();
    let body = |i: usize| serde_json::from_str::<serde_json::Value>(&seen[i].body).unwrap();
    assert_eq!(body(1)["variables"]["commit"], "deadbeef");
    let line = &body(2)["variables"]["input"];
    assert_eq!(line["line"], 12);
    assert_eq!(line["startLine"], 10);
    assert_eq!(line["side"], "RIGHT");
    assert_eq!(line["subjectType"], "LINE");
    let file = &body(3)["variables"]["input"];
    assert_eq!(file["subjectType"], "FILE");
    assert!(
        file.get("line").is_none(),
        "file comments have no line: {file}"
    );
    assert_eq!(body(5)["variables"]["event"], "REQUEST_CHANGES");
    assert_eq!(body(5)["variables"]["body"], "Needs work");
    assert!(
        body(6)["query"]
            .as_str()
            .unwrap()
            .contains("addPullRequestReviewThreadReply")
    );
    assert!(
        body(7)["query"]
            .as_str()
            .unwrap()
            .contains("resolveReviewThread")
    );
}

#[tokio::test]
async fn last_review_commit_skips_pending_reviews() {
    let (base, seen) = serve(vec![Reply::new(
        200,
        r#"{"data":{"repository":{"pullRequest":{"reviews":{"nodes":[
            {"state":"COMMENTED","commit":{"oid":"old"}},
            {"state":"APPROVED","commit":{"oid":"newer"}},
            {"state":"PENDING","commit":{"oid":"draft"}}]}}}}}"#,
    )])
    .await;
    let gh = client(&base, Store::disabled());
    let commit = gh
        .last_review_commit(&PrRef::parse("o/r#7").unwrap(), "me")
        .await
        .unwrap();
    assert_eq!(commit.as_deref(), Some("newer"));
    let seen = seen.lock().unwrap();
    let body: serde_json::Value = serde_json::from_str(&seen[0].body).unwrap();
    assert_eq!(body["variables"]["login"], "me");
}

/// A 502 after a mutation may mean GitHub applied it: sending it again
/// could post the reply twice.
#[tokio::test]
async fn mutations_are_not_retried_after_server_errors() {
    let (base, seen) = serve(vec![
        Reply::new(502, "bad gateway"),
        Reply::new(
            200,
            r#"{"data":{"addPullRequestReviewThreadReply":{"comment":{"id":"C_1"}}}}"#,
        ),
    ])
    .await;
    let gh = client(&base, Store::disabled());
    assert!(matches!(
        gh.reply("T_9", "Done").await,
        Err(ApiError::Http { status: 502, .. })
    ));
    assert_eq!(seen.lock().unwrap().len(), 1);
}

/// Queries also go out as POSTs, but they're safe to repeat.
#[tokio::test]
async fn graphql_queries_are_retried() {
    let (base, seen) = serve(vec![
        Reply::new(502, "bad gateway"),
        Reply::new(
            200,
            r#"{"data":{"repository":{"pullRequest":{"reviews":{"nodes":[]}}}}}"#,
        ),
    ])
    .await;
    let gh = client(&base, Store::disabled());
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
        gh.reply("T_9", "Done").await,
        Err(ApiError::Network(_))
    ));
    // Three attempts means two backoffs (500ms + 1s).
    assert!(started.elapsed() >= std::time::Duration::from_millis(1400));
}
