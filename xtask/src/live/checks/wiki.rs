//! The wiki, written with git: `,W` writes a new page, which ghtui commits
//! in its clone of the wiki and pushes. GitHub makes a wiki's git
//! repository only with its first page, made on github.com, so without
//! one the check is skipped and says so. The page is removed after, with
//! git.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{Result, bail};

use crate::live::{Check, Cx, gh, until};

pub const CHECK: Check = Check::new(
    "wiki",
    ",W writes a wiki page, committed and pushed with git",
    run,
);

const GITHUB: Duration = Duration::from_secs(60);

fn wiki_url() -> String {
    format!("https://github.com/{}.wiki.git", gh::REPO)
}

fn run(cx: &mut Cx) -> Result<()> {
    if git(None, &["ls-remote", &wiki_url()]).is_err() {
        return Err(Cx::skip(format!(
            "the test repository's wiki has no git repository yet: GitHub makes it with the first page, made on github.com (https://github.com/{}/wiki)",
            gh::REPO
        )));
    }
    let title = format!("ghtui live {}", cx.env.run_id);
    let file = format!("{}.md", title.replace(' ', "-"));
    let work = cx.env.work.join("wiki");
    let undo = (work.clone(), file.clone());
    cx.defer("remove the wiki page", move || {
        remove_page(&undo.0, &undo.1)
    });

    let ghtui = cx.ghtui(&[&format!("https://github.com/{}/wiki", gh::REPO)])?;
    ghtui.wait_for("Wiki")?;
    ghtui.wait_followed()?;
    cx.step(",W writes a new page: its title on the first line")?;
    ghtui.keys(", W")?;
    ghtui.wait_for("open in $EDITOR")?;
    ghtui.type_text(&title)?;
    ghtui.keys("Enter")?;
    ghtui.type_text("Written by ghtui's live check.")?;
    ghtui.keys("C-s")?;
    until("the page on GitHub's wiki", GITHUB, || {
        let _ = std::fs::remove_dir_all(&work);
        git(
            None,
            &[
                "clone",
                "-q",
                "--depth",
                "1",
                &wiki_url(),
                &crate::live::path(&work),
            ],
        )?;
        let text = std::fs::read_to_string(work.join(&file)).unwrap_or_default();
        Ok(text
            .contains("Written by ghtui's live check.")
            .then_some(()))
    })?;
    ghtui.wait_for(&title)?;
    Ok(())
}

/// Deletes the page with git, from a fresh clone.
fn remove_page(work: &Path, file: &str) -> Result<()> {
    let _ = std::fs::remove_dir_all(work);
    git(
        None,
        &["clone", "-q", &wiki_url(), &crate::live::path(work)],
    )?;
    if !work.join(file).exists() {
        return Ok(());
    }
    git(Some(work), &["rm", "-q", file])?;
    git(
        Some(work),
        &[
            "-c",
            "user.name=ghtui live check",
            "-c",
            "user.email=ghtui-live@example.com",
            "commit",
            "-q",
            "-m",
            "Remove the live check's page",
        ],
    )?;
    git(Some(work), &["push", "-q"])?;
    Ok(())
}

fn git(dir: Option<&Path>, args: &[&str]) -> Result<String> {
    let mut cmd = Command::new("git");
    if let Some(dir) = dir {
        cmd.arg("-C").arg(dir);
    }
    let out = cmd.args(args).env("GIT_TERMINAL_PROMPT", "0").output()?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}
