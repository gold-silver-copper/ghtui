//! Writes made with git in the diff's clone: someone's suggestion
//! committed (`:commit suggestion`), and a pull request's conflicts with
//! its base resolved (`:resolve conflicts`, `>`, `:commit merge`). Each
//! makes its own branches and pull request, from objects through GitHub's
//! API, so the test repository's `main` never changes.

use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde_json::json;

use crate::live::{Check, Cx, gh, until};

pub const SUGGESTION: Check = Check::new(
    "suggestion",
    ":commit suggestion commits a review's suggestion to the pull request's branch and resolves its thread",
    suggestion,
);

pub const CONFLICTS: Check = Check::new(
    "conflicts",
    ":resolve conflicts merges the base in with git, > takes its side, :commit merge pushes the merge",
    conflicts,
);

const GITHUB: Duration = Duration::from_secs(30);
/// GitHub works out whether a pull request conflicts after it's made.
const MERGEABLE: Duration = Duration::from_secs(120);

/// A branch made from `from` with `path` holding `text`, from objects;
/// its commit. Deleted, with its pull requests closed, when the check
/// ends.
pub(super) fn branch_with(
    cx: &mut Cx,
    name: &str,
    from: &str,
    path: &str,
    text: &str,
) -> Result<String> {
    let repo = gh::REPO;
    let base = gh::get(&format!("repos/{repo}/git/ref/heads/{from}"))?;
    let parent = base["object"]["sha"].as_str().context("no tip")?.to_owned();
    let tree = gh::send(
        "POST",
        &format!("repos/{repo}/git/trees"),
        Some(&json!({
            "base_tree": parent,
            "tree": [{"path": path, "mode": "100644", "type": "blob", "content": text}],
        })),
    )?;
    let commit = gh::send(
        "POST",
        &format!("repos/{repo}/git/commits"),
        Some(&json!({
            "message": cx.mark(&format!("{path} on {name}")),
            "tree": tree["sha"],
            "parents": [parent],
        })),
    )?;
    let sha = commit["sha"].as_str().context("no commit")?.to_owned();
    gh::send(
        "POST",
        &format!("repos/{repo}/git/refs"),
        Some(&json!({"ref": format!("refs/heads/{name}"), "sha": sha})),
    )?;
    let undo = name.to_owned();
    cx.defer(
        format!("close its pull request, delete {name}"),
        move || gh::drop_branch(&undo),
    );
    Ok(sha)
}

/// A pull request of `head` into `base`; its number.
fn open_pr(cx: &Cx, head: &str, base: &str) -> Result<u64> {
    let pr = gh::send(
        "POST",
        &format!("repos/{}/pulls", gh::REPO),
        Some(&json!({"title": cx.mark("pull request"), "head": head, "base": base})),
    )?;
    pr["number"].as_u64().context("no pull request number")
}

/// `path`'s text on `branch`, as GitHub has it.
fn text_on(branch: &str, path: &str) -> Result<String> {
    let query = "query($owner: String!, $name: String!, $at: String!) {
        repository(owner: $owner, name: $name) { object(expression: $at) { ... on Blob { text } } }
    }";
    let at = format!("{branch}:{path}");
    let data = gh::graphql(
        query,
        json!({"owner": gh::OWNER, "name": gh::NAME, "at": at}),
    )?;
    let text = data["repository"]["object"]["text"].as_str();
    Ok(text.context("no such file")?.to_owned())
}

/// Whether `number`'s first review thread is resolved on GitHub.
fn resolved(number: u64) -> Result<bool> {
    let query = "query($owner: String!, $name: String!, $n: Int!) {
        repository(owner: $owner, name: $name) {
            pullRequest(number: $n) { reviewThreads(first: 5) { nodes { isResolved } } }
        }
    }";
    let vars = json!({"owner": gh::OWNER, "name": gh::NAME, "n": number});
    let data = gh::graphql(query, vars)?;
    let threads = &data["repository"]["pullRequest"]["reviewThreads"]["nodes"];
    Ok(threads[0]["isResolved"] == true)
}

