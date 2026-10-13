//! Repository housekeeping: a label (`,o`, `,e`, `,D` on the labels
//! page), a milestone (`,o`, `,e`, `,x`, `,D`), branches and tags (`,o`
//! and `,D`, by GitHub's API and with git in a clone), files (`,e`, `,o`,
//! `,D`, committed by GitHub's API and with git in a clone; one changed
//! meanwhile isn't overwritten), a fork (`,F`) and a new repository
//! (`,C`). Forks and repositories go under the owner's account, named
//! `ghtui-live-…`, and are deleted after: that takes the token's
//! `delete_repo` scope, without which those two are skipped.

use std::path::Path;
use std::time::Duration;

use anyhow::{Result, bail, ensure};
use serde_json::Value;

use super::pull::{git, scratch_clone};
use crate::live::fux::{Launch, Pane};
use crate::live::{Check, Cx, gh, until};

pub const LABELS: Check = Check::new(
    "labels",
    ",o makes a label on the labels page, ,e edits it, ,D deletes it after asking",
    labels,
)
.sweep(sweep_labels);

pub const MILESTONES: Check = Check::new(
    "milestones",
    ",o makes a milestone with a due date and description, ,e edits it, ,x closes and reopens it, ,D deletes it",
    milestones,
)
.sweep(sweep_milestones);

pub const REFS: Check = Check::new(
    "refs",
    ",o makes a branch and a tag at main by GitHub's API, ,D deletes them; the default branch is refused",
    refs,
);

pub const REFS_GIT: Check = Check::new(
    "refs-git",
    "in a clone, ,o pushes a new branch with git after asking and ,D deletes it with git; the clone's branches stay",
    refs_git,
);

pub const FILES: Check = Check::new(
    "files",
    ",e edits a file and commits it by GitHub's API; one changed meanwhile isn't overwritten; a rename, ,o a new file on a new branch with a pull request offered, ,D a deletion",
    files,
);

pub const FILES_GIT: Check = Check::new(
    "files-git",
    "in a clone, ,e commits a file with git on GitHub's tip and pushes it; the clone's branch and files stay",
    files_git,
);

pub const FORK: Check = Check::new(
    "fork",
    ",F forks a repository into the owner's account (default branch only), and the fork's page opens; deleted after (delete_repo)",
    fork,
)
.sweep(sweep_repos);

pub const NEW_REPO: Check = Check::new(
    "new-repo",
    ",C makes a private repository with a README and a licence, it opens, and cloning it is offered and done; deleted after (delete_repo)",
    new_repo,
);

const GITHUB: Duration = Duration::from_secs(60);

/// Whether the selected row starts with `name` (a label's chip, a
/// branch's name, a form's row), on the marked screen.
fn selected_is(marked: &str, name: &str) -> bool {
    marked.lines().any(|l| {
        let Some((_, rest)) = l.split_once("⟦▌") else {
            return false;
        };
        let rest = rest.trim_start();
        rest.strip_prefix(name)
            .is_some_and(|after| after.is_empty() || after.starts_with([' ', '⟧']))
    })
}

/// Selects the row that starts with `name`, going down the page row by
/// row from the top: never a row that only mentions it.
fn select(ghtui: &Pane, name: &str) -> Result<()> {
    ghtui.keys("g g")?;
    for _ in 0..40 {
        if selected_is(&ghtui.screen_marked()?, name) {
            return Ok(());
        }
        ghtui.keys("}")?;
    }
    bail!("no row starts with {name}")
}

/// Answers the picker open now: types `text` (after clearing what's
/// there) and takes the first row.
fn pick(ghtui: &Pane, text: &str) -> Result<()> {
    ghtui.keys("C-u")?;
    ghtui.type_text(text)?;
    ghtui.keys("Enter")?;
    Ok(())
}

// ---- labels ------------------------------------------------------------------------------

