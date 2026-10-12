//! What github.com shows beside the comments, on throwaway issues and a
//! pull request: an issue closed and reopened while it's open in ghtui
//! (live: who did it shows), its sub-issue and progress, the pull request
//! that will close it (in its timeline and Development, and the pull
//! request's Closes), a commit comment written with `c` on the commit's
//! page, and the issue pinned at the top of the Issues tab.

use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde_json::json;

use crate::live::{Check, Cx, Launch, NARROW, gh, until};

pub const CHECK: Check = Check::new(
    "timeline",
    "an issue closed and reopened shows who did it, live; its sub-issue and progress; the pull request that will close it; a commit comment; pinned issues",
    run,
);

const GITHUB: Duration = Duration::from_secs(30);
/// An issue's page asks every 10s, doubling while nothing changes.
const LIVE: Duration = Duration::from_secs(120);

fn run(cx: &mut Cx) -> Result<()> {
    let login = cx.env.login.clone();
    cx.step("an issue with a sub-issue")?;
    let parent_title = cx.mark("parent");
    let parent = gh::create_issue(&parent_title, "An issue for the live check's timeline.")?;
    cx.defer(format!("delete issue #{parent}"), move || {
        gh::delete_issue(parent)
    });
    let child_title = cx.mark("sub-issue");
    let child = gh::create_issue(&child_title, "A sub-issue.")?;
    cx.defer(format!("delete issue #{child}"), move || {
        gh::delete_issue(child)
    });
    let child_id = gh::issue(child)?["id"].clone();
    gh::send(
        "POST",
        &format!("repos/{}/issues/{parent}/sub_issues", gh::REPO),
        Some(&json!({"sub_issue_id": child_id})),
    )?;

    let ghtui = cx.ghtui(&[&format!("{}#{parent}", gh::REPO)])?;
    ghtui.wait_for(&parent_title)?;
    ghtui.wait_for("Sub-issues · 0 of 1 done")?;
    ghtui.wait_for(&format!("◉ #{child} [ghtui live] sub-issue"))?;
    ghtui.wait_followed()?;

    cx.step("closed and reopened on GitHub: who did it shows, live")?;
    set_state(parent, "closed")?;
    ghtui.wait_for_within(&format!("{login} closed this as completed"), LIVE)?;
    set_state(parent, "open")?;
    ghtui.wait_for_within(&format!("{login} reopened this"), LIVE)?;

    cx.step("a pull request that will close it: the timeline, Development, and its Closes")?;
    let branch = format!("{}timeline-{}", gh::BRANCH_PREFIX, cx.env.run_id);
    let undo = branch.clone();
    cx.defer(
        format!("close the pull request, delete {branch}"),
        move || gh::drop_branch(&undo),
    );
    let head = gh::branch_with_file(&branch, "timeline.txt", "From the live check\n")?;
    let pr = gh::create_pr(&cx.mark("closes"), &branch, &format!("Closes #{parent}"))?;
    until("GitHub to link the pull request", GITHUB, || {
        let data = gh::graphql(
            "query($o: String!, $n: String!, $i: Int!) { repository(owner: $o, name: $n) {
                issue(number: $i) { closedByPullRequestsReferences(first: 5) { nodes { number } } } } }",
            json!({"o": gh::OWNER, "n": gh::NAME, "i": parent}),
        )?;
        let linked = data["repository"]["issue"]["closedByPullRequestsReferences"]["nodes"]
            .as_array()
            .is_some_and(|n| n.iter().any(|n| n["number"] == pr));
        Ok(linked.then_some(()))
    })?;
    ghtui.keys("r")?;
    ghtui.wait_for(&format!(
        "{login} linked a pull request that will close this issue"
    ))?;
    let development = aside_after(&ghtui.screen()?, "Development");
    ensure!(
        development.contains(&format!("#{pr}")),
        "Development doesn't list #{pr}: {development}"
    );

    cx.step("its sub-issue closed: the progress counts it")?;
    set_state(child, "closed")?;
    ghtui.keys("r")?;
    ghtui.wait_for("Sub-issues · 1 of 1 done")?;

    cx.step("at 90 columns, the sidebar's connections are said above the conversation")?;
    let narrow = cx.launch(Launch::new(&[&format!("{}#{parent}", gh::REPO)]).size(NARROW))?;
    narrow.wait_for(&format!("#{pr}"))?;
    narrow.wait_for(&format!("{login} reopened this"))?;

    cx.step("the pull request's sidebar says what it closes")?;
    let ghtui = cx.ghtui(&[&format!("{}#{pr}", gh::REPO)])?;
    ghtui.wait_for("wants to merge into")?;
    ghtui.wait_until("Closes with the issue", GITHUB, |screen| {
        aside_after(screen, "Closes")
            .contains(&format!("#{parent}"))
            .then_some(())
    })?;

    cx.step("c writes a comment on the pull request's commit")?;
    let url = format!("https://github.com/{}/commit/{head}", gh::REPO);
    let ghtui = cx.ghtui(&[&url])?;
    ghtui.wait_for("Add a comment")?;
    let comment = format!("A commit comment from the live check {}", cx.env.run_id);
    ghtui.keys("c")?;
    ghtui.wait_for(&format!("Comment on commit {}", gh::short(&head)))?;
    ghtui.type_text(&comment)?;
    ghtui.keys("C-s")?;
    let id = until("the commit comment on GitHub", GITHUB, || {
        let comments = gh::rest_list(&format!("repos/{}/commits/{head}/comments", gh::REPO))?;
        Ok(comments
            .iter()
            .find(|c| c["body"] == comment.as_str() && c["user"]["login"] == login.as_str())
            .and_then(|c| c["id"].as_u64()))
    })?;
    cx.defer("delete the commit comment", move || {
        gh::send("DELETE", &format!("repos/{}/comments/{id}", gh::REPO), None)?;
        Ok(())
    });
    ghtui.wait_for("Comments · 1")?;
    ghtui.wait_for(&comment)?;

    cx.step("pinned, the issue heads the Issues tab")?;
    let node = gh::issue(parent)?["node_id"]
        .as_str()
        .context("no node id")?
        .to_owned();
    // GitHub sometimes answers pinning a new issue with "Something went
    // wrong"; pinning again is harmless.
    until("the issue pinned on GitHub", GITHUB, || {
        let pinned = gh::graphql(
            "mutation($id: ID!) { pinIssue(input: {issueId: $id}) { issue { isPinned } } }",
            json!({"id": node}),
        );
        Ok(pinned
            .ok()
            .filter(|p| p["pinIssue"]["issue"]["isPinned"] == true)
            .map(drop))
    })?;
    cx.defer("unpin the issue", move || {
        // Deleting it (the cleanup before this one runs after) unpins it
        // too; unpinning first leaves nothing pinned if that fails.
        gh::graphql(
            "mutation($id: ID!) { unpinIssue(input: {issueId: $id}) { issue { isPinned } } }",
            json!({"id": node}),
        )?;
        Ok(())
    });
    let ghtui = cx.ghtui(&[&format!("https://github.com/{}/issues", gh::REPO)])?;
    ghtui.wait_for("Pinned issues")?;
    ghtui.wait_for(&format!("⚲ ◉ #{parent} {parent_title}"))?;
    Ok(())
}

fn set_state(number: u64, state: &str) -> Result<()> {
    gh::send(
        "PATCH",
        &format!("repos/{}/issues/{number}", gh::REPO),
        Some(&json!({"state": state})),
    )?;
    Ok(())
}

/// The sidebar's lines under `heading`, up to the next blank one, as
/// one text: what's on screen from the heading's column rightwards.
fn aside_after(screen: &str, heading: &str) -> String {
    let mut lines = screen.lines();
    let Some(column) = lines.by_ref().find_map(|l| {
        let at = l.rfind(&format!("   {heading}"))? + 3;
        Some(l.get(..at)?.chars().count())
    }) else {
        return String::new();
    };
    let mut out = String::new();
    for line in lines {
        let rest: String = line.chars().skip(column).collect();
        if rest.trim().is_empty() {
            break;
        }
        out.push_str(rest.trim());
        out.push('\n');
    }
    out
}
