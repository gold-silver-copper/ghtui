//! A repository's Settings tab, on the test repository only, putting
//! back everything it touches: a protection rule for `ghtui-live-`
//! branches made, edited and deleted (and `main`'s rule checked to be as
//! it was), a ruleset turned on and off, collaborators, a webhook made,
//! redelivered, turned off, edited and deleted, a deploy key added and
//! deleted, and archiving asked twice. The forms are written with
//! `$EDITOR`: a `sed` script the check writes before each `ctrl-e`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use crate::live::fux::Pane;
use crate::live::{Check, Cx, Launch, gh, path, until};

pub const BRANCHES: Check = Check::new(
    "settings-branches",
    ",n protects ghtui-live- branches with a rule written in the form, ,e edits it, ,D deletes it, ,x turns a ruleset on and off; main's rule stays as it was",
    branches,
)
.sweep(sweep_branches);

pub const ACCESS: Check = Check::new(
    "settings-access",
    ",n invites someone with a role (with GHTUI_LIVE_INVITEE), ,e changes it, ,D cancels it; the owner's access can't change",
    access,
);

pub const HOOKS: Check = Check::new(
    "settings-hooks",
    ",n adds a webhook, ,R redelivers its ping, ,x turns it off, ,e changes its events, ,D deletes it",
    hooks,
)
.sweep(sweep_hooks);

pub const KEYS: Check = Check::new(
    "settings-keys",
    ",n adds a read-only deploy key, ,D deletes it",
    keys,
)
.sweep(sweep_keys);

pub const ARCHIVE: Check = Check::new(
    "settings-archive",
    ",x archives a scratch repository after asking twice and unarchives it (with delete_repo)",
    archive,
)
.sweep(sweep_archive);

const GITHUB: Duration = Duration::from_secs(30);

fn settings_url(page: &str) -> String {
    format!("https://github.com/{}/settings{page}", gh::REPO)
}

/// ghtui with `$EDITOR` applying the sed script at `edits`.
fn launch(cx: &mut Cx, url: &str, edits: &Path) -> Result<Pane> {
    let editor = format!("sed -i -f '{}'", path(edits));
    let env = [
        ("EDITOR".to_owned(), editor.clone()),
        ("VISUAL".to_owned(), editor),
    ];
    cx.launch(Launch::new(&[url]).env(env))
}

/// Writes `script` for the next `ctrl-e`, then opens the form in it and
/// asks to save it: the confirmation saying `asks` shows.
fn write_form(ghtui: &Pane, edits: &Path, script: &str, asks: &str) -> Result<()> {
    std::fs::write(edits, script)?;
    ghtui.keys("C-e")?;
    ghtui.wait_for("ctrl-s")?;
    ghtui.keys("C-s")?;
    ghtui.wait_for(asks)?;
    Ok(())
}

/// Moves the selection to the row showing `text`.
fn select_row(ghtui: &Pane, text: &str) -> Result<()> {
    ghtui.keys("g g")?;
    if selected(&ghtui.screen_marked()?, text) {
        return Ok(());
    }
    ghtui.select("}", text, |s| selected(s, text))
}

/// Selects the row showing `row` and presses `keys` until `asks` shows:
/// a page refreshed meanwhile may have moved the selection.
fn ask_on(ghtui: &Pane, row: &str, keys: &str, asks: &str) -> Result<()> {
    for _ in 0..3 {
        select_row(ghtui, row)?;
        ghtui.keys(keys)?;
        if ghtui.wait_for_within(asks, Duration::from_secs(5)).is_ok() {
            return Ok(());
        }
        ghtui.keys("Escape")?;
        ghtui.wait_followed()?;
    }
    ghtui.wait_for(asks).map(drop)
}

fn selected(screen: &str, text: &str) -> bool {
    screen.lines().any(|l| l.contains("⟦▌") && l.contains(text))
}

