//! Client behavior against a scripted local HTTP server: ETag revalidation,
//! retries, rate limits, error mapping and GraphQL decoding.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "the scripted server isn't a #[test] function, but a panic in it is still a test failure"
)]

use std::sync::{Arc, Mutex};

use ghtui::api::auth::Token;
use ghtui::api::model::{NodeId, PrRef, RepoId};
use ghtui::api::{ApiError, GitHub};
use ghtui::store::Store;
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
    GitHub::with_base_uri(Token::new("test-token"), store, base)
}

/// A client without a cache, against a server playing `replies`.
async fn github(replies: Vec<Reply>) -> (GitHub, Arc<Mutex<Vec<Seen>>>) {
    let (base, seen) = serve(replies).await;
    (client(&base, Store::disabled()), seen)
}

/// "Done", replying to thread T_9.
fn reply() -> ghtui::api::change::Change {
    let (thread, body) = (NodeId::new("T_9"), "Done".into());
    ghtui::api::change::Change::Reply { thread, body }
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

const PR_RESPONSE: &str = r#"{"data":{"repository":{
  "mergeCommitAllowed": true, "squashMergeAllowed": true, "rebaseMergeAllowed": false,
  "viewerDefaultMergeMethod": "SQUASH", "viewerPermission": "WRITE",
  "autoMergeAllowed": true,
  "pullRequest":{
  "id": "PR_7", "mergeStateStatus": "BEHIND",
  "viewerCanClose": true, "viewerCanReopen": true, "viewerCanUpdate": true,
  "viewerCanMergeAsAdmin": false, "viewerDidAuthor": false,
  "maintainerCanModify": true, "isCrossRepository": true,
  "viewerCanEnableAutoMerge": false, "viewerCanDisableAutoMerge": true,
  "autoMergeRequest": {"enabledBy": {"login": "alice"}, "mergeMethod": "SQUASH"},
  "isMergeQueueEnabled": true, "mergeQueueEntry": {"position": 2, "state": "AWAITING_CHECKS"},
  "number": 7, "title": "Add thing", "body": "Body text", "url": "https://github.com/o/r/pull/7",
  "isDraft": false, "state": "OPEN", "createdAt": "2026-01-01T00:00:00Z", "updatedAt": "2026-01-02T00:00:00Z",
  "author": {"login": "alice"}, "repository": {"nameWithOwner": "o/r"},
  "baseRefName": "main", "headRefName": "feature", "baseRefOid": "aaa", "headRefOid": "bbb",
  "headRepository": {"nameWithOwner": "alice/r"},
  "additions": 10, "deletions": 2, "changedFiles": 3, "mergeable": "MERGEABLE",
  "reviewDecision": "APPROVED",
  "labels": {"totalCount": 3, "nodes": [{"name": "bug", "color": "d73a4a"}]},
  "milestone": {"number": 3, "title": "v1"},
  "viewerCanReact": true, "viewerCanLabel": false, "viewerCanAssign": true,
  "headRef": null, "viewerCanDeleteHeadRef": false, "locked": true, "activeLockReason": "TOO_HEATED",
  "reactionGroups": [{"content": "THUMBS_UP", "viewerHasReacted": false, "reactors": {"totalCount": 5}}],
  "assignees": {"totalCount": 1, "nodes": [{"login": "bob"}]},
  "commits": {"nodes": [{"commit": {"statusCheckRollup": {"state": "FAILURE"}}}]},
  "comments": {"totalCount": 4}
}}, "rateLimit": {"cost": 1, "limit": 5000, "remaining": 4999, "resetAt": "2026-01-01T01:00:00Z"}}}"#;

#[tokio::test]
async fn fetches_and_caches_pull_request() {
    use ghtui::api::browse::PrState;
    use ghtui::api::change::MergeMethod;
    use ghtui::api::model::{ChecksState, MergeState, Mergeable, ReviewDecision};

    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("cache.redb"));
    let behind = r#"{"data":{"repository":{"ref":{"compare":{"behindBy":3}}}}}"#;
    let (base, seen) = serve(vec![Reply::new(200, PR_RESPONSE), Reply::new(200, behind)]).await;
    let gh = client(&base, store);
    let pr = PrRef::parse("o/r#7").unwrap();

    assert!(gh.cached_pull_request(&pr).is_none());
    let detail = gh.pull_request(&pr).await.unwrap();
    assert_eq!(detail.title, "Add thing");
    assert_eq!(detail.state, PrState::Open);
    assert_eq!(detail.review, Some(ReviewDecision::Approved));
    assert_eq!(detail.checks, Some(ChecksState::Failing));
    assert_eq!(detail.mergeable, Mergeable::Yes);
    // Yours first, then the rest the repository allows.
    assert_eq!(
        detail.merge_methods,
        [MergeMethod::Squash, MergeMethod::Merge]
    );
    assert_eq!(detail.merge_state, MergeState::Behind);
    // GitHub's word on what you may do; a fork's branch takes the
    // author's leave to push to.
    assert!(detail.may.merge && detail.may.push_head && !detail.may.merge_as_admin);
    // Compared with its base, as GitHub only says so when it's required.
    assert_eq!(detail.behind_by, Some(3));
    assert!(seen.lock().unwrap()[1].body.contains("refs/heads/main"));
    assert_eq!(detail.labels[0].name, "bug");
    assert_eq!(detail.labels.left_out(), 2);
    assert_eq!(detail.head_oid, "bbb");
    assert_eq!(detail.milestone.as_ref().map(|m| m.number), Some(3));
    // Auto-merge and the merge queue, as the merge box shows them.
    let auto = detail.auto_merge.as_ref().unwrap();
    assert_eq!(
        (auto.by.as_str(), auto.method),
        ("alice", MergeMethod::Squash)
    );
    assert!(detail.auto_merge_allowed && detail.merge_queue);
    assert!(detail.may.disable_auto_merge && !detail.may.enable_auto_merge);
    let queued = detail.queued.unwrap();
    assert_eq!(
        (queued.position, queued.state),
        (2, ghtui::api::model::QueueState::AwaitingChecks)
    );
    assert_eq!(detail.assignees.items, ["bob"]);
    assert_eq!(detail.editable.reactions[0].count, 5);
    let may = detail.sidebar;
    assert!(may.assignees && !may.labels && !may.milestone && !may.reviewers);
    // Its branch is gone, and its conversation locked, as GitHub says why.
    assert!(detail.head_gone && !detail.may.delete_head);
    let locked = detail.locked.and_then(|l| l.reason);
    assert_eq!(locked, Some(ghtui::api::manage::LockReason::TooHeated));

    let cached = gh.cached_pull_request(&pr).unwrap();
    assert_eq!(cached.value, detail);

    let seen = seen.lock().unwrap();
    assert!(seen[0].request_line.starts_with("POST /graphql "));
    let body = seen[0].json();
    assert_eq!(body["variables"]["number"], 7);
    assert_eq!(body["variables"]["owner"], "o");
}

/// A root nulled by something other than NOT_FOUND (an untyped error, as
/// for a timeout) isn't called not found: GitHub's message says why, once.
#[tokio::test]
async fn a_root_nulled_by_a_timeout_says_githubs_message_once() {
    let timeout =
        "Something went wrong while executing your query. This may be the result of a timeout.";
    let (gh, _) = github(vec![
        Reply::new(
            200,
            format!(
                r#"{{"data":{{"repository":{{"pullRequest":null}},"rateLimit":null}},
                "errors":[{{"path":["repository","pullRequest"],"message":"{timeout}"}}]}}"#
            ),
        ),
        // A sibling root's NOT_FOUND (a user's login isn't an organization)
        // doesn't make the timed-out user not found.
        Reply::new(
            200,
            format!(
                r#"{{"data":{{"user":null,"organization":null,"user_readme":null,"org_readme":null}},
                "errors":[{{"path":["user"],"message":"{timeout}"}},
                          {{"type":"NOT_FOUND","path":["organization"],"message":"Could not resolve to an Organization with the login of 'u'."}}]}}"#
            ),
        ),
    ])
    .await;
    let pr = PrRef::parse("o/r#9").unwrap();
    match gh.pull_request(&pr).await {
        Err(ApiError::GraphQl(errors)) => assert_eq!(errors, [timeout]),
        other => panic!("{other:?}"),
    }
    match gh.profile("u").await {
        Err(ApiError::GraphQl(errors)) => assert_eq!(errors, [timeout]),
        other => panic!("{other:?}"),
    }
    assert_eq!(gh.take_left_out(), Vec::<String>::new());
}

