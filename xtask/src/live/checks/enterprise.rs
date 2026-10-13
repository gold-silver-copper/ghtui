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
    // A current server: it says its version, as one does.
    let standin = Arc::new(StandIn::start(
        &cx.env.work.join("ghes"),
        PORT,
        &[],
        Some("3.22.0"),
    )?);
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
    "an older Enterprise Server (3.19, refusing a field it lacks): its version is asked once, its pages load without what it lacks, and ghtui says what that is",
    older,
);

/// The version the stand-in says it is.
const VERSION: &str = "3.19.0";

/// A field 3.19 has, which the stand-in refuses as if it didn't: ghtui
/// has to learn it from the refusal.
const MISSING: &str = "mergeQueueEntry";

/// ghtui asks an older server its version, leaves out what that version
/// lacks, learns what else it lacks from its refusals, and says what the
/// pages go without; the pages load.
fn older(cx: &mut Cx) -> Result<()> {
    let strip = [format!("PullRequest.{MISSING}")];
    let standin = StandIn::start(&cx.env.work.join("ghes"), PORT + 1, &strip, Some(VERSION))?;
    cx.step(&format!(
        "a pull request's page loads from a server refusing {MISSING}, which ghtui names"
    ))?;
    let ghtui = cx.launch(Launch::new(&[&format!("{}#5", gh::REPO)]).env(standin.env()))?;
    ghtui.wait_for("Failing checks")?;
    ghtui.wait_for(&format!("`{MISSING}`: ghtui goes without"))?;
    let served = standin.requests();
    ensure!(
        served.iter().any(|r| r.starts_with("refused /api/graphql")),
        "the stand-in refused no query: {served:?}"
    );
    cx.step("an issue's page loads too, and the version was asked once")?;
    ghtui.keys(":")?;
    ghtui.type_text(&format!("{}#{ISSUE}", gh::REPO))?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Issue to close completed")?;
    let served = standin.requests();
    let asked = served
        .iter()
        .filter(|r| r.starts_with("GET /api/v3/meta "))
        .count();
    ensure!(
        asked == 1,
        "the version was asked {asked} times: {served:?}"
    );
    // Once learned, the field isn't asked again.
    let refused = served.iter().filter(|r| r.starts_with("refused ")).count();
    ensure!(
        refused <= 3,
        "{refused} queries were refused: the field was asked after it was learned"
    );
    Ok(())
}
