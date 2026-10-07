//! A crawl of real GitHub data through ghtui's own client and page
//! builders, to find links that open the browser. Run by hand:
//!
//! ```text
//! GHTUI_CRAWL_OUT=report.txt cargo test --release -p ghtui -- --ignored --nocapture link_crawl
//! ```
//!
//! It starts from a few busy repositories, users and an organization,
//! follows every link a few levels deep up to a page budget, and stops early
//! when the rate limit runs low. The report (link shapes that leave, how
//! often, and where) goes to `$GHTUI_CRAWL_OUT` (a relative path is
//! under the temporary directory), never the repository.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stderr,
    reason = "a test run by hand: failing loudly is the point"
)]

use std::collections::{BTreeMap, HashSet, VecDeque};

use ghtui_api::GitHub;
use ghtui_store::Store;
use ghtui_theme::{ColorDepth, DEFAULT_SEED, Mode, Theme};
use ghtui_ui::Icons;

use crate::browse::{DataKey, Need, needs};
use crate::diff_job::GitContext;
use crate::keymap::Keymap;
use crate::route::{Route, Target};
use crate::state::{Remote, State};

const PAGES: usize = 300;
const DEPTH: usize = 3;
/// Stop while this much of the hourly GraphQL budget is left.
const RESERVE: u64 = 1000;
/// And this much of the hourly REST budget.
const REST_RESERVE: u64 = 500;

const SEEDS: &[&str] = &[
    "https://github.com/rust-lang/rust",
    "https://github.com/rust-lang/rust/pull/163831",
    "https://github.com/rust-lang/rust/pull/163831/checks",
    "https://github.com/rust-lang/rust/issues/163852",
    "https://github.com/rust-lang/rust/commit/8d1a76430406c877b35d0b627e7f796dcf0dfeca",
    "https://github.com/rust-lang/rust/releases/tag/1.99.0",
    "https://github.com/rust-lang/rust/actions",
    "https://github.com/tokio-rs/tokio",
    "https://github.com/tokio-rs/tokio/pulls",
    "https://github.com/ratatui/ratatui",
    "https://github.com/ratatui/ratatui/issues",
    "https://github.com/cli/cli",
    "https://github.com/torvalds",
    "https://github.com/rust-lang",
];

/// A link's shape for the report: host and path, with owner and
/// repository, numbers and commit IDs as `*`.
fn shape(url: &str) -> String {
    let Ok(parsed) = url::Url::parse(url) else {
        return url.to_owned();
    };
    let host = parsed.host_str().unwrap_or_default();
    // Other hosts (raw files, gists) are one shape each.
    if host != "github.com" {
        return host.to_owned();
    }
    let path: Vec<&str> = parsed
        .path_segments()
        .into_iter()
        .flatten()
        .take(5)
        .enumerate()
        .map(|(i, s)| {
            let repo_part = host == "github.com" && i < 2;
            if repo_part || s.bytes().any(|b| b.is_ascii_digit()) {
                "*"
            } else {
                s
            }
        })
        .collect();
    format!("{host}/{}", path.join("/"))
}

/// Whether `url` is on one of GitHub's own hosts (not a project's
/// github.io site or another service).
fn is_github(url: &str) -> bool {
    let host = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default();
    host == "github.com"
        || host.ends_with(".github.com")
        || host.ends_with(".githubusercontent.com")
}

/// Fetches what `route` needs into `state` (a wiki through git, as the
/// app does); false if the page couldn't load.
async fn load(gh: &GitHub, git: &GitContext, state: &mut State, route: &Route) -> bool {
    for need in needs(route) {
        match need {
            Need::Inbox => {}
            Need::Pr(pr) => match gh.pull_request(&pr).await {
                Ok(detail) => {
                    let remote = state.prs.entry(pr).or_default();
                    remote.finish(Ok(detail));
                }
                Err(err) => {
                    eprintln!("  {route:?}: {err}");
                    return false;
                }
            },
            Need::Data(key) => match fetch(gh, git, &key).await {
                Ok(data) => {
                    let remote: &mut Remote<_> = state.data.entry(key).or_default();
                    remote.finish(Ok(data));
                }
                Err(err) => eprintln!("  {key:?}: {err}"),
            },
        }
    }
    state.data_gen += 1;
    true
}

