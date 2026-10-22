//! A repository's draft security advisory, written in GitHub's form from
//! the Security tab and closed after (GitHub's API can't delete one); and
//! an organization's team, made from its Teams page, given a repository
//! and its role changed and taken away, then deleted, when the account
//! owns an organization. Inviting someone (it emails a real person),
//! changing a member's role and answering an invitation sent to you
//! (that takes a second account to send it) are left to the tests.

use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

use crate::live::fux::Pane;
use crate::live::{Check, Cx, gh, say, until};

pub const ADVISORY: Check = Check::new(
    "advisory",
    ":new here on the Security tab writes a draft advisory in GitHub's form and ctrl-s makes it; it's closed after",
    advisory,
)
.sweep(sweep_advisories);

pub const ORGS: Check = Check::new(
    "orgs",
    "in an organization you own: a makes a team, a gives it a repository, e changes its role, D takes it away; the team is deleted after",
    orgs,
)
.sweep(sweep_teams);

const GITHUB: Duration = Duration::from_secs(60);

/// Moves the selection from the top down to the row that starts with
/// `name`, on the marked screen.
fn select(ghtui: &Pane, name: &str) -> Result<()> {
    let at = |marked: &str| {
        marked.lines().any(|l| {
            l.split_once("⟦▌")
                .is_some_and(|(_, rest)| rest.trim_start().starts_with(name))
        })
    };
    ghtui.keys("Home")?;
    for _ in 0..60 {
        if at(&ghtui.screen_marked()?) {
            return Ok(());
        }
        ghtui.keys("Down")?;
    }
    bail!("no row starts with {name}")
}

/// Sets the form's row `name` in its picker to `value`.
fn fill(ghtui: &Pane, name: &str, value: &str) -> Result<()> {
    select(ghtui, name)?;
    ghtui.keys("Enter")?;
    ghtui.keys("C-u")?;
    ghtui.type_text(value)?;
    ghtui.keys("Enter")?;
    Ok(())
}

fn advisories() -> Result<Vec<Value>> {
    gh::rest_list(&format!(
        "repos/{}/security-advisories?state=draft&per_page=100",
        gh::REPO
    ))
}

fn close_advisory(ghsa: &str) -> Result<()> {
    let path = format!("repos/{}/security-advisories/{ghsa}", gh::REPO);
    gh::send("PATCH", &path, Some(&json!({ "state": "closed" }))).map(drop)
}

fn advisory(cx: &mut Cx) -> Result<()> {
    let summary = cx.mark("advisory");
    let undo = summary.clone();
    cx.defer("close the draft advisory", move || {
        for a in advisories()? {
            if a["summary"] == undo.as_str()
                && let Some(ghsa) = a["ghsa_id"].as_str()
            {
                close_advisory(ghsa)?;
            }
        }
        Ok(())
    });
    let url = format!("https://github.com/{}/security/advisories", gh::REPO);
    let ghtui = cx.ghtui(&[&url])?;
    ghtui.wait_for("Security advisories")?;
    ghtui.wait_followed()?;

    cx.step("New draft security advisory opens GitHub's form, its title first")?;
    ghtui.palette("new here")?;
    ghtui.wait_for("New draft security advisory in")?;
    ghtui.wait_for("Type the title")?;
    ghtui.type_text(&summary)?;
    ghtui.keys("Enter")?;
    fill(&ghtui, "Ecosystem", "Other")?;
    fill(&ghtui, "Package", "ghtui-live")?;
    fill(&ghtui, "Affected", "< 1.0.0")?;
    fill(&ghtui, "Severity", "Low")?;
    fill(&ghtui, "Weaknesses", "CWE-20")?;

    cx.step("its description in the composer; ctrl-s there makes the draft, and its page opens")?;
    ghtui.keys("c")?;
    ghtui.type_text("Written by ghtui's live check.")?;
    ghtui.keys("C-s")?;
    let made = until("the draft on GitHub", GITHUB, || {
        Ok(advisories()?
            .into_iter()
            .find(|a| a["summary"] == summary.as_str()))
    })?;
    let package = &made["vulnerabilities"][0];
    ensure!(
        made["severity"] == "low"
            && package["package"]["ecosystem"] == "other"
            && package["package"]["name"] == "ghtui-live"
            && package["vulnerable_version_range"] == "< 1.0.0"
            && made["description"] == "Written by ghtui's live check."
            && made["cwe_ids"] == json!(["CWE-20"]),
        "GitHub has it as {made}"
    );
    let ghsa = made["ghsa_id"].as_str().context("no GHSA ID")?;
    ghtui.wait_for(&format!("Drafted {ghsa}"))?;
    ghtui.wait_for("Draft")?;
    Ok(())
}

/// Drafts a killed run left: closed.
fn sweep_advisories() -> Result<()> {
    for a in advisories()? {
        let ours = a["summary"]
            .as_str()
            .is_some_and(|s| s.starts_with("[ghtui live]"));
        if ours && let Some(ghsa) = a["ghsa_id"].as_str() {
            close_advisory(ghsa)?;
        }
    }
    Ok(())
}

