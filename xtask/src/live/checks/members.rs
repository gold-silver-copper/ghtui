//! An organization's people, and the invitations sent to you. A second
//! account (`GHTUI_LIVE_SECOND_TOKEN`, a classic token of another GitHub
//! account) is the one invited, given a role and removed; ghtui runs as it
//! to answer what it was sent.
//!
//! `invitations`: ghtui as the second account declines an invitation to
//! the test repository on its page and accepts another from the inbox; in
//! an organization you own, ghtui as you invites it by login from the
//! People tab, and ghtui as it joins. `members`, in an organization you
//! own: an invitation by email to a reserved address (nobody is emailed)
//! is sent and cancelled; and with the second account made a member, its
//! role is changed both ways, it's added to a team of the check's, made
//! its maintainer and removed, and it's removed from the organization.
//!
//! Each undoes what it made, pass or fail. Access the second account had
//! before isn't taken away: the check leaves that part out, and says so.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use super::orgs::{delete_team, owned_org, select};
use super::settings::select_row;
use crate::live::fux::Pane;
use crate::live::{Check, Cx, Launch, gh, path, until};

pub const INVITATIONS: Check = Check::new(
    "invitations",
    "as a second account: Decline declines an invitation to the test repository, :answer invitation accepts one in the inbox; in an organization you own, a invites it and :answer invitation joins",
    invitations,
)
.sweep(sweep_invitations);

pub const MEMBERS: Check = Check::new(
    "members",
    "in an organization you own: a invites by email, D cancels it; e changes a member's role both ways; on a team's page a adds them, e makes them maintainer, D removes them; D removes them from the organization",
    members,
)
.sweep(sweep_members);

const GITHUB: Duration = Duration::from_secs(60);

/// Names the second account's token.
const SECOND: &str = "GHTUI_LIVE_SECOND_TOKEN";

/// Invitations by email go to `ghtui-live+…@example.com`: a reserved
/// domain, so nobody is emailed.
const EMAIL_PREFIX: &str = "ghtui-live+";

/// The second account: its login, and its token (never shown).
struct Second {
    login: String,
    token: String,
}

/// The second account, if [`SECOND`] names one.
fn second() -> Result<Option<Second>> {
    let Some(token) = std::env::var(SECOND)
        .ok()
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
    else {
        return Ok(None);
    };
    let user = gh::send_as(&token, "GET", "user", None)
        .with_context(|| format!("GitHub refused {SECOND}"))?;
    let login = user["login"]
        .as_str()
        .with_context(|| format!("GitHub names no account for {SECOND}"))?
        .to_owned();
    if login.eq_ignore_ascii_case(gh::OWNER) {
        bail!("{SECOND} is {login}'s: it takes another account's token");
    }
    Ok(Some(Second { login, token }))
}

fn left_out_second(cx: &Cx, what: &str) -> Result<()> {
    cx.step(&format!(
        "left out: {what} (needs {SECOND}, a classic token of another account)"
    ))
}

fn admin_org(cx: &Cx) -> bool {
    cx.env.scopes.iter().any(|s| s == "admin:org")
}

fn left_out_scope(cx: &Cx, what: &str) -> Result<()> {
    cx.step(&format!(
        "left out: {what} (needs the token's admin:org scope: gh auth refresh -s admin:org)"
    ))
}

/// ghtui as the second account, with a cache of its own.
fn as_second(cx: &mut Cx, who: &Second, url: &str) -> Result<Pane> {
    let home = cx.env.work.join("second");
    let mut env = vec![("GH_TOKEN".to_owned(), who.token.clone())];
    for dir in ["cache", "data", "state"] {
        let at = home.join(dir);
        std::fs::create_dir_all(&at)?;
        env.push((format!("XDG_{}_HOME", dir.to_uppercase()), path(&at)));
    }
    cx.launch(Launch::new(&[url]).env(env))
}

fn people_url(org: &str) -> String {
    format!("https://github.com/orgs/{org}/people")
}

// ---- the test repository's invitations ----------------------------------------------

