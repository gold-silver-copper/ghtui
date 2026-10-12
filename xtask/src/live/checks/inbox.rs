//! Marking notifications read and done, on the test repository's real
//! threads. `mention.yml` has the Actions bot mention the owner on issue
//! #8, which makes one unread. Only the test repository's threads are
//! touched: the inbox is filtered to it first, so `,N` uses the
//! repository-scoped endpoint, and the owner's own inbox is left alone.
//!
//! GitHub can take many minutes to deliver a notification (17 when this
//! was written), so the check uses any unread thread the repository has,
//! and is skipped when none has arrived: the mention it asked for then
//! serves the next run.

use std::time::Duration;

use anyhow::{Result, ensure};
use serde_json::{Value, json};

use crate::live::{Check, Cx, gh, until};

pub const CHECK: Check = Check::new(
    "inbox",
    "a mention (mention.yml) makes an unread thread; ,N marks the repository's threads read, ,d marks one done",
    run,
)
.prepare(prepare)
.sweep(sweep);

/// How long the check waits for an unread thread.
const NOTIFIED: Duration = Duration::from_secs(120);
const GITHUB: Duration = Duration::from_secs(30);
const ISSUE: u64 = 8;

/// Asks for the mention early, so the wait overlaps the checks before.
fn prepare(_: &mut Cx) -> Result<()> {
    gh::send(
        "POST",
        &format!(
            "repos/{}/actions/workflows/mention.yml/dispatches",
            gh::REPO
        ),
        Some(&json!({"ref": "main", "inputs": {"issue": ISSUE.to_string()}})),
    )?;
    Ok(())
}

fn run(cx: &mut Cx) -> Result<()> {
    cx.step("the test repository has an unread thread (mention.yml)")?;
    let unread = until("an unread notification", NOTIFIED, || {
        Ok(threads()?.into_iter().find(|t| t["unread"] == true))
    });
    let Ok(unread) = unread else {
        return Err(Cx::skip(
            "GitHub delivered no notification for the test repository in time; the mention asked for serves the next run",
        ));
    };
    let title = unread["subject"]["title"]
        .as_str()
        .unwrap_or_default()
        .to_owned();

    let ghtui = cx.ghtui(&[])?;
    ghtui.wait_for("Review requests")?;
    ghtui.wait_followed()?;
    ghtui.keys("g n")?;
    ghtui.wait_for("All   Unread   Read")?;
    // Never the owner's whole inbox: the test repository's threads only.
    ghtui.keys("/")?;
    ghtui.type_text(&format!("repo:{}", gh::REPO))?;
    ghtui.keys("Enter")?;
    ghtui.wait_for(&title)?;

    cx.step(",N marks the repository's threads read, after asking")?;
    ghtui.keys(", N")?;
    ghtui.wait_for("↵ do it")?;
    ghtui.keys("Enter")?;
    until("every thread read on GitHub", GITHUB, || {
        Ok(threads()?
            .iter()
            .all(|t| t["unread"] == false)
            .then_some(()))
    })?;

    cx.step(",d marks the first thread done")?;
    ghtui.wait_followed()?;
    ghtui.keys("gg , d")?;
    ghtui.wait_until("ghtui to say it's done", GITHUB, |screen| {
        let status = screen.lines().rev().find(|l| !l.trim().is_empty())?;
        status.contains("done").then_some(())
    })?;
    // GitHub's REST API goes on listing a thread marked done, read
    // (README, Known limitations): that it's read is all it shows.
    ensure!(
        threads()?.iter().all(|t| t["unread"] == false),
        "a thread is unread again"
    );
    Ok(())
}

/// The test repository's notification threads, read and unread.
fn threads() -> Result<Vec<Value>> {
    let all = gh::get(&format!("repos/{}/notifications?all=true", gh::REPO))?;
    Ok(all.as_array().cloned().unwrap_or_default())
}

/// The bot's mentions on #8 pile up: those over a day old go (newer ones
/// may still be waiting for GitHub to notify).
fn sweep() -> Result<()> {
    let old = gh::iso8601(gh::epoch().saturating_sub(86_400));
    for comment in gh::rest_list(&format!("repos/{}/issues/{ISSUE}/comments", gh::REPO))? {
        let mention = comment["user"]["login"] == "github-actions[bot]"
            && comment["body"]
                .as_str()
                .is_some_and(|b| b.contains("inbox check ("));
        if mention
            && comment["created_at"]
                .as_str()
                .is_some_and(|t| t < old.as_str())
            && let Some(id) = comment["id"].as_u64()
        {
            gh::send(
                "DELETE",
                &format!("repos/{}/issues/comments/{id}", gh::REPO),
                None,
            )?;
        }
    }
    Ok(())
}
