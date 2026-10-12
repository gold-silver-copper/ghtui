//! An issue's life in ghtui: created, its description edited, a comment
//! written, edited and deleted, a reaction, a label, a new title, closed.

use std::time::Duration;

use anyhow::{Result, ensure};
use serde_json::Value;

use crate::live::{Check, Cx, gh, until};

pub const CHECK: Check = Check::new(
    "issue",
    "creating an issue, editing it, a comment written, edited and deleted, a reaction, a label, a title, closing",
    run,
);

const GITHUB: Duration = Duration::from_secs(30);

fn run(cx: &mut Cx) -> Result<()> {
    let title = cx.mark("issue");
    let ghtui = cx.ghtui(&[gh::REPO])?;
    ghtui.wait_for("Pull requests")?;

    cx.step(",i writes a new issue; ctrl-s creates it")?;
    ghtui.keys(", i")?;
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
    cx.step(",e edits the description")?;
    ghtui.keys(", e")?;
    ghtui.wait_for("Edit the description")?;
    ghtui.type_text(" Edited.")?;
    ghtui.keys("C-s")?;
    until("the edited description", GITHUB, || {
        Ok((gh::issue(number)?["body"] == "Written by ghtui's live check. Edited.").then_some(()))
    })?;
    ghtui.wait_for("live check. Edited.")?;

    ghtui.wait_followed()?;
    cx.step("c writes a comment")?;
    let comment = format!("A comment from the live check {}", cx.env.run_id);
    ghtui.keys("c")?;
    ghtui.wait_for("open in $EDITOR")?;
    ghtui.type_text(&comment)?;
    ghtui.keys("C-s")?;
    let id = until("the comment on GitHub", GITHUB, || {
        Ok(comments(number)?
            .iter()
            .find(|c| c["body"] == comment.as_str())
            .and_then(|c| c["id"].as_u64()))
    })?;
    ghtui.wait_for(&comment)?;

    ghtui.wait_followed()?;
    cx.step(",e on the selected comment edits it")?;
    let selected = format!("⟦▌{comment}");
    ghtui.select("gg }", "the comment", |s| s.contains(&selected))?;
    ghtui.keys(", e")?;
    ghtui.wait_for("open in $EDITOR")?;
    ghtui.type_text(" (edited)")?;
    ghtui.keys("C-s")?;
    let edited = format!("{comment} (edited)");
    until("the edited comment", GITHUB, || {
        Ok(comments(number)?
            .iter()
            .any(|c| c["id"].as_u64() == Some(id) && c["body"] == edited.as_str())
            .then_some(()))
    })?;
    ghtui.wait_for(&edited)?;

    ghtui.wait_followed()?;
    cx.step(",D deletes it, after asking")?;
    ghtui.keys(", D")?;
    ghtui.wait_for("Delete gold-silver-copper's comment?")?;
    ghtui.keys("Enter")?;
    until("the comment gone from GitHub", GITHUB, || {
        Ok(comments(number)?.is_empty().then_some(()))
    })?;
    ghtui.wait_gone(&edited)?;

    ghtui.wait_followed()?;
    cx.step(",+ reacts to the issue")?;
    ghtui.keys(", +")?;
    ghtui.wait_for("gives or takes back your reaction")?;
    ghtui.keys("Enter")?;
    let login = cx.env.login.clone();
    until("the 👍 on GitHub", GITHUB, || {
        let reactions = gh::get(&format!("repos/{}/issues/{number}/reactions", gh::REPO))?;
        Ok(reactions
            .as_array()
            .is_some_and(|r| {
                r.iter()
                    .any(|r| r["content"] == "+1" && r["user"]["login"] == login.as_str())
            })
            .then_some(()))
    })?;
    ghtui.wait_for("👍 1")?;

    ghtui.wait_followed()?;
    cx.step(",l labels it bug")?;
    ghtui.keys(", l")?;
    ghtui.wait_for("ctrl-s applies")?;
    ghtui.type_text("bug")?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("✓ bug")?;
    ghtui.keys("C-s")?;
    until("the label on GitHub", GITHUB, || {
        let labels = gh::issue(number)?["labels"].clone();
        Ok(
            (labels.as_array().map(Vec::len) == Some(1) && labels[0]["name"] == "bug")
                .then_some(()),
        )
    })?;

    ghtui.wait_followed()?;
    cx.step(",T renames it")?;
    ghtui.keys(", T")?;
    ghtui.wait_for("Edit the title")?;
    ghtui.type_text(" (renamed)")?;
    ghtui.keys("Enter")?;
    let renamed = format!("{title} (renamed)");
    until("the new title on GitHub", GITHUB, || {
        Ok((gh::issue(number)?["title"] == renamed.as_str()).then_some(()))
    })?;
    ghtui.wait_for(&renamed)?;

    ghtui.wait_followed()?;
    cx.step(",x closes it as completed, after asking")?;
    ghtui.keys(", x")?;
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