fn repo_invitations(login: &str) -> Result<Vec<u64>> {
    let all = gh::rest_list(&format!("repos/{}/invitations?per_page=100", gh::REPO))?;
    Ok(all
        .iter()
        .filter(|i| {
            i["invitee"]["login"]
                .as_str()
                .is_some_and(|l| l.eq_ignore_ascii_case(login))
        })
        .filter_map(|i| i["id"].as_u64())
        .collect())
}

fn drop_repo_invitations(login: &str) -> Result<()> {
    for id in repo_invitations(login)? {
        let path = format!("repos/{}/invitations/{id}", gh::REPO);
        gh::send("DELETE", &path, None)?;
    }
    Ok(())
}

fn collaborator(login: &str) -> bool {
    gh::get(&format!("repos/{}/collaborators/{login}", gh::REPO)).is_ok()
}

/// Invites `login` to the test repository, to read it, and waits for
/// GitHub to list the invitation.
fn invite_to_repo(login: &str) -> Result<()> {
    let path = format!("repos/{}/collaborators/{login}", gh::REPO);
    gh::send("PUT", &path, Some(&json!({ "permission": "pull" })))?;
    until("the invitation on GitHub", GITHUB, || {
        Ok((!repo_invitations(login)?.is_empty()).then_some(()))
    })
}

// ---- an organization's people -------------------------------------------------------

fn membership(org: &str, login: &str) -> Option<Value> {
    gh::get(&format!("orgs/{org}/memberships/{login}")).ok()
}

fn in_state(org: &str, login: &str, state: &str) -> Option<Value> {
    membership(org, login).filter(|m| m["state"] == state)
}

/// Takes `login` out of `org`, or cancels its invitation to it.
fn drop_membership(org: &str, login: &str) -> Result<()> {
    if membership(org, login).is_some() {
        gh::send("DELETE", &format!("orgs/{org}/memberships/{login}"), None)?;
    }
    Ok(())
}

fn email_invitation(org: &str, email: &str) -> Result<Option<u64>> {
    let all = gh::rest_list(&format!("orgs/{org}/invitations?per_page=100"))?;
    Ok(all
        .iter()
        .find(|i| {
            i["email"]
                .as_str()
                .is_some_and(|e| e.eq_ignore_ascii_case(email))
        })
        .and_then(|i| i["id"].as_u64()))
}

fn team_role(org: &str, slug: &str, login: &str) -> Option<String> {
    let m = gh::get(&format!("orgs/{org}/teams/{slug}/memberships/{login}")).ok()?;
    m["role"].as_str().map(str::to_owned)
}

// ---- invitations --------------------------------------------------------------------

fn invitations(cx: &mut Cx) -> Result<()> {
    let Some(who) = second()? else {
        return left_out_second(cx, "the invitations sent to you");
    };
    repo_invitation(cx, &who)?;
    org_invitation(cx, &who)
}

fn repo_invitation(cx: &mut Cx, who: &Second) -> Result<()> {
    let login = who.login.clone();
    if collaborator(&login) {
        return cx.step(&format!(
            "left out: invitations to {} (needs @{login} not to be a collaborator already: the check takes away only the access it gave)",
            gh::REPO
        ));
    }
    let undo = login.clone();
    cx.defer(
        format!("take @{login}'s invitations to and access to {}", gh::REPO),
        move || {
            drop_repo_invitations(&undo)?;
            if collaborator(&undo) {
                let path = format!("repos/{}/collaborators/{undo}", gh::REPO);
                gh::send("DELETE", &path, None)?;
            }
            Ok(())
        },
    );
    let join = format!("Join {}?", gh::REPO);

    cx.step(&format!(
        "as @{login}, Decline on the invitation's page declines it, after asking"
    ))?;
    invite_to_repo(&login)?;
    let ghtui = as_second(
        cx,
        who,
        &format!("https://github.com/{}/invitations", gh::REPO),
    )?;
    ghtui.wait_for("Accept invitation")?;
    ghtui.wait_followed()?;
    select_row(&ghtui, "Decline")?;
    ghtui.keys("Enter")?;
    ghtui.wait_for(&join)?;
    ghtui.keys("Enter")?;
    until("the invitation declined on GitHub", GITHUB, || {
        Ok(repo_invitations(&login)?.is_empty().then_some(()))
    })?;
    if collaborator(&login) {
        bail!("@{login} is a collaborator after declining");
    }
    ghtui.wait_for(&format!("Declined the invitation to {}", gh::REPO))?;
    ghtui.wait_followed()?;

    cx.step(&format!(
        "as @{login}, :answer invitation in the inbox accepts another, after asking"
    ))?;
    invite_to_repo(&login)?;
    let ghtui = as_second(cx, who, "https://github.com/notifications")?;
    let row = format!("invited you to collaborate on {}", gh::REPO);
    ghtui.wait_for(&row)?;
    ghtui.wait_followed()?;
    select_row(&ghtui, &row)?;
    ghtui.palette("answer invitation")?;
    ghtui.wait_for(&join)?;
    ghtui.keys("Enter")?;
    until("a collaborator on GitHub", GITHUB, || {
        Ok(collaborator(&login).then_some(()))
    })?;
    ghtui.wait_for(&format!("You're a collaborator on {} now", gh::REPO))?;
    ghtui.wait_followed()?;
    Ok(())
}

