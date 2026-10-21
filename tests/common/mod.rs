//! What the integration tests share: an isolated `git` for fixture
//! repositories, a scripted GitHub on a local port, and the corpus, the
//! requests ghtui makes for a handful of real cases, run live to record
//! them (`contract_record_corpus`) and offline to replay them
//! (`tests/corpus.rs`), checking each answer adds up.

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "each test binary uses part of this, and a panic in it is a test failure"
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use ghtui::api::GitHub;
use ghtui::api::browse::{DiscussionsOf, SearchKind};
use ghtui::api::model::{PrRef, RepoId};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Runs git in `dir` for fixture setup, isolated from the user's
/// configuration; what it printed.
pub fn git_bytes(dir: &Path, args: &[&str]) -> Vec<u8> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

/// Writes `bytes` to `path` under `dir`, making its folders.
pub fn write(dir: &Path, path: &str, bytes: &[u8]) {
    let path = dir.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// [`git_bytes`], as trimmed text.
pub fn git(dir: &Path, args: &[&str]) -> String {
    String::from_utf8_lossy(&git_bytes(dir, args))
        .trim()
        .to_owned()
}

/// What the local GitHub answers a request with.
#[derive(Clone)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: String,
    /// Only for a request whose line contains this (for requests made
    /// concurrently, which arrive in any order).
    pub path: Option<&'static str>,
}

impl Reply {
    pub fn new(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into(),
            path: None,
        }
    }

    pub fn on(mut self, path: &'static str) -> Self {
        self.path = Some(path);
        self
    }

    pub fn header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }
}

/// A request the local GitHub was sent.
#[derive(Debug, Clone)]
pub struct Seen {
    pub request_line: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap()
    }

    /// The GraphQL document sent.
    pub fn query(&self) -> String {
        self.json()["query"].as_str().unwrap().to_owned()
    }
}

/// Answers every request on `listener` (several to a connection) with
/// `answer`'s reply; the server's base URL.
pub fn answer_on(
    listener: TcpListener,
    answer: impl Fn(Seen) -> Reply + Send + Sync + 'static,
) -> String {
    let addr = listener.local_addr().unwrap();
    let answer = Arc::new(answer);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let answer = answer.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                loop {
                    let Some((request, len)) = parse_request(&buf) else {
                        let mut chunk = [0u8; 8192];
                        match socket.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                        continue;
                    };
                    buf.drain(..len);
                    let reply = answer(request);
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
    format!("http://{addr}")
}

/// Serves `replies` in order (one per request, across connections) and
/// records every request.
pub async fn serve(replies: Vec<Reply>) -> (String, Arc<Mutex<Vec<Seen>>>) {
    serve_on(TcpListener::bind("127.0.0.1:0").await.unwrap(), replies)
}

/// [`serve`], on `listener`.
pub fn serve_on(listener: TcpListener, replies: Vec<Reply>) -> (String, Arc<Mutex<Vec<Seen>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let queue = Mutex::new(replies);
    let recorded = seen.clone();
    let base = answer_on(listener, move |request| {
        let line = request.request_line.clone();
        recorded.lock().unwrap().push(request);
        let mut queue = queue.lock().unwrap();
        let next = queue
            .iter()
            .position(|r| r.path.is_none_or(|p| line.contains(p)));
        next.map_or_else(|| Reply::new(500, "script exhausted"), |i| queue.remove(i))
    });
    (base, seen)
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
    assert_eq!(activity.commits.total(), 229);
    let checks = gh.pr_checks(&long).await.unwrap();
    assert_eq!(checks.oid, detail.head_oid);
    gh.review_threads(&long).await.unwrap();
    no_doubts(gh, "a long PR");

    // A PR merged with a merge commit: its diff is from its own base.
    let merged = gh.pull_request(&pr("cli/cli#14592")).await.unwrap();
    assert_eq!(merged.state, ghtui::api::browse::PrState::Merged);
    assert_eq!(merged.base_oid, "6fc1c29d5477bfe71da7af290eb481c0df7811f1");
    no_doubts(gh, "a merged PR");

    // A PR with more than 100 comments: the newest of its timeline are
    // kept, and the counts say how many there are.
    let talky = gh.pr_activity(&pr("rust-lang/rust#158734")).await.unwrap();
    assert!(talky.comment_count > 100, "{}", talky.comment_count);
    assert!(talky.timeline.left_out() > 0);
    // Every one of its reviews, a page at a time.
    let reviews = gh.reviews(&pr("rust-lang/rust#158734")).await.unwrap();
    assert!(reviews.iter().all(|r| r.id.is_some()));
    no_doubts(gh, "a PR with many comments");

    // A comparison of more than 250 commits: the newest 250.
    let rust = RepoId::new("rust-lang", "rust");
    let big = gh.compare(&rust, "1.98.0...1.99.0").await.unwrap();
    assert!(big.commits.total() > 250);
    assert_eq!(big.commits.len(), 250);
    no_doubts(gh, "a big comparison");

    // A repository with many branches.
    let cli = RepoId::new("cli", "cli");
    let refs = gh.refs(&cli).await.unwrap();
    assert!(refs.branches.total() > 100);
    assert_eq!(refs.branches.len() as u64, refs.branches.total().min(1000));
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
    assert!(bot.comments().all(|c| !c.author.is_empty()));
    let bot_pr = gh.pull_request(&pr("cli/cli#14543")).await.unwrap();
    assert_eq!(bot_pr.author, "dependabot");
    no_doubts(gh, "ghost and bot authors");

    // An answered discussion (its answer at the top); newer comments' IDs
    // overflow GraphQL's Int, so come back null.
    gh.discussion(&DiscussionsOf::Repo(cli.clone()), 8794)
        .await
        .unwrap();
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
    answer_on(listener, move |request| {
        let mut line = request.request_line.split(' ');
        let (method, path) = (
            line.next().unwrap_or_default(),
            line.next().unwrap_or_default(),
        );
        let sent = (!request.body.is_empty()).then(|| request.json());
        let key = ghtui::api::corpus::key(method, path, sent.as_ref());
        let reply = match ghtui::api::corpus::read(&dir, &key) {
            Some(recorded) => {
                let reply = Reply::new(recorded.status, recorded.response);
                match recorded.link {
                    Some(link) => reply.header("link", link),
                    None => reply,
                }
            }
            None => {
                let message = format!(r#"{{"message":"not recorded: {method} {path}"}}"#);
                Reply::new(404, message)
            }
        };
        reply.header("content-type", "application/json")
    })
}
