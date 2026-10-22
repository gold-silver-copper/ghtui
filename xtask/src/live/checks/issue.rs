//! An issue: created with `,i`, commented on with `c`, a comment made on
//! GitHub meanwhile shown live, and closed with `,x`.

use std::time::Duration;

use anyhow::{Result, ensure};
use serde_json::{Value, json};

use crate::live::{Check, Cx, gh, until};

pub const CHECK: Check = Check::new(
    "issue",
    ":new issue creates an issue, c comments on it, a comment made on GitHub shows live, x closes it",
    run,
);

const GITHUB: Duration = Duration::from_secs(30);
/// An issue's page asks every 10s, doubling while nothing changes.
const LIVE: Duration = Duration::from_secs(120);

fn run(cx: &mut Cx) -> Result<()> {
    let title = cx.mark("issue");
    let ghtui = cx.ghtui(&[gh::REPO])?;
    ghtui.wait_for("Pull requests")?;

    cx.step(":new issue writes a new issue; ctrl-s creates it")?;
    ghtui.palette("new issue")?;
    ghtui.wait_for("New issue in")?;
    ghtui.type_text(&title)?;
    ghtui.keys("Enter")?;
    ghtui.type_text("Written by ghtui's live check.")?;
    ghtui.keys("C-s")?;
    let number = until("the new issue on GitHub", GITHUB, || {
        let mine = gh::get(&format!(
            "repos/{}/issues?creator=@me&state=all&sort=created&per_page=10",
            gh::REPO
        ))?;
        Ok(mine.as_array().and_then(|issues| {
            issues
                .iter()
                .find(|i| i["title"] == title.as_str())
                .and_then(|i| i["number"].as_u64())
        }))
    })?;
    cx.defer(format!("delete issue #{number}"), move || {
        gh::delete_issue(number)
    });
    ghtui.wait_for(&format!("#{number}"))?;
    let issue = gh::issue(number)?;
    ensure!(
        issue["body"] == "Written by ghtui's live check.",
        "its description is {}",
        issue["body"]
    );

    ghtui.wait_followed()?;
    cx.step("c writes a comment")?;
    let comment = format!("A comment from the live check {}", cx.env.run_id);
    ghtui.keys("c")?;
    ghtui.wait_for("open in $EDITOR")?;
    ghtui.type_text(&comment)?;
    ghtui.keys("C-s")?;
    until("the comment on GitHub", GITHUB, || {
        Ok(comments(number)?
            .iter()
            .any(|c| c["body"] == comment.as_str())
            .then_some(()))
    })?;
    ghtui.wait_for(&comment)?;

    cx.step("a comment made on GitHub shows, live")?;
    let elsewhere = format!("A comment made elsewhere {}", cx.env.run_id);
    gh::send(
        "POST",
        &format!("repos/{}/issues/{number}/comments", gh::REPO),
        Some(&json!({"body": elsewhere})),
    )?;
    ghtui.wait_for_within(&elsewhere, LIVE)?;

    ghtui.wait_followed()?;
    cx.step("x closes it as completed, after asking")?;
    ghtui.keys("x")?;
    ghtui.wait_for("Close as completed")?;
    ghtui.keys("Enter")?;
    until("the issue closed on GitHub", GITHUB, || {
        let issue = gh::issue(number)?;
        Ok((issue["state"] == "closed" && issue["state_reason"] == "completed").then_some(()))
    })?;
    ghtui.wait_for("✓ Closed")?;
    Ok(())
}

fn comments(number: u64) -> Result<Vec<Value>> {
    gh::rest_list(&format!("repos/{}/issues/{number}/comments", gh::REPO))
}
