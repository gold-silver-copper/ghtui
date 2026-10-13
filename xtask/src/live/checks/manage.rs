//! Managing an issue, a pull request and a discussion, as GitHub's
//! sidebar and menus do: locking with a reason, pinning, sub-issues and
//! what blocks an issue, hiding a comment, a branch for an issue checked
//! out in a clone, closing as a duplicate; back to draft, a new base, the
//! branch deleted and restored after merging, a revert; a discussion
//! locked, closed with a reason and reopened, and a new one started.

use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use super::pull::{git, scratch_clone};
use crate::live::{Check, Cx, Launch, gh, until};

pub const ISSUE: Check = Check::new(
    "manage-issue",
    ",L locks and unlocks, the menu pins, ,S adds, moves and removes sub-issues, ,K marks it blocked, ,H hides a comment, ,c makes a branch and checks it out, ,x closes as a duplicate",
    issue,
)
.sweep(sweep);

pub const PULL: Check = Check::new(
    "manage-pull",
    ",r converts to draft, the menu changes the base, ,b deletes and restores the merged branch, the menu reverts it",
    pull,
);

pub const DISCUSSION: Check = Check::new(
    "manage-discussion",
    ",L locks a discussion, ,x closes it as outdated and reopens it, the menu starts a new one",
    discussion,
);

pub const TRANSFER: Check = Check::new(
    "manage-transfer",
    "the menu transfers an issue to another repository of the owner's",
    transfer,
);

const GITHUB: Duration = Duration::from_secs(30);

/// An issue's fields that REST doesn't have.
fn issue_graph(number: u64) -> Result<Value> {
    let data = gh::graphql(
        "query($o: String!, $n: String!, $i: Int!) { repository(owner: $o, name: $n) { issue(number: $i) {
            id isPinned locked
            subIssues(first: 10) { nodes { number } }
            blockedBy(first: 10) { nodes { number } }
            linkedBranches(first: 5) { nodes { ref { name } } } } } }",
        json!({"o": gh::OWNER, "n": gh::NAME, "i": number}),
    )?;
    Ok(data["repository"]["issue"].clone())
}

fn numbers(list: &Value) -> Vec<u64> {
    list["nodes"]
        .as_array()
        .map(|n| n.iter().filter_map(|n| n["number"].as_u64()).collect())
        .unwrap_or_default()
}

/// A throwaway issue, deleted after.
fn throwaway(cx: &mut Cx, what: &str) -> Result<u64> {
    let number = gh::create_issue(&cx.mark(what), "From ghtui's live check.")?;
    cx.defer(format!("delete issue #{number}"), move || {
        gh::delete_issue(number)
    });
    Ok(number)
}