fn suggestion(cx: &mut Cx) -> Result<()> {
    let branch = format!("{}suggest-{}", gh::BRANCH_PREFIX, cx.env.run_id);
    let path = format!("live-suggest-{}.txt", cx.env.run_id);
    let head = branch_with(cx, &branch, "main", &path, "one\ntwo\n")?;
    let number = open_pr(cx, &branch, "main")?;
    gh::send(
        "POST",
        &format!("repos/{}/pulls/{number}/comments", gh::REPO),
        Some(&json!({
            "body": "Better:\n```suggestion\nTWO\n```",
            "commit_id": head,
            "path": path,
            "line": 2,
            "side": "RIGHT",
        })),
    )?;

    let url = format!("https://github.com/{}/pull/{number}/files", gh::REPO);
    let ghtui = cx.ghtui(&[&url])?;
    ghtui.wait_for(&path)?;
    ghtui.wait_for("Better:")?;
    cx.step(":commit suggestion on its thread commits it, after asking")?;
    ghtui.palette("next thread")?;
    ghtui.palette("commit suggestion")?;
    ghtui.wait_for(&format!("Commit the suggestion to {branch}?"))?;
    ghtui.keys("Enter")?;
    until("the suggestion on GitHub's branch", GITHUB, || {
        Ok((text_on(&branch, &path)? == "one\nTWO\n").then_some(()))
    })?;
    let tip = gh::get(&format!("repos/{}/commits/{branch}", gh::REPO))?;
    let message = tip["commit"]["message"].as_str().unwrap_or_default();
    ensure!(
        message == format!("Update {path}"),
        "its message is {message}"
    );
    ghtui.wait_for("Committed the suggestion")?;

    cx.step("its thread is resolved")?;
    until("the thread resolved on GitHub", GITHUB, || {
        Ok(resolved(number)?.then_some(()))
    })?;
    ghtui.wait_for("at its new head")?;
    Ok(())
}

fn conflicts(cx: &mut Cx) -> Result<()> {
    let run = cx.env.run_id.clone();
    let (base, head) = (
        format!("{}base-{run}", gh::BRANCH_PREFIX),
        format!("{}head-{run}", gh::BRANCH_PREFIX),
    );
    let path = format!("live-conflict-{run}.txt");
    let theirs = branch_with(cx, &base, "main", &path, "theirs\n")?;
    let ours = branch_with(cx, &head, "main", &path, "ours\n")?;
    let number = open_pr(cx, &head, &base)?;
    until("GitHub to find the conflict", MERGEABLE, || {
        let pr = gh::get(&format!("repos/{}/pulls/{number}", gh::REPO))?;
        Ok((pr["mergeable"] == false).then_some(()))
    })?;

    let url = format!("https://github.com/{}/pull/{number}", gh::REPO);
    let ghtui = cx.ghtui(&[&url])?;
    ghtui.wait_for("wants to merge into")?;
    ghtui.wait_followed()?;
    cx.step(":resolve conflicts merges the base in, and shows the conflict")?;
    let marker = format!(">>>>>>> {base}");
    let mut shown = false;
    // The pull request ghtui loaded may predate GitHub finding the
    // conflict: then it says so, and `r` loads it again.
    for _ in 0..5 {
        ghtui.palette("resolve conflicts")?;
        if ghtui
            .wait_for_within(&marker, Duration::from_secs(20))
            .is_ok()
        {
            shown = true;
            break;
        }
        ghtui.keys("r")?;
        ghtui.wait_followed()?;
    }
    ensure!(shown, "the conflict never showed");
    ghtui.keys("/")?;
    ghtui.type_text("theirs")?;
    ghtui.keys("Enter")?;
    cx.step("> takes the base's side")?;
    ghtui.keys(">")?;
    ghtui.wait_for("all resolved")?;
    cx.step(":commit merge pushes the merge, after asking")?;
    ghtui.palette("commit merge")?;
    ghtui.wait_for(&format!("Commit the merge and push it to {head}?"))?;
    ghtui.keys("Enter")?;
    let merge = until("the merge on GitHub's branch", GITHUB, || {
        let tip = gh::get(&format!("repos/{}/commits/{head}", gh::REPO))?;
        Ok((tip["sha"] != ours.as_str()).then_some(tip))
    })?;
    let parents: Vec<&str> = (merge["parents"].as_array().into_iter().flatten())
        .filter_map(|p| p["sha"].as_str())
        .collect();
    ensure!(
        parents == [ours.as_str(), theirs.as_str()],
        "its parents are {parents:?}"
    );
    let message = merge["commit"]["message"].as_str().unwrap_or_default();
    ensure!(
        message == format!("Merge branch '{base}' into {head}"),
        "its message is {message}"
    );
    ensure!(
        text_on(&head, &path)? == "theirs\n",
        "the file isn't the base's"
    );
    ghtui.wait_for("No conflicts with the base branch")?;
    Ok(())
}