fn labels(cx: &mut Cx) -> Result<()> {
    let name = format!("{}label-{}", gh::BRANCH_PREFIX, cx.env.run_id);
    let undo = name.clone();
    cx.defer("delete the label", move || delete_label(&undo));
    let ghtui = cx.ghtui(&[&format!("https://github.com/{}/labels", gh::REPO)])?;
    ghtui.wait_for("New label")?;
    ghtui.wait_followed()?;

    cx.step(",o makes a label: its name first, then a colour typed")?;
    ghtui.keys(", o")?;
    ghtui.wait_for("Type the name")?;
    ghtui.type_text(&name)?;
    ghtui.keys("Enter")?;
    select(&ghtui, "Colour")?;
    ghtui.keys("Enter")?;
    pick(&ghtui, "#1d76db")?;
    ghtui.wait_for(&format!("Shows as  {name}  (#1d76db)"))?;
    ghtui.keys(", s")?;
    let made = until("the label on GitHub", GITHUB, || {
        Ok(gh::get(&format!("repos/{}/labels/{name}", gh::REPO)).ok())
    })?;
    ensure!(made["color"] == "1d76db", "its colour is {}", made["color"]);
    ghtui.wait_for(&format!("Created the label {name}"))?;
    ghtui.wait_followed()?;

    cx.step(",e on its row edits its description")?;
    select(&ghtui, &name)?;
    ghtui.keys(", e")?;
    ghtui.wait_for(&format!("Edit the label {name}"))?;
    select(&ghtui, "Description")?;
    ghtui.keys("Enter")?;
    pick(&ghtui, "Made by the live check")?;
    ghtui.keys(", s")?;
    until("the description on GitHub", GITHUB, || {
        let label = gh::get(&format!("repos/{}/labels/{name}", gh::REPO))?;
        Ok((label["description"] == "Made by the live check").then_some(()))
    })?;
    ghtui.wait_for("Made by the live check")?;
    ghtui.wait_followed()?;

    cx.step(",D deletes it, after asking")?;
    select(&ghtui, &name)?;
    ghtui.keys(", D")?;
    ghtui.wait_for(&format!("Delete the label {name}?"))?;
    ghtui.keys("Enter")?;
    until("the label gone from GitHub", GITHUB, || {
        Ok(gh::get(&format!("repos/{}/labels/{name}", gh::REPO))
            .is_err()
            .then_some(()))
    })?;
    ghtui.wait_for(&format!("Deleted the label {name}"))?;
    ghtui.wait_followed()?;
    let listed = ghtui
        .screen()?
        .lines()
        .any(|l| l.contains('│') && l.contains(&name));
    ensure!(!listed, "the labels page still lists it");
    Ok(())
}

fn delete_label(name: &str) -> Result<()> {
    if gh::get(&format!("repos/{}/labels/{name}", gh::REPO)).is_ok() {
        gh::send("DELETE", &format!("repos/{}/labels/{name}", gh::REPO), None)?;
    }
    Ok(())
}

fn sweep_labels() -> Result<()> {
    for label in gh::rest_list(&format!("repos/{}/labels?per_page=100", gh::REPO))? {
        if let Some(name) = label["name"].as_str()
            && name.starts_with(gh::BRANCH_PREFIX)
        {
            delete_label(name)?;
        }
    }
    Ok(())
}

// ---- milestones ------------------------------------------------------------------------

/// The test repository's milestone titled `title`, open or closed.
fn milestone(title: &str) -> Result<Option<Value>> {
    let all = gh::rest_list(&format!(
        "repos/{}/milestones?state=all&per_page=100",
        gh::REPO
    ))?;
    Ok(all.into_iter().find(|m| m["title"] == title))
}