fn issue(cx: &mut Cx) -> Result<()> {
    let number = throwaway(cx, "managed issue")?;
    let (b, c) = (throwaway(cx, "sub-issue")?, throwaway(cx, "another")?);
    let words = "Hide me: from the live check";
    let comment = gh::send(
        "POST",
        &format!("repos/{}/issues/{number}/comments", gh::REPO),
        Some(&json!({"body": words})),
    )?;
    let comment_id = comment["node_id"]
        .as_str()
        .context("no comment id")?
        .to_owned();
    let clone = scratch_clone(cx, "clone-issue-branch")?;
    let url = format!("https://github.com/{}/issues/{number}", gh::REPO);
    let ghtui = cx.launch(Launch::new(&[&url]).cwd(&clone))?;
    ghtui.wait_for(words)?;
    ghtui.wait_followed()?;

    cx.step(",L locks it as too heated")?;
    ghtui.keys(", L")?;
    ghtui.wait_for("Lock conversation")?;
    ghtui.type_text("heated")?;
    ghtui.keys("Enter")?;
    until("the issue locked on GitHub", GITHUB, || {
        let i = gh::issue(number)?;
        Ok((i["locked"] == true && i["active_lock_reason"] == "too heated").then_some(()))
    })?;
    ghtui.wait_for("Locked as too heated")?;
    ghtui.wait_followed()?;
    cx.step(",L unlocks it, after asking")?;
    ghtui.keys(", L")?;
    ghtui.wait_for("Unlock the conversation on this issue?")?;
    ghtui.keys("Enter")?;
    until("the issue unlocked on GitHub", GITHUB, || {
        Ok((gh::issue(number)?["locked"] == false).then_some(()))
    })?;
    ghtui.wait_gone("Locked as too heated")?;

    ghtui.wait_followed()?;
    cx.step("the menu pins it, then unpins it")?;
    let undo = number;
    cx.defer(format!("unpin #{number}"), move || unpin(undo));
    ghtui.keys("Space /")?;
    ghtui.type_text("pin the issue")?;
    ghtui.keys("Enter")?;
    until("the issue pinned on GitHub", GITHUB, || {
        Ok((issue_graph(number)?["isPinned"] == true).then_some(()))
    })?;
    ghtui.wait_for("Pinned")?;
    ghtui.wait_followed()?;
    ghtui.keys("Space /")?;
    ghtui.type_text("unpin")?;
    ghtui.keys("Enter")?;
    until("the issue unpinned on GitHub", GITHUB, || {
        Ok((issue_graph(number)?["isPinned"] == false).then_some(()))
    })?;

    ghtui.wait_followed()?;
    cx.step(",S adds two sub-issues")?;
    for sub in [b, c] {
        ghtui.keys(", S")?;
        ghtui.wait_for("alt-↑↓ moves")?;
        ghtui.type_text(&format!("#{sub}"))?;
        ghtui.keys("Enter")?;
        until("the sub-issue on GitHub", GITHUB, || {
            Ok(numbers(&issue_graph(number)?["subIssues"])
                .contains(&sub)
                .then_some(()))
        })?;
        ghtui.keys("Escape")?;
        ghtui.wait_followed()?;
    }
    cx.step(",S then alt-↓ moves the first below the second")?;
    ghtui.keys(", S")?;
    ghtui.wait_for(&format!("✓ #{b}"))?;
    ghtui.keys("M-Down")?;
    until("the sub-issues reordered on GitHub", GITHUB, || {
        Ok((numbers(&issue_graph(number)?["subIssues"]) == [c, b]).then_some(()))
    })?;
    cx.step("↵ on one removes it")?;
    ghtui.keys("Escape")?;
    ghtui.wait_followed()?;
    ghtui.keys(", S")?;
    ghtui.wait_for(&format!("✓ #{c}"))?;
    ghtui.keys("Enter")?;
    until("the sub-issue removed on GitHub", GITHUB, || {
        Ok((numbers(&issue_graph(number)?["subIssues"]) == [b]).then_some(()))
    })?;
    ghtui.keys("Escape")?;

    ghtui.wait_followed()?;
    cx.step(",K marks it blocked by another")?;
    ghtui.keys(", K")?;
    ghtui.wait_for("Blocked by")?;
    ghtui.type_text(&format!("#{c}"))?;
    ghtui.keys("Enter")?;
    until("blocked on GitHub", GITHUB, || {
        Ok((numbers(&issue_graph(number)?["blockedBy"]) == [c]).then_some(()))
    })?;
    ghtui.keys("Escape")?;
    ghtui.wait_followed()?;
    ghtui.keys(", K")?;
    ghtui.wait_for(&format!("✓ #{c}"))?;
    ghtui.keys("Enter")?;
    until("no longer blocked on GitHub", GITHUB, || {
        Ok(numbers(&issue_graph(number)?["blockedBy"])
            .is_empty()
            .then_some(()))
    })?;
    ghtui.keys("Escape")?;

    ghtui.wait_followed()?;
    cx.step(",H hides the comment as outdated; it shows folded")?;
    let selected = "⟦▌Hide me";
    ghtui.select("gg }", "the comment", |s| s.contains(selected))?;
    ghtui.keys(", H")?;
    ghtui.wait_for("Hide the comment as")?;
    ghtui.type_text("outdated")?;
    ghtui.keys("Enter")?;
    until("the comment hidden on GitHub", GITHUB, || {
        Ok((minimized(&comment_id)? == Some(true)).then_some(()))
    })?;
    ghtui.wait_for("Hidden as outdated")?;
    ghtui.wait_gone(words)?;
    ghtui.wait_followed()?;
    cx.step(",H on it unhides it")?;
    ghtui.select("gg }", "the hidden comment", |s| {
        s.lines()
            .any(|l| l.contains("⟦▌") && l.contains("Hidden as outdated"))
    })?;
    ghtui.keys(", H")?;
    until("the comment shown on GitHub", GITHUB, || {
        Ok((minimized(&comment_id)? == Some(false)).then_some(()))
    })?;
    ghtui.wait_for(words)?;

    ghtui.wait_followed()?;
    cx.step(",c makes a branch for it, linked, and checks it out in the clone")?;
    ghtui.keys(", c")?;
    ghtui.wait_for("Create a branch for the issue")?;
    // GitHub's name, which ghtui offers: the number, then the title's words.
    let title = cx.mark("managed issue");
    let parts = title
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase);
    let mut branch = number.to_string();
    for w in parts {
        if branch.len() + 1 + w.len() > 60 {
            break;
        }
        branch = format!("{branch}-{w}");
    }
    ghtui.wait_for(&format!("Create {branch} from main"))?;
    ghtui.keys("Enter")?;
    let undo = branch.clone();
    cx.defer(format!("delete branch {branch}"), move || {
        gh::drop_branch(&undo)
    });
    until("the linked branch on GitHub", GITHUB, || {
        let linked = issue_graph(number)?["linkedBranches"]["nodes"].clone();
        Ok(linked
            .as_array()
            .is_some_and(|l| l.iter().any(|b| b["ref"]["name"] == branch.as_str()))
            .then_some(()))
    })?;
    ghtui.wait_for(&format!("Check out {branch}"))?;
    ghtui.keys("Enter")?;
    until("the clone on the branch", GITHUB, || {
        let on = git(&clone, &["branch", "--show-current"])?;
        Ok((on == branch).then_some(()))
    })?;
    ghtui.wait_for(&format!("You're on {branch}"))?;

    ghtui.wait_followed()?;
    cx.step(",x closes it as a duplicate of another, chosen next")?;
    ghtui.keys(", x")?;
    ghtui.wait_for("Duplicate…")?;
    ghtui.keys("Tab Tab Enter")?;
    ghtui.wait_for("Close as a duplicate of")?;
    ghtui.type_text(&format!("#{b}"))?;
    ghtui.keys("Enter")?;
    until("the issue closed as a duplicate on GitHub", GITHUB, || {
        let i = gh::issue(number)?;
        Ok((i["state"] == "closed" && i["state_reason"] == "duplicate").then_some(()))
    })?;
    Ok(())
}

