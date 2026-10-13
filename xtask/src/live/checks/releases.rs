//! Changing a release from its page: `,U` uploads a file as an asset,
//! `↵` on it saves it (and asks before replacing a file), `,D` on it
//! deletes it; `,e` edits the release's title; `,D` deletes the release,
//! then, asked again, its tag. The release (made with the API) and its
//! tag are deleted after, whatever happened.

use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde_json::json;

use crate::live::{Check, Cx, gh, path, until};

pub const CHECK: Check = Check::new(
    "release-changes",
    "a release: ,U uploads an asset, ↵ saves it (asking before replacing), ,D deletes it; ,e edits the title; ,D deletes the release, then its tag",
    run,
);

const GITHUB: Duration = Duration::from_secs(30);

/// The release's assets' names, on GitHub.
fn assets(id: u64) -> Result<Vec<String>> {
    let listed = gh::rest_list(&format!("repos/{}/releases/{id}/assets", gh::REPO))?;
    Ok(listed
        .iter()
        .filter_map(|a| a["name"].as_str().map(str::to_owned))
        .collect())
}

fn run(cx: &mut Cx) -> Result<()> {
    cx.step("a release made with the API, at main")?;
    let tag = format!("{}{}-changes", gh::BRANCH_PREFIX, cx.env.run_id);
    let title = cx.mark("release changes");
    let undo = tag.clone();
    cx.defer(format!("delete the release {tag} and its tag"), move || {
        super::writes::delete_release(&undo)
    });
    let made = gh::send(
        "POST",
        &format!("repos/{}/releases", gh::REPO),
        Some(&json!({
            "tag_name": tag,
            "target_commitish": "main",
            "name": title,
            "body": "Made by the live check.",
            "make_latest": "false",
        })),
    )?;
    let id = made["id"].as_u64().context("no release id")?;
    let ghtui = cx.ghtui(&[&format!(
        "https://github.com/{}/releases/tag/{tag}",
        gh::REPO
    )])?;
    ghtui.wait_for(&title)?;
    ghtui.wait_for("No assets.")?;
    ghtui.wait_followed()?;

    cx.step(",U uploads a file, by its path, as an asset")?;
    let name = format!("{}{}.txt", gh::BRANCH_PREFIX, cx.env.run_id);
    let dir = cx.env.work.join("assets");
    std::fs::create_dir_all(dir.join("saved"))?;
    let file = dir.join(&name);
    let text = format!("An asset from the live check {}\n", cx.env.run_id);
    std::fs::write(&file, &text)?;
    ghtui.keys(", U")?;
    ghtui.wait_for("Upload an asset")?;
    ghtui.type_text(&path(&file))?;
    ghtui.wait_for(&format!("Upload {}", path(&file)))?;
    ghtui.keys("Enter")?;
    until("the asset on GitHub", GITHUB, || {
        Ok(assets(id)?.contains(&name).then_some(()))
    })?;
    ghtui.wait_for(&name)?;
    ghtui.wait_for("0 downloads")?;

    ghtui.wait_followed()?;
    cx.step("↵ on it saves it where it's told, then asks before replacing it")?;
    let saved = dir.join("saved");
    let into = format!("{}/", path(&saved));
    ghtui.select("G", "the asset", |s| {
        s.lines().any(|l| l.contains(&name) && l.contains('⟦'))
    })?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Save the asset")?;
    ghtui.type_text(&into)?;
    ghtui.wait_for(&format!("Save to {into}{name}"))?;
    ghtui.keys("Enter")?;
    let at = saved.join(&name);
    until("the asset saved", GITHUB, || {
        Ok((std::fs::read_to_string(&at).ok().as_deref() == Some(text.as_str())).then_some(()))
    })?;
    ghtui.wait_for(&format!("Saved {name}"))?;
    ghtui.keys("Enter")?;
    ghtui.wait_for("Save the asset")?;
    ghtui.type_text(&into)?;
    ghtui.wait_for(&format!("Replace {into}{name}"))?;
    ghtui.keys("Escape")?;
    ghtui.wait_gone("Save the asset")?;

    cx.step(",D on the asset deletes it, after asking")?;
    ghtui.keys(", D")?;
    ghtui.wait_for(&format!("Delete {name}?"))?;
    ghtui.keys("Enter")?;
    until("the asset gone on GitHub", GITHUB, || {
        Ok((!assets(id)?.contains(&name)).then_some(()))
    })?;
    ghtui.wait_for("No assets.")?;

    ghtui.wait_followed()?;
    cx.step(",e edits the release's title, and the page follows")?;
    ghtui.keys(", e")?;
    ghtui.wait_for(&format!("Editing {tag}"))?;
    ghtui.keys("c")?;
    ghtui.wait_for("its title on the first line")?;
    ghtui.type_text(" (edited)")?;
    ghtui.keys("C-s")?;
    ghtui.wait_for(&format!("Update {tag}?"))?;
    ghtui.keys("Enter")?;
    let edited = format!("{title} (edited)");
    until("the title on GitHub", GITHUB, || {
        let release = gh::get(&format!("repos/{}/releases/{id}", gh::REPO))?;
        Ok((release["name"] == edited.as_str()).then_some(()))
    })?;
    ghtui.wait_for(&edited)?;

    ghtui.wait_followed()?;
    cx.step(",D deletes the release, after asking; deleting its tag is asked apart")?;
    ghtui.keys("g g")?;
    ghtui.keys(", D")?;
    ghtui.wait_for(&format!("Delete the release {edited}?"))?;
    ghtui.keys("Enter")?;
    until("the release gone on GitHub", GITHUB, || {
        let gone = gh::get(&format!("repos/{}/releases/{id}", gh::REPO)).is_err();
        Ok(gone.then_some(()))
    })?;
    ghtui.wait_for(&format!("Delete the tag {tag} as well?"))?;
    let kept = gh::get(&format!("repos/{}/git/ref/tags/{tag}", gh::REPO));
    ensure!(kept.is_ok(), "the tag went with the release, before asking");
    ghtui.keys("Enter")?;
    until("the tag gone on GitHub", GITHUB, || {
        let gone = gh::get(&format!("repos/{}/git/ref/tags/{tag}", gh::REPO)).is_err();
        Ok(gone.then_some(()))
    })?;
    ghtui.wait_for(&format!("Deleted the tag {tag}"))?;
    Ok(())
}
