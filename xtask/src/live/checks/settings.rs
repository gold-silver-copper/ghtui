//! A repository's Settings tab, on the test repository: a protection rule
//! for `ghtui-live-` branches written in its form and saved, then deleted,
//! and `main`'s rule checked to be as it was. The form is written with
//! `$EDITOR`: a `sed` script the check writes before `ctrl-e`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use crate::live::fux::Pane;
use crate::live::{Check, Cx, Launch, gh, path, until};

pub const CHECK: Check = Check::new(
    "settings",
    ",n protects ghtui-live- branches with a rule written in the form, ,D deletes it; main's rule stays as it was",
    run,
)
.sweep(sweep);

const GITHUB: Duration = Duration::from_secs(30);

pub(super) fn settings_url(page: &str) -> String {
    format!("https://github.com/{}/settings{page}", gh::REPO)
}

/// ghtui with `$EDITOR` applying the sed script at `edits`.
pub(super) fn launch(cx: &mut Cx, url: &str, edits: &Path) -> Result<Pane> {
    let editor = format!("sed -i -f '{}'", path(edits));
    let env = [
        ("EDITOR".to_owned(), editor.clone()),
        ("VISUAL".to_owned(), editor),
    ];
    cx.launch(Launch::new(&[url]).env(env))
}

/// Writes `script` for the next `ctrl-e`, then opens the form in it and
/// asks to save it: the confirmation saying `asks` shows.
pub(super) fn write_form(ghtui: &Pane, edits: &Path, script: &str, asks: &str) -> Result<()> {
    std::fs::write(edits, script)?;
    ghtui.keys("C-e")?;
    ghtui.wait_for("ctrl-s")?;
    ghtui.keys("C-s")?;
    ghtui.wait_for(asks)?;
    Ok(())
}

/// Moves the selection to the row showing `text`.
pub(super) fn select_row(ghtui: &Pane, text: &str) -> Result<()> {
    ghtui.keys("g g")?;
    if selected(&ghtui.screen_marked()?, text) {
        return Ok(());
    }
    ghtui.select("}", text, |s| selected(s, text))
}

fn selected(screen: &str, text: &str) -> bool {
    screen.lines().any(|l| l.contains("⟦▌") && l.contains(text))
}

fn edits_file(cx: &Cx) -> PathBuf {
    cx.env.work.join("settings-edits.sed")
}

const RULES: &str = "query($o: String!, $n: String!) { repository(owner: $o, name: $n) { branchProtectionRules(first: 100) { nodes { id pattern requiresApprovingReviews requiredApprovingReviewCount dismissesStaleReviews requiresCodeOwnerReviews requiresStatusChecks requiresStrictStatusChecks requiredStatusCheckContexts isAdminEnforced requiresLinearHistory allowsForcePushes allowsDeletions requiresConversationResolution lockBranch } } } }";

fn rules() -> Result<Vec<Value>> {
    let data = gh::graphql(RULES, json!({"o": gh::OWNER, "n": gh::NAME}))?;
    let nodes = &data["repository"]["branchProtectionRules"]["nodes"];
    Ok(nodes.as_array().cloned().unwrap_or_default())
}

fn rule(pattern: &str) -> Result<Option<Value>> {
    Ok(rules()?.into_iter().find(|r| r["pattern"] == pattern))
}

fn delete_rule(id: &str) -> Result<()> {
    gh::graphql(
        "mutation($id: ID!) { deleteBranchProtectionRule(input: {branchProtectionRuleId: $id}) { clientMutationId } }",
        json!({"id": id}),
    )
    .map(drop)
}

fn run(cx: &mut Cx) -> Result<()> {
    let main_before = rule("main")?.context("main has no protection rule")?;
    let pattern = format!("{}{}-*", gh::BRANCH_PREFIX, cx.env.run_id);
    let edits = edits_file(cx);
    let ghtui = launch(cx, &settings_url("/branches"), &edits)?;
    ghtui.wait_for("Branch protection rules")?;
    ghtui.wait_followed()?;

    cx.step(",n writes a new rule in the form; it's named before it's made")?;
    ghtui.keys(", n")?;
    ghtui.wait_for("New branch protection rule")?;
    write_form(
        &ghtui,
        &edits,
        &format!(
            "s/^Branch name pattern:.*/Branch name pattern: {pattern}/\ns/^Require linear history:.*/Require linear history: yes/\n"
        ),
        &format!("Protect branches matching {pattern}"),
    )?;
    ghtui.wait_for("Require linear history: no → yes")?;
    ghtui.keys("Enter")?;
    let made = until("the rule on GitHub", GITHUB, || rule(&pattern))?;
    let id = made["id"].as_str().unwrap_or_default().to_owned();
    let undo = id.clone();
    cx.defer(format!("delete the rule for {pattern}"), move || {
        if rules()?.iter().any(|r| r["id"] == undo.as_str()) {
            delete_rule(&undo)?;
        }
        Ok(())
    });
    ensure!(
        made["requiresLinearHistory"] == true,
        "not as written: {made}"
    );
    ghtui.wait_for(&pattern)?;
    ghtui.wait_followed()?;

    cx.step(",D deletes it, after asking")?;
    select_row(&ghtui, &pattern)?;
    ghtui.keys(", D")?;
    ghtui.wait_for(&format!("Delete the protection rule for {pattern}?"))?;
    ghtui.keys("Enter")?;
    until("the rule gone from GitHub", GITHUB, || {
        Ok(rule(&pattern)?.is_none().then_some(()))
    })?;
    ghtui.wait_gone(&pattern)?;
    ghtui.wait_followed()?;

    cx.step("main's protection is as it was")?;
    let main_after = rule("main")?.context("main lost its protection rule")?;
    ensure!(
        main_after == main_before,
        "main's rule changed:\n{main_before}\n{main_after}"
    );
    Ok(())
}

/// Rules for `ghtui-live-` branches a killed run left.
fn sweep() -> Result<()> {
    for r in rules()? {
        let pattern = r["pattern"].as_str().unwrap_or_default();
        if pattern.starts_with(gh::BRANCH_PREFIX)
            && let Some(id) = r["id"].as_str()
        {
            delete_rule(id)?;
        }
    }
    Ok(())
}
