//! Seeing reviews, on a throwaway pull request: the Actions bot
//! (`botreview.yml`, with `review: REQUEST_CHANGES`) submits a review
//! with a summary and an inline comment while the conversation is open.
//! It shows live, marked New, its comment grouped under it, the sidebar
//! names the reviewer's state, and the status bar says what came. The
//! Review tab lists every review; the bot's opens on its comments and on
//! "the changes it reviewed" and "what changed since" (after a push),
//! each opening Files changed that way. `,e` edits your own review's
//! summary.

use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde_json::json;

use crate::live::{Check, Cx, Launch, NARROW, gh, until};

pub const CHECK: Check = Check::new(
    "reviews",
    "a bot's review with an inline comment shows live, marked New, under the review and in the sidebar; the Review tab opens it, its changes and those since; ,e edits your review",
    run,
);

const GITHUB: Duration = Duration::from_secs(30);
/// A workflow takes a minute or so to start and review; ghtui asks every
/// 10s, doubling while nothing changes.
const LIVE: Duration = Duration::from_secs(200);
const FILE: &str = "review.txt";

fn run(cx: &mut Cx) -> Result<()> {
    cx.step("a pull request to review, and a review of your own on it")?;
    let branch = format!("{}review-{}", gh::BRANCH_PREFIX, cx.env.run_id);
    let undo = branch.clone();
    cx.defer(
        format!("close the pull request, delete {branch}"),
        move || gh::drop_branch(&undo),
    );
    let reviewed = gh::branch_with_file(&branch, FILE, "line one\nline two\nline three\n")?;
    let title = cx.mark("reviews");
    let pr = gh::create_pr(
        &title,
        &branch,
        "A pull request for the live check's reviews.",
    )?;
    let mine = format!("My review's summary {}", cx.env.run_id);
    let review = gh::send(
        "POST",
        &format!("repos/{}/pulls/{pr}/reviews", gh::REPO),
        Some(&json!({"event": "COMMENT", "body": mine})),
    )?;
    let my_review = review["id"].as_u64().context("no review id")?;

    let ghtui = cx.ghtui(&[&format!("{}#{pr}", gh::REPO)])?;
    ghtui.wait_for(&mine)?;
    ghtui.wait_followed()?;

    cx.step("the bot requests changes with an inline comment (botreview.yml), shown live")?;
    let since = gh::iso8601(gh::epoch().saturating_sub(30));
    gh::dispatch_with(
        "botreview.yml",
        &json!({"pr": pr.to_string(), "review": "REQUEST_CHANGES", "path": FILE, "line": "2"}),
    )?;
    let bot = until("the bot's review on GitHub", LIVE, || {
        let reviews = gh::rest_list(&format!("repos/{}/pulls/{pr}/reviews", gh::REPO))?;
        Ok(reviews.into_iter().find(|r| {
            r["user"]["login"] == "github-actions[bot]"
                && r["submitted_at"]
                    .as_str()
                    .is_some_and(|t| t >= since.as_str())
        }))
    })?;
    ensure!(
        bot["state"] == "CHANGES_REQUESTED",
        "GitHub has the bot's review as {}",
        bot["state"]
    );
    let summary = bot["body"].as_str().unwrap_or_default().to_owned();
    let bot_id = bot["id"].as_u64().unwrap_or_default();
    let inline = until("the review's inline comment on GitHub", GITHUB, || {
        let comments = gh::rest_list(&format!(
            "repos/{}/pulls/{pr}/reviews/{bot_id}/comments",
            gh::REPO
        ))?;
        Ok(comments
            .first()
            .and_then(|c| c["body"].as_str())
            .map(str::to_owned))
    })?;
    // The status bar says what came as it comes.
    let status = ghtui.wait_until("the bot's review on screen", LIVE, |screen| {
        screen.contains(&summary).then(|| last_line(screen))
    })?;
    ghtui.wait_for(&inline)?;
    let screen = ghtui.screen()?;
    let head = screen
        .lines()
        .find(|l| l.contains("github-actions requested changes") && l.contains('╭'))
        .context("no review box for the bot")?;
    ensure!(
        head.contains("New"),
        "the bot's review isn't marked New: {head}"
    );
    ensure!(
        under_review(&screen, &summary, &inline),
        "the inline comment isn't under its review"
    );
    // The sidebar's Reviewers: one space after the mark (the box's
    // heading has two).
    ensure!(
        screen.contains("✗ github-actions requested changes"),
        "the sidebar doesn't say the bot requested changes"
    );
    ensure!(
        status.contains("github-actions requested changes"),
        "the status bar doesn't name the review: {status}"
    );

    cx.step("after a push, the Review tab lists both reviews; the bot's opens on its comments")?;
    let pushed = gh::write_file(&branch, FILE, "line one\nline two, changed\nline three\n")?;
    let (reviewed, pushed) = (gh::short(&reviewed), gh::short(&pushed));
    ghtui.wait_for_within("New push", LIVE)?;
    ghtui.keys("r")?;
    ghtui.wait_followed()?;
    ghtui.keys("g r")?;
    ghtui.wait_for("Reviews · 2")?;
    ghtui.keys("gg")?;
    ghtui.select("}", "the bot's review", |s| {
        s.contains("⟦▌✗  github-actions requested changes")
    })?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Its changes")?;
    ghtui.wait_for(&inline)?;
    ghtui.wait_for(&format!(
        "The changes it reviewed, as they were at {reviewed}"
    ))?;
    ghtui.wait_for(&format!("What changed since, from {reviewed} to {pushed}"))?;

    cx.step("\"what changed since\" opens Files changed from the review's commit")?;
    ghtui.select("G", "what changed since", |s| {
        s.contains("⟦▌What changed since")
    })?;
    ghtui.keys("Enter")?;
    ghtui.wait_for(&format!("Showing changes since {reviewed}"))?;
    cx.step("\"the changes it reviewed\" opens Files changed as it was then")?;
    ghtui.keys("C-o")?;
    ghtui.wait_for("Its changes")?;
    ghtui.select("G k", "the changes it reviewed", |s| {
        s.contains("⟦▌The changes it reviewed")
    })?;
    ghtui.keys("Enter")?;
    ghtui.wait_for(&format!("Files · as reviewed at {reviewed}"))?;

    cx.step(",e on your own review edits its summary")?;
    ghtui.keys("g r")?;
    ghtui.wait_for("Reviews · 2")?;
    ghtui.keys("gg")?;
    let login = cx.env.login.clone();
    ghtui.select("}", "your review", |s| {
        s.contains(&format!("⟦▌◉  {login} reviewed"))
    })?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Its changes")?;
    ghtui.wait_followed()?;
    ghtui.keys(", e")?;
    ghtui.wait_for(&format!("Edit {login}'s review"))?;
    ghtui.type_text(" (edited)")?;
    ghtui.keys("C-s")?;
    let edited = format!("{mine} (edited)");
    until("the edited summary on GitHub", GITHUB, || {
        let review = gh::get(&format!(
            "repos/{}/pulls/{pr}/reviews/{my_review}",
            gh::REPO
        ))?;
        Ok((review["body"] == edited.as_str()).then_some(()))
    })?;
    ghtui.wait_for(&edited)?;

    cx.step("at 90 columns, the reviewers' states are said above the conversation")?;
    let narrow = cx.launch(Launch::new(&[&format!("{}#{pr}", gh::REPO)]).size(NARROW))?;
    narrow.wait_for("Reviewers: ✗ github-actions")?;
    narrow.wait_for(&summary)?;
    Ok(())
}

/// Whether `inline` is in the same box as the review `summary` heads.
fn under_review(screen: &str, summary: &str, inline: &str) -> bool {
    let mut in_review = false;
    for line in screen.lines() {
        if line.contains(summary) {
            in_review = true;
        } else if in_review && line.contains(inline) {
            return true;
        } else if in_review && line.contains('╰') {
            return false;
        }
    }
    false
}

fn last_line(screen: &str) -> String {
    screen
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default()
        .trim()
        .to_owned()
}
