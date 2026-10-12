//! Writes that used to need the browser: a release (`,V`, a new tag made
//! when it's published), a secret gist (`,G`, then `,e` and `,D`), and
//! how you watch the repository and a thread (`,w`).

use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

use crate::live::{Check, Cx, gh, until};

pub const RELEASE: Check = Check::new(
    "release",
    ",V makes a release with a new tag, published; it and its tag are deleted after",
    release,
)
.sweep(sweep_releases);

pub const GIST: Check = Check::new(
    "gist",
    ",G makes a secret gist (description, a named file), ,e edits its file, ,D deletes it",
    gist,
)
.sweep(sweep_gists);

pub const WATCH: Check = Check::new(
    "watch",
    ",w watches all of the repository's activity, then back to participating; the setting is put back after",
    watch,
);

pub const SUBSCRIBE: Check = Check::new(
    "subscribe",
    ",w on an issue subscribes or unsubscribes; without the token's notifications scope, it says so",
    subscribe,
);

const GITHUB: Duration = Duration::from_secs(30);

fn release(cx: &mut Cx) -> Result<()> {
    let tag = format!("{}{}", gh::BRANCH_PREFIX, cx.env.run_id);
    let title = cx.mark("release");
    let undo = tag.clone();
    cx.defer(format!("delete the release {tag} and its tag"), move || {
        delete_release(&undo)
    });
    let ghtui = cx.ghtui(&[gh::REPO])?;
    ghtui.wait_for("Pull requests")?;
    ghtui.wait_followed()?;

    cx.step(",V starts with a tag: a new one, made when it's published")?;
    ghtui.keys(", V")?;
    ghtui.wait_for("Choose a tag")?;
    ghtui.type_text(&tag)?;
    ghtui.wait_for(&format!("Create new tag: {tag}"))?;
    ghtui.keys("Enter")?;
    ghtui.wait_for(&format!("{tag} is a new tag"))?;

    cx.step("the composer takes its title and notes; ctrl-s publishes it, after asking")?;
    ghtui.wait_for("its title on the first line")?;
    ghtui.type_text(&title)?;
    ghtui.keys("Enter")?;
    ghtui.type_text("Notes from the live check.")?;
    ghtui.keys("C-s")?;
    ghtui.wait_for(&format!("Publish {tag} in {}?", gh::REPO))?;
    ghtui.keys("Enter")?;
    let made = until("the release on GitHub", GITHUB, || {
        Ok(gh::get(&format!("repos/{}/releases/tags/{tag}", gh::REPO)).ok())
    })?;
    ensure!(
        made["name"] == title.as_str()
            && made["body"] == "Notes from the live check."
            && made["draft"] == false,
        "GitHub has the release as {} / {} / draft {}",
        made["name"],
        made["body"],
        made["draft"]
    );
    let main = gh::get(&format!("repos/{}/git/ref/heads/main", gh::REPO))?;
    let at = gh::get(&format!("repos/{}/git/ref/tags/{tag}", gh::REPO))?;
    ensure!(
        at["object"]["sha"] == main["object"]["sha"],
        "the tag isn't at main"
    );
    ghtui.wait_for(&title)?;
    ghtui.wait_for("released")?;
    Ok(())
}

fn delete_release(tag: &str) -> Result<()> {
    if let Ok(release) = gh::get(&format!("repos/{}/releases/tags/{tag}", gh::REPO))
        && let Some(id) = release["id"].as_u64()
    {
        gh::send("DELETE", &format!("repos/{}/releases/{id}", gh::REPO), None)?;
    }
    if gh::get(&format!("repos/{}/git/ref/tags/{tag}", gh::REPO)).is_ok() {
        gh::send(
            "DELETE",
            &format!("repos/{}/git/refs/tags/{tag}", gh::REPO),
            None,
        )?;
    }
    Ok(())
}

/// Releases and tags a killed run left.
fn sweep_releases() -> Result<()> {
    for release in gh::rest_list(&format!("repos/{}/releases?per_page=100", gh::REPO))? {
        if let Some(tag) = release["tag_name"].as_str()
            && tag.starts_with(gh::BRANCH_PREFIX)
        {
            delete_release(tag)?;
        }
    }
    for tag in gh::rest_list(&format!("repos/{}/tags?per_page=100", gh::REPO))? {
        if let Some(name) = tag["name"].as_str()
            && name.starts_with(gh::BRANCH_PREFIX)
        {
            delete_release(name)?;
        }
    }
    Ok(())
}