#[tokio::test]
async fn graphql_errors_without_data_are_errors() {
    let (gh, _) = github(vec![Reply::new(
        200,
        r#"{"errors":[{"message":"Something went wrong"}]}"#,
    )])
    .await;
    match gh
        .search(ghtui::api::browse::SearchKind::Pulls, "q", None)
        .await
    {
        Err(ApiError::GraphQl(errors)) => assert_eq!(errors, ["Something went wrong"]),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn viewed_files_paginate() {
    use ghtui::api::model::ViewedState;
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

/// A change sends its mutation once, and a refusal is GitHub's message.
#[tokio::test]
async fn a_merge_sends_its_method_and_head_and_a_refusal_says_why() {
    use ghtui::api::change::{Change, MergeMethod};
    let (gh, seen) = github(vec![
        Reply::new(200, r#"{"data":{"mergePullRequest":{"clientMutationId":null}}}"#),
        Reply::new(
            200,
            r#"{"data":{"mergePullRequest":null},"errors":[{"type":"UNPROCESSABLE","message":"Pull Request is not mergeable"}]}"#,
        ),
    ])
    .await;
    let merge = Change::Merge {
        pr: NodeId::new("PR_7"),
        method: MergeMethod::Squash,
        head: "bbb".into(),
    };
    gh.change(&merge).await.unwrap();
    match gh.change(&merge).await {
        Err(ApiError::GraphQl(errors)) => assert_eq!(errors, ["Pull Request is not mergeable"]),
        other => panic!("{other:?}"),
    }
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    let sent = seen[0].json();
    assert!(seen[0].query().contains("mergePullRequest"));
    assert_eq!(sent["variables"]["pr"], "PR_7");
    assert_eq!(sent["variables"]["method"], "SQUASH");
    assert_eq!(sent["variables"]["head"], "bbb");
}

/// Auto-merge and the merge queue: each its own mutation, about the pull
/// request, and enabling or queueing names the head it's for.
#[tokio::test]
async fn auto_merge_and_the_merge_queue_send_their_mutations() {
    use ghtui::api::change::{Change, MergeMethod};
    let ok = |field: &str| {
        Reply::new(
            200,
            format!(r#"{{"data":{{"{field}":{{"clientMutationId":null}}}}}}"#),
        )
    };
    let (gh, seen) = github(vec![
        ok("enablePullRequestAutoMerge"),
        ok("disablePullRequestAutoMerge"),
        ok("enqueuePullRequest"),
        ok("dequeuePullRequest"),
    ])
    .await;
    let pr = NodeId::new("PR_7");
    let changes = [
        Change::EnableAutoMerge {
            pr: pr.clone(),
            method: MergeMethod::Rebase,
            head: "bbb".into(),
        },
        Change::DisableAutoMerge { pr: pr.clone() },
        Change::Enqueue {
            pr: pr.clone(),
            head: "bbb".into(),
        },
        Change::Dequeue { pr },
    ];
    for change in &changes {
        gh.change(change).await.unwrap();
    }
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 4);
    let enable = seen[0].json();
    assert!(seen[0].query().contains("enablePullRequestAutoMerge"));
    assert!(seen[0].query().contains("expectedHeadOid"));
    assert_eq!(enable["variables"]["pr"], "PR_7");
    assert_eq!(enable["variables"]["method"], "REBASE");
    assert_eq!(enable["variables"]["head"], "bbb");
    assert!(seen[1].query().contains("disablePullRequestAutoMerge"));
    assert_eq!(seen[1].json()["variables"]["id"], "PR_7");
    let enqueue = seen[2].json();
    assert!(seen[2].query().contains("enqueuePullRequest"));
    assert_eq!(enqueue["variables"]["pr"], "PR_7");
    assert_eq!(enqueue["variables"]["head"], "bbb");
    assert!(seen[3].query().contains("dequeuePullRequest"));
    assert_eq!(seen[3].json()["variables"]["id"], "PR_7");
}

/// Re-runs and cancels post to Actions' paths; a refusal (here, a token
/// without the permission) is GitHub's status and message, never retried.
#[tokio::test]
async fn actions_changes_post_to_their_paths() {
    use ghtui::api::change::Change;
    let (gh, seen) = github(vec![
        Reply::new(201, "{}"),
        Reply::new(202, "{}"),
        Reply::new(
            403,
            r#"{"message":"Resource not accessible by personal access token"}"#,
        ),
    ])
    .await;
    let repo = RepoId::new("o", "r");
    gh.change(&Change::Rerun {
        repo: repo.clone(),
        run: 5,
        failed_only: true,
    })
    .await
    .unwrap();
    gh.change(&Change::CancelRun {
        repo: repo.clone(),
        run: 5,
    })
    .await
    .unwrap();
    match gh.change(&Change::RerunJob { repo, job: 9 }).await {
        Err(ApiError::Http { status, message }) => {
            assert_eq!(status, 403);
            assert_eq!(message, "Resource not accessible by personal access token");
        }
        other => panic!("{other:?}"),
    }
    let lines: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|s| s.request_line.clone())
        .collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].starts_with("POST /repos/o/r/actions/runs/5/rerun-failed-jobs "));
    assert!(lines[1].starts_with("POST /repos/o/r/actions/runs/5/cancel "));
    assert!(lines[2].starts_with("POST /repos/o/r/actions/jobs/9/rerun "));
}

/// Each edit sends its mutation, with the node IDs it names: labels and
/// assignees are added and removed by two mutations, and only what
/// changes is sent; asking for reviews names everyone asked; no milestone
/// is a null.
#[tokio::test]
async fn edits_send_their_mutations() {
    use ghtui::api::change::Change;
    use ghtui::api::edit::{Edit, Named, ReactionContent, Written};
    let ok = |field: &str| {
        Reply::new(
            200,
            format!(r#"{{"data":{{"{field}":{{"clientMutationId":null}}}}}}"#),
        )
    };
    let (gh, seen) = github(vec![
        ok("updateIssueComment"),
        ok("updatePullRequestReviewComment"),
        ok("updatePullRequest"),
        ok("deleteIssueComment"),
        ok("updateIssue"),
        ok("addReaction"),
        ok("removeReaction"),
        ok("addLabelsToLabelable"),
        ok("removeLabelsFromLabelable"),
        ok("removeAssigneesFromAssignable"),
        ok("requestReviews"),
        ok("updateIssue"),
    ])
    .await;
    let id = |s: &str| NodeId::new(s);
    let named = |s: &str| Named {
        id: id(s),
        name: s.to_lowercase(),
    };
    let edits = [
        Edit::Body {
            of: Written::IssueComment,
            id: id("IC_1"),
            body: "Better".into(),
        },
        Edit::Body {
            of: Written::ReviewComment,
            id: id("PRRC_1"),
            body: "Nit".into(),
        },
        Edit::Body {
            of: Written::Pr,
            id: id("PR_7"),
            body: "Description".into(),
        },
        Edit::Delete {
            of: Written::IssueComment,
            id: id("IC_1"),
        },
        Edit::Title {
            pr: false,
            id: id("I_3"),
            title: "New title".into(),
        },
        Edit::React {
            subject: id("IC_1"),
            content: ReactionContent::ThumbsUp,
            on: true,
        },
        Edit::React {
            subject: id("IC_1"),
            content: ReactionContent::Heart,
            on: false,
        },
        Edit::Labels {
            subject: id("I_3"),
            add: vec![named("L_bug")],
            remove: vec![named("L_ui")],
        },
        Edit::Assignees {
            subject: id("I_3"),
            add: Vec::new(),
            remove: vec![named("U_bob")],
        },
        Edit::Reviewers {
            pr: id("PR_7"),
            users: vec![named("U_amy")],
            teams: vec![named("T_core")],
            bots: Vec::new(),
        },
        Edit::Milestone {
            pr: false,
            id: id("I_3"),
            milestone: None,
        },
    ];
    for edit in edits {
        gh.change(&Change::Edit(edit)).await.unwrap();
    }
    let seen = seen.lock().unwrap();
    let sent: Vec<(String, serde_json::Value)> = seen
        .iter()
        .map(|s| (s.query(), s.json()["variables"].clone()))
        .collect();
    let field = |i: usize, name: &str| {
        let (query, _) = &sent[i];
        assert!(query.contains(name), "{i}: {query}");
    };
    let var = |i: usize, name: &str| sent[i].1[name].clone();
    field(0, "updateIssueComment(input: {id: $id, body: $body})");
    assert_eq!(
        (var(0, "id"), var(0, "body")),
        ("IC_1".into(), "Better".into())
    );
    field(1, "pullRequestReviewCommentId: $id");
    field(
        2,
        "updatePullRequest(input: {pullRequestId: $id, body: $body})",
    );
    field(3, "deleteIssueComment");
    field(4, "updateIssue(input: {id: $id, title: $title})");
    assert_eq!(var(4, "title"), "New title");
    field(5, "addReaction");
    assert_eq!(
        (var(5, "subject"), var(5, "content")),
        ("IC_1".into(), "THUMBS_UP".into())
    );
    field(6, "removeReaction");
    assert_eq!(var(6, "content"), "HEART");
    field(7, "addLabelsToLabelable");
    assert_eq!(var(7, "ids"), serde_json::json!(["L_bug"]));
    field(8, "removeLabelsFromLabelable");
    assert_eq!(var(8, "ids"), serde_json::json!(["L_ui"]));
    // Nothing to add: only the removal.
    field(9, "removeAssigneesFromAssignable");
    field(10, "requestReviews");
    assert_eq!(
        (var(10, "users"), var(10, "teams"), var(10, "bots")),
        (
            serde_json::json!(["U_amy"]),
            serde_json::json!(["T_core"]),
            serde_json::json!([])
        )
    );
    field(11, "milestoneId: $milestone");
    assert_eq!(var(11, "milestone"), serde_json::Value::Null);
    assert_eq!(sent.len(), 12);
}

/// Managing an issue, a pull request or a discussion sends GitHub's own
/// mutations (and REST for a branch); an issue named by its number is
/// looked up first, and what makes something new says what it made.
#[tokio::test]
async fn manage_changes_send_their_mutations() {
    use ghtui::api::change::Change;
    use ghtui::api::manage::{DiscussionReason, HideReason, IssueAt, LockReason, Manage};
    let ok = |field: &str| {
        Reply::new(
            200,
            format!(r#"{{"data":{{"{field}":{{"clientMutationId":null}}}}}}"#),
        )
    };
    let id_of = |id: &str| {
        Reply::new(
            200,
            format!(r#"{{"data":{{"repository":{{"issueOrPullRequest":{{"id":"{id}"}}}}}}}}"#),
        )
    };
    let (gh, seen) = github(vec![
        Reply::new(204, ""),
        Reply::new(201, "{}"),
        ok("convertPullRequestToDraft"),
        ok("updatePullRequest"),
        ok("lockLockable"),
        ok("unlockLockable"),
        ok("pinIssue"),
        ok("minimizeComment"),
        id_of("I_3"),
        ok("closeIssue"),
        id_of("I_5"),
        ok("addSubIssue"),
        id_of("I_5"),
        id_of("I_6"),
        ok("reprioritizeSubIssue"),
        id_of("I_4"),
        ok("removeBlockedBy"),
        Reply::new(
            200,
            r#"{"data":{"repository":{"defaultBranchRef":{"target":{"oid":"abc"}}}}}"#,
        ),
        ok("createLinkedBranch"),
        ok("closeDiscussion"),
        Reply::new(
            200,
            r#"{"data":{"revertPullRequest":{"revertPullRequest":{"number":20}}}}"#,
        ),
        Reply::new(
            200,
            r#"{"data":{"transferIssue":{"issue":{"number":1,"repository":{"nameWithOwner":"o/elsewhere"}}}}}"#,
        ),
        Reply::new(
            200,
            r#"{"data":{"createDiscussion":{"discussion":{"number":7}}}}"#,
        ),
    ])
    .await;
    let id = |s: &str| NodeId::new(s);
    let repo = RepoId::new("o", "r");
    let at = |n| IssueAt {
        repo: repo.clone(),
        number: n,
    };
    let changes = [
        Manage::DeleteBranch {
            repo: repo.clone(),
            branch: "fix".into(),
        },
        Manage::RestoreBranch {
            repo: repo.clone(),
            branch: "fix".into(),
            oid: "abc".into(),
        },
        Manage::ConvertToDraft { pr: id("PR_7") },
        Manage::ChangeBase {
            pr: id("PR_7"),
            base: "next".into(),
        },
        Manage::Lock {
            subject: id("I_1"),
            reason: Some(LockReason::OffTopic),
        },
        Manage::Unlock { subject: id("I_1") },
        Manage::Pin {
            issue: id("I_1"),
            on: true,
        },
        Manage::Hide {
            comment: id("IC_1"),
            reason: HideReason::Outdated,
        },
        Manage::CloseAsDuplicate {
            issue: id("I_1"),
            of: Some(at(3)),
        },
        Manage::SubIssue {
            parent: id("I_1"),
            child: at(5),
            add: true,
        },
        Manage::MoveSubIssue {
            parent: id("I_1"),
            child: at(5),
            sibling: at(6),
            before: true,
        },
        Manage::BlockedBy {
            issue: id("I_1"),
            by: at(4),
            add: false,
        },
        Manage::LinkBranch {
            issue: id("I_1"),
            repo: repo.clone(),
            name: "1-fix".into(),
        },
        Manage::CloseDiscussion {
            discussion: id("D_1"),
            reason: DiscussionReason::Outdated,
        },
    ];
    for change in changes {
        gh.change(&Change::Manage(change)).await.unwrap();
    }
    assert_eq!(gh.revert(&id("PR_7")).await.unwrap(), 20);
    let moved = gh.transfer(&id("I_1"), &id("R_2")).await.unwrap();
    assert_eq!(moved.to_string(), "o/elsewhere#1");
    let (repo_id, category) = (id("R_1"), id("DIC_1"));
    let started = gh.start_discussion(&repo_id, &category, "Hi", "There");
    assert_eq!(started.await.unwrap(), 7);

    let seen = seen.lock().unwrap();
    assert!(
        seen[0]
            .request_line
            .starts_with("DELETE /repos/o/r/git/refs/heads/fix "),
        "{}",
        seen[0].request_line
    );
    assert!(
        seen[1]
            .request_line
            .starts_with("POST /repos/o/r/git/refs "),
        "{}",
        seen[1].request_line
    );
    assert_eq!(
        seen[1].json(),
        serde_json::json!({"ref": "refs/heads/fix", "sha": "abc"})
    );
    let sent: Vec<(String, serde_json::Value)> = (seen.iter().skip(2))
        .map(|s| (s.query(), s.json()["variables"].clone()))
        .collect();
    let field = |i: usize, name: &str| {
        let (query, _) = &sent[i];
        assert!(query.contains(name), "{i}: {query}");
    };
    let var = |i: usize, name: &str| sent[i].1[name].clone();
    field(0, "convertPullRequestToDraft(input: {pullRequestId: $id})");
    field(1, "baseRefName: $base");
    assert_eq!(var(1, "base"), "next");
    field(2, "lockLockable");
    assert_eq!(var(2, "reason"), "OFF_TOPIC");
    field(3, "unlockLockable");
    field(4, "pinIssue");
    field(5, "minimizeComment");
    assert_eq!(var(5, "reason"), "OUTDATED");
    // #3's ID, then the close.
    field(6, "issueOrPullRequest(number: $number)");
    assert_eq!(var(6, "number"), 3);
    field(7, "stateReason: DUPLICATE, duplicateIssueId: $of");
    assert_eq!((var(7, "id"), var(7, "of")), ("I_1".into(), "I_3".into()));
    field(9, "addSubIssue");
    assert_eq!(var(9, "child"), "I_5");
    field(12, "reprioritizeSubIssue");
    assert_eq!(
        (var(12, "before"), var(12, "after")),
        ("I_6".into(), serde_json::Value::Null)
    );
    field(14, "removeBlockedBy");
    assert_eq!(var(14, "by"), "I_4");
    field(15, "defaultBranchRef");
    field(16, "createLinkedBranch");
    assert_eq!(
        (var(16, "oid"), var(16, "name")),
        ("abc".into(), "1-fix".into())
    );
    field(17, "closeDiscussion");
    assert_eq!(var(17, "reason"), "OUTDATED");
    field(18, "revertPullRequest");
    field(19, "transferIssue");
    assert_eq!(var(19, "repo"), "R_2");
    field(20, "createDiscussion");
    assert_eq!(var(20, "title"), "Hi");
    assert_eq!(sent.len(), 21);
}

/// A pull request's sidebar comes with the node IDs edits need, and what
/// its repository offers; its organization's teams are asked apart, so a
/// token that can't read them loses only them, and says why.
#[tokio::test]
async fn a_sidebar_keeps_what_its_teams_say() {
    let sidebar = r#"{"data":{"repository":{
      "issueOrPullRequest":{"__typename":"PullRequest",
        "labels":{"totalCount":1,"nodes":[{"id":"L1","name":"bug"}]},
        "assignees":{"totalCount":0,"nodes":[]},
        "milestone":null,
        "reviewRequests":{"totalCount":3,"nodes":[
          {"requestedReviewer":{"__typename":"User","id":"U1","login":"amy"}},
          {"requestedReviewer":{"__typename":"Team","id":"T1","combinedSlug":"o/core"}},
          {"requestedReviewer":{"__typename":"Bot","id":"B1","login":"copilot"}}]},
        "author":{"login":"bob"}},
      "labels":{"totalCount":150,"nodes":[{"id":"L1","name":"bug"}]},
      "assignableUsers":{"totalCount":1,"nodes":[{"id":"U1","login":"amy"}]},
      "milestones":{"totalCount":0,"nodes":[]},
      "owner":{"__typename":"Organization","login":"o"}}}}"#;
    let forbidden = r#"{"data":{"organization":null},"errors":[{"type":"FORBIDDEN",
      "path":["organization","teams"],"message":"Resource not accessible by personal access token"}]}"#;
    let (gh, seen) = github(vec![Reply::new(200, sidebar), Reply::new(200, forbidden)]).await;
    let s = gh.sidebar(&RepoId::new("o", "r"), 7).await.unwrap();
    assert_eq!(s.labels[0].id, NodeId::new("L1"));
    assert_eq!(s.all_labels.left_out(), 149);
    let names =
        |list: &[ghtui::api::edit::Named]| list.iter().map(|n| n.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&s.requested.users), ["amy"]);
    assert_eq!(names(&s.requested.teams), ["o/core"]);
    assert_eq!(names(&s.requested.bots), ["copilot"]);
    assert_eq!(s.author.as_deref(), Some("bob"));
    match &s.teams {
        Some(Err(why)) => assert!(why.contains("not accessible"), "{why}"),
        other => panic!("{other:?}"),
    }
    let seen = seen.lock().unwrap();
    assert_eq!(seen[1].json()["variables"]["org"], "o");
}

/// A new issue sends its title, text, labels, assignees and milestone, and
/// comes back as its number; a new pull request its branches and whether
/// it's a draft. Neither is sent twice.
#[tokio::test]
async fn new_issues_and_pull_requests_send_what_they_hold() {
    use ghtui::api::create::{NewIssue, NewPr};
    let (gh, seen) = github(vec![
        Reply::new(200, r#"{"data":{"createIssue":{"issue":{"number":42}}}}"#),
        Reply::new(
            200,
            r#"{"data":{"createPullRequest":{"pullRequest":{"number":7}}}}"#,
        ),
        Reply::new(
            200,
            r#"{"data":{"createPullRequest":null},"errors":[{"type":"UNPROCESSABLE","message":"No commits between main and topic"}]}"#,
        ),
    ])
    .await;
    let issue = NewIssue {
        repo: NodeId::new("R_1"),
        title: "It crashes".into(),
        body: "### Version\n\n1.0".into(),
        labels: vec![NodeId::new("LA_bug")],
        assignees: vec![NodeId::new("U_me")],
        milestone: Some(NodeId::new("MI_1")),
    };
    assert_eq!(gh.create_issue(&issue).await.unwrap(), 42);
    let pr = NewPr {
        repo: NodeId::new("R_1"),
        base: "main".into(),
        head: "fork:topic".into(),
        title: "Fix it".into(),
        body: String::new(),
        draft: true,
    };
    assert_eq!(gh.create_pull_request(&pr).await.unwrap(), 7);
    match gh.create_pull_request(&pr).await {
        Err(ApiError::GraphQl(errors)) => assert_eq!(errors, ["No commits between main and topic"]),
        other => panic!("{other:?}"),
    }
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 3);
    assert!(seen[0].query().contains("createIssue"));
    let vars = &seen[0].json()["variables"];
    assert_eq!(vars["repo"], "R_1");
    assert_eq!(vars["title"], "It crashes");
    assert_eq!(vars["body"], "### Version\n\n1.0");
    assert_eq!(vars["labels"], serde_json::json!(["LA_bug"]));
    assert_eq!(vars["assignees"], serde_json::json!(["U_me"]));
    assert_eq!(vars["milestone"], "MI_1");
    assert!(seen[1].query().contains("createPullRequest"));
    let vars = &seen[1].json()["variables"];
    assert_eq!(
        (vars["base"].as_str(), vars["head"].as_str()),
        (Some("main"), Some("fork:topic"))
    );
    assert_eq!(
        (vars["title"].as_str(), vars["draft"].as_bool()),
        (Some("Fix it"), Some(true))
    );
}

/// A new form's choices, templates and permissions come from one query;
/// a pull request's commits, with their whole messages, from a comparison.
#[tokio::test]
async fn a_new_form_reads_its_choices_and_templates() {
    let (gh, seen) = github(vec![
        Reply::new(
            200,
            r#"{"data":{"repository":{"id":"R_1","viewerCanCreateIssues":true,"hasIssuesEnabled":true,"isArchived":false,"viewerPermission":"READ","defaultBranchRef":{"name":"main"},
              "labels":{"totalCount":120,"nodes":[{"id":"LA_1","name":"bug","color":"d73a4a","description":null}]},
              "assignableUsers":{"totalCount":1,"nodes":[{"id":"U_1","login":"me","name":null}]},
              "milestones":{"totalCount":0,"nodes":[]},
              "issueTemplates":{"entries":[{"name":"bug.md","object":{"text":"---\nname: Bug\n---\nBody"}},{"name":"logo.png","object":{"text":null}}]},
              "pullRequestTemplates":[{"filename":"pull_request_template.md","body":"Why:"}]}}}"#,
        ),
        Reply::new(
            200,
            r#"{"total_commits":2,"commits":[{"sha":"aaa","commit":{"message":"First\n\nWhy."}},{"sha":"bbb","commit":{"message":"Second"}}]}"#,
        ),
    ])
    .await;
    let repo = RepoId::new("o", "r");
    let form = gh.new_form(&repo).await.unwrap();
    assert!(!form.can_triage, "read access can't set labels");
    assert_eq!(form.labels.left_out(), 119);
    assert_eq!(
        form.issue_templates,
        [("bug.md".to_owned(), "---\nname: Bug\n---\nBody".to_owned())]
    );
    assert_eq!(form.pr_template.as_deref(), Some("Why:"));
    let commits = gh
        .commit_messages(&repo, "main", "fork:topic")
        .await
        .unwrap();
    assert_eq!(commits[0].message, "First\n\nWhy.");
    let seen = seen.lock().unwrap();
    assert!(
        seen[1]
            .request_line
            .starts_with("GET /repos/o/r/compare/main...fork:topic "),
        "{}",
        seen[1].request_line
    );
}

