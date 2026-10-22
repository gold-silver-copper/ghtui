//! Following, your status and an issue's type: `:follow` on an
//! organization's profile follows it and unfollows it again, `:set status`
//! sets your status (clearing in 30 minutes, should the cleanup fail) and
//! clears it, and `:edit issue type` sets a new issue's type where the
//! test repository has types. These write to the account, not only the
//! test repository: each puts back what was there, pass or fail.

use std::time::Duration;

use anyhow::{Result, ensure};
use serde_json::{Value, json};

use crate::live::fux::Pane;
use crate::live::{Check, Cx, gh, until};

pub const CHECK: Check = Check::new(
    "people",
    ":follow follows an organization and unfollows it, :set status sets your status and clears it, :edit issue type sets an issue's type",
    run,
);

const GITHUB: Duration = Duration::from_secs(60);

/// Followed and unfollowed: an organization, so nobody is told.
const ORG: &str = "github";

const STATUS: &str =
    "query { viewer { status { emoji message indicatesLimitedAvailability expiresAt } } }";

/// Runs the action named `name` from the palette.
fn palette(ghtui: &Pane, name: &str) -> Result<()> {
    ghtui.keys(":")?;
    ghtui.type_text(name)?;
    ghtui.keys("Enter")?;
    Ok(())
}

fn has(cx: &Cx, scopes: &[&str]) -> bool {
    cx.env.scopes.iter().any(|s| scopes.contains(&s.as_str()))
}

fn run(cx: &mut Cx) -> Result<()> {
    if has(cx, &["user", "user:follow"]) {
        follow(cx)?;
    } else {
        cx.step(
            "following: left out, the token lacks user:follow (gh auth refresh -s user:follow)",
        )?;
    }
    if has(cx, &["user"]) {
        status(cx)?;
    } else {
        cx.step("your status: left out, the token lacks user (gh auth refresh -s user)")?;
    }
    issue_type(cx)
}

/// Whether you follow [`ORG`], as GitHub says.
fn following() -> Result<bool> {
    let data = gh::graphql(
        "query($login: String!) { organization(login: $login) { viewerIsFollowing } }",
        json!({ "login": ORG }),
    )?;
    Ok(data["organization"]["viewerIsFollowing"] == true)
}

fn set_following(on: bool) -> Result<()> {
    let id = gh::graphql(
        "query($login: String!) { organization(login: $login) { id } }",
        json!({ "login": ORG }),
    )?["organization"]["id"]
        .clone();
    let field = if on {
        "followOrganization"
    } else {
        "unfollowOrganization"
    };
    let query = format!(
        "mutation($id: ID!) {{ {field}(input: {{organizationId: $id}}) {{ clientMutationId }} }}"
    );
    gh::graphql(&query, json!({ "id": id }))?;
    Ok(())
}

fn follow(cx: &mut Cx) -> Result<()> {
    let was = following()?;
    cx.defer(format!("put back following @{ORG}"), move || {
        if following()? != was {
            set_following(was)?;
        }
        Ok(())
    });
    let ghtui = cx.ghtui(&[&format!("https://github.com/{ORG}")])?;
    ghtui.wait_for("Organization")?;
    ghtui.wait_followed()?;
    for on in [!was, was] {
        let doing = if on { "follows" } else { "unfollows" };
        cx.step(&format!(":follow {doing} @{ORG}"))?;
        palette(&ghtui, "follow")?;
        until("GitHub to show the follow", GITHUB, || {
            Ok((following()? == on).then_some(()))
        })?;
        let said = if on { "Following @" } else { "Unfollowed @" };
        ghtui.wait_for(&format!("{said}{ORG}"))?;
        ghtui.wait_followed()?;
    }
    Ok(())
}

fn status_now() -> Result<Value> {
    Ok(gh::graphql(STATUS, json!({}))?["viewer"]["status"].clone())
}