fn milestones(cx: &mut Cx) -> Result<()> {
    let title = cx.mark("milestone");
    let undo = title.clone();
    cx.defer("delete the milestone", move || {
        for t in [undo.clone(), format!("{undo} (edited)")] {
            if let Some(n) = milestone(&t)?.and_then(|m| m["number"].as_u64()) {
                gh::send(
                    "DELETE",
                    &format!("repos/{}/milestones/{n}", gh::REPO),
                    None,
                )?;
            }
        }
        Ok(())
    });
    let ghtui = cx.ghtui(&[&format!("https://github.com/{}/milestones", gh::REPO)])?;
    ghtui.wait_for("Milestones")?;
    ghtui.wait_followed()?;

    cx.step(",o makes a milestone: its title, a due date and a description")?;
    ghtui.keys(", o")?;
    ghtui.wait_for("Type the title")?;
    ghtui.type_text(&title)?;
    ghtui.keys("Enter")?;
    select(&ghtui, "Due date")?;
    ghtui.keys("Enter")?;
    pick(&ghtui, "2031-02-28")?;
    ghtui.keys("c")?;
    ghtui.wait_for("The milestone's description")?;
    ghtui.type_text("Made by the live check.")?;
    ghtui.keys("C-s")?;
    let made = until("the milestone on GitHub", GITHUB, || milestone(&title))?;
    let due = made["due_on"].as_str().unwrap_or_default();
    ensure!(due.starts_with("2031-02-28"), "it's due {due}");
    ensure!(
        made["description"] == "Made by the live check.",
        "its description is {}",
        made["description"]
    );
    let number = made["number"].as_u64().unwrap_or_default();
    ghtui.wait_for(&format!("Created the milestone {title}"))?;
    ghtui.wait_followed()?;

    cx.step(",e edits its title")?;
    select(&ghtui, &title)?;
    ghtui.keys(", e")?;
    ghtui.wait_for("Edit the milestone")?;
    select(&ghtui, "Title")?;
    ghtui.keys("Enter")?;
    ghtui.type_text(" (edited)")?;
    ghtui.keys("Enter")?;
    ghtui.keys(", s")?;
    let edited = format!("{title} (edited)");
    until("the new title on GitHub", GITHUB, || milestone(&edited))?;
    ghtui.wait_for(&edited)?;
    ghtui.wait_followed()?;

    cx.step(",x closes it, and it leaves the open ones")?;
    select(&ghtui, &edited)?;
    ghtui.keys(", x")?;
    until("the milestone closed on GitHub", GITHUB, || {
        let m = gh::get(&format!("repos/{}/milestones/{number}", gh::REPO))?;
        Ok((m["state"] == "closed").then_some(()))
    })?;
    ghtui.wait_followed()?;
    ensure!(!ghtui.screen()?.contains(&edited), "it's still listed open");

    cx.step("on its own page, ,x reopens it and ,D deletes it after asking")?;
    ghtui.keys(":")?;
    ghtui.type_text(&format!("{}/milestone/{number}", gh::REPO))?;
    ghtui.keys("Enter")?;
    ghtui.wait_for(&edited)?;
    ghtui.wait_followed()?;
    ghtui.keys(", x")?;
    until("the milestone open on GitHub", GITHUB, || {
        let m = gh::get(&format!("repos/{}/milestones/{number}", gh::REPO))?;
        Ok((m["state"] == "open").then_some(()))
    })?;
    ghtui.wait_followed()?;
    ghtui.keys(", D")?;
    ghtui.wait_for(&format!("Delete the milestone {edited}?"))?;
    ghtui.keys("Enter")?;
    until("the milestone gone from GitHub", GITHUB, || {
        Ok(milestone(&edited)?.is_none().then_some(()))
    })?;
    ghtui.wait_for("Deleted the milestone")?;
    Ok(())
}

fn sweep_milestones() -> Result<()> {
    for m in gh::rest_list(&format!(
        "repos/{}/milestones?state=all&per_page=100",
        gh::REPO
    ))? {
        if m["title"]
            .as_str()
            .is_some_and(|t| t.starts_with("[ghtui live]"))
            && let Some(n) = m["number"].as_u64()
        {
            gh::send(
                "DELETE",
                &format!("repos/{}/milestones/{n}", gh::REPO),
                None,
            )?;
        }
    }
    Ok(())
}

// ---- branches and tags ----------------------------------------------------------------

/// Where `refs/{kind}/{name}` points on GitHub, if it's there.
fn ref_at(kind: &str, name: &str) -> Option<String> {
    let at = gh::get(&format!("repos/{}/git/ref/{kind}/{name}", gh::REPO)).ok()?;
    at["object"]["sha"].as_str().map(str::to_owned)
}

fn main_sha() -> Result<String> {
    ref_at("heads", "main").ok_or_else(|| anyhow::anyhow!("no main"))
}

