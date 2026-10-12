//! GitHub Enterprise Server, end to end, against the stand-in on
//! `https://localhost:8443` ([`crate::ghes`]): pages load through its API,
//! Files changed fetches over git from its clone URL, a comment is written
//! through it, and `y` and `o` give its own links.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, ensure};

use crate::ghes::StandIn;
use crate::live::{Check, Cx, Launch, NARROW, gh, until};

pub const CHECK: Check = Check::new(
    "enterprise",
    "ghtui on a stand-in Enterprise Server: pages, a git fetch through its clone URL, a comment, copied and opened links",
    run,
);

const PORT: u16 = 8443;
const GITHUB: Duration = Duration::from_secs(30);
/// Issue #8 stays open for checks to write on.
const ISSUE: u64 = 8;

fn run(cx: &mut Cx) -> Result<()> {
    let standin = Arc::new(StandIn::start(&cx.env.work.join("ghes"), PORT, &[])?);
    let stop = Arc::clone(&standin);
    cx.defer("stop the stand-in server", move || {
        drop(stop);
        Ok(())
    });
    let host = standin.host.clone();
    let web = format!("https://{host}/{}", gh::REPO);

    cx.step("an issue's page loads through the stand-in's API")?;
    let ghtui = cx.launch(Launch::new(&[&format!("{}#{ISSUE}", gh::REPO)]).env(standin.env()))?;
    ghtui.wait_for("Issue to close completed")?;
    let served = standin.requests();
    ensure!(
        served.iter().any(|r| r.starts_with("POST /api/graphql ")),
        "nothing asked the stand-in's GraphQL API"
    );

    cx.step("c writes a comment through it")?;
    let comment = format!("A comment through the stand-in server {}", cx.env.run_id);
    ghtui.keys("c")?;
    ghtui.wait_for("open in $EDITOR")?;
    ghtui.type_text(&comment)?;
    ghtui.keys("C-s")?;
    let id = until("the comment on GitHub", GITHUB, || {
        let comments = gh::rest_list(&format!(
            "repos/{}/issues/{ISSUE}/comments?since={}",
            gh::REPO,
            gh::iso8601(gh::epoch().saturating_sub(600))
        ))?;
        Ok(comments
            .iter()
            .find(|c| c["body"] == comment.as_str())
            .and_then(|c| c["id"].as_u64()))
    })?;
    cx.defer("delete the comment", move || {
        gh::send(
            "DELETE",
            &format!("repos/{}/issues/comments/{id}", gh::REPO),
            None,
        )?;
        Ok(())
    });
    // Issue #8 is long: the new comment is at its end.
    ghtui.wait_followed()?;
    ghtui.keys("G")?;
    ghtui.wait_for(&comment)?;

    cx.step("y copies the stand-in's link, and o opens it")?;
    ghtui.keys("gg y")?;
    let link = format!("{web}/issues/{ISSUE}");
    ghtui.wait_for(&format!("Copied {link}"))?;
    let before = cx.env.opened().len();
    ghtui.keys("o")?;
    until("the link given to the browser", GITHUB, || {
        Ok(cx
            .opened_since(before)
            .into_iter()
            .find(|u| u.starts_with(&link)))
    })?;

    cx.step("Files changed fetches the pull request over git from the stand-in")?;
    ghtui.keys(":")?;
    ghtui.type_text(&format!("{web}/pull/11/files"))?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("bot-bot-1791523358.txt")?;
    ghtui.wait_for("more")?;
    let served = standin.requests();
    ensure!(
        served
            .iter()
            .any(|r| r.contains(&format!("/{}.git/git-upload-pack 200", gh::REPO))),
        "no git fetch went through the stand-in: {}",
        served
            .iter()
            .filter(|r| r.contains(".git"))
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    );

    cx.step("at 90 columns, the repository's page loads and its crumbs end in …")?;
    let narrow = cx.launch(
        Launch::new(&[&format!("{web}/blob/main/.github/workflows/botreview.yml")])
            .size(NARROW)
            .env(standin.env()),
    )?;
    narrow.wait_for("name: botreview")?;
    narrow.expect_whole_or_cut("gold-silver-copper / ghtui-test-writes")?;
    narrow.expect_whole_or_cut("ghtui-test-writes / .github / workflows / botreview.yml")?;
    Ok(())
}

pub const OLDER: Check = Check::new(
    "enterprise-old",
    "an older Enterprise Server (a field stripped from GraphQL answers): ghtui says what's missing",
    older,
);

/// A field github.com has and an older server's schema may lack.
const MISSING: &str = "mergeQueueEntry";

/// ghtui doesn't work around a field an older server lacks (README, Known
/// limitations): it says which, and offers to try again.
fn older(cx: &mut Cx) -> Result<()> {
    let standin = StandIn::start(&cx.env.work.join("ghes"), PORT + 1, &[MISSING.to_owned()])?;
    cx.step(&format!(
        "a pull request's page, from answers without {MISSING}"
    ))?;
    let ghtui = cx.launch(Launch::new(&[&format!("{}#5", gh::REPO)]).env(standin.env()))?;
    ghtui.wait_for(&format!("`{MISSING}`"))?;
    ghtui.wait_for("missing field")?;
    ghtui.wait_for("r tries again")?;
    Ok(())
}
