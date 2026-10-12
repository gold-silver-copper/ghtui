//! Pull requests from a clone: proposing its branch (`,p`), pushing it
//! (`,P`) and creating the pull request (`,s`); checking one out (`,c`),
//! refused while the tree is dirty.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};

use crate::live::{Check, Cx, Launch, gh, path, until};

pub const CREATE: Check = Check::new(
    "pr-create",
    "a pull request from a clone's branch: ,p proposes it, ,P pushes the branch, ,s creates it",
    create,
);

pub const CHECKOUT: Check = Check::new(
    "checkout",
    ",c checks a pull request out in a clone, and refuses while the tree is dirty",
    checkout,
);

const GITHUB: Duration = Duration::from_secs(30);

fn create(cx: &mut Cx) -> Result<()> {
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
    let undo_branch = branch.clone();
    cx.defer(format!("delete branch {branch}"), move || {
        // Closing its pull request first; deleting the branch alone also
        // closes it.
        for pr in gh::rest_list(&format!(
            "repos/{}/pulls?head={}:{undo_branch}&state=open",
            gh::REPO,
            gh::OWNER
        ))? {
            if let Some(number) = pr["number"].as_u64() {
                gh::close_pr(number)?;
            }
        }
        match gh::get(&format!("repos/{}/branches/{undo_branch}", gh::REPO)) {
            Ok(_) => gh::delete_branch(&undo_branch),
            Err(_) => Ok(()),
        }
    });

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
    Ok(())
}

fn checkout(cx: &mut Cx) -> Result<()> {
    let clone = scratch_clone(cx, "clone-checkout")?;
    // A pull request the test repository keeps open for this.
    let number = 2;
    let pr = gh::get(&format!("repos/{}/pulls/{number}", gh::REPO))?;
    let (branch, head) = (
        pr["head"]["ref"].as_str().unwrap_or_default().to_owned(),
        pr["head"]["sha"].as_str().unwrap_or_default().to_owned(),
    );
    let readme = clone.join("README.md");
    let text = std::fs::read_to_string(&readme)?;
    std::fs::write(&readme, format!("{text}\nNot committed.\n"))?;

    let ghtui = cx.launch(Launch::new(&[&format!("{}#{number}", gh::REPO)]).cwd(&clone))?;
    ghtui.wait_for("wants to merge into")?;
    cx.step(",c refuses while README.md isn't committed, and names it")?;
    ghtui.keys(", c")?;
    ghtui.wait_for("Not checked out: README.md has changes that aren't committed")?;
    ensure!(
        git(&clone, &["branch", "--show-current"])? == "main",
        "the clone left main"
    );

    cx.step("with the tree clean, ,c switches to the pull request's branch")?;
    git(&clone, &["checkout", "-q", "README.md"])?;
    ghtui.keys(", c")?;
    until("the clone on the pull request's branch", GITHUB, || {
        let on = git(&clone, &["branch", "--show-current"])?;
        Ok((on == branch).then_some(()))
    })?;
    let at = git(&clone, &["rev-parse", "HEAD"])?;
    ensure!(at == head, "the branch is at {at}, GitHub's head is {head}");
    Ok(())
}

/// A fresh clone of the test repository, removed when the check ends.
pub fn scratch_clone(cx: &mut Cx, name: &str) -> Result<PathBuf> {
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
pub fn git(dir: &Path, args: &[&str]) -> Result<String> {
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