const THREADS: &str = r#"{"data":{"repository":{"pullRequest":{"reviewThreads":{
  "pageInfo":{"hasNextPage":false,"endCursor":null},
  "nodes":[
    {"id":"T1","path":"src/a.rs","diffSide":"RIGHT","startDiffSide":"RIGHT","line":12,"startLine":10,
     "originalLine":12,"originalStartLine":10,"isOutdated":false,"isResolved":false,"subjectType":"LINE",
     "viewerCanReply":true,"viewerCanResolve":true,"viewerCanUnresolve":false,
     "first_comment":{"totalCount":150,"nodes":[
       {"id":"C1","diffHunk":"@@ -10,3 +10,3 @@\n-old\n+new","pullRequestReview":{"fullDatabaseId":"77"},"author":{"login":"alice"},"body":"Why?","createdAt":"2026-10-01T00:00:00Z",
        "url":"https://github.com/o/r/pull/7#discussion_r1","originalCommit":{"oid":"abc"},"state":"SUBMITTED","viewerCanUpdate":false,"viewerCanDelete":false,"viewerCanReact":true,"reactionGroups":[{"content":"HOORAY","viewerHasReacted":true,"reactors":{"totalCount":2}},{"content":"EYES","viewerHasReacted":false,"reactors":{"totalCount":0}}]}]},
     "comments":{"totalCount":150,"nodes":[
       {"id":"C2","author":null,"body":"Because.","createdAt":"2026-10-02T00:00:00Z",
        "url":"https://github.com/o/r/pull/7#discussion_r2","originalCommit":{"oid":"abc"},"state":"PENDING","viewerCanUpdate":true,"viewerCanDelete":true,"viewerCanReact":true,"reactionGroups":[],"pullRequestReview":null}]}},
    {"id":"T2","path":"src/b.rs","diffSide":"LEFT","startDiffSide":null,"line":null,"startLine":null,
     "originalLine":5,"originalStartLine":null,"isOutdated":true,"isResolved":true,"subjectType":"LINE",
     "viewerCanReply":true,"viewerCanResolve":false,"viewerCanUnresolve":true,
     "first_comment":{"totalCount":0,"nodes":[]},"comments":{"totalCount":0,"nodes":[]}},
    {"id":"T3","path":"README.md","diffSide":"RIGHT","startDiffSide":null,"line":null,"startLine":null,
     "originalLine":null,"originalStartLine":null,"isOutdated":false,"isResolved":false,"subjectType":"FILE",
     "viewerCanReply":true,"viewerCanResolve":true,"viewerCanUnresolve":false,
     "first_comment":{"totalCount":1,"nodes":[
       {"id":"C3","diffHunk":"","author":{"login":"bob"},"body":"Typo","createdAt":"2026-10-03T00:00:00Z",
        "url":"https://github.com/o/r/pull/7#discussion_r3","originalCommit":{"oid":"abc"},"state":"SUBMITTED","viewerCanUpdate":true,"viewerCanDelete":true,"viewerCanReact":true,"reactionGroups":[],"pullRequestReview":null}]},
     "comments":{"totalCount":1,"nodes":[
       {"id":"C3","author":{"login":"bob"},"body":"Typo","createdAt":"2026-10-03T00:00:00Z",
        "url":"https://github.com/o/r/pull/7#discussion_r3","originalCommit":{"oid":"abc"},"state":"SUBMITTED","viewerCanUpdate":true,"viewerCanDelete":true,"viewerCanReact":true,"reactionGroups":[],"pullRequestReview":null}]}}
  ]}}}}}"#;

#[tokio::test]
async fn decodes_review_threads() {
    use ghtui::api::model::Side;
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
    // A long thread: its first comment and newest replies, and how many
    // replies between them were left out.
    assert_eq!(t1.comments.len(), 2);
    assert_eq!(t1.comments.left_out(), 148);
    // A short one's first comment is also among its newest: once.
    assert_eq!(threads[2].comments.len(), 1);
    assert_eq!(threads[2].comments.left_out(), 0);
    assert_eq!(t1.comments[1].author, "ghost");
    assert!(t1.comments[1].pending);
    assert_eq!(t1.comments[0].original_commit.as_deref(), Some("abc"));
    // The review its first comment came in, and the diff it was made on.
    assert_eq!(t1.comments[0].review, Some(77));
    assert!(t1.diff_hunk.ends_with("+new"));
    // What GitHub lets you do to each comment, and its reactions (only
    // those someone gave).
    let first = &t1.comments[0].editable;
    assert!(!first.can_update && !first.can_delete && first.can_react);
    let hooray = ghtui::api::edit::Reaction {
        content: ghtui::api::edit::ReactionContent::Hooray,
        count: 2,
        mine: true,
    };
    assert_eq!(first.reactions, [hooray]);
    assert!(t1.comments[1].editable.can_update);
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

/// A run's artifacts, as REST lists them.
const ARTIFACTS: &str = r#"{"total_count":1,"artifacts":[{"id":31,"name":"report","size_in_bytes":2048,"expires_at":"2026-12-31T00:00:00Z","expired":false}]}"#;

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
        Reply::new(200, ARTIFACTS).on("/artifacts"),
    ])
    .await;
    let run = gh
        .workflow_run(&RepoId::new("o", "r"), 9, None)
        .await
        .unwrap();
    assert_eq!(run.jobs.len(), 102);
    assert_eq!(seen.lock().unwrap().len(), 4);
    assert_eq!(run.artifacts.len(), 1);
}

/// An organization's discussions are found in whichever of its
/// repositories holds them, past the first page of a search.
#[tokio::test]
async fn an_organizations_discussions_are_found_past_the_first_page() {
    let (gh, seen) = github(vec![
        Reply::new(
            200,
            r#"{"data":{"search":{"discussionCount":9,"pageInfo":{"hasNextPage":true,"endCursor":"c1"},"nodes":[{},{"url":"https://github.com/acme/app/discussions/1","repository":{"nameWithOwner":"acme/app"}}]}}}"#,
        ),
        Reply::new(
            200,
            r#"{"data":{"search":{"discussionCount":9,"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[{"url":"https://github.com/orgs/acme/discussions/9","repository":{"nameWithOwner":"acme/community"}}]}}}"#,
        ),
        Reply::new(200, r#"{"data":{"repository":null}}"#),
    ])
    .await;
    let of = ghtui::api::browse::DiscussionsOf::Org("acme".into());
    let result = gh.discussion(&of, 9).await;
    assert!(
        matches!(result, Err(ApiError::NotFound(ref m)) if m.contains("acme/community")),
        "{result:?}"
    );
    assert_eq!(seen.lock().unwrap().len(), 3);
}

/// A searched pull request that's an open draft reads as a draft, through
/// the same state-and-draft fragment the PR page reads. Its checks come
/// from a second request by its ID, after the page was shown without them:
/// one request reading both runs past GitHub's 10 seconds for busy accounts.
#[tokio::test]
async fn a_searched_pr_is_shown_then_gets_its_checks() {
    use ghtui::api::browse::PrState;
    use ghtui::api::model::ChecksState;
    let (gh, seen) = github(vec![
        Reply::new(
            200,
            r#"{"data":{"search":{"issueCount":1,"repositoryCount":0,"userCount":0,"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[{"__typename":"PullRequest","id":"PR_5","number":5,"title":"t","isDraft":true,"state":"OPEN","author":null,"createdAt":"2026-10-01T00:00:00Z","updatedAt":"2026-10-01T00:00:00Z","reviewDecision":null,"latestOpinionatedReviews":null,"comments":{"totalCount":0},"labels":null,"repository":{"nameWithOwner":"o/r"}}]}}}"#,
        ),
        Reply::new(
            200,
            r#"{"data":{"nodes":[{"__typename":"PullRequest","id":"PR_5","commits":{"nodes":[{"commit":{"statusCheckRollup":{"state":"FAILURE"}}}]}}]}}"#,
        ),
    ])
    .await;
    let shown = Mutex::new(None);
    let results = gh
        .search_showing(ghtui::api::browse::SearchKind::Pulls, "q", None, |r| {
            *shown.lock().unwrap() = Some(r.clone());
        })
        .await
        .unwrap();
    let ghtui::api::browse::SearchResults::Issues(r) = results else {
        panic!("{results:?}");
    };
    let checks = |i: &ghtui::api::browse::IssueSummary| match i.kind {
        ghtui::api::browse::IssueKind::Pr { state, checks, .. } => (i.number, state, checks),
        ghtui::api::browse::IssueKind::Issue(_) => panic!("{i:?}"),
    };
    assert_eq!(
        r.items.iter().map(checks).collect::<Vec<_>>(),
        [(5, PrState::Draft, Some(ChecksState::Failing))]
    );
    let Some(ghtui::api::browse::SearchResults::Issues(first)) = shown.into_inner().unwrap() else {
        panic!("the page wasn't shown before its checks");
    };
    assert_eq!(checks(&first.items[0]), (5, PrState::Draft, None));
    let seen = seen.lock().unwrap();
    assert!(
        !seen[0].query().contains("statusCheckRollup"),
        "{}",
        seen[0].query()
    );
    assert_eq!(
        seen[1].json()["variables"]["ids"],
        serde_json::json!(["PR_5"])
    );
}

/// GraphQL finds nothing for a search naming a user or repository that
/// isn't there; REST says that's why, and the search says so.
#[tokio::test]
async fn a_search_github_refuses_says_why() {
    let (gh, seen) = github(vec![
        Reply::new(
            200,
            r#"{"data":{"search":{"issueCount":0,"repositoryCount":0,"userCount":0,"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[]}}}"#,
        ),
        Reply::new(
            422,
            r#"{"message":"Validation Failed","errors":[{"message":"The listed users cannot be searched either because the users do not exist or you do not have permission to view the users.","resource":"Search","field":"q","code":"invalid"}],"status":"422"}"#,
        ),
    ])
    .await;
    let result = gh
        .search(
            ghtui::api::browse::SearchKind::Pulls,
            "author:nobody-here",
            None,
        )
        .await;
    match result {
        Err(ApiError::Http {
            status: 422,
            message,
        }) => {
            assert!(
                message.starts_with("The listed users cannot be searched"),
                "{message}"
            );
        }
        other => panic!("{other:?}"),
    }
    let seen = seen.lock().unwrap();
    assert!(
        seen[1]
            .request_line
            .contains("/search/issues?q=author%3Anobody-here+is%3Apr"),
        "{}",
        seen[1].request_line
    );
}

/// An empty commit or code search, which GitHub refuses, finds nothing
/// without asking.
#[tokio::test]
async fn an_empty_code_search_asks_nothing() {
    let (gh, seen) = github(Vec::new()).await;
    let results = gh
        .search(ghtui::api::browse::SearchKind::Code, " ", None)
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
        .search(ghtui::api::browse::SearchKind::Discussions, "q", None)
        .await
        .unwrap();
    let ghtui::api::browse::SearchResults::Discussions(r) = results else {
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
            r#"{{"totalCount":1050,"pageInfo":{{"hasNextPage":{},"endCursor":{}}},"nodes":[{}]}}"#,
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
    assert_eq!(*refs.branches, ["a", "b", "c"]);
    assert_eq!(
        (refs.branches.total(), refs.tags.len(), refs.tags.total()),
        (1050, 2, 7)
    );
    let seen = seen.lock().unwrap();
    assert!(seen[0].query().contains("ALPHABETICAL"));
    assert_eq!(seen[1].json()["variables"]["after"], "c1");
}

/// A commit's checks are read to the last page: a check run again has
/// its newest run wherever it lands, and that's the one shown.
#[tokio::test]
async fn checks_page_to_a_checks_newest_run() {
    let run = |name: &str, conclusion: &str, started: &str| {
        format!(
            r#"{{"__typename":"CheckRun","name":"{name}","status":"COMPLETED","conclusion":"{conclusion}","startedAt":"{started}","completedAt":"{started}","title":null,"detailsUrl":null,"checkSuite":null,"annotations":null}}"#
        )
    };
    let page = |runs: &[String], next: Option<&str>| {
        format!(
            r#"{{"totalCount":3,"pageInfo":{{"hasNextPage":{},"endCursor":{}}},"nodes":[{}]}}"#,
            next.is_some(),
            next.map_or("null".to_owned(), |c| format!("\"{c}\"")),
            runs.join(",")
        )
    };
    let first = page(
        &[
            run("test", "FAILURE", "2026-10-01T10:00:00Z"),
            run("lint", "SUCCESS", "2026-10-01T10:00:00Z"),
        ],
        Some("p2"),
    );
    let second = page(&[run("test", "SUCCESS", "2026-10-01T12:00:00Z")], None);
    let (gh, seen) = github(vec![
        Reply::new(
            200,
            format!(
                r#"{{"data":{{"repository":{{"object":{{"__typename":"Commit","oid":"abc","statusCheckRollup":{{"contexts":{first}}}}}}}}}}}"#
            ),
        ),
        Reply::new(
            200,
            format!(
                r#"{{"data":{{"repository":{{"object":{{"__typename":"Commit","statusCheckRollup":{{"contexts":{second}}}}}}}}}}}"#
            ),
        ),
    ])
    .await;
    let checks = gh
        .commit_checks(&RepoId::new("o", "r"), "main")
        .await
        .unwrap();
    let test = checks.items.iter().find(|c| c.name == "test").unwrap();
    assert_eq!(test.outcome, ghtui::api::browse::CheckOutcome::Success);
    assert_eq!((checks.items.len(), checks.items.total()), (2, 2));
    assert_eq!(seen.lock().unwrap()[1].json()["variables"]["after"], "p2");
}

/// What a partial result leaves out (GitHub's errors beside the data, for
/// what the token can't see) is kept to report, once.
#[tokio::test]
async fn partial_results_say_what_they_left_out() {
    let sso = "Resource protected by organization SAML enforcement.";
    let (gh, _) = github(vec![Reply::new(
        200,
        format!(
            r#"{{"data":{{"repository":{{"heads":{{"totalCount":1,"pageInfo":{{"hasNextPage":false,"endCursor":null}},"nodes":[{{"name":"main"}},null]}},"tags":{{"totalCount":0,"pageInfo":{{"hasNextPage":false,"endCursor":null}},"nodes":[]}}}}}},"errors":[{{"message":"{sso}"}},{{"message":"{sso}"}}]}}"#
        ),
    )])
    .await;
    let refs = gh.refs(&RepoId::new("o", "r")).await.unwrap();
    assert_eq!(*refs.branches, ["main"]);
    assert_eq!(gh.take_left_out(), [sso]);
    assert!(gh.take_left_out().is_empty());
}

/// A REST search that timed out says it found only some results.
#[tokio::test]
async fn a_timed_out_search_says_so() {
    let (gh, _) = github(vec![Reply::new(
        200,
        r#"{"total_count":0,"incomplete_results":true,"items":[]}"#,
    )])
    .await;
    gh.search(ghtui::api::browse::SearchKind::Code, "x", None)
        .await
        .unwrap();
    assert_eq!(
        gh.take_left_out(),
        ["the search took too long and found only some results"]
    );
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

/// A comparison whose newest commit listed isn't the head its permalink
/// names, or that lists other than min(total, 250) commits (more as well
/// as fewer), doesn't add
/// up, and says so (the 250-commit bug's shape: the oldest page, not the
/// newest).
#[tokio::test]
async fn a_comparison_is_checked_against_itself() {
    let compare = |permalink_head: &str, total: u64| {
        format!(
            r#"{{"permalink_url":"https://github.com/o/r/compare/o:bcf4368...o:{permalink_head}","status":"ahead","ahead_by":{total},"behind_by":0,"total_commits":{total},"base_commit":{{"sha":"b"}},"merge_base_commit":{{"sha":"m"}},"commits":[{{"sha":"d2d91f754c87458c6d07863eca20f3ea8ae319ce","commit":{{"message":"m"}}}}]}}"#
        )
    };
    let (gh, _) = github(vec![
        Reply::new(200, compare("d2d91f7", 1)),
        Reply::new(200, compare("efd1e47", 20)),
        Reply::new(200, compare("d2d91f7", 0)),
    ])
    .await;
    let repo = RepoId::new("o", "r");
    gh.compare(&repo, "a...b").await.unwrap();
    assert_eq!(gh.take_doubts(), Vec::<String>::new());
    gh.compare(&repo, "v0.26.0...v0.26.1").await.unwrap();
    let doubts = gh.take_doubts();
    assert!(
        doubts[0].contains("the newest commit listed is d2d91f7, but the head is efd1e47"),
        "{doubts:?}"
    );
    assert!(
        doubts[1].contains("1 commits are listed of 20, not 20"),
        "{doubts:?}"
    );
    // More listed than counted.
    gh.compare(&repo, "c...d").await.unwrap();
    let doubts = gh.take_doubts();
    assert!(
        doubts[0].contains("1 commits are listed of 0"),
        "{doubts:?}"
    );
}

/// Checks for a commit other than the PR's head don't add up.
#[tokio::test]
async fn a_prs_checks_are_for_its_head() {
    let (gh, _) = github(vec![Reply::new(
        200,
        r#"{"data":{"repository":{"pullRequest":{"headRefOid":"new0000","commits":{"nodes":[{"commit":{"oid":"old0000","statusCheckRollup":null}}]}}}}}"#,
    )])
    .await;
    let checks = gh.pr_checks(&PrRef::parse("o/r#7").unwrap()).await.unwrap();
    assert_eq!(checks.oid, "old0000");
    let doubts = gh.take_doubts();
    assert!(
        doubts[0].contains("o/r#7's checks are for old0000, but its head is new0000"),
        "{doubts:?}"
    );
}

/// A page shorter than its size that says there are more, or with more
/// items than the total, doesn't add up.
#[tokio::test]
async fn a_list_page_is_checked_against_itself() {
    let (gh, _) = github(vec![Reply::new(
        200,
        r#"{"data":{"user":{"gists":{"totalCount":1,"pageInfo":{"hasNextPage":true,"endCursor":"c"},"nodes":[
            {"name":"a","description":null,"updatedAt":"2026-10-01T00:00:00Z","stargazerCount":0,"files":[],"comments":{"totalCount":0}},
            {"name":"b","description":null,"updatedAt":"2026-10-01T00:00:00Z","stargazerCount":0,"files":[],"comments":{"totalCount":0}}]}}}}"#,
    )])
    .await;
    gh.gists("me", None).await.unwrap();
    let doubts = gh.take_doubts();
    assert!(
        doubts
            .iter()
            .any(|d| d.contains("me's gists: a page of 2 (of 30) says there are more")),
        "{doubts:?}"
    );
    assert!(
        doubts
            .iter()
            .any(|d| d.contains("me's gists: a page of 2, of 1 in all")),
        "{doubts:?}"
    );
    // The list as kept is checked too (named by its cache key).
    assert!(
        doubts
            .iter()
            .any(|d| d.contains("gists:me: a page of 2, of 1 in all")),
        "{doubts:?}"
    );
}

