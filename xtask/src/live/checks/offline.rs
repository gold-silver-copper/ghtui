//! Writing offline, against the stand-in Enterprise Server
//! ([`crate::ghes`]), which is stopped to make GitHub go away and started
//! again to bring it back: the page stays readable from the cache, marked
//! offline; a comment written meanwhile is queued, shown on its page, kept
//! across a restart, and sent exactly once when GitHub is back.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, ensure};
use serde_json::Value;

use crate::ghes::StandIn;
use crate::live::{Check, Cx, Launch, gh, until};

pub const CHECK: Check = Check::new(
    "offline",
    "GitHub goes away: the page from the cache, marked offline; a comment queued, kept across a restart, sent once when it's back",
    run,
);

/// Off `cargo xtask ghes`'s default port, so one running by hand is left be.
const PORT: u16 = 8451;
const GITHUB: Duration = Duration::from_secs(60);

fn start(cx: &Cx) -> Result<StandIn> {
    StandIn::start(&cx.env.work.join("ghes"), PORT, &[], Some("3.22.0"))
}

fn run(cx: &mut Cx) -> Result<()> {
    let title = cx.mark("offline");
    let issue = gh::create_issue(&title, "For the offline check.")?;
    cx.defer(format!("delete issue #{issue}"), move || {
        gh::delete_issue(issue)
    });
    let standin = Arc::new(Mutex::new(Some(start(cx)?)));
    let stop = Arc::clone(&standin);
    cx.defer("stop the stand-in server", move || {
        if let Ok(mut s) = stop.lock() {
            s.take();
        }
        Ok(())
    });
    let env = standin
        .lock()
        .ok()
        .and_then(|s| s.as_ref().map(StandIn::env))
        .unwrap_or_default();
    let url = format!("{}#{issue}", gh::REPO);

    cx.step("the issue loads through the stand-in, and is cached")?;
    let ghtui = cx.launch(Launch::new(&[&url]).env(env.clone()))?;
    ghtui.wait_for(&title)?;
    ghtui.wait_followed()?;

    cx.step("GitHub goes away: the page stays, marked offline")?;
    if let Ok(mut s) = standin.lock() {
        s.take();
    }
    ghtui.keys("r")?;
    ghtui.wait_for("⚠ Offline")?;
    ghtui.wait_for(&title)?;

    cx.step("c writes a comment: it's queued, and shown on the page")?;
    let comment = format!("Written offline by the live check {}", cx.env.run_id);
    ghtui.keys("c")?;
    ghtui.wait_for("open in $EDITOR")?;
    ghtui.type_text(&comment)?;
    ghtui.keys("C-s")?;
    ghtui.wait_for("Queued for GitHub  1")?;
    ghtui.wait_for(&comment)?;

    cx.step("ghtui restarts, still offline: the comment is still queued")?;
    let ghtui = cx.launch(Launch::new(&[&url]).env(env))?;
    ghtui.wait_for("⚠ Offline   1 queued")?;
    ghtui.wait_for(&comment)?;
    ensure!(
        comments(issue)?.is_empty(),
        "the comment reached GitHub while it was away"
    );

    cx.step("GitHub is back: the comment is sent, once")?;
    if let Ok(mut s) = standin.lock() {
        *s = Some(start(cx)?);
    }
    until("the comment on GitHub", GITHUB, || {
        Ok(comments(issue)?
            .iter()
            .any(|c| c["body"] == comment.as_str())
            .then_some(()))
    })?;
    ghtui.wait_gone("Queued for GitHub")?;
    // Long enough for a second send, if there were one.
    std::thread::sleep(Duration::from_secs(10));
    let sent = comments(issue)?
        .iter()
        .filter(|c| c["body"] == comment.as_str())
        .count();
    ensure!(sent == 1, "the comment is on GitHub {sent} times");
    ghtui.wait_gone("⚠ Offline")?;
    Ok(())
}

fn comments(number: u64) -> Result<Vec<Value>> {
    gh::rest_list(&format!("repos/{}/issues/{number}/comments", gh::REPO))
}
