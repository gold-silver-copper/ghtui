//! What only an organization's repositories have, in an organization you
//! own: a custom property (its definition made with `gh api`, its value
//! set in ghtui's form), and an issue's type (the type and the issue made
//! with `gh api`, the type set and cleared in ghtui). The definition, the
//! type and the issue are deleted after, pass or fail.

use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use super::orgs::owned_org;
use super::settings::{launch, select_row, write_form};
use crate::live::{Check, Cx, gh, until};

pub const PROPERTIES: Check = Check::new(
    "properties",
    "in an organization you own: e on a repository's custom properties sets one in the form",
    properties,
)
.sweep(sweep_properties);

pub const ISSUE_TYPES: Check = Check::new(
    "issue-types",
    "in an organization you own: :edit issue type sets an issue's type, and clears it",
    issue_types,
)
.sweep(sweep_issue_types);

const GITHUB: Duration = Duration::from_secs(60);

/// Custom properties the check makes start with this.
const PROP_PREFIX: &str = "ghtui_live_";

/// A repository of `org`'s, not archived, with issues.
fn org_repo(org: &str) -> Result<Option<String>> {
    let repos = gh::rest_list(&format!("orgs/{org}/repos?type=all&per_page=100"))?;
    Ok(repos
        .into_iter()
        .find(|r| r["archived"] == false && r["has_issues"] == true)
        .and_then(|r| r["name"].as_str().map(str::to_owned)))
}

/// The organization you own and its repository, or why `what` is left out.
fn place(cx: &Cx, what: &str) -> Result<Option<(String, String)>> {
    let Some(org) = owned_org()? else {
        cx.step(&format!("left out: {what} (needs an organization you own)"))?;
        return Ok(None);
    };
    let Some(repo) = org_repo(&org)? else {
        cx.step(&format!(
            "left out: {what} (needs a repository in @{org}, with issues, not archived)"
        ))?;
        return Ok(None);
    };
    Ok(Some((org, repo)))
}

fn property_value(full: &str, name: &str) -> Result<Option<Value>> {
    let values = gh::get(&format!("repos/{full}/properties/values"))?;
    Ok((values.as_array().into_iter().flatten())
        .find(|v| v["property_name"] == name)
        .map(|v| v["value"].clone())
        .filter(|v| !v.is_null()))
}

fn properties(cx: &mut Cx) -> Result<()> {
    let what = "custom properties";
    let Some((org, repo)) = place(cx, what)? else {
        return Ok(());
    };
    let name = format!("{PROP_PREFIX}{}", cx.env.run_id);
    let schema = format!("orgs/{org}/properties/schema/{name}");
    let definition = json!({
        "value_type": "string",
        "values_editable_by": "org_and_repo_actors",
        "description": "Made by ghtui's live check.",
    });
    if let Err(err) = gh::send("PUT", &schema, Some(&definition)) {
        return cx.step(&format!(
            "left out: {what} (needs GitHub to let you define one in @{org}; it said: {err:#})"
        ));
    }
    cx.defer(format!("delete the custom property {name}"), move || {
        gh::send("DELETE", &schema, None).map(drop)
    });
    let full = format!("{org}/{repo}");
    let value = format!("live-{}", cx.env.run_id);
    let edits = cx.env.work.join("properties-edits.sed");

    cx.step(&format!(
        "e sets {name} on {full} in the form, after asking"
    ))?;
    let url = format!("https://github.com/{full}/settings/custom-properties");
    let ghtui = launch(cx, &url, &edits)?;
    ghtui.wait_for(&name)?;
    ghtui.wait_followed()?;
    select_row(&ghtui, &name)?;
    ghtui.keys("e")?;
    ghtui.wait_for("Custom properties: a value after each")?;
    write_form(
        &ghtui,
        &edits,
        &format!("s/^{name}:.*/{name}: {value}/\n"),
        &format!("{name}: (unset) → {value}"),
    )?;
    ghtui.keys("Enter")?;
    until("the value on GitHub", GITHUB, || {
        Ok(property_value(&full, &name)?.filter(|v| *v == value.as_str()))
    })?;
    ghtui.wait_for(&format!("Set {name}"))?;
    ghtui.wait_followed()?;
    ghtui.wait_for(&value)?;
    Ok(())
}