fn edits_file(cx: &Cx) -> PathBuf {
    cx.env.work.join("settings-edits.sed")
}

// ---- branch protection and rulesets ----------------------------------------------------

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

fn rulesets() -> Result<Vec<Value>> {
    gh::rest_list(&format!("repos/{}/rulesets", gh::REPO))
}

fn ruleset_enforcement(id: u64) -> Result<String> {
    let r = gh::get(&format!("repos/{}/rulesets/{id}", gh::REPO))?;
    Ok(r["enforcement"].as_str().unwrap_or_default().to_owned())
}

fn branches(cx: &mut Cx) -> Result<()> {
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

    cx.step(",e asks for a pull request with 2 approvals; only that is sent")?;
    select_row(&ghtui, &pattern)?;
    ghtui.keys(", e")?;
    ghtui.wait_for("The protection rule for")?;
    write_form(
        &ghtui,
        &edits,
        "s/^Require a pull request before merging:.*/Require a pull request before merging: yes/\ns/^Required approvals:.*/Required approvals: 2/\n",
        "Change the protection rule for",
    )?;
    ghtui.wait_for("Required approvals: 0 → 2")?;
    ghtui.keys("Enter")?;
    until("the rule edited on GitHub", GITHUB, || {
        let r = rule(&pattern)?;
        Ok(r.filter(|r| {
            r["requiresApprovingReviews"] == true
                && r["requiredApprovingReviewCount"] == 2
                && r["requiresLinearHistory"] == true
        }))
    })?;
    ghtui.wait_for("2 approvals")?;
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

    cx.step(",x turns a ruleset on, then off, after asking")?;
    let name = cx.mark("ruleset");
    let made = gh::send(
        "POST",
        &format!("repos/{}/rulesets", gh::REPO),
        Some(&json!({
            "name": name,
            "target": "branch",
            "enforcement": "disabled",
            "conditions": {"ref_name": {"include": [format!("refs/heads/{}*", gh::BRANCH_PREFIX)], "exclude": []}},
            "rules": [{"type": "deletion"}],
        })),
    )?;
    let ruleset = made["id"].as_u64().context("no ruleset id")?;
    cx.defer(format!("delete ruleset {ruleset}"), move || {
        gh::send(
            "DELETE",
            &format!("repos/{}/rulesets/{ruleset}", gh::REPO),
            None,
        )
        .map(drop)
    });
    let ghtui = launch(cx, &settings_url("/branches"), &edits)?;
    ghtui.wait_for(&name)?;
    ghtui.wait_followed()?;
    for (verb, to) in [("Enable", "active"), ("Disable", "disabled")] {
        ask_on(&ghtui, &name, ", x", &format!("{verb} ruleset"))?;
        ghtui.keys("Enter")?;
        until(&format!("the ruleset {to} on GitHub"), GITHUB, || {
            Ok((ruleset_enforcement(ruleset)? == to).then_some(()))
        })?;
        ghtui.wait_followed()?;
    }

    cx.step("main's protection is as it was")?;
    let main_after = rule("main")?.context("main lost its protection rule")?;
    ensure!(
        main_after == main_before,
        "main's rule changed:\n{main_before}\n{main_after}"
    );
    Ok(())
}

/// What a killed run left: rules for `ghtui-live-` branches, and the
/// check's rulesets.
fn sweep_branches() -> Result<()> {
    for r in rules()? {
        let pattern = r["pattern"].as_str().unwrap_or_default();
        if pattern.starts_with(gh::BRANCH_PREFIX)
            && let Some(id) = r["id"].as_str()
        {
            delete_rule(id)?;
        }
    }
    for r in rulesets()? {
        let name = r["name"].as_str().unwrap_or_default();
        if name.starts_with("[ghtui live]")
            && let Some(id) = r["id"].as_u64()
        {
            gh::send("DELETE", &format!("repos/{}/rulesets/{id}", gh::REPO), None)?;
        }
    }
    Ok(())
}

