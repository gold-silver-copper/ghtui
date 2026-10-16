//! Repository housekeeping, on the labels page: `,o` makes a label with a
//! colour, `,D` deletes it after asking.

use std::time::Duration;

use anyhow::{Result, bail, ensure};

use crate::live::fux::Pane;
use crate::live::{Check, Cx, gh, until};

pub const CHECK: Check = Check::new(
    "labels",
    ",o makes a label on the labels page, ,D deletes it after asking",
    run,
)
.sweep(sweep);

const GITHUB: Duration = Duration::from_secs(60);

/// Whether the selected row starts with `name` (a label's chip, a
/// form's row), on the marked screen.
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

fn run(cx: &mut Cx) -> Result<()> {
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

/// Labels a killed run left.
fn sweep() -> Result<()> {
    for label in gh::rest_list(&format!("repos/{}/labels?per_page=100", gh::REPO))? {
        if let Some(name) = label["name"].as_str()
            && name.starts_with(gh::BRANCH_PREFIX)
        {
            delete_label(name)?;
        }
    }
    Ok(())
}