/// "Go to file" lists files only, and says when GitHub cut the tree short.
#[tokio::test]
async fn the_file_list_is_files_and_says_when_cut() {
    let (gh, seen) = github(vec![Reply::new(
        200,
        r#"{"tree":[{"path":"src","type":"tree"},{"path":"src/a.rs","type":"blob"},{"path":"sub","type":"commit"}],"truncated":true}"#,
    )])
    .await;
    let (files, cut) = gh.file_list(&RepoId::new("o", "r"), "main").await.unwrap();
    assert_eq!((files, cut), (vec!["src/a.rs".to_owned()], true));
    assert!(
        seen.lock().unwrap()[0]
            .request_line
            .contains("/git/trees/main?recursive=1")
    );
}

/// Checks asked for by full commit ID are that commit's, or don't add up;
/// by a branch name they're whatever it points at.
#[tokio::test]
async fn a_commits_checks_are_for_that_commit() {
    let checks = |oid: &str| {
        Reply::new(
            200,
            format!(
                r#"{{"data":{{"repository":{{"object":{{"__typename":"Commit","oid":"{oid}","statusCheckRollup":null}}}}}}}}"#
            ),
        )
    };
    let full = "a".repeat(40);
    let (gh, _) = github(vec![
        checks(&full),
        checks(&"b".repeat(40)),
        checks(&"c".repeat(40)),
    ])
    .await;
    let repo = RepoId::new("o", "r");
    gh.commit_checks(&repo, &full).await.unwrap();
    assert_eq!(gh.take_doubts(), Vec::<String>::new());
    gh.commit_checks(&repo, &full).await.unwrap();
    let doubts = gh.take_doubts();
    assert!(
        doubts[0].contains("the checks asked for aaaaaaa are for bbbbbbb"),
        "{doubts:?}"
    );
    gh.commit_checks(&repo, "main").await.unwrap();
    assert_eq!(gh.take_doubts(), Vec::<String>::new());
}

/// A category not among the first 50 of more may be past them; a page
/// exactly as long as the total adds up.
#[tokio::test]
async fn a_category_past_those_fetched_may_exist() {
    let categories = |total: u32| {
        Reply::new(
            200,
            format!(
                r#"{{"data":{{"repository":{{"discussionCategories":{{"totalCount":{total},"nodes":[{{"id":"C1","name":"Ideas","slug":"ideas"}}]}}}}}}}}"#
            ),
        )
    };
    let (gh, _) = github(vec![categories(60), categories(1)]).await;
    let of = ghtui::api::browse::DiscussionsOf::Repo(RepoId::new("o", "r"));
    match gh.discussions(&of, Some("nope"), None).await {
        Err(ApiError::NotFound(why)) => assert!(why.contains("past the first 1 of 60"), "{why}"),
        other => panic!("{other:?}"),
    }
    match gh.discussions(&of, Some("nope"), None).await {
        Err(ApiError::NotFound(why)) => assert!(!why.contains("past"), "{why}"),
        other => panic!("{other:?}"),
    }
    let (gh, _) = github(vec![Reply::new(
        200,
        r#"{"data":{"user":{"gists":{"totalCount":1,"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[
            {"name":"a","description":null,"updatedAt":"2026-10-01T00:00:00Z","stargazerCount":0,"files":[],"comments":{"totalCount":0}}]}}}}"#,
    )])
    .await;
    gh.gists("me", None).await.unwrap();
    assert_eq!(gh.take_doubts(), Vec::<String>::new());
}

/// A category the repository doesn't have is not found, not every
/// discussion under its name.
#[tokio::test]
async fn an_unknown_discussion_category_is_not_found() {
    let (gh, _) = github(vec![Reply::new(
        200,
        r#"{"data":{"repository":{"discussionCategories":{"totalCount":1,"nodes":[{"id":"C1","name":"Ideas","slug":"ideas"}]}}}}"#,
    )])
    .await;
    let of = ghtui::api::browse::DiscussionsOf::Repo(RepoId::new("o", "r"));
    let result = gh.discussions(&of, Some("nope"), None).await;
    assert!(matches!(result, Err(ApiError::NotFound(_))), "{result:?}");
}