// ---- collaborators ---------------------------------------------------------------------

fn invitation(login: &str) -> Result<Option<Value>> {
    let all = gh::rest_list(&format!("repos/{}/invitations", gh::REPO))?;
    Ok(all.into_iter().find(|i| {
        i["invitee"]["login"]
            .as_str()
            .is_some_and(|l| l.eq_ignore_ascii_case(login))
    }))
}

fn access(cx: &mut Cx) -> Result<()> {
    let edits = edits_file(cx);
    let ghtui = launch(cx, &settings_url("/access"), &edits)?;
    ghtui.wait_for(&format!("@{}", gh::OWNER))?;
    ghtui.wait_followed()?;

    cx.step("the owner's access can't change")?;
    select_row(&ghtui, &format!("@{}", gh::OWNER))?;
    ghtui.keys(", D")?;
    ghtui.wait_for("their access can't change")?;

    let invitee = std::env::var("GHTUI_LIVE_INVITEE")
        .ok()
        .filter(|l| !l.is_empty());
    let Some(invitee) = invitee else {
        cx.step(",n asks a login, then a role, and names both before inviting (not sent)")?;
        ghtui.keys(", n")?;
        ghtui.wait_for("Their GitHub login")?;
        ghtui.type_text("octocat")?;
        ghtui.keys("Enter")?;
        ghtui.wait_for("Choose a role")?;
        ghtui.type_text("read")?;
        ghtui.keys("Enter")?;
        ghtui.wait_for("Invite @octocat to")?;
        ghtui.keys("Escape")?;
        ensure!(invitation("octocat")?.is_none(), "an invitation was sent");
        return Err(Cx::skip(
            "inviting is only asked, not sent: set GHTUI_LIVE_INVITEE to an account that agreed to be invited",
        ));
    };

    cx.step(&format!(",n invites @{invitee} to read"))?;
    let undo = invitee.clone();
    cx.defer(format!("cancel the invitation to @{invitee}"), move || {
        if let Some(i) = invitation(&undo)?
            && let Some(id) = i["id"].as_u64()
        {
            gh::send(
                "DELETE",
                &format!("repos/{}/invitations/{id}", gh::REPO),
                None,
            )?;
        }
        Ok(())
    });
    ghtui.keys(", n")?;
    ghtui.wait_for("Their GitHub login")?;
    ghtui.type_text(&invitee)?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Choose a role")?;
    ghtui.type_text("read")?;
    ghtui.keys("Enter")?;
    ghtui.wait_for(&format!("Invite @{invitee} to"))?;
    ghtui.keys("Enter")?;
    until("the invitation on GitHub", GITHUB, || {
        Ok(invitation(&invitee)?.filter(|i| i["permissions"] == "read"))
    })?;
    ghtui.wait_followed()?;

    cx.step(",e changes the invitation's role to triage")?;
    select_row(&ghtui, &format!("@{invitee}"))?;
    ghtui.keys(", e")?;
    ghtui.wait_for("Choose a role")?;
    ghtui.type_text("triage")?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("with Triage access instead?")?;
    ghtui.keys("Enter")?;
    until("the invitation's new role on GitHub", GITHUB, || {
        Ok(invitation(&invitee)?.filter(|i| i["permissions"] == "triage"))
    })?;
    ghtui.wait_followed()?;

    cx.step(",D cancels it, after asking")?;
    select_row(&ghtui, &format!("@{invitee}"))?;
    ghtui.keys(", D")?;
    ghtui.wait_for(&format!("Cancel the invitation to @{invitee}?"))?;
    ghtui.keys("Enter")?;
    until("the invitation gone from GitHub", GITHUB, || {
        Ok(invitation(&invitee)?.is_none().then_some(()))
    })?;
    Ok(())
}

// ---- webhooks --------------------------------------------------------------------------

fn hook_url(run: &str) -> String {
    format!("https://example.com/ghtui-live/{run}")
}