fn gist(cx: &mut Cx) -> Result<()> {
    let description = cx.mark("gist");
    let text = format!("A gist from the live check {}", cx.env.run_id);
    let file = "live.md";
    let undo = description.clone();
    cx.defer("delete the gist", move || {
        for id in gists_named(&undo)? {
            delete_gist(&id)?;
        }
        Ok(())
    });
    let ghtui = cx.ghtui(&[gh::REPO])?;
    ghtui.wait_for("Pull requests")?;
    ghtui.wait_followed()?;

    cx.step(",G writes a gist's file in the composer, then its description and name")?;
    ghtui.keys(", G")?;
    ghtui.wait_for("Gist file 1")?;
    ghtui.type_text(&text)?;
    ghtui.keys("Escape")?;
    ghtui.wait_for("Create gist…")?;
    ghtui.select("gg", "the description", |s| s.contains("⟦▌ Description"))?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Gist description")?;
    ghtui.type_text(&description)?;
    ghtui.keys("Enter")?;
    ghtui.wait_for(&format!("Description {description}"))?;
    // The file's name is its box's title, a link: `i` letters it.
    ghtui.keys("i")?;
    let screen = ghtui.screen_marked()?;
    let letter = screen
        .find("⟧Filename including extension…")
        .and_then(|at| screen.get(..at))
        .and_then(|before| before.chars().last())
        .filter(char::is_ascii_lowercase)
        .context("the file's name has no letter to follow")?;
    ghtui.keys(&letter.to_string())?;
    ghtui.wait_for("Rename the file")?;
    ghtui.type_text(file)?;
    ghtui.keys("Enter")?;
    ghtui.wait_gone("Filename including extension…")?;

    cx.step(",s creates it secret, after asking")?;
    ghtui.keys(", s")?;
    ghtui.wait_for("Create the gist?")?;
    ghtui.wait_for("Create secret gist")?;
    ghtui.keys("Enter")?;
    let id = until("the gist on GitHub", GITHUB, || {
        Ok(gists_named(&description)?.into_iter().next())
    })?;
    let made = gh::get(&format!("gists/{id}"))?;
    ensure!(made["public"] == false, "the gist is public");
    ensure!(
        made["files"][file]["content"] == text.as_str(),
        "its {file} is {}",
        made["files"][file]["content"]
    );
    ghtui.wait_for(&format!("{} / {file}", cx.env.login))?;
    ghtui.wait_for("Secret")?;

    ghtui.wait_followed()?;
    cx.step(",e edits its file")?;
    ghtui.keys(", e")?;
    ghtui.wait_for("Editing gist")?;
    ghtui.select("j", "its file", |s| s.contains(&format!("⟦▌{text}")))?;
    ghtui.keys("Enter")?;
    ghtui.wait_for(&format!("Gist file {file}"))?;
    ghtui.type_text(" (edited)")?;
    ghtui.keys("C-s")?;
    let edited = format!("{text} (edited)");
    until("the edited file on GitHub", GITHUB, || {
        let gist = gh::get(&format!("gists/{id}"))?;
        Ok((gist["files"][file]["content"] == edited.as_str()).then_some(()))
    })?;
    ghtui.wait_for(&edited)?;

    ghtui.wait_followed()?;
    cx.step(",D deletes it, after asking")?;
    ghtui.keys(", D")?;
    ghtui.wait_for("Delete this gist?")?;
    ghtui.keys("Enter")?;
    until("the gist gone from GitHub", GITHUB, || {
        Ok(gh::get(&format!("gists/{id}")).is_err().then_some(()))
    })?;
    ghtui.wait_gone(&edited)?;
    Ok(())
}

