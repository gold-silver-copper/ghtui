//! Reading offline, against the stand-in Enterprise Server
//! ([`crate::ghes`]), which is stopped to make GitHub go away and started
//! again to bring it back. Pages stay readable from the cache, marked
//! offline; a comment and a reaction written meanwhile are queued, shown
//! on their page, kept across a restart, and sent once (exactly once)
//! when GitHub is back; a comment GitHub refuses then (its issue deleted
//! meanwhile) is shown refused, with its text, and thrown away.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, ensure};
use serde_json::Value;

use crate::ghes::StandIn;
use crate::live::{Check, Cx, Launch, Pane, gh, until};

pub const CHECK: Check = Check::new(
    "offline",
    "GitHub goes away: pages from the cache, marked offline; a comment and a reaction queued, kept across a restart, sent once when it's back; a refused one kept, then thrown away",
    run,
);

/// Its own port: the enterprise checks have 8443 and 8444.
const PORT: u16 = 8451;
const GITHUB: Duration = Duration::from_secs(60);

fn start(cx: &Cx) -> Result<StandIn> {
    StandIn::start(&cx.env.work.join("ghes"), PORT, &[], Some("3.22.0"))
}

/// Goes to `owner/repo#n` with the palette.
fn go(ghtui: &Pane, number: u64) -> Result<()> {
    ghtui.keys(":")?;
    ghtui.type_text(&format!("{}#{number}", gh::REPO))?;
    ghtui.keys("Enter")
}

fn run(cx: &mut Cx) -> Result<()> {
    let kept = cx.mark("offline, sent when back");
    let gone = cx.mark("offline, deleted meanwhile");
    let issue = gh::create_issue(&kept, "For the offline check.")?;
    cx.defer(format!("delete issue #{issue}"), move || {
        gh::delete_issue(issue)
    });
    let doomed = gh::create_issue(&gone, "Deleted while ghtui is offline.")?;
    cx.defer(format!("delete issue #{doomed}"), move || {
        // Deleted by the check, unless it stopped first.
        let _ = gh::delete_issue(doomed);
        Ok(())
    });

    let standin = Arc::new(std::sync::Mutex::new(Some(start(cx)?)));
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

    cx.step("both issues load through the stand-in, and are cached")?;
    let launch = || Launch::new(&[&format!("{}#{doomed}", gh::REPO)]).env(env.clone());
    let ghtui = cx.launch(launch())?;
    ghtui.wait_for(&gone)?;
    go(&ghtui, issue)?;
    ghtui.wait_for(&kept)?;
    ghtui.wait_followed()?;

    cx.step("GitHub goes away: the page stays, marked offline, and the header says so")?;
    if let Ok(mut s) = standin.lock() {
        s.take();
    }
    ghtui.keys("r")?;
    ghtui.wait_for("⚠ Offline")?;
    ghtui.wait_for("Offline: showing what was loaded")?;
    ghtui.wait_for(&kept)?;

    cx.step("c writes a comment: it's queued, and shown on the page")?;
    let comment = format!("Written offline by the live check {}", cx.env.run_id);
    ghtui.keys("c")?;
    ghtui.wait_for("open in $EDITOR")?;
    ghtui.type_text(&comment)?;
    ghtui.keys("C-s")?;
    ghtui.wait_for("Queued for GitHub  1")?;
    ghtui.wait_for(&comment)?;

    cx.step(",+ reacts: queued too")?;
    ghtui.keys(", +")?;
    ghtui.wait_for("gives or takes back your reaction")?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Queued for GitHub  2")?;
    ghtui.wait_for("⚠ Offline · 2 queued")?;

    cx.step("a comment on the other issue, which is deleted meanwhile")?;
    let refused = format!("Refused when back {}", cx.env.run_id);
    go(&ghtui, doomed)?;
    ghtui.wait_for(&gone)?;
    ghtui.keys("c")?;
    ghtui.wait_for("open in $EDITOR")?;
    ghtui.type_text(&refused)?;
    ghtui.keys("C-s")?;
    ghtui.wait_for("⚠ Offline · 3 queued")?;
    gh::delete_issue(doomed)?;

    cx.step("ghtui restarts, still offline: the queue is as it was")?;
    let ghtui = cx.launch(Launch::new(&[&format!("{}#{issue}", gh::REPO)]).env(env.clone()))?;
    ghtui.wait_for("⚠ Offline · 3 queued")?;
    ghtui.wait_for(&kept)?;
    ghtui.wait_for(&comment)?;
    ensure!(
        comments(issue)?.is_empty(),
        "a comment reached GitHub while it was away"
    );

    cx.step("GitHub is back: what was queued is sent, once")?;
    if let Ok(mut s) = standin.lock() {
        *s = Some(start(cx)?);
    }
    let login = cx.env.login.clone();
    until("the comment and the reaction on GitHub", GITHUB, || {
        let commented = comments(issue)?.iter().any(|c| c["body"] == comment.as_str());
        Ok((commented && thumbs(issue, &login)? > 0).then_some(()))
    })?;
    ghtui.wait_gone("Queued for GitHub")?;
    // Long enough for a second send, if there were one.
    std::thread::sleep(Duration::from_secs(10));
    let sent = comments(issue)?
        .iter()
        .filter(|c| c["body"] == comment.as_str())
        .count();
    ensure!(sent == 1, "the comment is on GitHub {sent} times");
    let thumbs = thumbs(issue, &login)?;
    ensure!(thumbs == 1, "{thumbs} 👍 on GitHub");
    ghtui.wait_gone("⚠ Offline")?;

    cx.step("the refused comment is kept on the Offline page, then thrown away")?;
    ghtui.wait_for("⚠ 1 not sent")?;
    ghtui.keys(":")?;
    ghtui.type_text("offline")?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Queued for GitHub  1")?;
    ghtui.wait_for("Refused")?;
    ghtui.wait_for(&refused)?;
    ghtui.select("gg }", "the refused comment", |s| s.contains("⟦▌ Refused"))?;
    ghtui.keys("Delete")?;
    ghtui.wait_for("Queued for GitHub  0")?;
    ghtui.wait_gone("⚠ 1 not sent")?;
    Ok(())
}

fn comments(number: u64) -> Result<Vec<Value>> {
    gh::rest_list(&format!("repos/{}/issues/{number}/comments", gh::REPO))
}

/// How many 👍 `login` gave the issue.
fn thumbs(number: u64, login: &str) -> Result<usize> {
    let reactions = gh::get(&format!("repos/{}/issues/{number}/reactions", gh::REPO))?;
    Ok(reactions.as_array().map_or(0, |r| {
        r.iter()
            .filter(|r| r["content"] == "+1" && r["user"]["login"] == login)
            .count()
    }))
}
