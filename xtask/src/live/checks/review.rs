//! Live review comments on PR #11's Files changed: one of yours posted in
//! the first seconds after it opens shows (PR #41's bug: it never did),
//! unmarked; another author's (the Actions bot's, from `botreview.yml`)
//! shows marked New, and the status bar names it.

use std::time::Duration;

use anyhow::{Result, ensure};
use serde_json::json;

use crate::live::{Check, Cx, gh, until};

pub const CHECK: Check = Check::new(
    "live-review",
    "PR #11's Files changed shows review comments as they come: yours from the first seconds, and the bot's marked New",
    run,
);

const PR: u64 = 11;
/// The file `botreview.yml` comments on.
const FILE: &str = "bot-bot-1791523358.txt";
/// ghtui asks every 10s, doubling while nothing changes; a workflow takes
/// a minute or so to start and comment.
const LIVE: Duration = Duration::from_secs(200);

fn run(cx: &mut Cx) -> Result<()> {
    let head = gh::get(&format!("repos/{}/pulls/{PR}", gh::REPO))?["head"]["sha"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let url = format!("https://github.com/{}/pull/{PR}/files", gh::REPO);
    let ghtui = cx.ghtui(&[&url])?;
    ghtui.wait_for(FILE)?;
    ghtui.wait_for("more")?;

    cx.step("a review comment posted in the first seconds after opening shows")?;
    let mine = format!("A review comment from the live check {}", cx.env.run_id);
    let posted = gh::send(
        "POST",
        &format!("repos/{}/pulls/{PR}/comments", gh::REPO),
        Some(&json!({"body": mine, "commit_id": head, "path": FILE, "line": 1, "side": "RIGHT"})),
    )?;
    let mine_id = posted["id"].as_u64().unwrap_or_default();
    cx.defer("delete the check's review comment", move || {
        delete_review_comment(mine_id)
    });
    let since = gh::iso8601(gh::epoch().saturating_sub(30));
    gh::dispatch("botreview.yml")?;
    let after = since.clone();
    cx.defer("delete the bot's review comment", move || {
        for id in bot_comments(&after)? {
            delete_review_comment(id)?;
        }
        Ok(())
    });
    ghtui.wait_for_within(&mine, Duration::from_secs(60))?;
    // A comment's author line is above its text, with New on it if new.
    let screen = ghtui.screen()?;
    let lines: Vec<&str> = screen.lines().collect();
    let at = lines
        .iter()
        .position(|l| l.contains(&mine))
        .unwrap_or_default();
    let author = at
        .checked_sub(1)
        .and_then(|i| lines.get(i))
        .copied()
        .unwrap_or_default();
    ensure!(
        author.contains(&cx.env.login) && !author.contains("New"),
        "your own comment is marked: {}",
        author.trim()
    );

    cx.step("the Actions bot's comment (botreview.yml) shows, marked New")?;
    let bot_body = until("the bot's review comment on GitHub", LIVE, || {
        let comments = gh::rest_list(&format!(
            "repos/{}/pulls/{PR}/comments?since={since}",
            gh::REPO
        ))?;
        Ok(comments.into_iter().find_map(|c| {
            let body = c["body"].as_str()?;
            (c["user"]["login"] == "github-actions[bot]"
                && c["created_at"].as_str()? >= since.as_str())
            .then(|| body.to_owned())
        }))
    })?;
    let line = ghtui.wait_for_within(&bot_body, LIVE)?;
    let screen = ghtui.screen()?;
    let lines: Vec<&str> = screen.lines().collect();
    let at = lines.iter().position(|l| *l == line).unwrap_or_default();
    let author = at
        .checked_sub(1)
        .and_then(|i| lines.get(i))
        .copied()
        .unwrap_or_default();
    ensure!(
        author.contains("github-actions") && author.contains("New"),
        "the bot's comment isn't marked New: {}",
        author.trim()
    );
    Ok(())
}

/// The bot's review comments on #11 since `since`.
fn bot_comments(since: &str) -> Result<Vec<u64>> {
    let comments = gh::rest_list(&format!(
        "repos/{}/pulls/{PR}/comments?since={since}",
        gh::REPO
    ))?;
    Ok(comments
        .iter()
        .filter(|c| {
            c["user"]["login"] == "github-actions[bot]"
                && c["created_at"].as_str().is_some_and(|t| t >= since)
        })
        .filter_map(|c| c["id"].as_u64())
        .collect())
}

fn delete_review_comment(id: u64) -> Result<()> {
    gh::send(
        "DELETE",
        &format!("repos/{}/pulls/comments/{id}", gh::REPO),
        None,
    )?;
    Ok(())
}