fn hooks_list() -> Result<Vec<Value>> {
    gh::rest_list(&format!("repos/{}/hooks", gh::REPO))
}

fn hook_with(url: &str) -> Result<Option<Value>> {
    Ok(hooks_list()?
        .into_iter()
        .find(|h| h["config"]["url"] == url))
}

fn deliveries(id: u64) -> Result<Vec<Value>> {
    gh::get(&format!("repos/{}/hooks/{id}/deliveries", gh::REPO))
        .map(|v| v.as_array().cloned().unwrap_or_default())
}

fn hooks(cx: &mut Cx) -> Result<()> {
    if let Err(err) = hooks_list() {
        return Err(Cx::skip(format!(
            "can't list the test repository's webhooks ({err:#}): the token needs admin:repo_hook (`gh auth refresh -s admin:repo_hook`)"
        )));
    }
    let url = hook_url(&cx.env.run_id);
    let edits = edits_file(cx);
    let ghtui = launch(cx, &settings_url("/hooks"), &edits)?;
    ghtui.wait_for("Webhooks")?;
    ghtui.wait_followed()?;

    cx.step(",n adds a webhook from the form")?;
    ghtui.keys(", n")?;
    ghtui.wait_for("New webhook")?;
    write_form(
        &ghtui,
        &edits,
        &format!("s|^Payload URL:.*|Payload URL: {url}|\n"),
        "Add a webhook to",
    )?;
    ghtui.keys("Enter")?;
    let made = until("the webhook on GitHub", GITHUB, || hook_with(&url))?;
    let id = made["id"].as_u64().context("no webhook id")?;
    cx.defer(format!("delete webhook {id}"), move || {
        if hooks_list()?.iter().any(|h| h["id"] == id) {
            gh::send("DELETE", &format!("repos/{}/hooks/{id}", gh::REPO), None)?;
        }
        Ok(())
    });
    ghtui.wait_for(&url)?;
    ghtui.wait_followed()?;

    cx.step("↵ opens it with its deliveries; ,R redelivers GitHub's ping")?;
    until("GitHub's ping delivered", GITHUB, || {
        Ok((!deliveries(id)?.is_empty()).then_some(()))
    })?;
    select_row(&ghtui, &url)?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Recent deliveries")?;
    ghtui.keys("r")?;
    ghtui.wait_for("ping")?;
    ghtui.wait_followed()?;
    select_row(&ghtui, "ping")?;
    ghtui.keys(", R")?;
    ghtui.wait_for("Redeliver the ping delivery?")?;
    ghtui.keys("Enter")?;
    until("the redelivery on GitHub", GITHUB, || {
        Ok(deliveries(id)?
            .iter()
            .any(|d| d["redelivery"] == true)
            .then_some(()))
    })?;
    ghtui.wait_followed()?;

    cx.step(",x turns it off, after asking")?;
    ghtui.keys(", x")?;
    ghtui.wait_for("Disable this webhook?")?;
    ghtui.keys("Enter")?;
    until("the webhook off on GitHub", GITHUB, || {
        Ok(hook_with(&url)?.filter(|h| h["active"] == false))
    })?;
    ghtui.wait_for("Inactive")?;
    ghtui.wait_followed()?;

    cx.step(",e changes its events; only they are sent")?;
    ghtui.keys(", e")?;
    ghtui.wait_for("The webhook:")?;
    write_form(
        &ghtui,
        &edits,
        "s/^Events:.*/Events: push, issues/\n",
        "Change this webhook?",
    )?;
    ghtui.wait_for("Events: push → push, issues")?;
    ghtui.keys("Enter")?;
    until("the webhook's events on GitHub", GITHUB, || {
        Ok(hook_with(&url)?.filter(|h| {
            let mut events: Vec<&str> = (h["events"].as_array().into_iter().flatten())
                .filter_map(Value::as_str)
                .collect();
            events.sort_unstable();
            events == ["issues", "push"] && h["active"] == false
        }))
    })?;
    ghtui.wait_followed()?;

    cx.step(",D deletes it, after asking, and the list shows")?;
    ghtui.keys(", D")?;
    ghtui.wait_for("Delete this webhook?")?;
    ghtui.keys("Enter")?;
    until("the webhook gone from GitHub", GITHUB, || {
        Ok(hook_with(&url)?.is_none().then_some(()))
    })?;
    ghtui.wait_for("Add webhook")?;
    ghtui.wait_gone(&url)?;
    Ok(())
}