/// Definitions a killed run left (deleting one clears its values).
fn sweep_properties() -> Result<()> {
    let Some(org) = owned_org()? else { return Ok(()) };
    let Ok(all) = gh::get(&format!("orgs/{org}/properties/schema")) else {
        return Ok(());
    };
    for p in all.as_array().into_iter().flatten() {
        if let Some(name) = p["property_name"].as_str()
            && name.starts_with(PROP_PREFIX)
        {
            gh::send(
                "DELETE",
                &format!("orgs/{org}/properties/schema/{name}"),
                None,
            )?;
        }
    }
    Ok(())
}

fn delete_issue(node: &str) -> Result<()> {
    gh::graphql(
        "mutation($id: ID!) { deleteIssue(input: {issueId: $id}) { clientMutationId } }",
        json!({ "id": node }),
    )
    .map(drop)
}

fn issue_types(cx: &mut Cx) -> Result<()> {
    let what = "issue types";
    let Some((org, repo)) = place(cx, what)? else {
        return Ok(());
    };
    let name = format!("{}{}", gh::BRANCH_PREFIX, cx.env.run_id);
    let body = json!({ "name": name, "is_enabled": true });
    let made = match gh::send("POST", &format!("orgs/{org}/issue-types"), Some(&body)) {
        Ok(made) => made,
        Err(err) => {
            return cx.step(&format!(
                "left out: {what} (needs GitHub to let you make one in @{org}; it said: {err:#})"
            ));
        }
    };
    let id = made["id"].as_u64().context("no issue type id")?;
    let undo_org = org.clone();
    cx.defer(format!("delete the issue type {name}"), move || {
        gh::send("DELETE", &format!("orgs/{undo_org}/issue-types/{id}"), None).map(drop)
    });
    let title = cx.mark("issue type");
    let issue = gh::send(
        "POST",
        &format!("repos/{org}/{repo}/issues"),
        Some(&json!({ "title": title, "body": "Written by ghtui's live check." })),
    )?;
    let number = issue["number"].as_u64().context("no issue number")?;
    let node = issue["node_id"].as_str().context("no issue id")?.to_owned();
    cx.defer(
        format!("delete issue #{number} of {org}/{repo}"),
        move || delete_issue(&node),
    );
    let query = "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { issue(number: $number) { issueType { name } } } }";
    let vars = json!({ "owner": org, "name": repo, "number": number });
    let kind = |want: Option<String>| {
        let vars = vars.clone();
        move || -> Result<Option<()>> {
            let now = gh::graphql(query, vars.clone())?;
            let now = now["repository"]["issue"]["issueType"]["name"].as_str();
            Ok((now == want.as_deref()).then_some(()))
        }
    };
    let ghtui = cx.ghtui(&[&format!("https://github.com/{org}/{repo}/issues/{number}")])?;
    ghtui.wait_for(&title)?;
    ghtui.wait_followed()?;

    cx.step(&format!(":edit issue type sets it to {name}"))?;
    ghtui.palette("edit issue type")?;
    ghtui.wait_for("No type")?;
    ghtui.type_text(&name)?;
    ghtui.keys("Enter")?;
    until("the type on GitHub", GITHUB, kind(Some(name.clone())))?;
    ghtui.wait_for("Type set")?;
    ghtui.wait_followed()?;

    cx.step(":edit issue type clears it")?;
    ghtui.palette("edit issue type")?;
    ghtui.wait_for("No type")?;
    ghtui.type_text("No type")?;
    ghtui.keys("Enter")?;
    until("the type cleared on GitHub", GITHUB, kind(None))?;
    ghtui.wait_for("Type cleared")?;
    ghtui.wait_followed()?;
    Ok(())
}

/// Issue types a killed run left, and its issues in the organization's
/// repository.
fn sweep_issue_types() -> Result<()> {
    let Some(org) = owned_org()? else { return Ok(()) };
    if let Ok(all) = gh::get(&format!("orgs/{org}/issue-types")) {
        for t in all.as_array().into_iter().flatten() {
            let ours = t["name"]
                .as_str()
                .is_some_and(|n| n.starts_with(gh::BRANCH_PREFIX));
            if ours && let Some(id) = t["id"].as_u64() {
                gh::send("DELETE", &format!("orgs/{org}/issue-types/{id}"), None)?;
            }
        }
    }
    let Some(repo) = org_repo(&org)? else { return Ok(()) };
    let issues = gh::rest_list(&format!(
        "repos/{org}/{repo}/issues?state=all&creator=@me&per_page=100"
    ))?;
    for i in issues {
        let ours = i["title"]
            .as_str()
            .is_some_and(|t| t.starts_with("[ghtui live]"));
        if ours
            && i.get("pull_request").is_none()
            && let Some(node) = i["node_id"].as_str()
        {
            delete_issue(node)?;
        }
    }
    Ok(())
}