fn org_invitation(cx: &mut Cx, who: &Second) -> Result<()> {
    let what = "an organization's invitation";
    let Some(org) = owned_org()? else {
        return cx.step(&format!("left out: {what} (needs an organization you own)"));
    };
    if !admin_org(cx) {
        return left_out_scope(cx, what);
    }
    let login = who.login.clone();
    if in_state(&org, &login, "active").is_some() {
        return cx.step(&format!(
            "left out: {what} (needs @{login} not to be in @{org} already: the check removes only whom it added)"
        ));
    }
    let (undo_org, undo) = (org.clone(), login.clone());
    cx.defer(format!("take @{login} out of @{org}"), move || {
        drop_membership(&undo_org, &undo)
    });

    cx.step(&format!(
        "a on @{org}'s People tab invites @{login} as a member, after asking"
    ))?;
    let ghtui = cx.ghtui(&[&people_url(&org)])?;
    ghtui.wait_for("Pending invitations")?;
    ghtui.wait_followed()?;
    ghtui.keys("a")?;
    ghtui.wait_for("Their GitHub login")?;
    ghtui.type_text(&login)?;
    ghtui.wait_for(&format!("Invite @{login}"))?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Invite them as")?;
    ghtui.type_text("Member")?;
    ghtui.keys("Enter")?;
    ghtui.wait_for(&format!("Invite @{login} to @{org} as a member?"))?;
    ghtui.keys("Enter")?;
    let invited = until("the invitation on GitHub", GITHUB, || {
        Ok(in_state(&org, &login, "pending"))
    })?;
    if invited["role"] != "member" {
        bail!("GitHub has it as {invited}");
    }
    ghtui.wait_followed()?;

    cx.step(&format!(
        "as @{login}, :answer invitation on its page joins @{org}"
    ))?;
    let ghtui = as_second(
        cx,
        who,
        &format!("https://github.com/orgs/{org}/invitation"),
    )?;
    ghtui.wait_for(&format!("@{org} invited you to join"))?;
    ghtui.wait_followed()?;
    ghtui.palette("answer invitation")?;
    ghtui.wait_for(&format!("Join @{org}?"))?;
    ghtui.keys("Enter")?;
    until("the membership active on GitHub", GITHUB, || {
        Ok(in_state(&org, &login, "active"))
    })?;
    ghtui.wait_for(&format!("You're a member of @{org} now"))?;
    ghtui.wait_followed()?;
    Ok(())
}

/// What a killed run left: the second account's invitations, to the test
/// repository and to the organization you own (its access isn't touched:
/// the checks leave out an account that has some).
fn sweep_invitations() -> Result<()> {
    let Some(who) = second().ok().flatten() else {
        return Ok(());
    };
    drop_repo_invitations(&who.login)?;
    if let Some(org) = owned_org()?
        && in_state(&org, &who.login, "pending").is_some()
    {
        drop_membership(&org, &who.login)?;
    }
    Ok(())
}

// ---- members ------------------------------------------------------------------------