fn unpin(number: u64) -> Result<()> {
    let id = issue_graph(number)?["id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    if issue_graph(number)?["isPinned"] == true {
        gh::graphql(
            "mutation($id: ID!) { unpinIssue(input: {issueId: $id}) { clientMutationId } }",
            json!({"id": id}),
        )?;
    }
    Ok(())
}

fn minimized(id: &str) -> Result<Option<bool>> {
    let data = gh::graphql(
        "query($id: ID!) { node(id: $id) { ... on IssueComment { isMinimized } } }",
        json!({"id": id}),
    )?;
    Ok(data["node"]["isMinimized"].as_bool())
}

fn pull(cx: &mut Cx) -> Result<()> {
    let run = cx.env.run_id.clone();
    let base = format!("{}base-{run}", gh::BRANCH_PREFIX);
    let head = format!("{}head-{run}", gh::BRANCH_PREFIX);
    for branch in [&base, &head] {
        let undo = branch.clone();
        cx.defer(format!("delete branch {branch}"), move || {
            gh::drop_branch(&undo)
        });
    }
    gh::branch_with_file(&base, &format!("live-base-{run}.txt"), "A base\n")?;
    gh::branch_with_file(&head, &format!("live-head-{run}.txt"), "A change\n")?;
    let title = cx.mark("managed pull request");
    let number = gh::create_pr(&title, &head, "From ghtui's live check.")?;
    let url = format!("https://github.com/{}/pull/{number}", gh::REPO);
    let ghtui = cx.ghtui(&[&url])?;
    ghtui.wait_for("wants to merge into")?;
    ghtui.wait_followed()?;

    cx.step(",r converts it to a draft, after asking")?;
    ghtui.keys(", r")?;
    ghtui.wait_for("to a draft?")?;
    ghtui.keys("Enter")?;
    until("a draft on GitHub", GITHUB, || {
        Ok((pr(number)?["draft"] == true).then_some(()))
    })?;
    ghtui.wait_for("still a draft")?;
    ghtui.wait_followed()?;
    ghtui.keys(", r")?;
    until("ready on GitHub", GITHUB, || {
        Ok((pr(number)?["draft"] == false).then_some(()))
    })?;

    ghtui.wait_followed()?;
    cx.step("the menu changes its base, after asking")?;
    ghtui.keys("Space /")?;
    ghtui.type_text("base branch")?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Find a branch")?;
    ghtui.type_text(&base)?;
    // The branch was just made: wait for the list to have it as a row,
    // not only as what's typed.
    ghtui.wait_until("the base branch listed", GITHUB, |screen| {
        (screen.matches(base.as_str()).count() >= 2).then_some(())
    })?;
    ghtui.keys("Enter")?;
    ghtui.wait_for(&format!("#{number}'s base to"))?;
    ghtui.keys("Enter")?;
    until("the new base on GitHub", GITHUB, || {
        Ok((pr(number)?["base"]["ref"] == base.as_str()).then_some(()))
    })?;
    ghtui.wait_followed()?;

    cx.step(",b deletes its branch once merged, and restores it")?;
    gh::send(
        "PUT",
        &format!("repos/{}/pulls/{number}/merge", gh::REPO),
        Some(&json!({"merge_method": "merge"})),
    )?;
    ghtui.wait_for_within("merged into", Duration::from_secs(90))?;
    ghtui.wait_followed()?;
    let head_sha = pr(number)?["head"]["sha"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    ghtui.keys(", b")?;
    until("the branch gone from GitHub", GITHUB, || {
        Ok(gh::get(&format!("repos/{}/branches/{head}", gh::REPO))
            .is_err()
            .then_some(()))
    })?;
    ghtui.wait_for("branch deleted")?;
    ghtui.wait_followed()?;
    ghtui.keys(", b")?;
    until("the branch back on GitHub", GITHUB, || {
        Ok(gh::get(&format!("repos/{}/branches/{head}", gh::REPO))
            .ok()
            .filter(|b| b["commit"]["sha"] == head_sha.as_str()))
    })?;
    ghtui.wait_gone("branch deleted")?;

    ghtui.wait_followed()?;
    cx.step("the menu reverts it, after asking; the revert opens")?;
    ghtui.keys("Space /")?;
    ghtui.type_text("revert")?;
    ghtui.keys("Enter")?;
    ghtui.wait_for(&format!("Revert #{number}?"))?;
    ghtui.keys("Enter")?;
    let revert = until("the revert on GitHub", GITHUB, || {
        let open = gh::rest_list(&format!("repos/{}/pulls?state=open&base={base}", gh::REPO))?;
        Ok(open.into_iter().find(|p| {
            p["head"]["ref"]
                .as_str()
                .is_some_and(|r| r.starts_with(&format!("revert-{number}-")))
        }))
    })?;
    let revert_number = revert["number"].as_u64().context("no number")?;
    let revert_branch = revert["head"]["ref"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    cx.defer(format!("delete branch {revert_branch}"), move || {
        gh::drop_branch(&revert_branch)
    });
    ghtui.wait_for(&format!("#{revert_number}"))?;
    ghtui.wait_for("Revert")?;
    Ok(())
}

fn pr(number: u64) -> Result<Value> {
    gh::get(&format!("repos/{}/pulls/{number}", gh::REPO))
}

fn discussion(cx: &mut Cx) -> Result<()> {
    let repo = gh::graphql(
        "query($o: String!, $n: String!) { repository(owner: $o, name: $n) { id hasDiscussionsEnabled
            discussionCategories(first: 20) { nodes { id name } } } }",
        json!({"o": gh::OWNER, "n": gh::NAME}),
    )?;
    let repo = &repo["repository"];
    if repo["hasDiscussionsEnabled"] != true {
        return Err(Cx::skip(format!("{} has Discussions off", gh::REPO)));
    }
    let general = repo["discussionCategories"]["nodes"]
        .as_array()
        .and_then(|c| c.iter().find(|c| c["name"] == "General"))
        .and_then(|c| c["id"].as_str())
        .context("the repository has no General category")?
        .to_owned();
    let title = cx.mark("managed discussion");
    let made = gh::graphql(
        "mutation($r: ID!, $c: ID!, $t: String!) { createDiscussion(input: {repositoryId: $r, categoryId: $c, title: $t, body: \"From ghtui's live check.\"}) { discussion { id number } } }",
        json!({"r": repo["id"], "c": general, "t": title}),
    )?;
    let made = &made["createDiscussion"]["discussion"];
    let id = made["id"].as_str().context("no discussion id")?.to_owned();
    let number = made["number"].as_u64().context("no number")?;
    let undo = id.clone();
    cx.defer("delete the discussion", move || delete_discussion(&undo));
    let url = format!("https://github.com/{}/discussions/{number}", gh::REPO);
    let ghtui = cx.ghtui(&[&url])?;
    ghtui.wait_for(&title)?;
    ghtui.wait_followed()?;

    cx.step(",L locks it, without a reason")?;
    ghtui.keys(", L")?;
    ghtui.wait_for("Lock without a reason")?;
    ghtui.keys("Enter")?;
    until("locked on GitHub", GITHUB, || {
        Ok((discussion_graph(&id)?["locked"] == true).then_some(()))
    })?;
    ghtui.wait_for("Locked")?;
    ghtui.wait_followed()?;
    ghtui.keys(", L")?;
    ghtui.wait_for("Unlock the conversation")?;
    ghtui.keys("Enter")?;
    until("unlocked on GitHub", GITHUB, || {
        Ok((discussion_graph(&id)?["locked"] == false).then_some(()))
    })?;

    ghtui.wait_followed()?;
    cx.step(",x closes it as outdated, after asking, and reopens it")?;
    ghtui.keys(", x")?;
    ghtui.wait_for("Close as outdated")?;
    ghtui.keys("Tab Enter")?;
    until("closed on GitHub", GITHUB, || {
        let d = discussion_graph(&id)?;
        Ok((d["closed"] == true && d["stateReason"] == "OUTDATED").then_some(()))
    })?;
    ghtui.wait_for("Closed as outdated")?;
    ghtui.wait_followed()?;
    ghtui.keys(", x")?;
    ghtui.wait_for("Reopen")?;
    ghtui.keys("Enter")?;
    until("reopened on GitHub", GITHUB, || {
        Ok((discussion_graph(&id)?["closed"] == false).then_some(()))
    })?;
    ghtui.wait_gone("Closed as outdated")?;

    ghtui.wait_followed()?;
    cx.step("the menu starts a new discussion in a category")?;
    let list = format!("https://github.com/{}/discussions", gh::REPO);
    let ghtui = cx.ghtui(&[&list])?;
    ghtui.wait_for(&title)?;
    ghtui.keys("Space /")?;
    ghtui.type_text("start a new discussion")?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("choose a category")?;
    ghtui.type_text("General")?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("New discussion in")?;
    let started = cx.mark("started discussion");
    ghtui.type_text(&started)?;
    ghtui.keys("Enter")?;
    ghtui.type_text("Started by ghtui's live check.")?;
    ghtui.keys("C-s")?;
    let found = until("the new discussion on GitHub", GITHUB, || {
        let data = gh::graphql(
            "query($o: String!, $n: String!) { repository(owner: $o, name: $n) {
                discussions(first: 20, orderBy: {field: CREATED_AT, direction: DESC}) { nodes { id title body category { name } } } } }",
            json!({"o": gh::OWNER, "n": gh::NAME}),
        )?;
        Ok(data["repository"]["discussions"]["nodes"]
            .as_array()
            .and_then(|d| d.iter().find(|d| d["title"] == started.as_str()).cloned()))
    })?;
    let new_id = found["id"].as_str().unwrap_or_default().to_owned();
    cx.defer("delete the new discussion", move || {
        delete_discussion(&new_id)
    });
    ensure!(
        found["category"]["name"] == "General" && found["body"] == "Started by ghtui's live check.",
        "it's {found}"
    );
    ghtui.wait_for("Started by ghtui's live check.")?;
    Ok(())
}