fn sweep_hooks() -> Result<()> {
    let Ok(all) = hooks_list() else {
        return Ok(());
    };
    for h in all {
        let url = h["config"]["url"].as_str().unwrap_or_default();
        if url.starts_with("https://example.com/ghtui-live/")
            && let Some(id) = h["id"].as_u64()
        {
            gh::send("DELETE", &format!("repos/{}/hooks/{id}", gh::REPO), None)?;
        }
    }
    Ok(())
}

// ---- deploy keys -----------------------------------------------------------------------

fn deploy_keys() -> Result<Vec<Value>> {
    gh::rest_list(&format!("repos/{}/keys", gh::REPO))
}

fn keys(cx: &mut Cx) -> Result<()> {
    let file = cx.env.work.join("settings-deploy-key");
    let made = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "ghtui-live", "-f"])
        .arg(&file)
        .status();
    if !made.is_ok_and(|s| s.success()) {
        return Err(Cx::skip("ssh-keygen can't make a key"));
    }
    let public = std::fs::read_to_string(file.with_extension("pub"))?;
    let public = public.trim().to_owned();
    let title = format!("{}{}", gh::BRANCH_PREFIX, cx.env.run_id);
    let edits = edits_file(cx);
    let ghtui = launch(cx, &settings_url("/keys"), &edits)?;
    ghtui.wait_for("Deploy keys")?;
    ghtui.wait_followed()?;

    cx.step(",n adds a read-only key from the form")?;
    ghtui.keys(", n")?;
    ghtui.wait_for("New deploy key")?;
    write_form(
        &ghtui,
        &edits,
        &format!("s|^Title:.*|Title: {title}|\ns|^Key:.*|Key: {public}|\n"),
        &format!("Add deploy key “{title}”"),
    )?;
    ghtui.wait_for("Read-only: it can clone, not push")?;
    ghtui.keys("Enter")?;
    let key = until("the key on GitHub", GITHUB, || {
        Ok(deploy_keys()?
            .into_iter()
            .find(|k| k["title"] == title.as_str()))
    })?;
    let id = key["id"].as_u64().context("no key id")?;
    cx.defer(format!("delete deploy key {id}"), move || {
        if deploy_keys()?.iter().any(|k| k["id"] == id) {
            gh::send("DELETE", &format!("repos/{}/keys/{id}", gh::REPO), None)?;
        }
        Ok(())
    });
    ensure!(key["read_only"] == true, "it can write: {key}");
    ghtui.wait_for(&title)?;
    ghtui.wait_followed()?;

    cx.step(",D deletes it, after asking")?;
    select_row(&ghtui, &title)?;
    ghtui.keys(", D")?;
    ghtui.wait_for(&format!("Delete deploy key “{title}”?"))?;
    ghtui.keys("Enter")?;
    until("the key gone from GitHub", GITHUB, || {
        Ok((!deploy_keys()?.iter().any(|k| k["id"] == id)).then_some(()))
    })?;
    ghtui.wait_gone(&title)?;
    Ok(())
}

fn sweep_keys() -> Result<()> {
    for k in deploy_keys()? {
        let title = k["title"].as_str().unwrap_or_default();
        if title.starts_with(gh::BRANCH_PREFIX)
            && let Some(id) = k["id"].as_u64()
        {
            gh::send("DELETE", &format!("repos/{}/keys/{id}", gh::REPO), None)?;
        }
    }
    Ok(())
}

