//! What the corpus covers: the requests ghtui makes for a handful of real
//! cases, run live to record them (`contract_record_corpus`) and offline
//! to replay them (`tests/corpus.rs`), checking each answer adds up.

#![allow(dead_code, reason = "each test binary uses part of this")]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ghtui_api::GitHub;
use ghtui_api::browse::{DiscussionsOf, SearchKind};
use ghtui_api::model::{PrRef, PrState, RepoId};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// The committed corpus.
pub fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus")
}

fn pr(s: &str) -> PrRef {
    PrRef::parse(s).unwrap()
}

fn no_doubts(gh: &GitHub, case: &str) {
    assert_eq!(gh.take_doubts(), Vec::<String>::new(), "{case}");
}

/// Every case's requests, through ghtui's client; each answer is checked
/// against itself (the doubts) and against what's known of the case.
pub async fn run(gh: &GitHub) {
    // A long PR: 229 commits, more than a page of anything.
    let long = pr("rust-lang/rust#160692");
    let detail = gh.pull_request(&long).await.unwrap();
    let activity = gh.pr_activity(&long).await.unwrap();
    assert_eq!(activity.total_commits, 229);
    assert!(activity.commits.len() as u64 <= activity.total_commits);
    let checks = gh.pr_checks(&long).await.unwrap();
    assert_eq!(checks.oid, detail.head_oid);
    gh.review_threads(&long).await.unwrap();
    no_doubts(gh, "a long PR");

    // A PR merged with a merge commit: its diff is from its own base.
    let merged = gh.pull_request(&pr("cli/cli#14592")).await.unwrap();
    assert_eq!(merged.summary.state, PrState::Merged);
    assert_eq!(merged.base_oid, "6fc1c29d5477bfe71da7af290eb481c0df7811f1");
    no_doubts(gh, "a merged PR");

    // A PR with more than 100 comments: the newest are kept, and the
    // count says how many there are.
    let talky = gh.pr_activity(&pr("rust-lang/rust#158734")).await.unwrap();
    assert!(talky.total_comments > 100, "{}", talky.total_comments);
    assert_eq!(talky.comments.len(), 100);
    no_doubts(gh, "a PR with many comments");

    // A comparison of more than 250 commits: the newest 250.
    let rust = RepoId::new("rust-lang", "rust");
    let big = gh.compare(&rust, "1.98.0...1.99.0").await.unwrap();
    assert!(big.total_commits > 250);
    assert_eq!(big.commits.len(), 250);
    no_doubts(gh, "a big comparison");

    // A repository with many branches.
    let cli = RepoId::new("cli", "cli");
    let refs = gh.refs(&cli).await.unwrap();
    assert!(refs.branch_total > 100);
    assert_eq!(refs.branches.len() as u64, refs.branch_total.min(1000));
    let branches = gh.branches(&cli, None).await.unwrap();
    assert!(branches.next.is_some() && branches.items.len() == 30);
    no_doubts(gh, "many branches");

    // An archived repository.
    let atom = gh.repo(&RepoId::new("atom", "atom")).await.unwrap();
    assert!(atom.summary.archived);
    no_doubts(gh, "an archived repository");

    // A deleted ("ghost") author, and a bot.
    let ghost = gh.issue(&rust, 97).await.unwrap().unwrap();
    assert_eq!(ghost.author, "ghost");
    let bot = gh.pr_activity(&pr("cli/cli#14543")).await.unwrap();
    assert!(bot.comments.iter().all(|c| !c.author.is_empty()));
    let bot_pr = gh.pull_request(&pr("cli/cli#14543")).await.unwrap();
    assert_eq!(bot_pr.summary.author, "dependabot");
    no_doubts(gh, "ghost and bot authors");

    // A discussion whose newer comments' IDs overflow GraphQL's Int, so
    // come back null.
    let discussion = gh
        .discussion(&DiscussionsOf::Repo(cli.clone()), 14603)
        .await
        .unwrap();
    assert!(discussion.comments.len() as u64 <= discussion.total_comments);
    no_doubts(gh, "a discussion");

    // A commit search, whose dates carry offsets.
    gh.search(
        SearchKind::Commits,
        "repo:torvalds/linux author-date:2020-01-01..2020-01-02",
        None,
    )
    .await
    .unwrap();
    no_doubts(gh, "a commit search");
}

/// Serves the recorded corpus in `dir`: each request gets the response
/// recorded for it, or a 404 saying it wasn't recorded.
pub async fn replay(dir: PathBuf) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let dir = Arc::new(dir);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let dir = dir.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                loop {
                    let Some((method, path, body, len)) = request(&buf) else {
                        let mut chunk = [0u8; 8192];
                        match socket.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                        continue;
                    };
                    buf.drain(..len);
                    let sent: Option<serde_json::Value> =
                        (!body.is_empty()).then(|| serde_json::from_str(&body).unwrap());
                    let key = ghtui_api::corpus::key(&method, &path, sent.as_ref());
                    let (status, link, text) = match ghtui_api::corpus::read(&dir, &key) {
                        Some(r) => (r.status, r.link, r.response),
                        None => (
                            404,
                            None,
                            format!(r#"{{"message":"not recorded: {method} {path}"}}"#),
                        ),
                    };
                    let link = link.map(|l| format!("link: {l}\r\n")).unwrap_or_default();
                    let head = format!(
                        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\n{link}content-length: {}\r\n\r\n",
                        text.len()
                    );
                    if socket.write_all(head.as_bytes()).await.is_err()
                        || socket.write_all(text.as_bytes()).await.is_err()
                    {
                        return;
                    }
                }
            });
        }
    });
    format!("http://{addr}")
}

/// A whole request at the start of `buf`: its method, path, body, and
/// length.
fn request(buf: &[u8]) -> Option<(String, String, String, usize)> {
    let end = buf.windows(4).position(|w| w == b"\r\n\r\n")? + 4;
    let head = std::str::from_utf8(&buf[..end]).ok()?;
    let mut lines = head.lines();
    let mut first = lines.next()?.split(' ');
    let (method, path) = (first.next()?.to_owned(), first.next()?.to_owned());
    let length: usize = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse().ok())
        .unwrap_or(0);
    let body = buf.get(end..end + length)?;
    Some((
        method,
        path,
        String::from_utf8_lossy(body).into_owned(),
        end + length,
    ))
}