fn status(cx: &mut Cx) -> Result<()> {
    let was = status_now()?;
    cx.defer("put back your status", move || {
        let vars = if was.is_null() {
            json!({ "emoji": null, "message": null, "busy": false, "expires": null })
        } else {
            json!({
                "emoji": was["emoji"],
                "message": was["message"],
                "busy": was["indicatesLimitedAvailability"],
                "expires": was["expiresAt"],
            })
        };
        gh::graphql(
            "mutation($emoji: String, $message: String, $busy: Boolean, $expires: DateTime) { changeUserStatus(input: {emoji: $emoji, message: $message, limitedAvailability: $busy, expiresAt: $expires}) { clientMutationId } }",
            vars,
        )?;
        Ok(())
    });
    let login = cx.env.login.clone();
    let ghtui = cx.ghtui(&[&format!("https://github.com/{login}")])?;
    ghtui.wait_for(&login)?;
    ghtui.wait_followed()?;

    cx.step(":set status sets one that clears in 30 minutes")?;
    let message = cx.mark("status");
    palette(&ghtui, "set status")?;
    ghtui.wait_for("Your status")?;
    ghtui.keys("C-u")?;
    ghtui.type_text(&format!("🧪 {message}"))?;
    ghtui.wait_for("clears in 30 minutes")?;
    ghtui.keys("Down Enter")?;
    let set = until("the status on GitHub", GITHUB, || {
        let now = status_now()?;
        Ok((now["message"] == message.as_str()).then_some(now))
    })?;
    ensure!(!set["expiresAt"].is_null(), "it doesn't clear: {set}");
    ensure!(
        set["indicatesLimitedAvailability"] == false,
        "it's busy: {set}"
    );
    ghtui.wait_for("Status set")?;
    ghtui.wait_followed()?;
    ghtui.wait_for(&message)?;

    cx.step(":set status clears it")?;
    palette(&ghtui, "set status")?;
    ghtui.wait_for("Your status")?;
    ghtui.keys("C-u")?;
    ghtui.wait_for("Clear your status")?;
    ghtui.keys("Down Down Enter")?;
    until("the status cleared on GitHub", GITHUB, || {
        let now = status_now()?;
        Ok((now.is_null() || now["message"].is_null()).then_some(()))
    })?;
    ghtui.wait_for("Status cleared")?;
    ghtui.wait_followed()?;
    Ok(())
}

fn issue_type(cx: &mut Cx) -> Result<()> {
    let types = gh::graphql(
        "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { issueTypes(first: 10) { nodes { name isEnabled } } } }",
        json!({ "owner": gh::OWNER, "name": gh::NAME }),
    )?;
    let first = (types["repository"]["issueTypes"]["nodes"].as_array())
        .into_iter()
        .flatten()
        .find(|t| t["isEnabled"] == true)
        .and_then(|t| t["name"].as_str())
        .map(str::to_owned);
    let Some(kind) = first else {
        return cx.step(&format!(
            "issue types: left out, {} has none (they're an organization's)",
            gh::REPO
        ));
    };
    let title = cx.mark("issue type");
    let number = gh::create_issue(&title, "Written by ghtui's live check.")?;
    cx.defer(format!("delete issue #{number}"), move || {
        gh::delete_issue(number)
    });
    let ghtui = cx.ghtui(&[&format!("https://github.com/{}/issues/{number}", gh::REPO)])?;
    ghtui.wait_for(&title)?;
    ghtui.wait_followed()?;

    cx.step(&format!(":edit issue type sets it to {kind}"))?;
    palette(&ghtui, "edit issue type")?;
    ghtui.wait_for("No type")?;
    ghtui.type_text(&kind)?;
    ghtui.keys("Enter")?;
    let query = "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { issue(number: $number) { issueType { name } } } }";
    let vars = json!({ "owner": gh::OWNER, "name": gh::NAME, "number": number });
    until("the type on GitHub", GITHUB, || {
        let now = gh::graphql(query, vars.clone())?;
        Ok((now["repository"]["issue"]["issueType"]["name"] == kind.as_str()).then_some(()))
    })?;
    ghtui.wait_for("Type set")?;
    ghtui.wait_followed()?;
    Ok(())
}