fn discussion_graph(id: &str) -> Result<Value> {
    let data = gh::graphql(
        "query($id: ID!) { node(id: $id) { ... on Discussion { locked closed stateReason } } }",
        json!({"id": id}),
    )?;
    Ok(data["node"].clone())
}

fn delete_discussion(id: &str) -> Result<()> {
    gh::graphql(
        "mutation($id: ID!) { deleteDiscussion(input: {id: $id}) { clientMutationId } }",
        json!({"id": id}),
    )?;
    Ok(())
}

/// Moving an issue needs a second repository of the owner's: one made
/// for it and deleted after, which takes the `delete_repo` scope.
fn transfer(cx: &mut Cx) -> Result<()> {
    if !cx.env.scopes.iter().any(|s| s == "delete_repo") {
        return Err(Cx::skip(
            "transferring needs a repository made and deleted for it: the delete_repo scope (gh auth refresh -s delete_repo)",
        ));
    }
    let name = format!("{}transfer-{}", gh::BRANCH_PREFIX, cx.env.run_id);
    gh::send(
        "POST",
        "user/repos",
        Some(&json!({"name": name, "private": true, "has_issues": true})),
    )?;
    let to = format!("{}/{name}", gh::OWNER);
    let undo = to.clone();
    cx.defer(format!("delete {to}"), move || {
        gh::send("DELETE", &format!("repos/{undo}"), None).map(drop)
    });
    let number = gh::create_issue(&cx.mark("moved issue"), "From ghtui's live check.")?;
    let url = format!("https://github.com/{}/issues/{number}", gh::REPO);
    let ghtui = cx.ghtui(&[&url])?;
    ghtui.wait_for("opened this issue")?;
    ghtui.wait_followed()?;
    cx.step("the menu transfers it, after asking")?;
    ghtui.keys("Space /")?;
    ghtui.type_text("transfer")?;
    ghtui.keys("Enter")?;
    ghtui.wait_for(&to)?;
    ghtui.type_text(&name)?;
    ghtui.keys("Enter")?;
    ghtui.wait_for(&format!("to {to}?"))?;
    ghtui.keys("Enter")?;
    let moved = until("the issue in the other repository", GITHUB, || {
        let issues = gh::rest_list(&format!("repos/{to}/issues?state=all"))?;
        Ok(issues.into_iter().next())
    })?;
    let moved = moved["number"].as_u64().context("no number")?;
    // The page goes with it: the other repository's issue, under its
    // number there (the "Moved …" notice may have gone by now).
    let (owner, repo) = to.split_once('/').context("an owner/name")?;
    ghtui.wait_for(&format!("{owner} / {repo}"))?;
    ghtui.wait_for(&format!("#{moved}"))?;
    Ok(())
}

/// Branches a killed run left: an issue's (`80-ghtui-live-…`) and a
/// revert's (`revert-78-ghtui-live-…`), which don't start with the
/// prefix other sweeps look for.
fn sweep() -> Result<()> {
    for b in gh::rest_list(&format!("repos/{}/branches", gh::REPO))? {
        let Some(name) = b["name"].as_str() else {
            continue;
        };
        if name.contains(gh::BRANCH_PREFIX) && !name.starts_with(gh::BRANCH_PREFIX) {
            gh::drop_branch(name)?;
        }
    }
    Ok(())
}
