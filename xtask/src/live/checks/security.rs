//! The Security tab's alerts: `,x` dismisses code scanning alert #1 (from
//! the SARIF uploaded to the test repository) with a reason and a
//! comment, and `,x` again reopens it, after asking. The alert is left
//! open, as it was.

use std::time::Duration;

use anyhow::Result;
use serde_json::{Value, json};

use crate::live::{Check, Cx, gh, until};

pub const CHECK: Check = Check::new(
    "security",
    ",x dismisses code scanning alert #1 with a reason and a comment, and ,x reopens it",
    run,
)
.sweep(reopen);

const ALERT: u64 = 1;
const GITHUB: Duration = Duration::from_secs(30);

fn run(cx: &mut Cx) -> Result<()> {
    let alert = match get() {
        Ok(alert) => alert,
        Err(err) => {
            return Err(Cx::skip(format!(
                "can't read code scanning alert #{ALERT}: {err:#}"
            )));
        }
    };
    if alert["state"] != "open" {
        reopen()?;
    }
    cx.defer(format!("reopen code scanning alert #{ALERT}"), reopen);
    let url = format!(
        "https://github.com/{}/security/code-scanning/{ALERT}",
        gh::REPO
    );
    let ghtui = cx.ghtui(&[&url])?;
    ghtui.wait_for(&format!("Code scanning alert #{ALERT}"))?;
    ghtui.wait_followed()?;

    cx.step(",x dismisses it as used in tests, with a comment")?;
    let comment = format!("from the live check {}", cx.env.run_id);
    ghtui.keys(", x")?;
    ghtui.wait_for("Dismiss the alert")?;
    ghtui.type_text(&comment)?;
    ghtui.select("Down", "Used in tests", |s| s.contains("⟦▌ Used in tests"))?;
    ghtui.keys("Enter")?;
    until("the alert dismissed on GitHub", GITHUB, || {
        let alert = get()?;
        Ok((alert["state"] == "dismissed"
            && alert["dismissed_reason"] == "used in tests"
            && alert["dismissed_comment"] == comment.as_str())
        .then_some(()))
    })?;
    ghtui.wait_for("Dismissed")?;
    ghtui.wait_for(&comment)?;

    ghtui.wait_followed()?;
    cx.step(",x on the dismissed alert reopens it, after asking")?;
    ghtui.keys(", x")?;
    ghtui.wait_for(&format!("Reopen code scanning alert #{ALERT}?"))?;
    ghtui.keys("Enter")?;
    until("the alert open on GitHub", GITHUB, || {
        Ok((get()?["state"] == "open").then_some(()))
    })?;
    ghtui.wait_gone("Dismissed")?;
    Ok(())
}

fn get() -> Result<Value> {
    gh::get(&format!("repos/{}/code-scanning/alerts/{ALERT}", gh::REPO))
}

/// The test repository keeps the alert open.
fn reopen() -> Result<()> {
    if get().is_ok_and(|a| a["state"] == "dismissed") {
        gh::send(
            "PATCH",
            &format!("repos/{}/code-scanning/alerts/{ALERT}", gh::REPO),
            Some(&json!({"state": "open"})),
        )?;
    }
    Ok(())
}