async fn fetch(
    gh: &GitHub,
    git: &GitContext,
    key: &DataKey,
) -> Result<crate::browse::Data, ghtui_api::ApiError> {
    match key {
        DataKey::Wiki(repo, page) => crate::wiki::page(git, repo, page.as_deref())
            .await
            .map(|w| crate::browse::Data::Wiki(Box::new(w))),
        key => crate::runtime::fetch(gh, key).await,
    }
}

#[tokio::test]
#[ignore = "crawls github.com; run by hand"]
async fn link_crawl() {
    let (token, _) = ghtui_api::auth::resolve_token().expect("a GitHub token");
    let cache = tempfile::tempdir().unwrap();
    let gh = GitHub::new(token, Store::open(&cache.path().join("cache.redb")));
    let git = GitContext {
        cache_root: cache.path().to_owned(),
        credentials: ghtui_git::credentials::Credentials::Ambient,
        cwd: cache.path().to_owned(),
    };
    let theme = Theme::new(DEFAULT_SEED, Mode::Dark, ColorDepth::TrueColor);
    let mut state = State::new(theme, Icons::default(), Keymap::default(), (140, 50));

    let mut queue: VecDeque<(Route, usize)> = SEEDS
        .iter()
        .filter_map(|u| match Target::from_url(u) {
            Target::Page(route) => Some((route, 0)),
            _ => None,
        })
        .collect();
    let mut seen: HashSet<Route> = queue.iter().map(|(r, _)| r.clone()).collect();
    // Shape → (times seen, example, page it was on).
    let mut leaving: BTreeMap<String, (usize, String, String)> = BTreeMap::new();
    let (mut pages, mut links, mut inside) = (0, 0, 0);
    while let Some((route, depth)) = queue.pop_front() {
        if pages >= PAGES {
            break;
        }
        let limits = gh.rate_limits();
        if let Some(b) = limits.graphql
            && b.remaining < RESERVE
        {
            eprintln!("stopping: {} GraphQL points left", b.remaining);
            break;
        }
        if let Some(b) = limits.rest
            && b.remaining < REST_RESERVE
        {
            eprintln!("stopping: {} REST requests left", b.remaining);
            break;
        }
        if !load(&gh, &git, &mut state, &route).await {
            continue;
        }
        pages += 1;
        eprintln!("[{pages}] {}", route.url());
        let page = state.build_page(&route, 140, ghtui_store::now());
        for url in page.links.iter().filter_map(|l| l.url()) {
            links += 1;
            match Target::from_url(url) {
                Target::External(url) if is_github(&url) => {
                    let entry = leaving
                        .entry(shape(&url))
                        .or_insert_with(|| (0, url.clone(), route.url()));
                    entry.0 += 1;
                }
                Target::External(_) => {}
                Target::Files(_) => inside += 1,
                Target::Page(next) => {
                    inside += 1;
                    if depth < DEPTH && seen.insert(next.clone()) {
                        queue.push_back((next, depth + 1));
                    }
                }
            }
        }
    }

    let mut report = format!(
        "{pages} pages, {links} links: {inside} open in ghtui, {} shapes leave\n\n",
        leaving.len()
    );
    let mut rows: Vec<_> = leaving.into_iter().collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.1.0));
    for (shape, (n, example, from)) in rows {
        report.push_str(&format!(
            "{n:>5}  {shape}\n       {example}\n       on {from}\n"
        ));
    }
    // A relative path is under the temporary directory: tests run in the
    // crate, and the report never goes into the repository.
    let out = std::env::var("GHTUI_CRAWL_OUT").map_or_else(
        |_| std::env::temp_dir().join("ghtui-link-crawl.txt"),
        |path| std::env::temp_dir().join(path),
    );
    let out = out.display().to_string();
    std::fs::write(&out, &report).unwrap();
    eprintln!("{report}\nwritten to {out}");
}
