//! Git first: a file and its blame in the test repository are read by git
//! at GitHub's head of main (the page's first line says so), and code
//! navigation reads gold-silver-copper/ghtui's clone (read-only): `gd`
//! goes to a definition in another file, `gR` lists where a word is used,
//! `gS` finds a symbol in the repository. Line numbers are checked
//! against the files as GitHub has them.

use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::json;

use crate::live::{Check, Cx, Pane, gh};

pub const CHECK: Check = Check::new(
    "local-reads",
    "a file and its blame read by git at main's head; gd, gR and gS on gold-silver-copper/ghtui from its clone",
    run,
);

/// The cache's clone of ghtui is made in the background the first time.
const CLONE: Duration = Duration::from_secs(180);
/// The repository code navigation reads, only read.
const NAV_OWNER: &str = "gold-silver-copper";
const NAV_NAME: &str = "ghtui";

fn run(cx: &mut Cx) -> Result<()> {
    cx.step("a file of the test repository: read by git at main's head")?;
    let main = gh::get(&format!("repos/{}/git/ref/heads/main", gh::REPO))?;
    let head = gh::short(main["object"]["sha"].as_str().unwrap_or_default());
    let url = format!("https://github.com/{}/blob/main/README.md", gh::REPO);
    let ghtui = cx.ghtui(&[&url])?;
    read_by_git(&ghtui)?;
    ghtui.wait_for(&format!("At {head} · read by git"))?;

    cx.step("gb blames it with git: each run of lines names its commit")?;
    let commits = gh::get(&format!(
        "repos/{}/commits?path=README.md&sha=main&per_page=100",
        gh::REPO
    ))?;
    let first = commits
        .as_array()
        .and_then(|c| c.last())
        .and_then(|c| c["sha"].as_str())
        .map(gh::short)
        .context("GitHub lists no commit of README.md")?;
    ghtui.keys("g b")?;
    ghtui.wait_for("Blame ·")?;
    ghtui.wait_for(&first)?;
    ghtui.wait_for("read by git")?;

    cx.step("gold-silver-copper/ghtui's src/lib.rs, read from its clone")?;
    let lib = blob("src/lib.rs")?;
    let credentials = blob("src/git/credentials.rs")?;
    let used_at = line_of(&lib, "credentials::ASKPASS_FLAG")?;
    let defined_at = line_of(&credentials, "pub const ASKPASS_FLAG")?;
    let token_at = line_of(&credentials, "pub const ASKPASS_TOKEN")?;
    let url = format!("https://github.com/{NAV_OWNER}/{NAV_NAME}/blob/main/src/lib.rs");
    let ghtui = cx.ghtui(&[&url])?;
    read_by_git(&ghtui)?;
    ghtui.wait_followed()?;

    cx.step("gd on ASKPASS_FLAG goes to its definition in src/git/credentials.rs")?;
    ghtui.keys("/")?;
    ghtui.type_text("ASKPASS_FLAG")?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Match 1 of")?;
    ghtui.keys("g d")?;
    ghtui.wait_until("the definition selected", CLONE, |screen| {
        selected_line(screen, defined_at, "pub const ASKPASS_FLAG")
    })?;
    ghtui.keys("gg")?;
    ghtui.wait_for("credentials.rs")?;

    cx.step("gR lists where it's used, from git grep")?;
    ghtui.keys("C-o")?;
    ghtui.wait_until("back on src/lib.rs's use", CLONE, |screen| {
        selected_line(screen, used_at, "ASKPASS_FLAG")
    })?;
    ghtui.keys("g R")?;
    ghtui.wait_for("References")?;
    ghtui.wait_for(&format!("src/git/credentials.rs:{defined_at}"))?;
    ghtui.wait_for(&format!("src/lib.rs:{used_at}"))?;
    ghtui.keys("Escape")?;

    cx.step("gS finds a symbol anywhere in the repository and goes there")?;
    ghtui.keys("g S")?;
    ghtui.wait_for("Symbols in the repository")?;
    ghtui.type_text("ASKPASS_TOKEN")?;
    ghtui.wait_for(&format!("src/git/credentials.rs:{token_at}"))?;
    ghtui.keys("Enter")?;
    ghtui.wait_until("the symbol selected", CLONE, |screen| {
        selected_line(screen, token_at, "pub const ASKPASS_TOKEN")
    })?;
    Ok(())
}

/// Waits for the page to say it was read by git, refreshing while the
/// cache's clone is still being made.
fn read_by_git(ghtui: &Pane) -> Result<()> {
    let began = Instant::now();
    loop {
        let screen = ghtui.screen()?;
        if screen.contains("read by git") {
            return Ok(());
        }
        if began.elapsed() > CLONE {
            let first = screen
                .lines()
                .find(|l| l.contains("From GitHub's API"))
                .unwrap_or("nothing says where it was read")
                .trim()
                .to_owned();
            bail!("not read by git after {}s: {first}", CLONE.as_secs());
        }
        if screen.contains("From GitHub's API") {
            // It says why: the clone is still being made. Look again soon.
            let _ = ghtui.wait_for_within("read by git", Duration::from_secs(10));
            ghtui.keys("r")?;
        } else {
            let _ = ghtui.wait_for_within("read by git", Duration::from_secs(5));
        }
    }
}

/// The selected line (`▌` before its number) is line `n`, with `text`.
fn selected_line(screen: &str, n: usize, text: &str) -> Option<()> {
    screen
        .lines()
        .any(|l| {
            l.split_once('▌').is_some_and(|(_, rest)| {
                rest.trim_start().starts_with(&n.to_string()) && rest.contains(text)
            })
        })
        .then_some(())
}

/// A file of gold-silver-copper/ghtui at main, as GitHub has it.
fn blob(path: &str) -> Result<String> {
    let data = gh::graphql(
        "query($o: String!, $n: String!, $e: String!) { repository(owner: $o, name: $n) {
            object(expression: $e) { ... on Blob { text } } } }",
        json!({"o": NAV_OWNER, "n": NAV_NAME, "e": format!("main:{path}")}),
    )?;
    data["repository"]["object"]["text"]
        .as_str()
        .map(str::to_owned)
        .with_context(|| format!("GitHub has no {path} on {NAV_OWNER}/{NAV_NAME}'s main"))
}

/// The first line (from 1) of `text` that has `what`.
fn line_of(text: &str, what: &str) -> Result<usize> {
    text.lines()
        .position(|l| l.contains(what))
        .map(|i| i + 1)
        .with_context(|| format!("no line has {what}"))
}