// ---- archiving -------------------------------------------------------------------------

/// Deletes a scratch repository: GitHub refuses (409) while it's still
/// unarchiving it, so it's asked again for a while.
fn delete_repo(repo: &str) -> Result<()> {
    until(&format!("{repo} deleted"), GITHUB, || {
        Ok(gh::send("DELETE", &format!("repos/{repo}"), None)
            .is_ok()
            .then_some(()))
    })
}

/// A killed run's scratch repositories, where the token may delete them.
fn sweep_archive() -> Result<()> {
    let prefix = format!("{}archive-", gh::BRANCH_PREFIX);
    let Ok(repos) = gh::rest_list("user/repos?affiliation=owner") else {
        return Ok(());
    };
    for r in repos {
        let name = r["name"].as_str().unwrap_or_default();
        if name.starts_with(&prefix) {
            // Without delete_repo it stays, named for the next run to try.
            let _ = gh::send("DELETE", &format!("repos/{}/{name}", gh::OWNER), None);
        }
    }
    Ok(())
}

fn archived(repo: &str) -> Result<bool> {
    Ok(gh::get(&format!("repos/{repo}"))?["archived"] == true)
}

/// `,x`, the repository's name, then the last ask: `send` answers it.
fn ask_twice(ghtui: &Pane, repo: &str, verb: &str, send: bool) -> Result<()> {
    ghtui.keys(", x")?;
    ghtui.wait_for("Type its owner/name to confirm")?;
    ghtui.type_text(repo)?;
    ghtui.keys("Enter")?;
    ghtui.wait_for(&format!("{verb} {repo}?"))?;
    ghtui.wait_for("The last ask")?;
    ghtui.keys(if send { "Enter" } else { "Escape" })
}

fn archive(cx: &mut Cx) -> Result<()> {
    if !cx.env.scopes.iter().any(|s| s == "delete_repo") {
        // Archiving the test repository would make it read-only under
        // the other checks: only both asks are run on it, and cancelled.
        let edits = edits_file(cx);
        let ghtui = launch(cx, &settings_url(""), &edits)?;
        ghtui.wait_for("Archive this repository")?;
        ghtui.wait_followed()?;
        cx.step(",x asks for the name, then once more; cancelled")?;
        ask_twice(&ghtui, gh::REPO, "Archive", false)?;
        ensure!(!archived(gh::REPO)?, "the test repository is archived");
        return Err(Cx::skip(
            "archiving itself runs on a scratch repository the check deletes: that takes delete_repo (`gh auth refresh -s delete_repo`)",
        ));
    }
    let name = format!("{}archive-{}", gh::BRANCH_PREFIX, cx.env.run_id);
    let repo = format!("{}/{name}", gh::OWNER);
    gh::send(
        "POST",
        "user/repos",
        Some(&json!({"name": name, "private": true, "auto_init": true})),
    )?;
    let undo = repo.clone();
    cx.defer(format!("delete {repo}"), move || delete_repo(&undo));
    let edits = edits_file(cx);
    let url = format!("https://github.com/{repo}/settings");
    let ghtui = launch(cx, &url, &edits)?;
    ghtui.wait_for("Archive this repository")?;
    ghtui.wait_followed()?;

    cx.step(",x archives it, asking for its name and once more")?;
    ask_twice(&ghtui, &repo, "Archive", true)?;
    until("it archived on GitHub", GITHUB, || {
        Ok(archived(&repo)?.then_some(()))
    })?;
    ghtui.wait_for("Unarchive this repository")?;
    ghtui.wait_followed()?;

    cx.step(",x unarchives it, the same way")?;
    ask_twice(&ghtui, &repo, "Unarchive", true)?;
    until("it unarchived on GitHub", GITHUB, || {
        Ok((!archived(&repo)?).then_some(()))
    })?;
    ghtui.wait_for("Archive this repository")?;
    Ok(())
}
