//! Auto-merge on PR #5 (failing checks, a review required, so it never
//! merges): `,m` turns it on, `,m` again turns it off. The repository's
//! "Allow auto-merge" is turned on for the check and back off after.

use std::time::Duration;

use anyhow::{Result, ensure};
use serde_json::{Value, json};

use crate::live::{Check, Cx, gh, until};

pub const CHECK: Check = Check::new(
    "auto-merge",
    ",m turns auto-merge on for PR #5, and ,m turns it off (,u first if its branch is behind)",
    run,
)
.sweep(sweep);

const PR: u64 = 5;
const GITHUB: Duration = Duration::from_secs(45);

fn run(cx: &mut Cx) -> Result<()> {
    let repo = gh::get(&format!("repos/{}", gh::REPO))?;
    if repo["allow_auto_merge"] != true {
        allow_auto_merge(true)?;
        cx.defer("turn the repository's auto-merge back off", || {
            allow_auto_merge(false)
        });
    }
    cx.defer(format!("turn auto-merge off on #{PR}"), disable);

    let ghtui = cx.ghtui(&[&format!("{}#{PR}", gh::REPO)])?;
    ghtui.wait_for("wants to merge into")?;
    if behind_by()? > 0 {
        // The base requires an up-to-date branch, and ghtui offers no
        // merge until it is.
        cx.step(",u brings its branch up to date with main, after asking")?;
        ghtui.keys(", u")?;
        ghtui.wait_for("Merge main into it")?;
        ghtui.keys("Enter")?;
        until("the branch up to date on GitHub", GITHUB, || {
            Ok((behind_by()? == 0).then_some(()))
        })?;
    }
    // Until GitHub knows whether it can merge, ghtui offers merging now.
    until("GitHub to work out its merge state", GITHUB, || {
        Ok((state()?["mergeStateStatus"] == "BLOCKED").then_some(()))
    })?;
    ghtui.keys("r")?;
    ghtui.wait_for("No conflicts with the base branch")?;

    cx.step(",m offers merging when ready, and turns auto-merge on")?;
    ghtui.keys(", m")?;
    ghtui.wait_for("Create a merge commit when ready")?;
    let screen = ghtui.screen()?;
    ensure!(
        screen.matches("Create a merge commit").count()
            == screen.matches("Create a merge commit when ready").count(),
        "it offers merging now too, though checks fail"
    );
    ghtui.keys("Enter")?;
    let login = cx.env.login.clone();
    until("auto-merge on, on GitHub", GITHUB, || {
        let auto = state()?["autoMergeRequest"].clone();
        Ok(
            (auto["mergeMethod"] == "MERGE" && auto["enabledBy"]["login"] == login.as_str())
                .then_some(()),
        )
    })?;
    ghtui.wait_for("Auto-merge enabled by")?;

    cx.step(",m again turns it off, after asking")?;
    ghtui.keys(", m")?;
    ghtui.wait_for("Disable auto-merge for")?;
    ghtui.keys("Enter")?;
    until("auto-merge off, on GitHub", GITHUB, || {
        Ok(state()?["autoMergeRequest"].is_null().then_some(()))
    })?;
    ghtui.wait_gone("Auto-merge enabled by")?;
    Ok(())
}

fn state() -> Result<Value> {
    let data = gh::graphql(
        "query($o: String!, $n: String!, $pr: Int!) { repository(owner: $o, name: $n) {
            pullRequest(number: $pr) { id mergeStateStatus autoMergeRequest { mergeMethod enabledBy { login } } } } }",
        json!({"o": gh::OWNER, "n": gh::NAME, "pr": PR}),
    )?;
    Ok(data["repository"]["pullRequest"].clone())
}

fn behind_by() -> Result<u64> {
    let pr = gh::get(&format!("repos/{}/pulls/{PR}", gh::REPO))?;
    let head = pr["head"]["ref"].as_str().unwrap_or_default().to_owned();
    let compare = gh::get(&format!("repos/{}/compare/main...{head}", gh::REPO))?;
    Ok(compare["behind_by"].as_u64().unwrap_or_default())
}

fn allow_auto_merge(on: bool) -> Result<()> {
    gh::send(
        "PATCH",
        &format!("repos/{}", gh::REPO),
        Some(&json!({"allow_auto_merge": on})),
    )?;
    Ok(())
}

fn disable() -> Result<()> {
    let pr = state()?;
    if pr["autoMergeRequest"].is_null() {
        return Ok(());
    }
    gh::graphql(
        "mutation($id: ID!) { disablePullRequestAutoMerge(input: {pullRequestId: $id}) { clientMutationId } }",
        json!({"id": pr["id"]}),
    )?;
    Ok(())
}

/// The test repository keeps auto-merge off, and #5 without it.
fn sweep() -> Result<()> {
    disable()?;
    if gh::get(&format!("repos/{}", gh::REPO))?["allow_auto_merge"] == true {
        allow_auto_merge(false)?;
    }
    Ok(())
}