/// A directory's entries past the first 100 get their latest commits too,
/// a query for each 100.
#[tokio::test]
async fn last_commits_cover_big_directories() {
    let commit = r#"{"nodes":[{"oid":"abc","messageHeadline":"h","committedDate":"2026-10-01T00:00:00Z","author":{"name":"a","user":null}}]}"#;
    let reply = |n: usize| {
        let fields: Vec<String> = (0..n).map(|i| format!(r#""e{i}":{commit}"#)).collect();
        Reply::new(
            200,
            format!(
                r#"{{"data":{{"repository":{{"object":{{{}}}}}}}}}"#,
                fields.join(",")
            ),
        )
    };
    let (gh, seen) = github(vec![reply(100), reply(50)]).await;
    let names: Vec<String> = (0..150).map(|i| format!("f{i}")).collect();
    let found = gh
        .last_commits(&RepoId::new("o", "r"), "HEAD", "src", &names)
        .await
        .unwrap();
    assert_eq!(found.len(), 150);
    assert!(found.contains_key("f149"));
    let seen = seen.lock().unwrap();
    assert!(
        seen[1]
            .query()
            .contains(r#"e0: history(first: 1, path: "src/f100")"#)
    );
}

/// Advisories page by the cursor in GitHub's `Link` header (as
/// api.github.com sends it), and a full page says there are more.
#[tokio::test]
async fn advisories_page_by_cursor() {
    let advisory = r#"{"ghsa_id":"GHSA-1","summary":"s","severity":"high","published_at":null,"vulnerabilities":[]}"#;
    let (gh, seen) = github(vec![
        Reply::new(200, format!("[{advisory}]")).header(
            "link",
            r#"<https://api.github.com/advisories?type=reviewed&per_page=30&after=Y3Vyc29yOnYyOpK0%3D%3D>; rel="next""#,
        ),
        Reply::new(200, format!("[{advisory}]")),
    ])
    .await;
    let first = gh.advisories(None, None).await.unwrap();
    assert_eq!(first.next.as_deref(), Some("Y3Vyc29yOnYyOpK0=="));
    let second = gh.advisories(None, first.next).await.unwrap();
    assert_eq!(second.next, None);
    let seen = seen.lock().unwrap();
    assert!(
        seen[1]
            .request_line
            .contains("&after=Y3Vyc29yOnYyOpK0%3D%3D "),
        "{}",
        seen[1].request_line
    );
}

/// A job's REST body, `status` and `conclusion` as given.
fn job_json(status: &str, conclusion: &str) -> String {
    format!(
        r#"{{"id":2,"run_id":7,"name":"test","status":"{status}","conclusion":{conclusion},"started_at":null,"completed_at":null,"steps":[]}}"#
    )
}

/// A running job has no log yet (GitHub's link for it is a missing blob),
/// so none is fetched; an expired one says it has expired. Both bodies are
/// GitHub's.
#[tokio::test]
async fn a_jobs_log_says_why_it_isnt_there() {
    let repo = RepoId::new("o", "r");
    let (gh, seen) = github(vec![Reply::new(200, job_json("in_progress", "null"))]).await;
    let log = gh.job_log(&repo, 2).await.unwrap();
    assert!(log.running && log.text.is_empty());
    assert_eq!(seen.lock().unwrap().len(), 1);

    let (gh, _) = github(vec![
        Reply::new(200, job_json("completed", r#""failure""#)),
        Reply::new(
            410,
            r#"{"message":"Server Error","documentation_url":"https://docs.github.com/rest/actions/workflow-jobs#download-job-logs-for-a-workflow-run","status":"410"}"#,
        ),
    ])
    .await;
    match gh.job_log(&repo, 2).await {
        Err(ApiError::Gone(why)) => assert!(why.contains("expired"), "{why}"),
        other => panic!("{other:?}"),
    }
}

/// A job that's completed without saying how isn't running, so its log is
/// fetched rather than waited for.
#[tokio::test]
async fn a_completed_job_without_a_conclusion_has_its_log_fetched() {
    let (gh, seen) = github(vec![
        Reply::new(200, job_json("completed", "null")),
        Reply::new(200, "2026-10-01T00:00:00.0000000Z done\n"),
    ])
    .await;
    let log = gh.job_log(&RepoId::new("o", "r"), 2).await.unwrap();
    assert!(!log.running, "{log:?}");
    assert_eq!(seen.lock().unwrap().len(), 2);
}

/// A pending review comes with the commit it's on, so comments for another
/// diff aren't added to it, and how many comments it holds, so none are
/// published unseen.
#[tokio::test]
async fn a_pending_review_says_its_commit() {
    let (gh, seen) = github(vec![Reply::new(
        200,
        r#"{"data":{"repository":{"pullRequest":{"id":"PR_1","reviews":{"nodes":[{"id":"R_1","commit":{"oid":"abc"},"comments":{"totalCount":2}}]}}}}}"#,
    )])
    .await;
    let (_, pending) = gh
        .pending_review(&PrRef::parse("o/r#7").unwrap())
        .await
        .unwrap();
    let pending = pending.unwrap();
    assert_eq!(
        (pending.id, pending.commit, pending.comments),
        (NodeId::new("R_1"), Some("abc".into()), 2)
    );
    assert!(seen.lock().unwrap()[0].body.contains("commit"));
}

#[tokio::test]
async fn review_submission_calls() {
    use ghtui::api::model::{NewThread, ReviewEvent, Side};
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
    gh.change(&reply()).await.unwrap();
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
    let (gh, seen) = github(vec![
        Reply::new(
            200,
            r#"{"data":{"repository":{"pullRequest":{"reviews":{"nodes":[
                {"state":"COMMENTED","commit":{"oid":"old"}},
                {"state":"APPROVED","commit":{"oid":"newer"}},
                {"state":"PENDING","commit":{"oid":"draft"}}]}}}}}"#,
        ),
        // No reviews on a PR that's there is no last review, not a missing PR.
        Reply::new(
            200,
            r#"{"data":{"repository":{"pullRequest":{"reviews":null}}}}"#,
        ),
    ])
    .await;
    let pr = PrRef::parse("o/r#7").unwrap();
    let commit = gh.last_review_commit(&pr, "me").await.unwrap();
    assert_eq!(commit.as_deref(), Some("newer"));
    assert_eq!(gh.last_review_commit(&pr, "me").await.unwrap(), None);
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
        gh.change(&reply()).await,
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
    // A privileged port, which no test's server can be given: a port freed
    // here could be another test's by the time this connects.
    let gh = client("http://127.0.0.1:1", Store::disabled());
    let started = std::time::Instant::now();
    assert!(matches!(
        gh.change(&reply()).await,
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

/// A README that couldn't be fetched isn't reported (or kept) as none,
/// and the overview doesn't wait on it.
#[tokio::test]
async fn a_failed_readme_isnt_taken_for_no_readme() {
    let recorded: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/corpus/3e059505efc156de.json"
        ))
        .unwrap(),
    )
    .unwrap();
    let overview = recorded["response"].as_str().unwrap().to_owned();
    let bad = || Reply::new(502, r#"{"message":"bad gateway"}"#).on("/readme");
    let (base, _) = serve(vec![
        Reply::new(200, overview).on("/graphql"),
        bad(),
        bad(),
        bad(),
        Reply::new(404, r#"{"message":"Not Found"}"#).on("/readme"),
    ])
    .await;
    let dir = tempfile::tempdir().unwrap();
    let gh = client(&base, Store::open(&dir.path().join("cache.redb")));
    let repo = RepoId::new("atom", "atom");
    gh.repo(&repo).await.unwrap();
    let readme = gh.readme(&repo).await;
    assert!(
        readme.is_err(),
        "the README failed with a 502, but it reads as {readme:?}"
    );
    let key = ghtui::api::browse::keys::readme(&repo);
    let kept: Option<ghtui::store::Cached<Option<ghtui::api::browse::Readme>>> = gh.cached(&key);
    assert!(kept.is_none(), "a failed README was kept: {kept:?}");
    assert_eq!(gh.readme(&repo).await.unwrap(), None, "a 404 is no README");
    let kept: Option<ghtui::store::Cached<Option<ghtui::api::browse::Readme>>> = gh.cached(&key);
    assert_eq!(kept.map(|k| k.value), Some(None), "no README is kept");
}

/// An organization's discussions are found however many pages of search
/// results (within GitHub's thousand) come before them.
#[tokio::test]
async fn an_organizations_discussions_are_found_past_the_fifth_page() {
    let other = r#"{"url":"https://github.com/acme/app/discussions/1","repository":{"nameWithOwner":"acme/app"}}"#;
    let mut replies: Vec<Reply> = (1..=5)
        .map(|n| {
            Reply::new(
                200,
                format!(
                    r#"{{"data":{{"search":{{"discussionCount":900,"pageInfo":{{"hasNextPage":true,"endCursor":"c{n}"}},"nodes":[{other}]}}}}}}"#
                ),
            )
        })
        .collect();
    replies.push(Reply::new(
        200,
        r#"{"data":{"search":{"discussionCount":9,"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[{"url":"https://github.com/orgs/acme/discussions/9","repository":{"nameWithOwner":"acme/community"}}]}}}"#,
    ));
    replies.push(Reply::new(200, r#"{"data":{"repository":null}}"#));
    let (gh, _) = github(replies).await;
    let of = ghtui::api::browse::DiscussionsOf::Org("acme".into());
    let result = gh.discussion(&of, 9).await;
    assert!(
        matches!(result, Err(ApiError::NotFound(ref m)) if m.contains("acme/community")),
        "the sixth page's repository wasn't looked in: {result:?}"
    );
}

/// A search that never finds an organization's repository stops at its
/// thousand results: the count says it stopped short, so it's just not
/// found, not data left out.
#[tokio::test]
async fn an_organizations_discussions_not_found_leave_nothing_out() {
    let other = r#"{"url":"https://github.com/acme/app/discussions/1","repository":{"nameWithOwner":"acme/app"}}"#;
    let replies = (1..=12).map(|n| {
        Reply::new(
            200,
            format!(
                r#"{{"data":{{"search":{{"discussionCount":2000,"pageInfo":{{"hasNextPage":true,"endCursor":"c{n}"}},"nodes":[{other}]}}}}}}"#
            ),
        )
    });
    let (gh, seen) = github(replies.collect()).await;
    let of = ghtui::api::browse::DiscussionsOf::Org("acme".into());
    let result = gh.discussion(&of, 9).await;
    assert!(matches!(result, Err(ApiError::NotFound(_))), "{result:?}");
    assert_eq!(seen.lock().unwrap().len(), 10);
    assert_eq!(gh.take_left_out(), Vec::<String>::new());
}

/// A cursor GitHub hands back twice ends the reading instead of asking for
/// the same page again and again.
#[tokio::test]
async fn a_repeated_cursor_ends_the_review_threads() {
    let page = r#"{"data":{"repository":{"pullRequest":{"reviewThreads":{"pageInfo":{"hasNextPage":true,"endCursor":"same"},"nodes":[]}}}}}"#;
    let (gh, seen) = github(vec![Reply::new(200, page); 20]).await;
    let result = gh.review_threads(&PrRef::parse("o/r#7").unwrap()).await;
    let asked = seen.lock().unwrap().len();
    assert_eq!((asked, gh.take_doubts().len()), (2, 1), "{result:?}");
    assert_eq!(
        gh.take_left_out(),
        Vec::<String>::new(),
        "a doubt, not a cut"
    );
}

/// A PR's patches stop at GitHub's cap of 3000 files, and say they did.
#[tokio::test]
async fn patches_past_the_cap_are_left_out() {
    let full: Vec<String> = (0..100)
        .map(|i| format!(r#"{{"filename":"f{i}.rs"}}"#))
        .collect();
    let full = format!("[{}]", full.join(","));
    let (gh, seen) = github(vec![Reply::new(200, full); 31]).await;
    let files = gh
        .pr_patches(&PrRef::parse("o/r#7").unwrap())
        .await
        .unwrap();
    assert_eq!((files.len(), seen.lock().unwrap().len()), (3000, 30));
    assert_eq!(gh.take_left_out(), ["o/r#7's patches: only the first 3000"]);
}

/// A run with more jobs than are fetched says how many it left out: 1050
/// jobs, of which ten pages of 100 are read, keep the other 50 counted.
#[tokio::test]
async fn a_runs_jobs_past_the_cap_are_counted() {
    let job = |id: u64| {
        format!(
            r#"{{"id":{id},"run_id":9,"name":"j{id}","status":"completed","conclusion":"success"}}"#
        )
    };
    let page = |ids: std::ops::Range<u64>| {
        let jobs: Vec<String> = ids.map(job).collect();
        format!(r#"{{"total_count":1050,"jobs":[{}]}}"#, jobs.join(","))
    };
    let base = 7_000_000;
    let mut replies = vec![
        Reply::new(200, r#"{"id":9,"run_number":1,"event":"push","head_sha":"abc","status":"completed","conclusion":"success"}"#)
            .on("/runs/9 "),
        Reply::new(200, page(base..base + 100)).on("jobs?per_page=100 "),
        Reply::new(200, ARTIFACTS).on("/artifacts"),
    ];
    for n in 2..=11u64 {
        let on: &'static str = Box::leak(format!("&page={n} ").into_boxed_str());
        let start = base + (n - 1) * 100;
        let end = (start + 100).min(base + 1050);
        replies.push(Reply::new(200, page(start..end)).on(on));
    }
    let (gh, _) = github(replies).await;
    let run = gh
        .workflow_run(&RepoId::new("o", "r"), 9, None)
        .await
        .unwrap();
    assert_eq!((run.jobs.len(), run.jobs.total()), (1000, 1050));
    assert!(
        gh.take_left_out().is_empty(),
        "the total says what's left out"
    );
}

/// A search answer without GitHub's count of discussions is an error, not
/// a search that looks cut off.
#[tokio::test]
async fn an_uncounted_discussion_search_is_an_error() {
    let page =
        r#"{"data":{"search":{"pageInfo":{"hasNextPage":true,"endCursor":"c"},"nodes":[]}}}"#;
    let (gh, seen) = github(vec![Reply::new(200, page); 12]).await;
    let of = ghtui::api::browse::DiscussionsOf::Org("acme".into());
    let result = gh.discussion(&of, 9).await;
    assert!(result.is_err(), "{result:?}");
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(gh.take_left_out(), Vec::<String>::new());
}

/// GitHub's answer for a root it can't resolve: the root is null and one
/// NOT_FOUND error, pathed at it (`path` as JSON), says why.
fn root_not_found(data: &str, path: &str, message: &str) -> Reply {
    Reply::new(
        200,
        format!(
            r#"{{"data":{data},"errors":[{{"type":"NOT_FOUND","path":{path},"message":"{message}"}}]}}"#
        ),
    )
}

#[tokio::test]
async fn a_missing_pull_request_isnt_also_left_out() {
    let (gh, _) = github(vec![root_not_found(
        r#"{"repository":{"pullRequest":null,"mergeCommitAllowed":true,"squashMergeAllowed":true,"rebaseMergeAllowed":true,"viewerDefaultMergeMethod":"MERGE","viewerPermission":"READ","autoMergeAllowed":false},"rateLimit":null}"#,
        r#"["repository","pullRequest"]"#,
        "Could not resolve to a PullRequest with the number of 999999.",
    )])
    .await;
    let pr = PrRef::parse("o/r#999999").unwrap();
    assert!(matches!(
        gh.pull_request(&pr).await,
        Err(ApiError::NotFound(_))
    ));
    assert_eq!(gh.take_left_out(), Vec::<String>::new());
}

#[tokio::test]
async fn a_missing_repository_isnt_also_left_out() {
    let (gh, _) = github(vec![root_not_found(
        r#"{"repository":null,"rateLimit":null}"#,
        r#"["repository"]"#,
        "Could not resolve to a Repository with the name 'nosuch/repo'.",
    )])
    .await;
    let repo = RepoId::new("nosuch", "repo");
    assert!(matches!(gh.repo(&repo).await, Err(ApiError::NotFound(_))));
    assert_eq!(gh.take_left_out(), Vec::<String>::new());
}

#[tokio::test]
async fn a_missing_repositorys_milestones_arent_also_left_out() {
    let (gh, _) = github(vec![root_not_found(
        r#"{"repository":null}"#,
        r#"["repository"]"#,
        "Could not resolve to a Repository with the name 'nosuch/repo'.",
    )])
    .await;
    let repo = RepoId::new("nosuch", "repo");
    assert!(matches!(
        gh.milestones(&repo, false, None).await,
        Err(ApiError::NotFound(_))
    ));
    assert_eq!(gh.take_left_out(), Vec::<String>::new());
}

#[tokio::test]
async fn a_missing_profile_isnt_also_left_out() {
    let (gh, _) = github(vec![root_not_found(
        r#"{"user":null,"organization":null,"user_readme":null,"org_readme":null}"#,
        r#"["user"]"#,
        "Could not resolve to a User with the login of 'nosuch'.",
    )])
    .await;
    let got = gh.profile("nosuch").await;
    assert!(matches!(got, Err(ApiError::NotFound(_))), "{:?}", got.err());
    assert_eq!(gh.take_left_out(), Vec::<String>::new());
}

/// An organization's profile: GitHub's NOT_FOUND for the user of the same
/// login (and the README it doesn't have) only explains those nulls.
#[tokio::test]
async fn an_organizations_profile_leaves_nothing_out() {
    let (gh, _) = github(vec![Reply::new(
        200,
        r#"{"data":{"user":null,"organization":{"login":"o","name":null,"description":null,"location":null,"websiteUrl":null,"pinnedItems":{"nodes":[]},"repositories":{"totalCount":0,"nodes":[]},"isVerified":false,"membersWithRole":{"totalCount":0,"nodes":[]}},"user_readme":null,"org_readme":null},
            "errors":[{"type":"NOT_FOUND","path":["user"],"message":"Could not resolve to a User with the login of 'o'."},
                      {"type":"NOT_FOUND","path":["user_readme"],"message":"Could not resolve to a Repository with the name 'o/o'."}]}"#,
    )])
    .await;
    let got = gh.profile("o").await;
    assert!(got.is_ok(), "{:?}", got.err());
    assert_eq!(gh.take_left_out(), Vec::<String>::new());
}

/// Only a NOT_FOUND explains a null: what the token may not see is still
/// left out, beside a repository that was found.
#[tokio::test]
async fn a_forbidden_field_beside_a_found_repo_is_still_left_out() {
    let forbidden = "Resource not accessible by integration";
    let (gh, _) = github(vec![Reply::new(
        200,
        format!(
            r#"{{"data":{{"repository":{{"heads":{{"totalCount":0,"pageInfo":{{"hasNextPage":false,"endCursor":null}},"nodes":[null]}},"tags":{{"totalCount":0,"pageInfo":{{"hasNextPage":false,"endCursor":null}},"nodes":[]}}}}}},"errors":[{{"type":"FORBIDDEN","path":["repository","heads","nodes",0],"message":"{forbidden}"}}]}}"#
        ),
    )])
    .await;
    gh.refs(&RepoId::new("o", "r")).await.unwrap();
    assert_eq!(gh.take_left_out(), [forbidden]);
}

/// An error without a message is still an error: it's left out by its
/// type, or as unexplained, beside the data.
#[tokio::test]
async fn an_error_without_a_message_is_still_left_out() {
    let refs = r#""heads":{"totalCount":0,"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[]},"tags":{"totalCount":0,"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[]}"#;
    let (gh, _) = github(vec![Reply::new(
        200,
        format!(
            r#"{{"data":{{"repository":{{{refs}}}}},"errors":[{{"path":["repository"]}},{{"type":"FORBIDDEN"}}]}}"#
        ),
    )])
    .await;
    gh.refs(&RepoId::new("o", "r")).await.unwrap();
    assert_eq!(gh.take_left_out(), ["an unexplained error", "FORBIDDEN"]);
}

/// A mutation answered with an error without a message still fails: the
/// change wasn't made.
#[tokio::test]
async fn a_mutation_answered_with_a_bare_error_still_fails() {
    let (gh, _) = github(vec![Reply::new(
        200,
        r#"{"data":{"markFileAsViewed":null},"errors":[{"type":"FORBIDDEN","path":["markFileAsViewed"]}]}"#,
    )])
    .await;
    match gh.set_viewed(&NodeId::new("n"), "x", true).await {
        Err(ApiError::GraphQl(errors)) => assert_eq!(errors, ["FORBIDDEN"]),
        other => panic!("{other:?}"),
    }
}

/// A repository gone while a commit's checks are paged is not found, not
/// the checks of the pages fetched so far.
#[tokio::test]
async fn a_repository_gone_between_check_pages_is_not_found() {
    let contexts =
        r#"{"totalCount":2,"pageInfo":{"hasNextPage":true,"endCursor":"p2"},"nodes":[]}"#;
    let (gh, _) = github(vec![
        Reply::new(
            200,
            format!(
                r#"{{"data":{{"repository":{{"object":{{"__typename":"Commit","oid":"abc","statusCheckRollup":{{"contexts":{contexts}}}}}}}}}}}"#
            ),
        ),
        root_not_found(
            r#"{"repository":null}"#,
            r#"["repository"]"#,
            "Could not resolve to a Repository with the name 'o/r'.",
        ),
    ])
    .await;
    let result = gh.commit_checks(&RepoId::new("o", "r"), "main").await;
    assert!(matches!(result, Err(ApiError::NotFound(_))), "{result:?}");
    assert_eq!(gh.take_left_out(), Vec::<String>::new());
}

/// A repository gone by the time its discussions (or its entries' last
/// commits) are asked for is not found, not an empty or undecodable list.
#[tokio::test]
async fn a_repository_gone_under_a_raw_query_is_not_found() {
    let gone = || {
        root_not_found(
            r#"{"repository":null}"#,
            r#"["repository"]"#,
            "Could not resolve to a Repository with the name 'o/r'.",
        )
    };
    let categories = Reply::new(
        200,
        r#"{"data":{"repository":{"discussionCategories":{"totalCount":0,"nodes":[]}}}}"#,
    );
    // Each is checked for being blocked, and isn't.
    let missing = || Reply::new(404, "{}");
    let (gh, _) = github(vec![categories, gone(), missing(), gone(), missing()]).await;
    let repo = RepoId::new("o", "r");
    let of = ghtui::api::browse::DiscussionsOf::Repo(repo.clone());
    let result = gh.discussions(&of, None, None).await;
    assert!(matches!(result, Err(ApiError::NotFound(_))), "{result:?}");
    let names = ["a".to_owned()];
    let result = gh.last_commits(&repo, "HEAD", "", &names).await;
    assert!(matches!(result, Err(ApiError::NotFound(_))), "{result:?}");
    assert_eq!(gh.take_left_out(), Vec::<String>::new());
}

/// A NOT_FOUND pathed at something the data does hold explains no null, so
/// it is still left out.
#[tokio::test]
async fn a_not_found_on_a_present_field_is_still_left_out() {
    let odd = "Could not resolve to a Ref.";
    let (gh, _) = github(vec![Reply::new(
        200,
        format!(
            r#"{{"data":{{"repository":{{"heads":{{"totalCount":0,"pageInfo":{{"hasNextPage":false,"endCursor":null}},"nodes":[]}},"tags":{{"totalCount":0,"pageInfo":{{"hasNextPage":false,"endCursor":null}},"nodes":[]}}}}}},"errors":[{{"type":"NOT_FOUND","path":["repository","heads"],"message":"{odd}"}}]}}"#
        ),
    )])
    .await;
    gh.refs(&RepoId::new("o", "r")).await.unwrap();
    assert_eq!(gh.take_left_out(), [odd]);
}

/// A mutation fails on any error, a NOT_FOUND that explains its null too.
#[tokio::test]
async fn a_mutation_answered_with_not_found_still_fails() {
    let (gh, _) = github(vec![root_not_found(
        r#"{"markFileAsViewed":null}"#,
        r#"["markFileAsViewed"]"#,
        "Could not resolve to a node with the global id of 'bad'.",
    )])
    .await;
    match gh.set_viewed(&NodeId::new("bad"), "x", true).await {
        Err(ApiError::GraphQl(errors)) => assert_eq!(
            errors,
            ["Could not resolve to a node with the global id of 'bad'."]
        ),
        other => panic!("{other:?}"),
    }
    assert_eq!(gh.take_left_out(), Vec::<String>::new());
}

/// A raw query's root nulled by SSO (FORBIDDEN) isn't called not found: the
/// branches fail with GitHub's reason, said once, not also left out.
#[tokio::test]
async fn a_raw_root_nulled_by_sso_says_why_once() {
    let sso = "Resource protected by organization SAML enforcement.";
    let (gh, _) = github(vec![Reply::new(
        200,
        format!(
            r#"{{"data":{{"repository":null}},"errors":[{{"type":"FORBIDDEN","path":["repository"],"message":"{sso}"}}]}}"#
        ),
    )])
    .await;
    match gh.branches(&RepoId::new("o", "r"), None).await {
        Err(ApiError::GraphQl(errors)) => assert_eq!(errors, [sso]),
        other => panic!("{other:?}"),
    }
    assert_eq!(gh.take_left_out(), Vec::<String>::new());
}

/// A REST 404 says what wasn't found, not GitHub's bare "Not Found".
#[tokio::test]
async fn a_missing_job_names_what_wasnt_found() {
    let (gh, _) = github(vec![Reply::new(
        404,
        r#"{"message":"Not Found","documentation_url":"https://docs.github.com/rest/actions/workflow-jobs#get-a-job-for-a-workflow-run","status":"404"}"#,
    )])
    .await;
    match gh.job(&RepoId::new("o", "r"), 2).await {
        Err(err @ ApiError::NotFound(_)) => {
            let said = err.to_string();
            assert!(said.contains("o/r"), "{said}");
        }
        other => panic!("{other:?}"),
    }
}

/// A missing repository's stargazers are not found by the repository's
/// name, not by ghtui's debug output.
#[tokio::test]
async fn missing_stargazers_name_the_repository() {
    let (gh, _) = github(vec![root_not_found(
        r#"{"node":null}"#,
        r#"["node"]"#,
        "Could not resolve to a Repository with the name 'o/r'.",
    )])
    .await;
    let list = ghtui::api::browse::UserList::Stargazers(RepoId::new("o", "r"));
    match gh.users(&list, None).await {
        Err(err @ ApiError::NotFound(_)) => {
            let said = err.to_string();
            assert!(said.contains("o/r") && !said.contains("RepoId"), "{said}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(gh.take_left_out(), Vec::<String>::new());
}

/// A repository GitHub blocks says so, why, and where the notice is, on
/// every page: REST answers 451, and GraphQL calls it missing, so its
/// "not found" is checked with REST.
#[tokio::test]
async fn a_blocked_repository_says_why_on_every_page() {
    const BLOCKED: &str = r#"{"message":"Repository access blocked","block":{"reason":"dmca","created_at":"2025-01-03T15:17:52Z","html_url":"https://github.com/github/dmca/blob/master/2025/01/notice.md"}}"#;
    let missing = r#"{"data":{"repository":null},"errors":[{"type":"NOT_FOUND","path":["repository"],"message":"Could not resolve to a Repository with the name 'o/r'."}]}"#;
    let says_why = |err: ApiError| {
        let text = err.to_string();
        assert!(matches!(err, ApiError::Blocked(_)), "{err:?}");
        assert!(
            text.contains("DMCA") && text.contains("/notice.md"),
            "{text}"
        );
    };
    let repo = RepoId::new("o", "r");
    let (gh, seen) = github(vec![Reply::new(200, missing), Reply::new(451, BLOCKED)]).await;
    says_why(gh.repo(&repo).await.unwrap_err());
    assert!(
        seen.lock().unwrap()[1]
            .request_line
            .starts_with("GET /repos/o/r ")
    );
    let (gh, _) = github(vec![Reply::new(451, BLOCKED)]).await;
    says_why(gh.runs(&repo, "workflow:ci.yml", None).await.unwrap_err());

    // One that's only missing is not found, by name.
    let (gh, _) = github(vec![Reply::new(200, missing), Reply::new(404, "{}")]).await;
    let err = gh.repo(&repo).await.unwrap_err();
    assert_eq!(err.to_string(), "not found: o/r");
    let (gh, _) = github(vec![Reply::new(404, r#"{"message":"Not Found"}"#)]).await;
    let err = gh.job(&repo, 2).await.unwrap_err();
    assert_eq!(err.to_string(), "not found: o/r's job 2");
}

/// The Actions tab's filter asks REST in its own words: a workflow named
/// is found by its name, a later word of a kind wins, and what runs can't
/// be filtered by is left out (and said, by the page).
#[tokio::test]
async fn a_runs_filter_becomes_rest_parameters() {
    let workflows = r#"{"total_count":1,"workflows":[{"id":42,"name":"Build and test","path":".github/workflows/ci.yml","state":"active"}]}"#;
    let runs = r#"{"total_count":0,"workflow_runs":[]}"#;
    let (gh, seen) = github(vec![Reply::new(200, workflows), Reply::new(200, runs)]).await;
    let query = r#"workflow:"build and test" branch:main is:failure is:success event:push actor:octocat label:bug"#;
    let repo = RepoId::new("o", "r");
    gh.runs(&repo, query, Some("2".into())).await.unwrap();
    let lines: Vec<String> = (seen.lock().unwrap().iter())
        .map(|s| s.request_line.clone())
        .collect();
    assert!(
        lines[0].starts_with("GET /repos/o/r/actions/workflows?per_page=100 "),
        "{lines:?}"
    );
    assert!(
        lines[1].starts_with(
            "GET /repos/o/r/actions/workflows/42/runs?branch=main&status=success&event=push&actor=octocat&per_page=30&page=2 "
        ),
        "{lines:?}"
    );
    let filter = ghtui::api::browse::RunFilter::parse(query);
    assert_eq!(filter.ignored, ["label:bug"]);

    // Without a workflow, all of the repository's runs.
    let (gh, seen) = github(vec![Reply::new(200, runs)]).await;
    gh.runs(&repo, "", None).await.unwrap();
    let line = seen.lock().unwrap()[0].request_line.clone();
    assert!(
        line.starts_with("GET /repos/o/r/actions/runs?per_page=30&page=1 "),
        "{line}"
    );
}

/// A thread in REST's words, about pull request `id`.
fn thread_json(id: u32, reason: &str, unread: bool) -> String {
    format!(
        r#"{{"id":"{id}","unread":{unread},"reason":"{reason}","updated_at":"2026-10-03T12:00:00Z",
          "subject":{{"title":"T{id}","type":"PullRequest","url":"https://api.github.com/repos/o/r/pulls/{id}",
                      "latest_comment_url":"https://api.github.com/repos/o/r/issues/comments/9"}},
          "repository":{{"full_name":"o/r"}}}}"#
    )
}

/// The inbox's filter asks REST in its own words: `repo:` is the
/// repository's notifications, `is:unread` leaves out `all`,
/// `reason:participating` is REST's own; reasons and `is:read` are kept
/// here. A next page is what GitHub's `Link` says, even from the cache.
#[tokio::test]
async fn an_inbox_filter_becomes_rest_parameters() {
    let body = format!(
        "[{},{},{}]",
        thread_json(1, "mention", true),
        thread_json(2, "review_requested", true),
        thread_json(3, "mention", false)
    );
    let next = r#"<https://api.github.com/repos/o/r/notifications?page=3>; rel="next""#;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("cache.redb"));
    let (base, seen) = serve(vec![
        Reply::new(200, body.clone())
            .header("etag", "\"v1\"")
            .header("link", next),
        Reply::new(304, ""),
        Reply::new(200, body),
    ])
    .await;
    let gh = client(&base, store);
    let query = "repo:o/r is:unread";
    for _ in 0..2 {
        let inbox = gh.notifications(query, Some("2".into())).await.unwrap();
        let ids: Vec<&str> = inbox.list.items.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["1", "2"]);
        assert_eq!(inbox.list.next.as_deref(), Some("3"));
        assert_eq!(
            inbox.list.items[0].url,
            "https://github.com/o/r/pull/1#issuecomment-9"
        );
    }
    let inbox = gh
        .notifications("is:read reason:participating", None)
        .await
        .unwrap();
    let ids: Vec<&str> = inbox.list.items.iter().map(|n| n.id.as_str()).collect();
    assert_eq!((ids, inbox.list.next), (vec!["3"], None));
    let lines: Vec<String> = (seen.lock().unwrap().iter())
        .map(|s| s.request_line.clone())
        .collect();
    assert!(
        lines[0].starts_with("GET /repos/o/r/notifications?per_page=30&page=2 "),
        "{lines:?}"
    );
    assert!(
        lines[2].starts_with("GET /notifications?all=true&participating=true&per_page=30&page=1 "),
        "{lines:?}"
    );
}

/// A reason is filtered here, so a page of what GitHub sends can hold
/// few that match: pages are fetched until one fills (30), or the inbox
/// ends, or ten have been searched, which the inbox says.
#[tokio::test]
async fn a_reason_filter_fetches_pages_until_one_fills() {
    // 30 threads a page, `mentions` of them mentions.
    let page = |n: u32, mentions: u32, last: bool| {
        let threads: Vec<String> = (0..30)
            .map(|i| {
                let reason = if i < mentions {
                    "mention"
                } else {
                    "subscribed"
                };
                thread_json(n * 100 + i, reason, true)
            })
            .collect();
        let reply = Reply::new(200, format!("[{}]", threads.join(",")));
        if last {
            reply
        } else {
            reply.header(
                "link",
                r#"<https://api.github.com/notifications?page=9>; rel="next""#,
            )
        }
    };
    let (gh, seen) = github(vec![
        page(1, 10, false),
        page(2, 10, false),
        page(3, 10, false),
    ])
    .await;
    let inbox = gh.notifications("reason:mention", None).await.unwrap();
    assert_eq!(inbox.list.items.len(), 30);
    assert_eq!(inbox.list.next.as_deref(), Some("4"));
    assert_eq!((inbox.searched, inbox.cut), (90, false));
    let lines: Vec<String> = (seen.lock().unwrap().iter())
        .map(|s| s.request_line.clone())
        .collect();
    assert!(
        lines[2].starts_with("GET /notifications?all=true&per_page=30&page=3 "),
        "{lines:?}"
    );

    // The inbox ends first.
    let (gh, _) = github(vec![page(1, 2, false), page(2, 1, true)]).await;
    let inbox = gh.notifications("reason:mention", None).await.unwrap();
    assert_eq!(inbox.list.items.len(), 3);
    assert_eq!((inbox.list.next, inbox.cut), (None, false));

    // Ten pages searched, it stops, and says so; Load more goes on.
    let (gh, seen) = github((1..=11).map(|n| page(n, 1, false)).collect()).await;
    let inbox = gh.notifications("reason:mention", None).await.unwrap();
    assert_eq!(seen.lock().unwrap().len(), 10);
    assert_eq!(inbox.list.items.len(), 10);
    assert_eq!(inbox.list.next.as_deref(), Some("11"));
    assert_eq!((inbox.searched, inbox.cut), (300, true));
}

/// The unread count is asked only if the inbox changed since GitHub's
/// `Last-Modified`, so an unchanged one is a 304 (free against the rate
/// limit), and polled as often as `X-Poll-Interval` asks.
#[tokio::test]
async fn the_unread_count_is_asked_if_modified_at_githubs_interval() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("cache.redb"));
    let modified = "Sat, 10 Oct 2026 12:00:00 GMT";
    let (base, seen) = serve(vec![
        Reply::new(200, r#"[{"id":"1"},{"id":"2"}]"#)
            .header("last-modified", modified)
            .header("x-poll-interval", "120")
            .header(
                "link",
                r#"<https://api.github.com/notifications?page=2>; rel="next""#,
            ),
        Reply::new(304, "").header("x-poll-interval", "90"),
    ])
    .await;
    let gh = client(&base, store);
    let unread = gh.unread().await.unwrap();
    assert_eq!(
        (unread.count, unread.more, unread.poll_secs),
        (2, true, 120)
    );
    let unread = gh.unread().await.unwrap();
    assert_eq!((unread.count, unread.more, unread.poll_secs), (2, true, 90));
    let seen = seen.lock().unwrap();
    assert!(
        seen[0]
            .request_line
            .starts_with("GET /notifications?per_page=50 "),
        "{}",
        seen[0].request_line
    );
    assert_eq!(seen[0].header("if-modified-since"), None);
    assert_eq!(seen[1].header("if-modified-since"), Some(modified));
}

/// Marking the inbox read is one request, up to the newest shown: the
/// whole inbox's, or a repository's. Done is a request a thread, and a
/// refusal partway says how far it got.
#[tokio::test]
async fn the_inbox_is_marked_read_at_once_and_done_thread_by_thread() {
    use ghtui::api::change::Change;
    let (gh, seen) = github(vec![
        Reply::new(205, ""),
        Reply::new(202, r#"{"message":"queued"}"#),
        Reply::new(204, ""),
        Reply::new(403, r#"{"message":"Forbidden"}"#),
    ])
    .await;
    let at = "2026-10-03T12:00:00Z".to_owned();
    for repo in [None, Some(RepoId::new("o", "r"))] {
        let change = Change::MarkAllRead {
            repo,
            last_read_at: at.clone(),
        };
        gh.change(&change).await.unwrap();
    }
    let thread = |id: &str| ghtui::api::notifications::ThreadAt {
        id: id.into(),
        updated_at: at.clone(),
    };
    let each = Change::MarkEach {
        threads: vec![thread("7"), thread("8")],
        done: true,
    };
    let err = gh.change(&each).await.unwrap_err();
    assert!(err.to_string().contains("(after 1 of 2)"), "{err}");
    let seen = seen.lock().unwrap();
    let lines: Vec<&str> = seen.iter().map(|s| s.request_line.as_str()).collect();
    assert!(lines[0].starts_with("PUT /notifications "), "{lines:?}");
    assert!(
        lines[1].starts_with("PUT /repos/o/r/notifications "),
        "{lines:?}"
    );
    assert!(
        lines[2].starts_with("DELETE /notifications/threads/7 "),
        "{lines:?}"
    );
    assert!(
        lines[3].starts_with("DELETE /notifications/threads/8 "),
        "{lines:?}"
    );
    let body = serde_json::json!({ "last_read_at": at, "read": true });
    assert_eq!(seen[0].json(), body);
}

/// Marking a thread read patches it; done deletes it.
#[tokio::test]
async fn threads_are_marked_read_and_done() {
    use ghtui::api::change::Change;
    let (gh, seen) = github(vec![Reply::new(205, ""), Reply::new(204, "")]).await;
    let thread = ghtui::api::notifications::ThreadAt {
        id: "42".into(),
        updated_at: "2026-10-03T12:00:00Z".into(),
    };
    gh.change(&Change::MarkRead {
        thread: thread.id.clone(),
    })
    .await
    .unwrap();
    gh.change(&Change::MarkDone { thread }).await.unwrap();
    let lines: Vec<String> = (seen.lock().unwrap().iter())
        .map(|s| s.request_line.clone())
        .collect();
    assert!(
        lines[0].starts_with("PATCH /notifications/threads/42 "),
        "{lines:?}"
    );
    assert!(
        lines[1].starts_with("DELETE /notifications/threads/42 "),
        "{lines:?}"
    );
}

/// GitHub's API goes on listing a thread marked done, read. The account
/// keeps the threads it marked done, and the inbox leaves each out until
/// GitHub updates it again: then it's back, for good.
#[tokio::test]
async fn a_thread_marked_done_stays_out_until_github_updates_it() {
    use ghtui::api::change::Change;
    use ghtui::api::notifications::ThreadAt;
    let thread =
        |id: u32, at: &str| thread_json(id, "mention", false).replace("2026-10-03T12:00:00Z", at);
    let (old, new) = ("2026-10-03T12:00:00Z", "2026-10-04T08:00:00Z");
    let inbox = |a: &str| Reply::new(200, format!("[{},{}]", thread(1, a), thread(2, old)));
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("cache.redb"));
    let (base, seen) = serve(vec![
        Reply::new(204, ""),
        inbox(old),
        inbox(new),
        inbox(old),
    ])
    .await;
    let gh = client(&base, store);
    let done = ThreadAt {
        id: "1".into(),
        updated_at: old.into(),
    };
    gh.change(&Change::MarkDone { thread: done }).await.unwrap();
    let ids = |inbox: ghtui::api::notifications::Inbox| -> Vec<String> {
        inbox.list.items.into_iter().map(|n| n.id).collect()
    };
    assert_eq!(ids(gh.notifications("", None).await.unwrap()), ["2"]);
    // A new comment: it's back, and stays.
    assert_eq!(ids(gh.notifications("", None).await.unwrap()), ["1", "2"]);
    assert_eq!(ids(gh.notifications("", None).await.unwrap()), ["1", "2"]);
    let first = seen.lock().unwrap()[0].request_line.clone();
    assert!(
        first.starts_with("DELETE /notifications/threads/1 "),
        "{first}"
    );
}

/// Polling asks with the ETag of the answer before, and a 304 is "the
/// same" (which GitHub doesn't count against the rate limit).
#[tokio::test]
async fn polls_ask_with_the_last_etag() {
    use ghtui::api::Polled;
    let body = r#"{"head":{"sha":"abc"}}"#;
    let (gh, seen) = github(vec![
        Reply::new(200, body).header("etag", "\"p1\""),
        Reply::new(304, ""),
    ])
    .await;
    let path = "/repos/o/r/pulls/7";
    let first = gh.poll(path, None).await.unwrap();
    let etag = "\"p1\"".to_owned();
    let body = body.to_owned();
    assert_eq!(first, Polled::Changed { etag, body });
    assert_eq!(gh.poll(path, Some("\"p1\"")).await.unwrap(), Polled::Same);

    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].header("if-none-match"), None);
    assert_eq!(seen[1].header("if-none-match"), Some("\"p1\""));
    assert!(seen[1].request_line.starts_with("GET /repos/o/r/pulls/7 "));
}

/// Who and what a comment can mention: the repository's assignable
/// users, and its issues and pull requests updated last.
#[tokio::test]
async fn mentionable_asks_for_assignees_and_recent_issues() {
    let (gh, seen) = github(vec![
        Reply::new(200, r#"[{"login":"alice"}]"#).on("/assignees"),
        Reply::new(200, r#"[{"number":12,"title":"Fix it"}]"#).on("/issues"),
    ])
    .await;
    let found = gh.mentionable(&RepoId::new("o", "r")).await.unwrap();
    assert_eq!(found.users, ["alice"]);
    assert_eq!(found.issues, [(12, "Fix it".to_owned())]);
    let seen = seen.lock().unwrap();
    let asked = |part: &str| seen.iter().any(|s| s.request_line.contains(part));
    assert!(asked("GET /repos/o/r/assignees?per_page=100 "));
    assert!(asked(
        "GET /repos/o/r/issues?state=all&sort=updated&per_page=100 "
    ));
}

/// Where `gd` looks for a name the file doesn't define: GitHub's code
/// search for the word in the repository, in the file's language.
#[tokio::test]
async fn code_search_asks_for_the_word_in_the_repository() {
    let body = r#"{"total_count":1,"incomplete_results":true,"items":[{"path":"src/a.rs","sha":"abc","repository":{"full_name":"o/r"}}]}"#;
    let (gh, seen) = github(vec![Reply::new(200, body).on("/search/code")]).await;
    let hits = gh
        .code_search("offset repo:o/r language:Rust")
        .await
        .unwrap();
    assert_eq!(hits.paths, ["src/a.rs"]);
    assert!(hits.incomplete, "a search that gave up says so");
    let seen = seen.lock().unwrap();
    let line = &seen[0].request_line;
    assert!(
        line.starts_with("GET /search/code?q=offset+repo%3Ao%2Fr+language%3ARust&per_page=10 "),
        "{line}"
    );
}

fn roadmap() -> ghtui::api::project::ProjectRef {
    use ghtui::api::project::{ProjectOwner, ProjectRef};
    ProjectRef {
        owner: ProjectOwner::Org("o".into()),
        number: 1,
    }
}

/// A project at a view: its fields and views, then the items the view's
/// filter finds, each with what it is and its values (a draft's body, a
/// number as GitHub writes it, the values ghtui doesn't read left out).
#[tokio::test]
async fn a_project_reads_its_view_and_the_items_its_filter_finds() {
    use ghtui::api::project::{FieldKind, FieldValue, ItemContent, Layout};
    let project = r#"{"data":{"repositoryOwner":{"projectV2":{"id":"PVT_1","title":"Roadmap","shortDescription":null,"url":"https://github.com/orgs/o/projects/1","closed":false,"public":true,"viewerCanUpdate":true,
        "fields":{"totalCount":3,"nodes":[{"id":"F_title","name":"Title","dataType":"TITLE"},
            {"id":"F_status","name":"Status","dataType":"SINGLE_SELECT","options":[{"id":"O_todo","name":"Todo","color":"GRAY","description":""},{"id":"O_done","name":"Done","color":"GREEN","description":"Shipped"}]},
            {"id":"F_sprint","name":"Sprint","dataType":"ITERATION","configuration":{"iterations":[{"id":"I_2","title":"Sprint 2","startDate":"2026-10-01","duration":14}],"completedIterations":[{"id":"I_1","title":"Sprint 1","startDate":"2026-09-17","duration":14}]}}]},
        "views":{"totalCount":2,"nodes":[
            {"number":1,"name":"Board","layout":"BOARD_LAYOUT","filter":null,"fields":{"totalCount":2,"nodes":[{"id":"F_title"},{"id":"F_status"}]},"verticalGroupByFields":{"nodes":[{"id":"F_status"}]},"groupByFields":{"nodes":[]},"sortByFields":{"totalCount":0,"nodes":[]}},
            {"number":2,"name":"Plan","layout":"TABLE_LAYOUT","filter":"-status:Done","fields":{"totalCount":1,"nodes":[{"id":"F_title"}]},"verticalGroupByFields":{"nodes":[]},"groupByFields":{"nodes":[]},"sortByFields":{"totalCount":1,"nodes":[{"direction":"DESC","field":{"id":"F_sprint"}}]}}]}}}}}"#;
    let items = r#"{"data":{"repositoryOwner":{"projectV2":{"items":{"totalCount":4,"pageInfo":{"hasNextPage":true,"endCursor":"c3"},"nodes":[
        {"id":"PVTI_1","fullDatabaseId":"1001","content":{"__typename":"Issue","number":14,"title":"Projects","url":"https://github.com/o/r/issues/14","state":"CLOSED","stateReason":"NOT_PLANNED","repository":{"nameWithOwner":"o/r"},"assignees":{"totalCount":1,"nodes":[{"login":"alice"}]},"labels":{"totalCount":0,"nodes":[]},"milestone":null},
         "fieldValues":{"totalCount":3,"nodes":[{"__typename":"ProjectV2ItemFieldTextValue","text":"Projects","field":{"id":"F_title"}},{"__typename":"ProjectV2ItemFieldSingleSelectValue","optionId":"O_todo","name":"Todo","field":{"id":"F_status"}},{"__typename":"ProjectV2ItemFieldLabelValue"}]}},
        {"id":"PVTI_2","fullDatabaseId":"1002","content":{"__typename":"DraftIssue","id":"DI_2","title":"Draft","body":"Its body","assignees":{"totalCount":0,"nodes":[]}},
         "fieldValues":{"totalCount":1,"nodes":[{"__typename":"ProjectV2ItemFieldNumberValue","number":2.0,"field":{"id":"F_est"}}]}},
        {"id":"PVTI_3","fullDatabaseId":"1003","content":null,"fieldValues":{"totalCount":0,"nodes":[]}}]}}}}}"#;
    // The view sorts its items, so they're all fetched: the rest come
    // after `c3`. Then the archived ones.
    let rest = r#"{"data":{"repositoryOwner":{"projectV2":{"items":{"totalCount":4,"pageInfo":{"hasNextPage":false,"endCursor":"c4"},"nodes":[
        {"id":"PVTI_4","fullDatabaseId":"1004","content":{"__typename":"DraftIssue","id":"DI_4","title":"Later","body":"","assignees":{"totalCount":0,"nodes":[]}},"fieldValues":{"totalCount":0,"nodes":[]}}]}}}}}"#;
    let archived = r#"{"data":{"repositoryOwner":{"projectV2":{"items":{"totalCount":1,"pageInfo":{"hasNextPage":false,"endCursor":"a1"},"nodes":[
        {"id":"PVTI_9","fullDatabaseId":"1009","isArchived":true,"content":{"__typename":"DraftIssue","id":"DI_9","title":"Old","body":"","assignees":{"totalCount":0,"nodes":[]}},"fieldValues":{"totalCount":0,"nodes":[]}}]}}}}}"#;
    let (gh, seen) = github(vec![
        Reply::new(200, project),
        Reply::new(200, items),
        Reply::new(200, rest),
        Reply::new(200, archived),
    ])
    .await;
    let mut p = gh.project(&roadmap(), Some(2), None).await.unwrap();
    assert_eq!((p.items.items.len(), p.items.next.as_deref()), (4, None));
    assert!(p.archived.items.iter().all(|it| it.archived));
    assert_eq!(p.item(1009).map(|it| it.id.as_str()), Some("PVTI_9"));
    p.items.items.truncate(3);
    assert_eq!((p.view, p.can_update), (2, true));
    let plan = p.shown().unwrap();
    assert_eq!(
        (plan.layout, plan.filter.as_str()),
        (Layout::Table, "-status:Done")
    );
    assert_eq!(plan.sort, [(NodeId::new("F_sprint"), true)]);
    let board = p.views.first().unwrap();
    assert_eq!(board.column, Some(NodeId::new("F_status")));
    let Some(FieldKind::Iteration(sprints)) = p.fields.get(2).map(|f| &f.kind) else {
        panic!("{:?}", p.fields);
    };
    assert_eq!(
        sprints.iter().map(|s| s.completed).collect::<Vec<_>>(),
        [false, true]
    );
    let [issue, draft, hidden] = p.items.items.as_slice() else {
        panic!("{:?}", p.items);
    };
    assert_eq!(issue.number, 1001);
    assert!(matches!(
        &issue.content,
        ItemContent::Issue(l, ghtui::api::browse::IssueState::NotPlanned) if l.number == 14
    ));
    let todo = FieldValue::Option {
        id: "O_todo".into(),
        name: "Todo".into(),
    };
    assert_eq!(issue.value(&NodeId::new("F_status")), Some(&todo));
    assert!(matches!(&draft.content, ItemContent::Draft(d) if d.body == "Its body"));
    assert_eq!(
        draft.value(&NodeId::new("F_est")),
        Some(&FieldValue::Number("2".into()))
    );
    assert_eq!(hidden.content, ItemContent::Redacted);
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].json()["variables"]["login"], "o");
    assert_eq!(seen[0].json()["variables"]["number"], 1);
    // The items are what the view's filter finds.
    assert!(seen[1].query().contains("items(first: 50"));
    assert_eq!(seen[1].json()["variables"]["query"], "-status:Done");
    assert_eq!(seen[2].json()["variables"]["after"], "c3");
    assert_eq!(seen[3].json()["variables"]["archived"][0], "ARCHIVED");
}