/// Makes `name` from main on the list on screen, then deletes it; `ask`:
/// the clone's question comes before pushing.
fn make_and_delete(cx: &Cx, ghtui: &Pane, kind: &str, name: &str, ask: bool) -> Result<()> {
    let noun = if kind == "heads" { "branch" } else { "tag" };
    cx.step(&format!(",o makes the {noun} {name} at main"))?;
    ghtui.keys(", o")?;
    ghtui.wait_for(&format!("New {noun}: its name"))?;
    ghtui.type_text(name)?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Where it starts")?;
    ghtui.type_text("main")?;
    ghtui.keys("Enter")?;
    if ask {
        ghtui.wait_for(&format!("Create the {noun} {name} at main?"))?;
        ghtui.wait_for("With git in your clone")?;
        ghtui.keys("Enter")?;
    }
    let main = main_sha()?;
    until(&format!("the {noun} on GitHub"), GITHUB, || {
        Ok(ref_at(kind, name).filter(|sha| *sha == main))
    })?;
    ghtui.wait_followed()?;
    ghtui.wait_for(name)?;

    cx.step(&format!(",D deletes the {noun}, after asking"))?;
    select(ghtui, name)?;
    ghtui.keys(", D")?;
    ghtui.wait_for(&format!("Delete the {noun} {name}?"))?;
    if ask {
        ghtui.wait_for(&format!("Your clone's own {noun} {name} stays"))?;
    }
    ghtui.keys("Enter")?;
    until(&format!("the {noun} gone from GitHub"), GITHUB, || {
        Ok(ref_at(kind, name).is_none().then_some(()))
    })?;
    ghtui.wait_for(&format!("Deleted the {noun} {name}"))?;
    Ok(())
}

fn delete_ref(kind: &'static str, name: String) -> impl FnOnce() -> Result<()> {
    move || {
        if ref_at(kind, &name).is_some() {
            gh::send(
                "DELETE",
                &format!("repos/{}/git/refs/{kind}/{name}", gh::REPO),
                None,
            )?;
        }
        Ok(())
    }
}

fn refs(cx: &mut Cx) -> Result<()> {
    let branch = format!("{}branch-{}", gh::BRANCH_PREFIX, cx.env.run_id);
    let tag = format!("{}tag-{}", gh::BRANCH_PREFIX, cx.env.run_id);
    cx.defer("delete the branch", delete_ref("heads", branch.clone()));
    cx.defer("delete the tag", delete_ref("tags", tag.clone()));
    let ghtui = cx.ghtui(&[&format!("https://github.com/{}/branches", gh::REPO)])?;
    ghtui.wait_for("Branches")?;
    ghtui.wait_followed()?;
    make_and_delete(cx, &ghtui, "heads", &branch, false)?;

    cx.step(",D refuses the default branch, and says why")?;
    select(&ghtui, "main")?;
    ghtui.keys(", D")?;
    ghtui.wait_for("main is the default branch: GitHub doesn't delete it")?;
    ensure!(ref_at("heads", "main").is_some(), "main is gone");

    ghtui.keys(":")?;
    ghtui.type_text(&format!("{}/tags", gh::REPO))?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Tags")?;
    ghtui.wait_followed()?;
    make_and_delete(cx, &ghtui, "tags", &tag, false)
}

fn refs_git(cx: &mut Cx) -> Result<()> {
    let clone = scratch_clone(cx, "clone-refs")?;
    let branch = format!("{}git-branch-{}", gh::BRANCH_PREFIX, cx.env.run_id);
    cx.defer("delete the branch", delete_ref("heads", branch.clone()));
    let url = format!("https://github.com/{}/branches", gh::REPO);
    let ghtui = cx.launch(Launch::new(&[&url]).cwd(&clone))?;
    ghtui.wait_for("Branches")?;
    ghtui.wait_followed()?;
    let before = git(&clone, &["for-each-ref", "refs/heads", "refs/tags"])?;
    make_and_delete(cx, &ghtui, "heads", &branch, true)?;
    let after = git(&clone, &["for-each-ref", "refs/heads", "refs/tags"])?;
    ensure!(before == after, "the clone's branches changed: {after}");
    Ok(())
}

// ---- files -------------------------------------------------------------------------------

/// `file` on `branch`, as GitHub has it; none when it isn't there.
fn file_on(branch: &str, file: &str) -> Option<String> {
    let at = gh::get(&format!("repos/{}/contents/{file}?ref={branch}", gh::REPO)).ok()?;
    let b64: String = at["content"].as_str()?.split_whitespace().collect();
    decode(&b64)
}

