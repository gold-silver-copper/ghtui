//! Projects: a throwaway project, linked to the test repository, with a
//! draft card and a board view. `,f` sets the card's Status in the table,
//! `,c` moves it to another column on the board. Another, empty one:
//! `,i` adds issue #8 and a new draft, `,X` archives #8 and restores it,
//! and its board's columns sit side by side at 216 columns, not at 90.
//! Needs the token's `project` scope (`gh auth refresh -s project`);
//! skipped without it.

use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde_json::json;

use crate::live::{Check, Cx, Launch, NARROW, gh, until};

pub const CHECK: Check = Check::new(
    "projects",
    "a throwaway project: ,f sets a card's field in the table, ,c moves it on the board",
    run,
)
.sweep(sweep);

const GITHUB: Duration = Duration::from_secs(30);

pub const ITEMS: Check = Check::new(
    "project-items",
    "a throwaway project: ,i adds issue #8 and a draft, ,X archives an item and restores it; a board's columns side by side",
    items,
);

/// A throwaway project linked to the test repository, deleted when the
/// check ends: its title, id and URL. Skipped without the project scope.
fn make_project(cx: &mut Cx, what: &str) -> Result<(String, String, String)> {
    if !cx.env.scopes.iter().any(|s| s == "project") {
        return Err(Cx::skip(
            "the token lacks the project scope (gh auth refresh -s project)",
        ));
    }
    let title = cx.mark(what);
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
    Ok((title, id, url))
}

fn run(cx: &mut Cx) -> Result<()> {
    cx.step("a project made with GraphQL: a draft card, and a board view")?;
    let (title, id, url) = make_project(cx, "project")?;
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

fn items(cx: &mut Cx) -> Result<()> {
    cx.step("an empty project with a board view")?;
    let (_, id, url) = make_project(cx, "items")?;
    gh::graphql(
        "mutation($p: ID!) { createProjectV2View(input: {projectId: $p, name: \"Board\", layout: BOARD_LAYOUT}) { projectV2View { id } } }",
        json!({"p": id}),
    )?;
    until("the board listed on GitHub", GITHUB, || {
        let data = gh::graphql(
            "query($id: ID!) { node(id: $id) { ... on ProjectV2 { views(first: 5) { totalCount } } } }",
            json!({"id": id}),
        )?;
        Ok((data["node"]["views"]["totalCount"] == 2).then_some(()))
    })?;
    let ghtui = cx.ghtui(&[&url])?;
    ghtui.wait_for("No items")?;
    ghtui.wait_followed()?;

    cx.step(",i adds issue #8 by its reference")?;
    let issue = format!("{}#8", gh::REPO);
    ghtui.keys(", i")?;
    ghtui.wait_for("Add an item")?;
    ghtui.type_text(&issue)?;
    ghtui.wait_for(&format!("Add {issue}"))?;
    ghtui.keys("Enter")?;
    let item = until("#8 in the project on GitHub", GITHUB, || {
        Ok(project_items(&id)?
            .into_iter()
            .find(|i| i["content"]["number"] == 8)
            .and_then(|i| i["id"].as_str().map(str::to_owned)))
    })?;
    ghtui.wait_for("Issue to close completed")?;

    ghtui.wait_followed()?;
    cx.step(",i writes a new draft in the composer")?;
    let draft = cx.mark("draft");
    ghtui.keys(", i")?;
    ghtui.wait_for("Add an item")?;
    ghtui.type_text(&draft)?;
    ghtui.wait_for(&format!("New draft “{draft}”"))?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("its title on the first line")?;
    ghtui.keys("Enter")?;
    ghtui.type_text("A draft's text.")?;
    ghtui.keys("C-s")?;
    until("the draft in the project on GitHub", GITHUB, || {
        Ok(project_items(&id)?
            .iter()
            .any(|i| {
                i["content"]["title"] == draft.as_str() && i["content"]["body"] == "A draft's text."
            })
            .then_some(()))
    })?;
    ghtui.wait_for(&draft)?;

    ghtui.wait_followed()?;
    cx.step(",X archives #8, after asking")?;
    ghtui.select("gg", "#8", |s| s.contains("⟦▌◉ Issue to close completed"))?;
    ghtui.keys(", X")?;
    ghtui.wait_for("Archive “Issue to close completed”?")?;
    ghtui.keys("Enter")?;
    until("#8 archived on GitHub", GITHUB, || {
        Ok((archived(&item)? == Some(true)).then_some(()))
    })?;
    ghtui.wait_for("Archived  1")?;

    ghtui.wait_followed()?;
    cx.step(",X on it under Archived restores it")?;
    ghtui.select("G", "#8 under Archived", |s| {
        s.contains("⟦▌◉ Issue to close completed")
    })?;
    ghtui.keys(", X")?;
    until("#8 restored on GitHub", GITHUB, || {
        Ok((archived(&item)? == Some(false)).then_some(()))
    })?;
    ghtui.wait_gone("Archived  1")?;

    cx.step("at 216 columns, the board's columns are side by side")?;
    ghtui.keys("g t")?;
    ghtui.wait_for("columns by Status")?;
    let side_by_side = |screen: &str| {
        screen
            .lines()
            .any(|l| l.contains("╭─") && l.contains(" Todo") && l.contains(" Done"))
    };
    ensure!(
        side_by_side(&ghtui.screen()?),
        "Todo and Done aren't side by side"
    );
    cx.step("at 90 columns, they don't all fit on one row")?;
    let narrow = cx.launch(Launch::new(&[&url]).size(NARROW))?;
    narrow.wait_for(&draft)?;
    narrow.keys("g t")?;
    narrow.wait_for("columns by Status")?;
    narrow.wait_for(" Done")?;
    ensure!(
        !side_by_side(&narrow.screen()?),
        "all of the board's columns are on one row at 90 columns"
    );
    Ok(())
}

/// The project's items, with what each is (an issue's number, a draft's
/// title and body).
fn project_items(id: &str) -> Result<Vec<serde_json::Value>> {
    let data = gh::graphql(
        "query($id: ID!) { node(id: $id) { ... on ProjectV2 { items(first: 20) { nodes { id
            content { ... on Issue { number } ... on DraftIssue { title body } } } } } } }",
        json!({"id": id}),
    )?;
    Ok(data["node"]["items"]["nodes"]
        .as_array()
        .cloned()
        .unwrap_or_default())
}

fn archived(item: &str) -> Result<Option<bool>> {
    let data = gh::graphql(
        "query($id: ID!) { node(id: $id) { ... on ProjectV2Item { isArchived } } }",
        json!({"id": item}),
    )?;
    Ok(data["node"]["isArchived"].as_bool())
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