/// Adding an issue names it by its node ID, which is asked for first; a
/// draft, archiving and restoring, and a draft's edit are a mutation
/// each.
#[tokio::test]
async fn project_items_are_added_archived_and_edited() {
    use ghtui::api::change::Change;
    use ghtui::api::project::ProjectChange;
    let (gh, seen) = github(vec![
        Reply::new(
            200,
            r#"{"data":{"repository":{"issueOrPullRequest":{"id":"I_20"}}}}"#,
        ),
        Reply::new(
            200,
            r#"{"data":{"addProjectV2ItemById":{"item":{"id":"PVTI_20"}}}}"#,
        ),
        Reply::new(
            200,
            r#"{"data":{"unarchiveProjectV2Item":{"item":{"id":"PVTI_9"}}}}"#,
        ),
        Reply::new(
            200,
            r#"{"data":{"updateProjectV2DraftIssue":{"draftIssue":{"id":"DI_2"}}}}"#,
        ),
    ])
    .await;
    let project = NodeId::new("PVT_1");
    for change in [
        ProjectChange::Add {
            project: project.clone(),
            repo: RepoId::new("o", "r"),
            number: 20,
        },
        ProjectChange::Archive {
            project: project.clone(),
            item: NodeId::new("PVTI_9"),
            archived: false,
        },
        ProjectChange::EditDraft {
            draft: NodeId::new("DI_2"),
            title: "Draft".into(),
            body: "Its body".into(),
        },
    ] {
        gh.change(&Change::Project(change)).await.unwrap();
    }
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].json()["variables"]["number"], 20);
    assert!(seen[1].query().contains("addProjectV2ItemById"));
    assert_eq!(seen[1].json()["variables"]["content"], "I_20");
    assert!(seen[2].query().contains("unarchiveProjectV2Item"));
    assert_eq!(seen[3].json()["variables"]["body"], "Its body");
}