fn decode(b64: &str) -> Option<String> {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bits = 0u32;
    let mut n = 0;
    let mut out = Vec::new();
    for c in b64.bytes().filter(|c| *c != b'=') {
        let v = ABC.iter().position(|a| *a == c)?;
        bits = bits << 6 | u32::try_from(v).ok()?;
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push(u8::try_from(bits >> n & 0xff).ok()?);
        }
    }
    String::from_utf8(out).ok()
}

fn files(cx: &mut Cx) -> Result<()> {
    let branch = format!("{}files-{}", gh::BRANCH_PREFIX, cx.env.run_id);
    let proposed = format!("{}files-pr-{}", gh::BRANCH_PREFIX, cx.env.run_id);
    let file = "live/housekeeping.md";
    {
        let (a, b) = (branch.clone(), proposed.clone());
        cx.defer("delete the branches", move || {
            gh::drop_branch(&a)?;
            gh::drop_branch(&b)
        });
    }
    gh::branch_with_file(&branch, file, "Line one.\n")?;
    let url = format!("https://github.com/{}/blob/{branch}/{file}", gh::REPO);
    let ghtui = cx.ghtui(&[&url])?;
    ghtui.wait_for("Line one.")?;
    ghtui.wait_followed()?;

    cx.step(",e opens the file in the composer; ctrl-s commits it after asking")?;
    ghtui.keys(", e")?;
    ghtui.wait_for(&format!("Editing {file}"))?;
    ghtui.wait_for("open in $EDITOR")?;
    ghtui.keys("C-End Enter")?;
    ghtui.type_text("Line two, by ghtui.")?;
    ghtui.keys("C-s")?;
    ghtui.wait_for("Commit changes?")?;
    ghtui.wait_for("On GitHub, by you")?;
    ghtui.keys("Enter")?;
    until("the commit on GitHub", GITHUB, || {
        Ok(file_on(&branch, file).filter(|t| t.contains("Line two, by ghtui.")))
    })?;
    ghtui.wait_for("Line two, by ghtui.")?;
    ghtui.wait_followed()?;

    cx.step("a file changed on GitHub while it's written isn't overwritten")?;
    ghtui.keys(", e")?;
    ghtui.wait_for("open in $EDITOR")?;
    gh::write_file(&branch, file, "Changed meanwhile, elsewhere.\n")?;
    ghtui.keys("C-End")?;
    ghtui.type_text(" This must not overwrite it.")?;
    ghtui.keys("C-s")?;
    ghtui.wait_for("Commit changes?")?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("changed on GitHub since you opened it")?;
    let now = file_on(&branch, file).unwrap_or_default();
    if now != "Changed meanwhile, elsewhere.\n" {
        bail!("the file changed meanwhile was overwritten: {now}");
    }
    ghtui.keys("Escape q")?;
    // ghtui takes GitHub's word on where a branch is for 30 seconds.
    until("the file's page showing GitHub's change", GITHUB, || {
        ghtui.keys("r")?;
        let shown = ghtui.wait_for_within("Changed meanwhile, elsewhere.", Duration::from_secs(4));
        Ok(shown.ok().map(drop))
    })?;
    ghtui.wait_followed()?;

    cx.step(",e renames it, with its path")?;
    let renamed = "live/renamed.md";
    ghtui.keys(", e")?;
    ghtui.wait_for("open in $EDITOR")?;
    ghtui.keys("Escape")?;
    select(&ghtui, "Path")?;
    ghtui.keys("Enter")?;
    pick(&ghtui, renamed)?;
    ghtui.wait_for("Rename housekeeping.md to renamed.md")?;
    ghtui.keys(", s")?;
    ghtui.wait_for(&format!("Renames {file} to {renamed}"))?;
    ghtui.keys("Enter")?;
    until("the rename on GitHub", GITHUB, || {
        let moved = file_on(&branch, renamed).is_some() && file_on(&branch, file).is_none();
        Ok(moved.then_some(()))
    })?;
    ghtui.wait_followed()?;

    cx.step(",o makes a file on a new branch, and a pull request is offered")?;
    ghtui.keys(", o")?;
    ghtui.wait_for("Name the file")?;
    ghtui.type_text("new.txt")?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("The new file's text")?;
    ghtui.type_text("A new file.")?;
    ghtui.keys("Escape")?;
    select(&ghtui, "Commit to")?;
    ghtui.keys("Enter")?;
    pick(&ghtui, &proposed)?;
    ghtui.keys(", s")?;
    ghtui.wait_for(&format!("Commits to a new branch, {proposed}"))?;
    ghtui.keys("Enter")?;
    until("the new file on the new branch", GITHUB, || {
        Ok(file_on(&proposed, "live/new.txt").filter(|t| t == "A new file."))
    })?;
    ghtui.wait_for(&format!("Open a pull request for {proposed}?"))?;
    ghtui.keys("Enter")?;
    ghtui.wait_for(&format!("compare: {proposed}"))?;
    ghtui.wait_for(&format!("base: {branch}"))?;
    ghtui.keys("q")?;
    ghtui.wait_followed()?;

    cx.step(",D deletes it, with a commit, after asking")?;
    ghtui.keys(", D")?;
    ghtui.wait_for("Deleting live/new.txt")?;
    ghtui.keys(", s")?;
    ghtui.wait_for("Delete live/new.txt?")?;
    ghtui.keys("Enter")?;
    until("the file gone from GitHub", GITHUB, || {
        Ok(file_on(&proposed, "live/new.txt").is_none().then_some(()))
    })?;
    ghtui.wait_followed()?;
    Ok(())
}

