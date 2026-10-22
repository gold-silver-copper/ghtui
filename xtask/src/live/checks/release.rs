//! A release: `,V` with a new tag, made when it's published. The release
//! and its tag are deleted after.

use std::time::Duration;

use anyhow::{Result, ensure};

use crate::live::{Check, Cx, gh, until};

pub const CHECK: Check = Check::new(
    "release",
    ":new release makes a release with a new tag, published; it and its tag are deleted after",
    run,
)
.sweep(sweep_releases);

const GITHUB: Duration = Duration::from_secs(30);

fn run(cx: &mut Cx) -> Result<()> {
    let tag = format!("{}{}", gh::BRANCH_PREFIX, cx.env.run_id);
    let title = cx.mark("release");
    let undo = tag.clone();
    cx.defer(format!("delete the release {tag} and its tag"), move || {
        delete_release(&undo)
    });
    let ghtui = cx.ghtui(&[gh::REPO])?;
    ghtui.wait_for("Pull requests")?;
    ghtui.wait_followed()?;

    cx.step(":new release starts with a tag: a new one, made when it's published")?;
    ghtui.palette("new release")?;
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