/// Setting a field sends its value as GitHub takes it; clearing it is a
/// mutation of its own; a token without the project scope says how to
/// get it.
#[tokio::test]
async fn setting_a_project_field_sends_its_value() {
    use ghtui::api::change::Change;
    use ghtui::api::project::FieldValue;
    let ok = r#"{"data":{"updateProjectV2ItemFieldValue":{"projectV2Item":{"id":"PVTI_1"}}}}"#;
    let cleared = r#"{"data":{"clearProjectV2ItemFieldValue":{"projectV2Item":{"id":"PVTI_1"}}}}"#;
    let scope = r#"{"data":{"updateProjectV2ItemFieldValue":null},"errors":[{"type":"INSUFFICIENT_SCOPES","message":"Your token has not been granted the required scopes to execute this query. The 'updateProjectV2ItemFieldValue' field requires one of the following scopes: ['project'], but your token has only been granted the: ['repo'] scopes."}]}"#;
    let (gh, seen) = github(vec![
        Reply::new(200, ok),
        Reply::new(200, ok),
        Reply::new(200, cleared),
        Reply::new(200, scope),
    ])
    .await;
    let set = |value| Change::SetProjectField {
        project: NodeId::new("PVT_1"),
        item: NodeId::new("PVTI_1"),
        field: NodeId::new("F_status"),
        value,
        said: "Status".into(),
    };
    let done = FieldValue::Option {
        id: "O_done".into(),
        name: "Done".into(),
    };
    gh.change(&set(Some(done.clone()))).await.unwrap();
    gh.change(&set(Some(FieldValue::Number("2.5".into()))))
        .await
        .unwrap();
    gh.change(&set(None)).await.unwrap();
    match gh.change(&set(Some(done))).await {
        Err(ApiError::GraphQl(errors)) => {
            assert!(
                errors[0].contains("gh auth refresh -s project"),
                "{errors:?}"
            );
        }
        other => panic!("{other:?}"),
    }
    let seen = seen.lock().unwrap();
    let vars = |i: usize| seen[i].json()["variables"].clone();
    assert!(seen[0].query().contains("updateProjectV2ItemFieldValue"));
    assert_eq!(vars(0)["project"], "PVT_1");
    assert_eq!(vars(0)["item"], "PVTI_1");
    assert_eq!(vars(0)["field"], "F_status");
    assert_eq!(
        vars(0)["value"],
        serde_json::json!({ "singleSelectOptionId": "O_done" })
    );
    assert_eq!(vars(1)["value"], serde_json::json!({ "number": 2.5 }));
    assert!(seen[2].query().contains("clearProjectV2ItemFieldValue"));
    assert!(vars(2).get("value").is_none());
}

/// An owner's projects are asked of the owner, whichever kind it is; a
/// token without read:project says how to get it.
#[tokio::test]
async fn an_owners_projects_page_and_say_what_scope_they_need() {
    use ghtui::api::project::ProjectsOf;
    let list = r#"{"data":{"repositoryOwner":{"projectsV2":{"totalCount":31,"pageInfo":{"hasNextPage":true,"endCursor":"p1"},"nodes":[
        {"number":1,"title":"Roadmap","shortDescription":"Next","url":"https://github.com/orgs/o/projects/1","closed":false,"public":true,"updatedAt":"2026-10-01T00:00:00Z","items":{"totalCount":12}}]}}}}"#;
    let scope = r#"{"data":{"repositoryOwner":{"projectsV2":null}},"errors":[{"type":"INSUFFICIENT_SCOPES","path":["repositoryOwner","projectsV2"],"message":"Your token has not been granted the required scopes to execute this query. The 'projectsV2' field requires one of the following scopes: ['read:project'], but your token has only been granted the: ['repo'] scopes."}]}"#;
    let (gh, seen) = github(vec![Reply::new(200, list), Reply::new(200, scope)]).await;
    let owner = ProjectsOf::Owner(roadmap().owner);
    let found = gh.projects(&owner, Some("p0".into())).await.unwrap();
    assert_eq!((found.total, found.next.as_deref()), (31, Some("p1")));
    assert_eq!(found.items[0].items, 12);
    match gh.projects(&owner, None).await {
        Err(ApiError::GraphQl(errors)) => {
            assert!(
                errors[0].contains("gh auth refresh -s read:project"),
                "{errors:?}"
            );
        }
        other => panic!("{other:?}"),
    }
    let seen = seen.lock().unwrap();
    assert!(seen[0].query().contains("projectsV2(first: 30"));
    assert_eq!(seen[0].json()["variables"]["login"], "o");
    assert_eq!(seen[0].json()["variables"]["after"], "p0");
}

// ---- the Security tab's alerts ------------------------------------------------

/// A code scanning alert as GitHub sends it.
const CODE_ALERT: &str = r#"{"number":2,"created_at":"2026-10-12T13:58:34Z","updated_at":"2026-10-12T13:58:34Z","state":"open","fixed_at":null,"dismissed_by":null,"dismissed_at":null,"dismissed_reason":null,"dismissed_comment":null,"rule":{"id":"ghtui/hardcoded-word","severity":"warning","description":"A hard-coded word","name":"HardcodedWord","tags":["security"],"full_description":"The file names a word.","help":"Read it from **configuration**.","security_severity_level":"high"},"tool":{"name":"ghtui-check","guid":null,"version":"1.0.0"},"most_recent_instance":{"ref":"refs/heads/main","commit_sha":"75c61df02bc009d61a80ad467606772ba0a92c22","message":{"text":"This line hard-codes a word."},"location":{"path":"file.txt","start_line":1,"end_line":1}}}"#;

/// An alert list sends its filter as the API's parameters, pages by the
/// `Link` header's own cursor, and reads each alert's file and line.
#[tokio::test]
async fn alert_lists_filter_and_page() {
    use ghtui::api::security::{AlertKind, Detail};
    let page: Vec<&str> = std::iter::repeat_n(CODE_ALERT, 30).collect();
    let next = r#"<https://api.github.com/repositories/1/code-scanning/alerts?per_page=30&page=2>; rel="next""#;
    let (gh, seen) = github(vec![
        Reply::new(200, format!("[{}]", page.join(","))).header("link", next),
        Reply::new(200, format!("[{CODE_ALERT}]")),
    ])
    .await;
    let repo = RepoId::new("o", "r");
    let kind = AlertKind::CodeScanning;
    let first = gh
        .alerts(&repo, kind, "is:open branch:main", None)
        .await
        .unwrap();
    assert_eq!(first.results.next.as_deref(), Some("page=2"));
    let Some(Detail::Code(code)) = first.results.items.first().map(|a| &a.detail) else {
        panic!("{first:?}");
    };
    assert_eq!(
        (code.path.as_deref(), code.lines),
        (Some("file.txt"), Some((1, 1)))
    );
    let more = gh
        .alerts(&repo, kind, "is:open branch:main", first.results.next)
        .await
        .unwrap();
    assert_eq!(more.results.next, None);
    let lines: Vec<String> = (seen.lock().unwrap().iter())
        .map(|s| s.request_line.clone())
        .collect();
    assert!(
        lines[0].starts_with(
            "GET /repos/o/r/code-scanning/alerts?per_page=30&state=open&ref=refs%2Fheads%2Fmain "
        ),
        "{lines:?}"
    );
    assert!(lines[1].contains("&page=2 "), "{lines:?}");
}

