//! Actions secrets and variables, caches, GitHub Pages and custom
//! properties, on the test repository: a variable made, edited and
//! deleted; a secret set (sealed) and deleted; a Pages site published
//! from `main` and unpublished. Its caches are deleted only when a run
//! left some, and custom properties only on an organization's
//! repository: the check says when it leaves them out. Forms are written
//! with `$EDITOR`, as the settings check does.

use std::time::Duration;

use anyhow::Result;
use serde_json::Value;

use super::settings::{launch, select_row, settings_url, write_form};
use crate::live::{Check, Cx, gh, until};

pub const CHECK: Check = Check::new(
    "secrets",
    "a makes a variable, e edits it and D deletes it; a sets a secret and D deletes it; a publishes a Pages site and D unpublishes it",
    run,
)
.sweep(sweep);

const GITHUB: Duration = Duration::from_secs(30);

/// Names the check makes start with this.
const PREFIX: &str = "GHTUI_LIVE_";

fn variable(name: &str) -> Result<Option<Value>> {
    let path = format!("repos/{}/actions/variables/{name}", gh::REPO);
    Ok(gh::get(&path).ok())
}

fn secret(name: &str) -> Result<Option<Value>> {
    let path = format!("repos/{}/actions/secrets/{name}", gh::REPO);
    Ok(gh::get(&path).ok())
}

fn site() -> Option<Value> {
    gh::get(&format!("repos/{}/pages", gh::REPO)).ok()
}

fn run(cx: &mut Cx) -> Result<()> {
    let name = format!("{PREFIX}{}", cx.env.run_id);
    let edits = cx.env.work.join("secrets-edits.sed");

    cx.step("a makes a variable in its form; e edits it; D deletes it")?;
    let ghtui = launch(cx, &settings_url("/variables/actions"), &edits)?;
    ghtui.wait_for("Repository variables")?;
    ghtui.wait_followed()?;
    ghtui.keys("a")?;
    ghtui.wait_for("New variable")?;
    let script = format!("s/^Name:.*/Name: {name}/\ns/^Value:.*/Value: one/\n");
    write_form(&ghtui, &edits, &script, &format!("Create variable {name}"))?;
    ghtui.keys("Enter")?;
    until("the variable on GitHub", GITHUB, || variable(&name))?;
    let undo = name.clone();
    cx.defer(format!("delete variable {name}"), move || {
        if variable(&undo)?.is_some() {
            let path = format!("repos/{}/actions/variables/{undo}", gh::REPO);
            gh::send("DELETE", &path, None)?;
        }
        Ok(())
    });
    ghtui.wait_for(&name)?;
    ghtui.wait_followed()?;
    select_row(&ghtui, &name)?;
    ghtui.keys("e")?;
    ghtui.wait_for(&format!("The variable {name}"))?;
    write_form(
        &ghtui,
        &edits,
        "s/^Value:.*/Value: two/\n",
        "Value: one → two",
    )?;
    ghtui.keys("Enter")?;
    until("the variable changed", GITHUB, || {
        Ok(variable(&name)?.filter(|v| v["value"] == "two"))
    })?;
    ghtui.wait_followed()?;
    select_row(&ghtui, &name)?;
    ghtui.keys("D")?;
    ghtui.wait_for(&format!("Delete variable {name}"))?;
    ghtui.keys("Enter")?;
    until("the variable gone", GITHUB, || {
        Ok(variable(&name)?.is_none().then_some(()))
    })?;
    ghtui.wait_gone(&name)?;

    cx.step("a sets a secret, sealed; D deletes it")?;
    let ghtui = launch(cx, &settings_url("/secrets/actions"), &edits)?;
    ghtui.wait_for("Repository secrets")?;
    ghtui.wait_followed()?;
    ghtui.keys("a")?;
    ghtui.wait_for("New secret")?;
    let script = format!("s/^Name:.*/Name: {name}/\ns/^Value:.*/Value: not shown/\n");
    write_form(&ghtui, &edits, &script, &format!("Set secret {name}"))?;
    ghtui.keys("Enter")?;
    until("the secret on GitHub", GITHUB, || secret(&name))?;
    let undo = name.clone();
    cx.defer(format!("delete secret {name}"), move || {
        if secret(&undo)?.is_some() {
            let path = format!("repos/{}/actions/secrets/{undo}", gh::REPO);
            gh::send("DELETE", &path, None)?;
        }
        Ok(())
    });
    ghtui.wait_for(&name)?;
    ghtui.wait_followed()?;
    select_row(&ghtui, &name)?;
    ghtui.keys("D")?;
    ghtui.wait_for(&format!("Delete secret {name}"))?;
    ghtui.keys("Enter")?;
    until("the secret gone", GITHUB, || {
        Ok(secret(&name)?.is_none().then_some(()))
    })?;
    ghtui.wait_gone(&name)?;

    cx.step("a publishes a Pages site from main; D unpublishes it")?;
    if site().is_some() {
        cx.step("the test repository has a Pages site of its own: left out")?;
    } else {
        let ghtui = launch(cx, &settings_url("/pages"), &edits)?;
        ghtui.wait_for("No site")?;
        ghtui.wait_followed()?;
        ghtui.keys("a")?;
        ghtui.wait_for("GitHub Pages: branch or actions")?;
        write_form(&ghtui, &edits, "", "Publish a site for")?;
        ghtui.keys("Enter")?;
        until("the site on GitHub", GITHUB, || Ok(site()))?;
        cx.defer("unpublish the Pages site", || {
            if site().is_some() {
                gh::send("DELETE", &format!("repos/{}/pages", gh::REPO), None)?;
            }
            Ok(())
        });
        ghtui.wait_for("Unpublish the site")?;
        ghtui.wait_followed()?;
        ghtui.keys("D")?;
        ghtui.wait_for(&format!("Unpublish {}'s site?", gh::REPO))?;
        ghtui.keys("Enter")?;
        until("the site gone", GITHUB, || {
            Ok(site().is_none().then_some(()))
        })?;
        ghtui.wait_for("No site")?;
    }

    cx.step("caches and custom properties")?;
    let caches = gh::get(&format!("repos/{}/actions/caches", gh::REPO))?;
    if caches["total_count"] == 0 {
        cx.step("no caches to delete: left out (a run that saves one makes them)")?;
    }
    let owner = gh::get(&format!("users/{}", gh::OWNER))?;
    if owner["type"] != "Organization" {
        cx.step("custom properties are an organization's: left out on a user's repository")?;
    }
    Ok(())
}

/// Variables and secrets a killed run left. A Pages site isn't swept:
/// the test repository may have one of its own.
fn sweep() -> Result<()> {
    for kind in ["variables", "secrets"] {
        let list = gh::get(&format!("repos/{}/actions/{kind}", gh::REPO))?;
        for item in list[kind].as_array().into_iter().flatten() {
            let name = item["name"].as_str().unwrap_or_default();
            if name.starts_with(PREFIX) {
                let path = format!("repos/{}/actions/{kind}/{name}", gh::REPO);
                gh::send("DELETE", &path, None)?;
            }
        }
    }
    Ok(())
}
