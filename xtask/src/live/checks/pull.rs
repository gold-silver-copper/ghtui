//! A pull request from a clone: proposing its branch (`,p`), pushing it
//! (`,P`), creating the pull request as a draft (`,s`), and marking it
//! ready for review (`,r`).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};

use crate::live::{Check, Cx, Launch, gh, path, until};

pub const CHECK: Check = Check::new(
    "pull",
    "a pull request from a clone's branch: ,p proposes it, ,P pushes the branch, ,s creates it as a draft, ,r marks it ready",
    run,
);

const GITHUB: Duration = Duration::from_secs(30);

fn run(cx: &mut Cx) -> Result<()> {
    let clone = scratch_clone(cx, "clone-pr")?;
    let branch = format!("{}{}", gh::BRANCH_PREFIX, cx.env.run_id);
    let title = cx.mark("pull request");
    git(&clone, &["checkout", "-q", "-b", &branch])?;
    std::fs::write(
        clone.join(format!("live-{}.txt", cx.env.run_id)),
        "From ghtui's live check\n",
    )?;
    git(&clone, &["add", "."])?;
    git(&clone, &["commit", "-q", "-m", &title])?;
    let head = git(&clone, &["rev-parse", "HEAD"])?;
    let undo = branch.clone();
    cx.defer(
        format!("close the pull request, delete {branch}"),
        move || gh::drop_branch(&undo),
    );

    let ghtui = cx.launch(Launch::new(&[gh::REPO]).cwd(&clone))?;
    ghtui.wait_for("Pull requests")?;
    cx.step(",p proposes the branch, which GitHub doesn't have yet")?;
    ghtui.keys(", p")?;
    ghtui.wait_for(&format!("{branch} isn't on GitHub"))?;

    cx.step(",P pushes it, after asking")?;
    ghtui.keys(", P")?;
    ghtui.wait_for(&format!("Push {branch} to origin?"))?;
    ghtui.keys("Enter")?;
    until("the branch on GitHub", GITHUB, || {
        Ok(gh::get(&format!("repos/{}/branches/{branch}", gh::REPO))
            .ok()
            .filter(|b| b["commit"]["sha"] == head.as_str()))
    })?;
    ghtui.wait_for(&format!("{branch} is on GitHub as it is in your clone"))?;

    cx.step(",s creates it as a draft, after asking")?;
    ghtui.keys(", s")?;
    ghtui.wait_for("Create draft pull request")?;
    ghtui.keys("Tab Enter")?;
    let pr = until("the pull request on GitHub", GITHUB, || {
        let open = gh::rest_list(&format!(
            "repos/{}/pulls?head={}:{branch}&state=open",
            gh::REPO,
            gh::OWNER
        ))?;
        Ok(open.into_iter().next())
    })?;
    ensure!(
        pr["title"] == title.as_str(),
        "its title is {}",
        pr["title"]
    );
    ensure!(pr["draft"] == true, "it isn't a draft");
    ensure!(
        pr["base"]["ref"] == "main",
        "its base is {}",
        pr["base"]["ref"]
    );
    ghtui.wait_for("wants to merge into")?;
    ghtui.wait_for("still a draft")?;
    let number = pr["number"].as_u64().context("no pull request number")?;

    ghtui.wait_followed()?;
    cx.step(",r marks it ready for review")?;
    ghtui.keys(", r")?;
    until("ready on GitHub", GITHUB, || {
        let pr = gh::get(&format!("repos/{}/pulls/{number}", gh::REPO))?;
        Ok((pr["draft"] == false).then_some(()))
    })?;
    ghtui.wait_gone("still a draft")?;
    Ok(())
}

/// A fresh clone of the test repository, removed when the check ends.
fn scratch_clone(cx: &mut Cx, name: &str) -> Result<PathBuf> {
    let dir = cx.env.work.join(name);
    let url = format!("https://github.com/{}", gh::REPO);
    let out = Command::new("git")
        .args(["clone", "-q", &url])
        .arg(&dir)
        .output()
        .context("cannot run git")?;
    if !out.status.success() {
        bail!(
            "git clone failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    git(&dir, &["config", "user.name", "ghtui live check"])?;
    git(&dir, &["config", "user.email", "ghtui-live@example.com"])?;
    let undo = dir.clone();
    cx.defer(format!("remove {}", path(&dir)), move || {
        Ok(std::fs::remove_dir_all(&undo)?)
    });
    Ok(dir)
}

/// `git ARGS` in `dir`; its output, trimmed.
fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output()?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}