/// Alerts that are off are an empty list that says so; a token without
/// the scope is told which and how to add it.
#[tokio::test]
async fn alerts_say_when_they_are_off_or_out_of_reach() {
    use ghtui::api::security::AlertKind;
    let (gh, _) = github(vec![
        Reply::new(
            403,
            r#"{"message":"Dependabot alerts are disabled for this repository."}"#,
        ),
        Reply::new(404, r#"{"message":"Not Found"}"#).header("x-oauth-scopes", "gist, read:org"),
    ])
    .await;
    let repo = RepoId::new("o", "r");
    let off = gh
        .alerts(&repo, AlertKind::Dependabot, "is:open", None)
        .await
        .unwrap();
    assert_eq!(
        off.off.as_deref(),
        Some("Dependabot alerts are disabled for this repository")
    );
    match gh
        .alerts(&repo, AlertKind::SecretScanning, "is:open", None)
        .await
    {
        Err(ApiError::Denied(why)) => {
            assert!(
                why.contains("`gh auth refresh -s security_events`"),
                "{why}"
            );
        }
        other => panic!("{other:?}"),
    }
}

/// Dismissing sends each kind's own words; reopening, the open state.
#[tokio::test]
async fn alerts_are_dismissed_and_reopened() {
    use ghtui::api::change::Change;
    use ghtui::api::security::{AlertChange, AlertKind, AlertRef};
    let (gh, seen) = github(vec![
        Reply::new(200, "{}"),
        Reply::new(200, "{}"),
        Reply::new(200, "{}"),
    ])
    .await;
    let alert = |kind| AlertRef {
        repo: RepoId::new("o", "r"),
        kind,
        number: 3,
    };
    for (kind, dismiss) in [
        (
            AlertKind::Dependabot,
            Some(("tolerable_risk".to_owned(), "fine".to_owned())),
        ),
        (
            AlertKind::SecretScanning,
            Some(("revoked".into(), String::new())),
        ),
        (AlertKind::CodeScanning, None),
    ] {
        let change = Change::Alert(AlertChange {
            alert: alert(kind),
            dismiss,
        });
        gh.change(&change).await.unwrap();
    }
    let seen = seen.lock().unwrap();
    let sent: Vec<(String, serde_json::Value)> = (seen.iter())
        .map(|s| (s.request_line.clone(), s.json()))
        .collect();
    assert!(
        sent[0]
            .0
            .starts_with("PATCH /repos/o/r/dependabot/alerts/3 ")
    );
    assert_eq!(
        sent[0].1,
        serde_json::json!({"state": "dismissed", "dismissed_reason": "tolerable_risk", "dismissed_comment": "fine"})
    );
    assert!(
        sent[1]
            .0
            .starts_with("PATCH /repos/o/r/secret-scanning/alerts/3 ")
    );
    assert_eq!(
        sent[1].1,
        serde_json::json!({"state": "resolved", "resolution": "revoked"})
    );
    assert!(
        sent[2]
            .0
            .starts_with("PATCH /repos/o/r/code-scanning/alerts/3 ")
    );
    assert_eq!(sent[2].1, serde_json::json!({"state": "open"}));
}

// ---- releases, gists and subscriptions ------------------------------------------------

/// A release is made with its tag, target, title and notes, and whether
/// it's a draft, a pre-release or the latest; a pre-release is never the
/// latest. GitHub's notes are asked for the tag; a 422 says what's wrong.
#[tokio::test]
async fn a_release_is_made_as_written() {
    use ghtui::api::release::NewRelease;
    let made = r#"{"tag_name":"v1.2","draft":false,"html_url":"https://github.com/o/r/releases/tag/v1.2"}"#;
    let notes = r###"{"name":"v1.2","body":"## What's Changed\n* Fix by @a"}"###;
    let taken = r#"{"message":"Validation Failed","errors":[{"resource":"Release","code":"already_exists","field":"tag_name"}]}"#;
    let (gh, seen) = github(vec![
        Reply::new(201, made),
        Reply::new(200, notes),
        Reply::new(422, taken),
    ])
    .await;
    let repo = RepoId::new("o", "r");
    let new = NewRelease {
        repo: repo.clone(),
        tag: "v1.2".into(),
        target: Some("main".into()),
        name: "Version 1.2".into(),
        notes: "Fixes".into(),
        draft: false,
        prerelease: true,
        latest: true,
    };
    let made = gh.create_release(&new).await.unwrap();
    assert!(!made.draft);
    let notes = gh.release_notes(&repo, "v1.2", None).await.unwrap();
    assert!(notes.body.starts_with("## What's Changed"));
    match gh.create_release(&new).await {
        Err(ApiError::Http {
            status: 422,
            message,
        }) => {
            assert_eq!(message, "tag_name already exists");
        }
        other => panic!("{other:?}"),
    }
    let seen = seen.lock().unwrap();
    assert!(
        seen[0]
            .request_line
            .starts_with("POST /repos/o/r/releases ")
    );
    let body = seen[0].json();
    assert_eq!(body["tag_name"], "v1.2");
    assert_eq!(body["target_commitish"], "main");
    assert_eq!(body["name"], "Version 1.2");
    assert_eq!(body["body"], "Fixes");
    assert_eq!(body["draft"], false);
    assert_eq!(body["prerelease"], true);
    assert_eq!(body["make_latest"], "false");
    assert!(
        seen[1]
            .request_line
            .starts_with("POST /repos/o/r/releases/generate-notes ")
    );
    assert_eq!(seen[1].json(), serde_json::json!({ "tag_name": "v1.2" }));
}

/// A gist is made with its files and visibility; an edit sends only what
/// changed (a removed file as null); deleting deletes it.
#[tokio::test]
async fn gists_are_made_changed_and_deleted() {
    use ghtui::api::gist::{FileChange, GistChange, NewGist};
    let made = r#"{"id":"aa5a315d61ae9438b18d","owner":{"login":"octocat"}}"#;
    let (gh, seen) = github(vec![
        Reply::new(201, made),
        Reply::new(200, made),
        Reply::new(204, ""),
    ])
    .await;
    let new = NewGist {
        description: "Notes".into(),
        public: false,
        files: vec![("a.md".into(), "# A".into())],
    };
    let made = gh.create_gist(&new).await.unwrap();
    assert_eq!(made.id, "aa5a315d61ae9438b18d");
    let change = GistChange {
        id: made.id.clone(),
        description: "More notes".into(),
        changed: vec![
            FileChange {
                was: "a.md".into(),
                now: Some(("b.md".into(), "# B".into())),
            },
            FileChange {
                was: "old.txt".into(),
                now: None,
            },
        ],
        added: vec![("c.rs".into(), "fn c() {}".into())],
    };
    gh.update_gist(&change).await.unwrap();
    gh.delete_gist(&made.id).await.unwrap();
    let seen = seen.lock().unwrap();
    assert!(seen[0].request_line.starts_with("POST /gists "));
    assert_eq!(
        seen[0].json(),
        serde_json::json!({
            "description": "Notes",
            "public": false,
            "files": { "a.md": { "content": "# A" } },
        })
    );
    assert!(
        seen[1]
            .request_line
            .starts_with("PATCH /gists/aa5a315d61ae9438b18d ")
    );
    assert_eq!(
        seen[1].json(),
        serde_json::json!({
            "description": "More notes",
            "files": {
                "a.md": { "filename": "b.md", "content": "# B" },
                "old.txt": null,
                "c.rs": { "content": "fn c() {}" },
            },
        })
    );
    assert!(
        seen[2]
            .request_line
            .starts_with("DELETE /gists/aa5a315d61ae9438b18d ")
    );
}

/// A thread's subscription is read from the issue or pull request, a
/// repository's from it, and set with GitHub's one mutation.
#[tokio::test]
async fn subscriptions_are_read_and_set() {
    use ghtui::api::watch::{SubscriptionState, Watched};
    let thread = r#"{"data":{"repository":{"issueOrPullRequest":{"__typename":"PullRequest","id":"PR_1","viewerSubscription":"UNSUBSCRIBED","viewerCanSubscribe":true}}}}"#;
    let repo = r#"{"data":{"repository":{"id":"R_1","viewerSubscription":"SUBSCRIBED","viewerCanSubscribe":true}}}"#;
    let set = r#"{"data":{"updateSubscription":{"clientMutationId":null}}}"#;
    let (gh, seen) = github(vec![
        Reply::new(200, thread),
        Reply::new(200, repo),
        Reply::new(200, set),
        Reply::new(200, "{}"),
        Reply::new(204, ""),
    ])
    .await;
    let o_r = RepoId::new("o", "r");
    let pr = Watched::Thread(o_r.clone(), 7);
    let s = gh.subscription(&pr).await.unwrap();
    assert_eq!(s.id, NodeId::new("PR_1"));
    assert_eq!(s.state, Some(SubscriptionState::Unsubscribed));
    let whole = Watched::Repo(o_r);
    let s = gh.subscription(&whole).await.unwrap();
    assert_eq!(s.state, Some(SubscriptionState::Subscribed));
    gh.subscribe(&pr, &NodeId::new("PR_1"), SubscriptionState::Ignored)
        .await
        .unwrap();
    // A repository's, through REST, which the repo scope allows.
    let id = NodeId::new("R_1");
    gh.subscribe(&whole, &id, SubscriptionState::Ignored)
        .await
        .unwrap();
    gh.subscribe(&whole, &id, SubscriptionState::Unsubscribed)
        .await
        .unwrap();
    let seen = seen.lock().unwrap();
    assert!(
        seen[3]
            .request_line
            .starts_with("PUT /repos/o/r/subscription ")
    );
    assert_eq!(
        seen[3].json(),
        serde_json::json!({ "subscribed": false, "ignored": true })
    );
    assert!(
        seen[4]
            .request_line
            .starts_with("DELETE /repos/o/r/subscription ")
    );
    assert!(
        seen[0]
            .query()
            .contains("issueOrPullRequest(number: $number)")
    );
    assert_eq!(seen[0].json()["variables"]["number"], 7);
    assert!(!seen[1].query().contains("$number"));
    assert!(seen[2].query().contains("updateSubscription"));
    assert_eq!(seen[2].json()["variables"]["id"], "PR_1");
    assert_eq!(seen[2].json()["variables"]["state"], "IGNORED");
}

/// Running a workflow by hand posts its branch and inputs, and asks for
/// the run it starts; a server that doesn't say (204) started one anyway.
#[tokio::test]
async fn a_workflow_dispatch_posts_its_inputs_and_reads_the_run() {
    let (gh, seen) = github(vec![
        Reply::new(
            200,
            r#"{"workflow_run_id":55,"run_url":"u","html_url":"h"}"#,
        ),
        Reply::new(204, ""),
    ])
    .await;
    let repo = RepoId::new("o", "r");
    let inputs = [("outcome".to_owned(), "pass".to_owned())];
    let run = gh.dispatch(&repo, "ci.yml", "main", &inputs).await.unwrap();
    assert_eq!(run, Some(55));
    assert_eq!(
        gh.dispatch(&repo, "ci.yml", "main", &[]).await.unwrap(),
        None
    );
    let seen = seen.lock().unwrap();
    assert!(
        seen[0]
            .request_line
            .starts_with("POST /repos/o/r/actions/workflows/ci.yml/dispatches "),
        "{}",
        seen[0].request_line
    );
    let body = seen[0].json();
    assert_eq!(body["ref"], "main");
    assert_eq!(body["inputs"]["outcome"], "pass");
    assert_eq!(body["return_run_details"], true);
}

/// An artifact is saved where asked, and a file already there is kept
/// unless replacing it was asked for.
#[tokio::test]
async fn an_artifact_is_saved_and_never_over_a_file_unasked() {
    let (gh, seen) = github(vec![Reply::new(200, "zip bytes"), Reply::new(200, "newer")]).await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("report.zip");
    let repo = RepoId::new("o", "r");
    let size = gh.download_artifact(&repo, 31, &dest, false).await.unwrap();
    assert_eq!(size, 9);
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "zip bytes");
    gh.download_artifact(&repo, 31, &dest, false)
        .await
        .unwrap_err();
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "zip bytes");
    assert!(
        seen.lock().unwrap()[0]
            .request_line
            .starts_with("GET /repos/o/r/actions/artifacts/31/zip ")
    );
}

/// A finished job's annotations come with it, worst first; a file's link
/// opens its lines, one about the run has none.
#[tokio::test]
async fn a_jobs_annotations_come_with_it() {
    let job = r#"{"id":2,"run_id":9,"name":"test","status":"completed","conclusion":"failure","run_attempt":1,"steps":[]}"#;
    let run = r#"{"id":9,"run_number":1,"event":"push","head_sha":"abc","status":"completed","conclusion":"failure","run_attempt":1}"#;
    let annotations = r#"[
        {"path":".github","blob_href":"https://github.com/o/r/blob/abc/.github","start_line":1,"end_line":1,"annotation_level":"warning","title":"","message":"Node 20"},
        {"path":"src/a.rs","blob_href":"https://github.com/o/r/blob/abc/src/a.rs","start_line":3,"end_line":4,"annotation_level":"failure","title":"t","message":"boom"}
    ]"#;
    let (gh, _) = github(vec![
        Reply::new(200, job),
        Reply::new(200, run),
        Reply::new(200, r#"{"output":{"annotations_count":2}}"#),
        Reply::new(200, annotations),
    ])
    .await;
    let job = gh.job(&RepoId::new("o", "r"), 2).await.unwrap();
    let a = &job.annotations;
    assert_eq!(a.len(), 2);
    assert_eq!(a[0].place(), "src/a.rs:3");
    assert_eq!(
        a[0].url.as_deref(),
        Some("https://github.com/o/r/blob/abc/src/a.rs#L3-L4")
    );
    assert_eq!(
        (a[1].place().as_str(), a[1].url.as_deref()),
        ("the run", None)
    );
}

/// A review's summary is edited, and your pending review deleted, with
/// their own mutations; a commit is commented on, and its comments read,
/// through REST.
#[tokio::test]
async fn reviews_and_commit_comments_send_their_requests() {
    use ghtui::api::change::Change;
    use ghtui::api::edit::{Edit, Written};
    let ok = |field: &str| {
        Reply::new(
            200,
            format!(r#"{{"data":{{"{field}":{{"clientMutationId":null}}}}}}"#),
        )
    };
    let comments = r#"[{"id":5,"node_id":"CC_5","user":{"login":"alice"},"body":"Nice",
        "created_at":"2026-10-01T00:00:00Z","path":"src/a.rs","line":3,
        "html_url":"https://github.com/o/r/commit/abc#commitcomment-5"},
        {"id":6,"node_id":"CC_6","user":null,"body":"Whole","created_at":"2026-10-02T00:00:00Z",
        "path":null,"line":null,"html_url":"https://github.com/o/r/commit/abc#commitcomment-6"}]"#;
    let (gh, seen) = github(vec![
        ok("updatePullRequestReview"),
        ok("deletePullRequestReview"),
        Reply::new(201, "{}"),
        Reply::new(200, comments),
    ])
    .await;
    let id = NodeId::new("PRR_1");
    let body = "Better words".to_owned();
    let of = Written::Review;
    gh.edit(&Edit::Body {
        of,
        id: id.clone(),
        body,
    })
    .await
    .unwrap();
    gh.edit(&Edit::Delete { of, id }).await.unwrap();
    let repo = RepoId::new("o", "r");
    gh.change(&Change::CommitComment {
        repo: repo.clone(),
        oid: "abc".into(),
        body: "Why this?".into(),
    })
    .await
    .unwrap();
    let read = gh.commit_comments(&repo, "abc").await.unwrap();
    assert_eq!(read.len(), 2);
    assert_eq!(read[0].path.as_deref(), Some("src/a.rs"));
    assert_eq!((read[1].author.as_str(), read[1].line), ("ghost", None));
    assert!(read[0].as_thread().is_some() && read[1].as_thread().is_none());
    let seen = seen.lock().unwrap();
    assert!(seen[0].query().contains("updatePullRequestReview("));
    assert_eq!(seen[0].json()["variables"]["body"], "Better words");
    assert!(seen[1].query().contains("deletePullRequestReview("));
    assert!(
        seen[2]
            .request_line
            .starts_with("POST /repos/o/r/commits/abc/comments ")
    );
    assert_eq!(seen[2].json()["body"], "Why this?");
    assert!(
        seen[3]
            .request_line
            .starts_with("GET /repos/o/r/commits/abc/comments")
    );
}
