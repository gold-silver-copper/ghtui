//! Projects: a throwaway project, linked to the test repository, with a
//! draft card and a board view. `,f` sets the card's Status in the table,
//! `,c` moves it to another column on the board. Needs the token's
//! `project` scope (`gh auth refresh -s project`); skipped without it.

use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::json;

use crate::live::{Check, Cx, Launch, NARROW, gh, until};

pub const CHECK: Check = Check::new(
    "projects",
    "a throwaway project: ,f sets a card's field in the table, ,c moves it on the board",
    run,
)
.sweep(sweep);

const GITHUB: Duration = Duration::from_secs(30);

fn run(cx: &mut Cx) -> Result<()> {
    if !cx.env.scopes.iter().any(|s| s == "project") {
        return Err(Cx::skip(
            "the token lacks the project scope (gh auth refresh -s project)",
        ));
    }
    cx.step("a project made with GraphQL: a draft card, and a board view")?;
    let title = cx.mark("project");
    let ids = gh::graphql(
        "query($o: String!, $n: String!) { user(login: $o) { id } repository(owner: $o, name: $n) { id } }",
        json!({"o": gh::OWNER, "n": gh::NAME}),
    )?;
    let made = gh::graphql(
        "mutation($owner: ID!, $repo: ID!, $title: String!) {
            createProjectV2(input: {ownerId: $owner, repositoryId: $repo, title: $title}) {
                projectV2 { id url } } }",
        json!({"owner": ids["user"]["id"], "repo": ids["repository"]["id"], "title": title}),
    )?;
    let project = made["createProjectV2"]["projectV2"].clone();
    let id = project["id"].as_str().context("no project id")?.to_owned();
    let url = project["url"]
        .as_str()
        .context("no project url")?
        .to_owned();
    let undo = id.clone();
    cx.defer("delete the project", move || delete_project(&undo));
    let draft = cx.mark("draft");
    let added = gh::graphql(
        "mutation($p: ID!, $title: String!) {
            addProjectV2DraftIssue(input: {projectId: $p, title: $title, body: \"A *draft* card\"}) { projectItem { id } }
            createProjectV2View(input: {projectId: $p, name: \"Board\", layout: BOARD_LAYOUT}) { projectV2View { id } } }",
        json!({"p": id, "title": draft}),
    )?;
    let item = added["addProjectV2DraftIssue"]["projectItem"]["id"]
        .as_str()
        .context("no item id")?
        .to_owned();

    // GitHub lists a new project's items a moment after adding them.
    until("the card listed on GitHub", GITHUB, || {
        let data = gh::graphql(
            "query($id: ID!) { node(id: $id) { ... on ProjectV2 {
                items(first: 5) { totalCount } views(first: 5) { totalCount } } } }",
            json!({"id": id}),
        )?;
        let node = &data["node"];
        Ok((node["items"]["totalCount"] == 1 && node["views"]["totalCount"] == 2).then_some(()))
    })?;
    let ghtui = cx.ghtui(&[&url])?;
    ghtui.wait_for(&draft)?;

    cx.step(",f sets the card's Status in the table")?;
    ghtui.keys(", f")?;
    ghtui.wait_for("Set a field")?;
    ghtui.type_text("Status")?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Set the field")?;
    ghtui.type_text("In Progress")?;
    ghtui.keys("Enter")?;
    until("the Status on GitHub", GITHUB, || {
        Ok((status(&item)? == "In Progress").then_some(()))
    })?;

    cx.step(",c moves it to Done on the board")?;
    ghtui.keys("g t")?;
    ghtui.wait_for("columns by Status")?;
    ghtui.keys(", c")?;
    ghtui.wait_for("Move to column")?;
    ghtui.type_text("Done")?;
    ghtui.keys("Enter")?;
    until("the card in Done on GitHub", GITHUB, || {
        Ok((status(&item)? == "Done").then_some(()))
    })?;
    ghtui.wait_until("the card in Done on screen", GITHUB, |screen| {
        let column = screen
            .lines()
            .find(|l| l.contains("╭─") && l.contains(" Done "))?;
        column.contains(" 1 ").then_some(())
    })?;

    cx.step("at 90 columns, the header's crumbs end in …")?;
    let narrow = cx.launch(Launch::new(&[&url]).size(NARROW))?;
    narrow.wait_for(&draft)?;
    narrow.expect_whole_or_cut(&format!("gold-silver-copper / Projects / {title}"))?;
    narrow.keys(":")?;
    narrow.wait_for("A command")?;
    narrow
        .expect_whole_or_cut("A command, owner/repo, owner/repo#123, @user, a URL, or a search")?;
    Ok(())
}

/// The item's Status, or "" without one.
fn status(item: &str) -> Result<String> {
    let data = gh::graphql(
        "query($id: ID!) { node(id: $id) { ... on ProjectV2Item {
            fieldValueByName(name: \"Status\") { ... on ProjectV2ItemFieldSingleSelectValue { name } } } } }",
        json!({"id": item}),
    )?;
    Ok(data["node"]["fieldValueByName"]["name"]
        .as_str()
        .unwrap_or_default()
        .to_owned())
}

fn delete_project(id: &str) -> Result<()> {
    gh::graphql(
        "mutation($id: ID!) { deleteProjectV2(input: {projectId: $id}) { clientMutationId } }",
        json!({"id": id}),
    )?;
    Ok(())
}

/// Projects a killed run left.
fn sweep() -> Result<()> {
    let data = gh::graphql(
        "query($o: String!) { user(login: $o) { projectsV2(first: 50, query: \"ghtui live\") { nodes { id title } } } }",
        json!({"o": gh::OWNER}),
    );
    // Without the project scope there's nothing of ours to find.
    let Ok(data) = data else { return Ok(()) };
    let nodes = data["user"]["projectsV2"]["nodes"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for project in nodes {
        if project["title"]
            .as_str()
            .is_some_and(|t| t.starts_with("[ghtui live]"))
            && let Some(id) = project["id"].as_str()
        {
            delete_project(id)?;
        }
    }
    Ok(())
}