/// Your gists described `description`.
fn gists_named(description: &str) -> Result<Vec<String>> {
    let gists = gh::get("gists?per_page=100")?;
    Ok(gists
        .as_array()
        .map(|g| {
            g.iter()
                .filter(|g| g["description"] == description)
                .filter_map(|g| g["id"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default())
}

fn delete_gist(id: &str) -> Result<()> {
    gh::send("DELETE", &format!("gists/{id}"), None)?;
    Ok(())
}

/// Gists a killed run left.
fn sweep_gists() -> Result<()> {
    let gists = gh::get("gists?per_page=100")?;
    for g in gists.as_array().cloned().unwrap_or_default() {
        if g["description"]
            .as_str()
            .is_some_and(|d| d.starts_with("[ghtui live]"))
            && let Some(id) = g["id"].as_str()
        {
            delete_gist(id)?;
        }
    }
    Ok(())
}

fn watch(cx: &mut Cx) -> Result<()> {
    let before = repo_subscription()?;
    cx.defer("put the repository's watch setting back", move || {
        restore_repo_subscription(before.as_ref())
    });
    let ghtui = cx.ghtui(&[gh::REPO])?;
    ghtui.wait_for("Pull requests")?;
    ghtui.wait_followed()?;

    cx.step(",w watches all of the repository's activity")?;
    ghtui.keys(", w")?;
    ghtui.wait_for("Watch the repository")?;
    ghtui.select("Down", "All Activity", |s| s.contains("⟦▌ All Activity"))?;
    ghtui.keys("Enter")?;
    until("watching on GitHub", GITHUB, || {
        Ok(repo_subscription()?
            .is_some_and(|s| s["subscribed"] == true && s["ignored"] == false)
            .then_some(()))
    })?;
    ghtui.wait_for(&format!("Watching all of {}'s activity", gh::REPO))?;

    ghtui.wait_followed()?;
    cx.step(",w again: participating and @mentions, the current choice marked")?;
    ghtui.keys(", w")?;
    ghtui.wait_for("Watch the repository")?;
    ghtui.wait_until("All Activity marked current", GITHUB, |s| {
        s.lines()
            .any(|l| l.contains("All Activity") && l.contains("current"))
            .then_some(())
    })?;
    ghtui.select("Up", "Participating", |s| {
        s.contains("⟦▌ Participating and @mentions")
    })?;
    ghtui.keys("Enter")?;
    until("not watching on GitHub", GITHUB, || {
        Ok(repo_subscription()?.is_none().then_some(()))
    })?;
    ghtui.wait_for("when you take part or are mentioned")?;
    Ok(())
}

fn subscribe(cx: &mut Cx) -> Result<()> {
    cx.step(",w on issue #8 subscribes to it, or says the scope it takes")?;
    let issue = 8;
    let was = thread_subscription(issue)?;
    let ghtui = cx.ghtui(&[&format!("{}#{issue}", gh::REPO)])?;
    ghtui.wait_for("Issue to close completed")?;
    ghtui.wait_followed()?;
    ghtui.keys(", w")?;
    ghtui.wait_for("Notifications")?;
    // The other of subscribed and unsubscribed.
    let (row, want) = if was == "SUBSCRIBED" {
        ("Unsubscribe", "UNSUBSCRIBED")
    } else {
        ("Subscribe", "SUBSCRIBED")
    };
    let on_row = |s: &str| s.contains(&format!("⟦▌ {row} "));
    if !on_row(&ghtui.screen_marked()?) {
        ghtui.select("Down", row, on_row)?;
    }
    ghtui.keys("Enter")?;
    if !cx.env.scopes.iter().any(|s| s == "notifications") {
        ghtui
            .wait_for("takes your token's notifications scope: gh auth refresh -s notifications")?;
        ensure!(
            thread_subscription(issue)? == was,
            "the subscription changed without the notifications scope"
        );
        return Err(Cx::skip(
            "a thread's subscription takes the token's notifications scope (gh auth refresh -s notifications), and ghtui said so",
        ));
    }
    let restore = was.clone();
    cx.defer(format!("put #{issue}'s subscription back"), move || {
        set_thread_subscription(issue, &restore)
    });
    until("the subscription on GitHub", GITHUB, || {
        Ok((thread_subscription(issue)? == want).then_some(()))
    })?;
    Ok(())
}

/// Your watch setting for the repository: `None` is participating and
/// @mentions (GitHub answers 404).
fn repo_subscription() -> Result<Option<Value>> {
    match gh::get(&format!("repos/{}/subscription", gh::REPO)) {
        Ok(s) => Ok(Some(s)),
        Err(err) if format!("{err:#}").contains("Not Found") => Ok(None),
        Err(err) => Err(err),
    }
}

fn restore_repo_subscription(before: Option<&Value>) -> Result<()> {
    let path = format!("repos/{}/subscription", gh::REPO);
    match before {
        None => {
            if repo_subscription()?.is_some() {
                gh::send("DELETE", &path, None)?;
            }
        }
        Some(s) => {
            let body = json!({"subscribed": s["subscribed"], "ignored": s["ignored"]});
            gh::send("PUT", &path, Some(&body))?;
        }
    }
    Ok(())
}

/// Your subscription to an issue: SUBSCRIBED, UNSUBSCRIBED or IGNORED.
fn thread_subscription(issue: u64) -> Result<String> {
    let data = gh::graphql(
        "query($o: String!, $n: String!, $i: Int!) { repository(owner: $o, name: $n) {
            issue(number: $i) { viewerSubscription } } }",
        json!({"o": gh::OWNER, "n": gh::NAME, "i": issue}),
    )?;
    data["repository"]["issue"]["viewerSubscription"]
        .as_str()
        .map(str::to_owned)
        .context("GitHub doesn't say your subscription")
}

fn set_thread_subscription(issue: u64, state: &str) -> Result<()> {
    let id = gh::issue(issue)?["node_id"].clone();
    let done = gh::graphql(
        "mutation($id: ID!, $state: SubscriptionState!) {
            updateSubscription(input: {subscribableId: $id, state: $state}) { subscribable { viewerSubscription } } }",
        json!({"id": id, "state": state}),
    )?;
    if done["updateSubscription"]["subscribable"]["viewerSubscription"] != state {
        bail!("GitHub didn't put the subscription back to {state}");
    }
    Ok(())
}
