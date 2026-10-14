//! Every kind of page opens against real GitHub: a repository and its
//! tabs, a file, its blame and commits, an issue, a pull request's
//! conversation and files, a workflow and a run, releases, discussions,
//! the wiki, security, settings, projects, Insights, Explore, a topic, a
//! profile, gists, home and the inbox. Each is started on its own, and
//! must show what GitHub says it has, finish loading, and fail to load
//! nothing. Only read: nothing is written.

use std::time::Duration;

use anyhow::{Result, bail};
use serde_json::json;

use crate::live::{Check, Cx, gh};

pub const CHECK: Check = Check::new(
    "pages",
    "each kind of page opens and loads without an error: the repository's tabs, an issue, a pull request, a run, settings, site pages, a profile, the inbox",
    run,
);

/// How long a page may take to load once it shows.
const LOADED: Duration = Duration::from_secs(60);
/// The pull request the test repository keeps open with failing checks.
const PR: u64 = 5;
/// The pull request whose Files changed has [`FILE`].
const FILES_PR: u64 = 11;
const FILE: &str = "bot-bot-1791523358.txt";
/// The issue the test repository keeps open.
const ISSUE: u64 = 8;

/// A page to open: what it is, its URL (none for home), and a text it
/// must show (none when GitHub can't say what's there).
struct Page {
    what: &'static str,
    url: String,
    shows: String,
}

fn page(what: &'static str, url: impl Into<String>, shows: impl Into<String>) -> Page {
    Page {
        what,
        url: url.into(),
        shows: shows.into(),
    }
}

/// The first characters of `text`: a list may cut the rest.
fn start_of(text: &str) -> String {
    text.chars().take(20).collect()
}

fn run(cx: &mut Cx) -> Result<()> {
    let web = format!("https://github.com/{}", gh::REPO);
    let pr_title = gh::get(&format!("repos/{}/pulls/{PR}", gh::REPO))?["title"]
        .as_str()
        .map(start_of)
        .unwrap_or_default();
    let head = gh::get(&format!("repos/{}/commits/main", gh::REPO))?;
    let sha = head["sha"].as_str().unwrap_or_default().to_owned();
    let message = head["commit"]["message"]
        .as_str()
        .and_then(|m| m.lines().next())
        .map(start_of)
        .unwrap_or_default();
    let workflows = format!("{web}/blob/main/.github/workflows");

    let mut pages = vec![
        page("the repository", gh::REPO, "Pull requests"),
        page(
            "a folder",
            format!("{web}/tree/main/.github/workflows"),
            "botreview.yml",
        ),
        page(
            "a file",
            format!("{workflows}/botreview.yml"),
            "name: botreview",
        ),
        page(
            "its blame",
            format!("{web}/blame/main/.github/workflows/botreview.yml"),
            "Blame ·",
        ),
        page("main's commits", format!("{web}/commits/main"), message),
        page("a commit", format!("{web}/commit/{sha}"), "Add a comment"),
        page("branches", format!("{web}/branches"), "Branches"),
        page("tags", format!("{web}/tags"), "Tags"),
        page(
            "issues",
            format!("{web}/issues"),
            "Issue to close completed",
        ),
        page(
            "an issue",
            format!("{web}/issues/{ISSUE}"),
            "Issue to close completed",
        ),
        page("milestones", format!("{web}/milestones"), "Milestones"),
        page("pull requests", format!("{web}/pulls"), pr_title),
        page(
            "a pull request's conversation",
            format!("{web}/pull/{PR}"),
            "wants to merge into",
        ),
        page(
            "a pull request's files",
            format!("{web}/pull/{FILES_PR}/files"),
            FILE,
        ),
        page("Actions", format!("{web}/actions"), ""),
        page(
            "a workflow",
            format!("{web}/actions/workflows/actions.yml"),
            ".github/workflows/actions.yml",
        ),
        page("releases", format!("{web}/releases"), ""),
        page("discussions", format!("{web}/discussions"), ""),
        page("the wiki", format!("{web}/wiki"), "Wiki"),
        page("security", format!("{web}/security"), ""),
        page(
            "code scanning alerts",
            format!("{web}/security/code-scanning"),
            "",
        ),
        page(
            "settings",
            format!("{web}/settings"),
            "Archive this repository",
        ),
        page(
            "Insights",
            format!("{web}/graphs/contributors"),
            "Contributors · Commit activity · Traffic · Dependency graph",
        ),
        page(
            "Explore",
            "https://github.com/trending/rust?since=weekly",
            "Trending ranks repositories",
        ),
        page("a topic", "https://github.com/topics/rust", "Related"),
        page(
            "a profile",
            format!("https://github.com/{}", gh::OWNER),
            gh::OWNER,
        ),
        page(
            "gists",
            format!("https://gist.github.com/{}", gh::OWNER),
            "",
        ),
        page("home", "", "Review requests"),
        page(
            "the inbox",
            "https://github.com/notifications",
            "All   Unread   Read",
        ),
    ];
    let runs = gh::get(&format!("repos/{}/actions/runs?per_page=1", gh::REPO))?;
    let latest = &runs["workflow_runs"][0];
    if let Some(id) = latest["id"].as_u64() {
        let name = latest["name"].as_str().unwrap_or_default();
        let number = &latest["run_number"];
        pages.push(page(
            "a workflow run",
            format!("{web}/actions/runs/{id}"),
            format!("{name} #{number}"),
        ));
    }
    let found = gh::graphql(
        "query($o: String!, $n: String!) { repository(owner: $o, name: $n) {
            discussions(first: 1) { nodes { number title } } } }",
        json!({"o": gh::OWNER, "n": gh::NAME}),
    )?;
    let discussion = &found["repository"]["discussions"]["nodes"][0];
    if let Some(number) = discussion["number"].as_u64() {
        let title = discussion["title"].as_str().unwrap_or_default();
        pages.push(page(
            "a discussion",
            format!("{web}/discussions/{number}"),
            start_of(title),
        ));
    }
    if gh::get(&format!("repos/{}/code-scanning/alerts/1", gh::REPO)).is_ok() {
        pages.push(page(
            "a code scanning alert",
            format!("{web}/security/code-scanning/1"),
            "Code scanning alert #1",
        ));
    }
    if cx
        .env
        .scopes
        .iter()
        .any(|s| s == "project" || s == "read:project")
    {
        pages.push(page("projects", format!("{web}/projects"), ""));
    } else {
        cx.step(
            "projects: left out, the token lacks read:project (gh auth refresh -s read:project)",
        )?;
    }

    for page in pages {
        open(cx, &page)?;
    }
    Ok(())
}

/// Starts ghtui on the page; it shows what it must, stops loading, and
/// says it couldn't load nothing.
fn open(cx: &mut Cx, page: &Page) -> Result<()> {
    cx.step(page.what)?;
    let args: &[&str] = if page.url.is_empty() {
        &[]
    } else {
        &[page.url.as_str()]
    };
    let ghtui = cx.ghtui(args)?;
    if !page.shows.is_empty() {
        ghtui.wait_for(&page.shows)?;
    }
    ghtui.wait_until(&format!("{} loaded", page.what), LOADED, |screen| {
        let loading = screen
            .lines()
            .any(|l| l.contains("Loading") && l.contains('…'));
        (!loading).then_some(())
    })?;
    ghtui.wait_followed()?;
    if let Some(line) = ghtui
        .screen()?
        .lines()
        .find(|l| l.contains("Couldn't load"))
    {
        bail!("{}: {}", page.what, line.trim());
    }
    Ok(())
}