/// An organization the account owns, if it owns one.
fn owned_org() -> Result<Option<String>> {
    let orgs = gh::rest_list("user/memberships/orgs?state=active&per_page=100")?;
    Ok(orgs.into_iter().find_map(|m| {
        (m["role"] == "admin").then(|| m["organization"]["login"].as_str().map(str::to_owned))?
    }))
}

fn team(org: &str, slug: &str) -> Result<Option<Value>> {
    Ok(gh::get(&format!("orgs/{org}/teams/{slug}")).ok())
}

fn delete_team(org: &str, slug: &str) -> Result<()> {
    if team(org, slug)?.is_some() {
        gh::send("DELETE", &format!("orgs/{org}/teams/{slug}"), None)?;
    }
    Ok(())
}

fn orgs(cx: &mut Cx) -> Result<()> {
    let Some(org) = owned_org()? else {
        say(
            "    · left out: the account owns no organization, so its people and teams aren't changed",
        );
        return Ok(());
    };
    let repos = gh::rest_list(&format!("orgs/{org}/repos?per_page=1"))?;
    let Some(repo) = repos
        .first()
        .and_then(|r| r["name"].as_str())
        .map(str::to_owned)
    else {
        say(&format!(
            "    · left out: @{org} has no repository to give a team"
        ));
        return Ok(());
    };
    let name = format!("{}{}", gh::BRANCH_PREFIX, cx.env.run_id);
    let (undo_org, undo_slug) = (org.clone(), name.clone());
    cx.defer(format!("delete the team {name}"), move || {
        delete_team(&undo_org, &undo_slug)
    });
    let ghtui = cx.ghtui(&[&format!("https://github.com/orgs/{org}/teams")])?;
    ghtui.wait_for("Teams")?;
    ghtui.wait_followed()?;

    cx.step("a makes a secret team: its name, then who can see it, after asking")?;
    ghtui.keys("a")?;
    ghtui.wait_for("The team's name")?;
    ghtui.type_text(&name)?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Secret")?;
    ghtui.type_text("Secret")?;
    ghtui.keys("Enter")?;
    ghtui.wait_for(&format!("Make the team “{name}” in @{org}?"))?;
    ghtui.keys("Enter")?;
    let made = until("the team on GitHub", GITHUB, || team(&org, &name))?;
    ensure!(made["privacy"] == "secret", "GitHub has it as {made}");
    ghtui.wait_followed()?;

    cx.step("on its page, a gives it a repository with a role, e changes it, D takes it away")?;
    let ghtui = cx.ghtui(&[&format!("https://github.com/orgs/{org}/teams/{name}")])?;
    ghtui.wait_for("Repositories")?;
    ghtui.wait_followed()?;
    ghtui.keys("a")?;
    ghtui.type_text(&format!("{org}/{repo}"))?;
    ghtui.wait_for(&format!("Give it access to {org}/{repo}"))?;
    ghtui.keys("Enter")?;
    ghtui.type_text("Triage")?;
    ghtui.keys("Enter")?;
    ghtui.keys("Enter")?;
    let role = |want: &'static str| {
        let (org, name, repo) = (org.clone(), name.clone(), repo.clone());
        move || -> Result<Option<()>> {
            let repos = gh::rest_list(&format!("orgs/{org}/teams/{name}/repos"))?;
            let found = repos.iter().find(|r| r["name"] == repo.as_str());
            Ok(match (found, want) {
                (None, "gone") => Some(()),
                (Some(r), want) => (r["role_name"] == want).then_some(()),
                (None, _) => None,
            })
        }
    };
    until(
        "the repository the team's, with triage",
        GITHUB,
        role("triage"),
    )?;
    ghtui.wait_followed()?;
    select(&ghtui, &format!("{org}/{repo}"))?;
    ghtui.keys("e")?;
    ghtui.type_text("Write")?;
    ghtui.keys("Enter")?;
    ghtui.keys("Enter")?;
    until("the team's role there, write", GITHUB, role("write"))?;
    ghtui.wait_followed()?;
    select(&ghtui, &format!("{org}/{repo}"))?;
    ghtui.keys("D")?;
    ghtui.wait_for(&format!("Remove {org}/{repo} from @{org}/{name}?"))?;
    ghtui.keys("Enter")?;
    until("the repository no longer the team's", GITHUB, role("gone"))?;
    ghtui.wait_followed()?;
    Ok(())
}

/// Teams a killed run left, in the organization the account owns.
fn sweep_teams() -> Result<()> {
    let Some(org) = owned_org()? else { return Ok(()) };
    for t in gh::rest_list(&format!("orgs/{org}/teams?per_page=100"))? {
        if let Some(slug) = t["slug"].as_str()
            && slug.starts_with(gh::BRANCH_PREFIX)
        {
            delete_team(&org, slug)?;
        }
    }
    Ok(())
}