fn members(cx: &mut Cx) -> Result<()> {
    let what = "an organization's people";
    let Some(org) = owned_org()? else {
        return cx.step(&format!("left out: {what} (needs an organization you own)"));
    };
    if !admin_org(cx) {
        return left_out_scope(cx, what);
    }
    by_email(cx, &org)?;
    let what = "a member's role, a team's members, removing a member";
    let Some(who) = second()? else {
        return left_out_second(cx, what);
    };
    let login = who.login.clone();
    if in_state(&org, &login, "active").is_some() {
        return cx.step(&format!(
            "left out: {what} (needs @{login} not to be in @{org} already: the check removes only whom it added)"
        ));
    }
    let (undo_org, undo) = (org.clone(), login.clone());
    cx.defer(format!("take @{login} out of @{org}"), move || {
        drop_membership(&undo_org, &undo)
    });
    cx.step(&format!(
        "@{login} made a member of @{org} (gh api: the invitations check presses that)"
    ))?;
    let path = format!("orgs/{org}/memberships/{login}");
    gh::send("PUT", &path, Some(&json!({ "role": "member" })))?;
    let path = format!("user/memberships/orgs/{org}");
    gh::send_as(
        &who.token,
        "PATCH",
        &path,
        Some(&json!({ "state": "active" })),
    )?;
    until("the membership active on GitHub", GITHUB, || {
        Ok(in_state(&org, &login, "active"))
    })?;
    roles(cx, &org, &login)?;
    team_members(cx, &org, &login)?;
    remove(cx, &org, &login)
}

fn by_email(cx: &mut Cx, org: &str) -> Result<()> {
    let email = format!("{EMAIL_PREFIX}{}@example.com", cx.env.run_id);
    let (undo_org, undo) = (org.to_owned(), email.clone());
    cx.defer(format!("cancel the invitation to {email}"), move || {
        if let Some(id) = email_invitation(&undo_org, &undo)? {
            gh::send("DELETE", &format!("orgs/{undo_org}/invitations/{id}"), None)?;
        }
        Ok(())
    });

    cx.step(&format!(
        "a on the People tab invites {email} as a member, after asking (a reserved domain: nobody is emailed); D cancels it"
    ))?;
    let ghtui = cx.ghtui(&[&people_url(org)])?;
    ghtui.wait_for("Pending invitations")?;
    ghtui.wait_followed()?;
    ghtui.keys("a")?;
    ghtui.wait_for("Their GitHub login")?;
    ghtui.type_text(&email)?;
    ghtui.wait_for(&format!("Invite {email} by email"))?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Invite them as")?;
    ghtui.type_text("Member")?;
    ghtui.keys("Enter")?;
    ghtui.wait_for(&format!("Invite {email} to @{org} as a member?"))?;
    ghtui.keys("Enter")?;
    until("the invitation on GitHub", GITHUB, || {
        email_invitation(org, &email)
    })?;
    ghtui.wait_followed()?;
    select(&ghtui, &email)?;
    ghtui.keys("D")?;
    ghtui.wait_for(&format!("Cancel the invitation to {email}?"))?;
    ghtui.keys("Enter")?;
    until("the invitation gone from GitHub", GITHUB, || {
        Ok(email_invitation(org, &email)?.is_none().then_some(()))
    })?;
    ghtui.wait_for(&format!("Cancelled the invitation to {email}"))?;
    ghtui.wait_followed()?;
    Ok(())
}

fn roles(cx: &mut Cx, org: &str, login: &str) -> Result<()> {
    cx.step(&format!(
        "e makes @{login} an owner of @{org}, then a member again, each after asking"
    ))?;
    let ghtui = cx.ghtui(&[&people_url(org)])?;
    ghtui.wait_for("Pending invitations")?;
    ghtui.wait_followed()?;
    for (role, wire, said) in [
        ("Owner", "admin", "an owner"),
        ("Member", "member", "a member"),
    ] {
        select(&ghtui, login)?;
        ghtui.keys("e")?;
        ghtui.wait_for("Change their role to")?;
        ghtui.type_text(role)?;
        ghtui.keys("Enter")?;
        ghtui.wait_for(&format!("Make @{login} {said} of @{org}?"))?;
        ghtui.keys("Enter")?;
        until(&format!("@{login} {said} on GitHub"), GITHUB, || {
            Ok(membership(org, login).filter(|m| m["role"] == wire))
        })?;
        ghtui.wait_for(&format!("@{login} is {said} of @{org}"))?;
        ghtui.wait_followed()?;
    }
    Ok(())
}

