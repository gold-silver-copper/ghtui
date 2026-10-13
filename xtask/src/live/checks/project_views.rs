//! A throwaway project with issue #8 in it, start and target dates, and
//! a roadmap view: `,f` on #8's own page sets its Status there, from its
//! sidebar, and the sidebar follows; the roadmap places #8 on a timeline
//! between its dates. (A board's swimlanes are set on github.com only:
//! GitHub's API can't group a view.) Needs the token's `project` scope.

use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::live::{Check, Cx, gh, until};

pub const CHECK: Check = Check::new(
    "project-sidebar",
    "a throwaway project with #8: ,f on #8's page sets its Status from the sidebar; a roadmap view puts #8 on a timeline by its dates",
    run,
);

const GITHUB: Duration = Duration::from_secs(30);
const ISSUE: u64 = 8;
const DAY: u64 = 86_400;

/// `2026-10-14` for Unix seconds.
fn date(secs: u64) -> String {
    gh::iso8601(secs).chars().take(10).collect()
}

/// `Oct 14 → 31`, or `Oct 28 → Nov 3`, as the roadmap writes a span.
fn span(start: &str, end: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let parts = |d: &str| -> (String, u32) {
        let month: usize = d.get(5..7).and_then(|m| m.parse().ok()).unwrap_or(1);
        let day = d.get(8..10).and_then(|m| m.parse().ok()).unwrap_or(1);
        let name = MONTHS.get(month.saturating_sub(1)).copied().unwrap_or("");
        (name.to_owned(), day)
    };
    let ((m1, d1), (m2, d2)) = (parts(start), parts(end));
    if m1 == m2 {
        format!("{m1} {d1} → {d2}")
    } else {
        format!("{m1} {d1} → {m2} {d2}")
    }
}

fn field(project: &str, name: &str) -> Result<String> {
    let made = gh::graphql(
        "mutation($p: ID!, $name: String!) { createProjectV2Field(input: {projectId: $p, dataType: DATE, name: $name}) {
            projectV2Field { ... on ProjectV2Field { id } } } }",
        json!({"p": project, "name": name}),
    )?;
    Ok(made["createProjectV2Field"]["projectV2Field"]["id"]
        .as_str()
        .context("no field id")?
        .to_owned())
}

fn set_date(project: &str, item: &str, field: &str, date: &str) -> Result<Value> {
    gh::graphql(
        "mutation($p: ID!, $i: ID!, $f: ID!, $d: Date!) { updateProjectV2ItemFieldValue(input: {projectId: $p, itemId: $i, fieldId: $f, value: {date: $d}}) { projectV2Item { id } } }",
        json!({"p": project, "i": item, "f": field, "d": date}),
    )
}

fn run(cx: &mut Cx) -> Result<()> {
    cx.step("a project with #8 in it, its dates set, and a roadmap view")?;
    let (title, id, url) = super::projects::make_project(cx, "sidebar")?;
    let issue = gh::graphql(
        "query($o: String!, $n: String!, $i: Int!) { repository(owner: $o, name: $n) { issue(number: $i) { id } } }",
        json!({"o": gh::OWNER, "n": gh::NAME, "i": ISSUE}),
    )?;
    let added = gh::graphql(
        "mutation($p: ID!, $c: ID!) { addProjectV2ItemById(input: {projectId: $p, contentId: $c}) { item { id } } }",
        json!({"p": id, "c": issue["repository"]["issue"]["id"]}),
    )?;
    let item = added["addProjectV2ItemById"]["item"]["id"]
        .as_str()
        .context("no item id")?
        .to_owned();
    let now = gh::epoch();
    let (start, end) = (date(now + 2 * DAY), date(now + 20 * DAY));
    let start_field = field(&id, "Start date")?;
    let end_field = field(&id, "Target date")?;
    set_date(&id, &item, &start_field, &start)?;
    set_date(&id, &item, &end_field, &end)?;
    gh::graphql(
        "mutation($p: ID!) { createProjectV2View(input: {projectId: $p, name: \"Roadmap\", layout: ROADMAP_LAYOUT}) { projectV2View { id } } }",
        json!({"p": id}),
    )?;
    until("#8 and the roadmap on GitHub", GITHUB, || {
        let data = gh::graphql(
            "query($id: ID!) { node(id: $id) { ... on ProjectV2 { items(first: 5) { totalCount } views(first: 5) { totalCount } } } }",
            json!({"id": id}),
        )?;
        let node = &data["node"];
        Ok((node["items"]["totalCount"] == 1 && node["views"]["totalCount"] == 2).then_some(()))
    })?;

    cx.step("#8's sidebar names the project; ,f sets its Status there")?;
    let ghtui = cx.ghtui(&[&format!("{}#{ISSUE}", gh::REPO)])?;
    // The sidebar may wrap a long title: its start is enough.
    let named = title.get(..20).unwrap_or(&title);
    ghtui.wait_for(named)?;
    ghtui.wait_followed()?;
    ghtui.keys(", f")?;
    // In another project too, it asks which first.
    let asked = ghtui.wait_until("the field picker", GITHUB, |screen| {
        if screen.contains("Set a field in a project") {
            Some(true)
        } else {
            screen.contains("Set a field").then_some(false)
        }
    })?;
    if asked {
        ghtui.type_text(&title)?;
        ghtui.keys("Enter")?;
        ghtui.wait_for("Set a field")?;
    }
    ghtui.type_text("Status")?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Set the field")?;
    ghtui.type_text("In Progress")?;
    ghtui.keys("Enter")?;
    until("the Status on GitHub", GITHUB, || {
        Ok((super::projects::status(&item)? == "In Progress").then_some(()))
    })?;
    ghtui.wait_for("· In Progress")?;

    cx.step("the roadmap puts #8 on a timeline from its start to its target date")?;
    let ghtui = cx.ghtui(&[&format!("{url}/views/2")])?;
    ghtui.wait_for("On a timeline by Start date → Target date")?;
    ghtui.wait_for("Issue to close completed")?;
    ghtui.wait_for(&span(&start, &end))?;
    Ok(())
}