fn files_git(cx: &mut Cx) -> Result<()> {
    let branch = format!("{}files-git-{}", gh::BRANCH_PREFIX, cx.env.run_id);
    let file = "live/housekeeping-git.md";
    {
        let b = branch.clone();
        cx.defer("delete the branch", move || gh::drop_branch(&b));
    }
    gh::branch_with_file(&branch, file, "Line one.\n")?;
    let clone = scratch_clone(cx, "clone-files")?;
    let head = git(&clone, &["rev-parse", "HEAD"])?;
    let url = format!("https://github.com/{}/blob/{branch}/{file}", gh::REPO);
    let ghtui = cx.launch(Launch::new(&[&url]).cwd(&clone))?;
    ghtui.wait_for("Line one.")?;
    ghtui.wait_followed()?;

    cx.step(",e commits with git in the clone, after asking, and pushes it")?;
    ghtui.keys(", e")?;
    ghtui.wait_for("open in $EDITOR")?;
    ghtui.keys("C-End Enter")?;
    ghtui.type_text("Line two, with git.")?;
    ghtui.keys("C-s")?;
    ghtui.wait_for("With git in your clone")?;
    ghtui.keys("Enter")?;
    until("the commit on GitHub", GITHUB, || {
        Ok(file_on(&branch, file).filter(|t| t.contains("Line two, with git.")))
    })?;
    let commit = gh::get(&format!("repos/{}/commits/{branch}", gh::REPO))?;
    ensure!(
        commit["commit"]["author"]["name"] == "ghtui live check",
        "the commit isn't the clone's git user's: {}",
        commit["commit"]["author"]["name"]
    );
    ghtui.wait_for("Line two, with git.")?;
    ensure!(
        git(&clone, &["rev-parse", "HEAD"])? == head,
        "the clone's HEAD moved"
    );
    ensure!(
        git(&clone, &["status", "--porcelain"])?.is_empty(),
        "the clone's working tree changed"
    );
    ensure!(
        git(&clone, &["branch", "--list", &branch])?.is_empty(),
        "the clone got a branch"
    );
    Ok(())
}

// ---- forks and new repositories ---------------------------------------------------------

/// Skips the check unless the token may delete the repository it makes.
fn can_delete(cx: &Cx) -> Result<()> {
    if cx.env.scopes.iter().any(|s| s == "delete_repo") {
        return Ok(());
    }
    Err(Cx::skip(
        "deleting the repository it makes takes the token's delete_repo scope: `gh auth refresh -s delete_repo` adds it",
    ))
}

fn delete_repo(full: String) -> impl FnOnce() -> Result<()> {
    move || {
        if gh::get(&format!("repos/{full}")).is_ok() {
            gh::send("DELETE", &format!("repos/{full}"), None)?;
        }
        Ok(())
    }
}