fn team_members(cx: &mut Cx, org: &str, login: &str) -> Result<()> {
    let slug = format!("{}{}", gh::BRANCH_PREFIX, cx.env.run_id);
    let (undo_org, undo) = (org.to_owned(), slug.clone());
    cx.defer(format!("delete the team {slug}"), move || {
        delete_team(&undo_org, &undo)
    });
    let made = gh::send(
        "POST",
        &format!("orgs/{org}/teams"),
        Some(&json!({ "name": slug, "privacy": "secret" })),
    )?;
    let slug = made["slug"].as_str().context("no team slug")?.to_owned();
    let team = format!("@{org}/{slug}");
    let role = |want: Option<&'static str>| {
        let (org, slug, login) = (org.to_owned(), slug.clone(), login.to_owned());
        move || -> Result<Option<()>> {
            Ok((team_role(&org, &slug, &login).as_deref() == want).then_some(()))
        }
    };

    cx.step(&format!(
        "on {team}'s page, a adds @{login} as a member, e makes them maintainer, D removes them, each after asking"
    ))?;
    let ghtui = cx.ghtui(&[&format!("https://github.com/orgs/{org}/teams/{slug}")])?;
    ghtui.wait_for("Members")?;
    ghtui.wait_followed()?;
    ghtui.keys("a")?;
    ghtui.wait_for("Add to the team")?;
    ghtui.type_text(login)?;
    ghtui.wait_for(&format!("Add @{login}"))?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Add them as")?;
    ghtui.type_text("Member")?;
    ghtui.keys("Enter")?;
    ghtui.wait_for(&format!("Add @{login} to {team} as a member?"))?;
    ghtui.keys("Enter")?;
    until(
        "a member of the team on GitHub",
        GITHUB,
        role(Some("member")),
    )?;
    ghtui.wait_followed()?;
    select(&ghtui, login)?;
    ghtui.keys("e")?;
    ghtui.wait_for("Change their role to")?;
    ghtui.type_text("Maintainer")?;
    ghtui.keys("Enter")?;
    ghtui.wait_for(&format!("Make @{login} a maintainer of {team}?"))?;
    ghtui.keys("Enter")?;
    until(
        "the team's maintainer on GitHub",
        GITHUB,
        role(Some("maintainer")),
    )?;
    ghtui.wait_followed()?;
    select(&ghtui, login)?;
    ghtui.keys("D")?;
    ghtui.wait_for(&format!("Remove @{login} from {team}?"))?;
    ghtui.keys("Enter")?;
    until("out of the team on GitHub", GITHUB, role(None))?;
    ghtui.wait_for(&format!("Removed @{login} from {team}"))?;
    ghtui.wait_followed()?;
    Ok(())
}

fn remove(cx: &mut Cx, org: &str, login: &str) -> Result<()> {
    cx.step(&format!(
        "D on the People tab removes @{login} from @{org}, after asking"
    ))?;
    let ghtui = cx.ghtui(&[&people_url(org)])?;
    ghtui.wait_for("Pending invitations")?;
    ghtui.wait_followed()?;
    select(&ghtui, login)?;
    ghtui.keys("D")?;
    ghtui.wait_for(&format!("Remove @{login} from @{org}?"))?;
    ghtui.keys("Enter")?;
    until("out of the organization on GitHub", GITHUB, || {
        Ok(membership(org, login).is_none().then_some(()))
    })?;
    ghtui.wait_for(&format!("Removed @{login} from @{org}"))?;
    ghtui.wait_followed()?;
    Ok(())
}

/// Invitations by email a killed run left, in the organization you own.
fn sweep_members() -> Result<()> {
    let Some(org) = owned_org()? else { return Ok(()) };
    let Ok(all) = gh::rest_list(&format!("orgs/{org}/invitations?per_page=100")) else {
        return Ok(());
    };
    for i in all {
        let ours = i["email"]
            .as_str()
            .is_some_and(|e| e.starts_with(EMAIL_PREFIX) && e.ends_with("@example.com"));
        if ours && let Some(id) = i["id"].as_u64() {
            gh::send("DELETE", &format!("orgs/{org}/invitations/{id}"), None)?;
        }
    }
    Ok(())
}