/// Repositories a killed run made under the owner's account.
fn sweep_repos() -> Result<()> {
    let (_, scopes) = gh::account()?;
    if !scopes.iter().any(|s| s == "delete_repo") {
        return Ok(());
    }
    for repo in gh::rest_list("user/repos?affiliation=owner&per_page=100")? {
        if let Some(full) = repo["full_name"].as_str()
            && repo["name"]
                .as_str()
                .is_some_and(|n| n.starts_with(gh::BRANCH_PREFIX))
        {
            delete_repo(full.to_owned())()?;
        }
    }
    Ok(())
}

fn fork(cx: &mut Cx) -> Result<()> {
    can_delete(cx)?;
    let source = "octocat/Spoon-Knife";
    let name = format!("{}fork-{}", gh::BRANCH_PREFIX, cx.env.run_id);
    let full = format!("{}/{name}", gh::OWNER);
    cx.defer("delete the fork", delete_repo(full.clone()));
    let ghtui = cx.ghtui(&[source])?;
    ghtui.wait_for("Spoon-Knife")?;
    ghtui.wait_followed()?;

    cx.step(",F forks it to the owner's account, under its own name")?;
    ghtui.keys(", F")?;
    ghtui.wait_for(&format!("Create a new fork of {source}"))?;
    ghtui.wait_for("Copy the default branch only")?;
    select(&ghtui, "Name")?;
    ghtui.keys("Enter")?;
    pick(&ghtui, &name)?;
    ghtui.wait_for(&format!("It will be {full}"))?;
    ghtui.keys(", s")?;
    let made = until("the fork on GitHub", Duration::from_secs(120), || {
        Ok(gh::get(&format!("repos/{full}")).ok())
    })?;
    ensure!(
        made["parent"]["full_name"] == source,
        "it's a fork of {}",
        made["parent"]["full_name"]
    );
    cx.step("the fork's page opens, once GitHub has made it")?;
    ghtui.wait_followed()?;
    ghtui.wait_for(&format!("{} / {name}", gh::OWNER))?;
    Ok(())
}

fn new_repo(cx: &mut Cx) -> Result<()> {
    can_delete(cx)?;
    let name = format!("{}repo-{}", gh::BRANCH_PREFIX, cx.env.run_id);
    let full = format!("{}/{name}", gh::OWNER);
    cx.defer("delete the repository", delete_repo(full.clone()));
    let ghtui = cx.ghtui(&[gh::REPO])?;
    ghtui.wait_for("Pull requests")?;
    ghtui.wait_followed()?;

    cx.step(",C makes a private repository, with a README and the MIT licence")?;
    ghtui.keys(", C")?;
    ghtui.wait_for("Create a new repository")?;
    ghtui.wait_for("Type the name")?;
    ghtui.type_text(&name)?;
    ghtui.keys("Enter")?;
    select(&ghtui, "Visibility")?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Private: you choose who can see it")?;
    select(&ghtui, "README")?;
    ghtui.keys("Enter")?;
    select(&ghtui, "Licence")?;
    ghtui.keys("Enter")?;
    pick(&ghtui, "MIT")?;
    ghtui.wait_for("MIT License")?;
    ghtui.keys(", s")?;
    let made = until("the repository on GitHub", GITHUB, || {
        Ok(gh::get(&format!("repos/{full}")).ok())
    })?;
    ensure!(made["private"] == true, "it's public");
    ensure!(
        made["license"]["spdx_id"] == "MIT",
        "its licence is {}",
        made["license"]["spdx_id"]
    );

    cx.step("it opens, and cloning it is offered and done here")?;
    ghtui.wait_for(&format!("Clone {full}?"))?;
    ghtui.keys("Enter")?;
    let dir = cx.env.work.join(&name);
    until("the clone", GITHUB, || {
        Ok(dir.join("README.md").exists().then_some(()))
    })?;
    let remote = git(&dir, &["remote", "get-url", "origin"])?;
    ensure!(remote.contains(&full), "it's a clone of {remote}");
    remove(&dir)?;
    Ok(())
}

fn remove(dir: &Path) -> Result<()> {
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
}
