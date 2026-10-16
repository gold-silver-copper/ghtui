//! The user's own clone, where ghtui runs: its branches against GitHub,
//! pushing one, and checking out a pull request. Unlike the diff's work
//! ([`crate::git::repo`]), this changes the clone, so only what you ask
//! for runs, with your own git and its credentials.

use std::path::{Path, PathBuf};

use crate::git::{GitError, Oid, github_repo_from_url, remotes, run};

/// A clone of a GitHub repository: where it is, the remote that is that
/// repository, and the branch checked out (none when detached).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clone {
    pub top: PathBuf,
    pub remote: String,
    pub branch: Option<String>,
}

/// The clone `dir` is in, if one of its remotes is `owner/name`.
pub async fn clone_of(dir: &Path, owner: &str, name: &str) -> Result<Option<Clone>, GitError> {
    let Ok(top) = run(Some(dir), &["rev-parse", "--show-toplevel"]).await else {
        return Ok(None);
    };
    let top = PathBuf::from(top.trim());
    let remotes = remotes(&top).await?;
    let Some(remote) = remotes.into_iter().find(|r| {
        github_repo_from_url(&r.url).is_some_and(|repo| {
            repo.owner.eq_ignore_ascii_case(owner) && repo.name.eq_ignore_ascii_case(name)
        })
    }) else {
        return Ok(None);
    };
    let branch = run(Some(&top), &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .await
        .ok()
        .map(|b| b.trim().to_owned())
        .filter(|b| !b.is_empty());
    Ok(Some(Clone {
        top,
        remote: remote.name,
        branch,
    }))
}

/// A commit, as a pull request is filled in from: its ID and message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitMessage {
    pub oid: String,
    pub message: String,
}

impl CommitMessage {
    pub fn subject(&self) -> &str {
        self.message.lines().next().unwrap_or_default().trim()
    }
}

/// A local branch against GitHub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub head: Oid,
    /// Where it's pushed: its push remote, and that remote's GitHub owner
    /// (a fork's, for a pull request from a fork).
    pub remote: String,
    pub owner: Option<String>,
    /// Its tip on GitHub; none when GitHub doesn't have it.
    pub on_github: Option<Oid>,
    /// Commits here that GitHub's doesn't have, and the other way round,
    /// when GitHub has it.
    pub ahead: Option<u64>,
    pub behind: Option<u64>,
    /// It already has an upstream to push to.
    pub tracked: bool,
}

impl Branch {
    /// GitHub has every commit it has here (and maybe more).
    pub fn pushed(&self) -> bool {
        self.on_github.is_some() && self.ahead == Some(0)
    }
}

/// The value of a git config key, if set.
async fn config(top: &Path, key: &str) -> Option<String> {
    let value = run(Some(top), &["config", "--get", key]).await.ok()?;
    Some(value.trim().to_owned()).filter(|v| !v.is_empty())
}

/// The commit `rev` names, if it names one here.
async fn commit(top: &Path, rev: &str) -> Option<Oid> {
    let rev = format!("{rev}^{{commit}}");
    let out = run(Some(top), &["rev-parse", "--verify", "--quiet", &rev]).await;
    Oid::parse(out.ok()?.trim()).ok()
}

/// Branch `name` against GitHub, asking the remote it's pushed to (as
/// `git push` would choose it: its push remote, the default one, its
/// upstream's, else `fallback`). None if there's no such local branch.
pub async fn branch(top: &Path, fallback: &str, name: &str) -> Result<Option<Branch>, GitError> {
    let Some(head) = commit(top, &format!("refs/heads/{name}")).await else {
        return Ok(None);
    };
    let upstream = config(top, &format!("branch.{name}.remote")).await;
    let remote = match config(top, &format!("branch.{name}.pushRemote")).await {
        Some(remote) => remote,
        None => match config(top, "remote.pushDefault").await {
            Some(remote) => remote,
            None => upstream.clone().unwrap_or_else(|| fallback.to_owned()),
        },
    };
    let url = config(top, &format!("remote.{remote}.url")).await;
    let owner = url.and_then(|u| github_repo_from_url(&u)).map(|r| r.owner);
    // GitHub's copy, fetched to compare with (none when it has none).
    let into = format!("refs/ghtui/remote/{remote}/{name}");
    let spec = format!("+refs/heads/{name}:{into}");
    let fetched = run(
        Some(top),
        &[
            "fetch",
            "--no-tags",
            "--no-write-fetch-head",
            &remote,
            &spec,
        ],
    )
    .await;
    let on_github = match fetched {
        Ok(_) => commit(top, &into).await,
        Err(GitError::Failed { stderr, .. }) if stderr.contains("couldn't find remote ref") => None,
        Err(err) => return Err(err),
    };
    let (mut ahead, mut behind) = (None, None);
    if let Some(there) = &on_github {
        let range = format!("{there}...{head}");
        let counts = run(Some(top), &["rev-list", "--left-right", "--count", &range]).await?;
        let mut counts = counts.split_whitespace().map(|n| n.parse::<u64>().ok());
        behind = counts.next().flatten();
        ahead = counts.next().flatten();
    }
    Ok(Some(Branch {
        name: name.to_owned(),
        head,
        remote,
        owner,
        on_github,
        ahead,
        behind,
        tracked: upstream.is_some(),
    }))
}

/// The commits on `head` that `base` (a branch on `remote`, fetched for
/// this) doesn't have, oldest first, with their messages: the newest 250.
pub async fn commits(
    top: &Path,
    remote: &str,
    base: &str,
    head: &str,
) -> Result<Vec<CommitMessage>, GitError> {
    let into = format!("refs/ghtui/base/{base}");
    let spec = format!("+refs/heads/{base}:{into}");
    run(
        Some(top),
        &["fetch", "--no-tags", "--no-write-fetch-head", remote, &spec],
    )
    .await?;
    let range = format!("{into}..refs/heads/{head}");
    let out = run(
        Some(top),
        &["log", "--max-count=250", "--format=%H%x1f%B%x1e", &range],
    )
    .await?;
    let mut commits: Vec<_> = (out.split('\u{1e}'))
        .filter_map(|c| c.trim_start_matches('\n').split_once('\u{1f}'))
        .map(|(oid, message)| CommitMessage {
            oid: oid.to_owned(),
            message: message.trim_end().to_owned(),
        })
        .collect();
    commits.reverse();
    Ok(commits)
}

/// Pushes `branch` to its branch of the same name on `remote`, setting
/// that as its upstream when it has none.
pub async fn push(top: &Path, remote: &str, branch: &str, upstream: bool) -> Result<(), GitError> {
    let spec = format!("refs/heads/{branch}:refs/heads/{branch}");
    let mut args = vec!["push"];
    if upstream {
        args.push("--set-upstream");
    }
    args.extend([remote, spec.as_str()]);
    run(Some(top), &args).await.map(drop)
}

/// The files with changes not committed (untracked files don't count:
/// switching branches keeps them).
pub async fn uncommitted(top: &Path) -> Result<Vec<String>, GitError> {
    let out = run(
        Some(top),
        &["status", "--porcelain", "--untracked-files=no"],
    )
    .await?;
    Ok((out.lines())
        .filter_map(|entry| entry.get(3..))
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect())
}

/// What checking out a pull request did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Checkout {
    /// Nothing: these files have changes that aren't committed.
    Uncommitted(Vec<String>),
    /// A new branch at the pull request's head.
    Created(String),
    /// The branch was already there, at the pull request's head or behind
    /// it (and now at it).
    Switched(String),
    /// The branch was already there with commits the pull request doesn't
    /// have: left as it is.
    Diverged(String),
}

/// Checks out `source`, a ref of `remote` (a pull request's head,
/// `refs/pull/N/head`, or a branch, `refs/heads/…`), on branch `name`,
/// with `remote`'s branch `track` as its upstream when there is one.
/// Refuses while the working tree has changes.
pub async fn checkout(
    top: &Path,
    remote: &str,
    source: &str,
    name: &str,
    track: Option<&str>,
) -> Result<Checkout, GitError> {
    let changed = uncommitted(top).await?;
    if !changed.is_empty() {
        return Ok(Checkout::Uncommitted(changed));
    }
    let head_ref = format!(
        "refs/ghtui/checkout/{}",
        source.strip_prefix("refs/").unwrap_or(source)
    );
    let mut specs = vec![format!("+{source}:{head_ref}")];
    if let Some(branch) = track {
        specs.push(format!(
            "+refs/heads/{branch}:refs/remotes/{remote}/{branch}"
        ));
    }
    let mut args = vec!["fetch", "--no-tags", "--no-write-fetch-head", remote];
    args.extend(specs.iter().map(String::as_str));
    run(Some(top), &args).await?;
    let head = commit(top, &head_ref)
        .await
        .ok_or_else(|| GitError::Parse(format!("{head_ref} isn't a commit")))?;
    let Some(local) = commit(top, &format!("refs/heads/{name}")).await else {
        run(Some(top), &["switch", "--quiet", "-c", name, &head]).await?;
        if let Some(branch) = track {
            let upstream = format!("--set-upstream-to={remote}/{branch}");
            run(Some(top), &["branch", "--quiet", &upstream, name]).await?;
        }
        return Ok(Checkout::Created(name.to_owned()));
    };
    run(Some(top), &["switch", "--quiet", name]).await?;
    if local == head {
        return Ok(Checkout::Switched(name.to_owned()));
    }
    let behind = run(Some(top), &["merge-base", "--is-ancestor", &local, &head]).await;
    if behind.is_err() {
        return Ok(Checkout::Diverged(name.to_owned()));
    }
    run(Some(top), &["merge", "--quiet", "--ff-only", &head]).await?;
    Ok(Checkout::Switched(name.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::git::set_up as git;

    /// A repository with one commit, as a known person.
    async fn repo(dir: &Path) {
        git(dir, &["init", "-q", "-b", "main"]).await;
        for (key, value) in [("user.name", "T"), ("user.email", "t@example.com")] {
            git(dir, &["config", key, value]).await;
        }
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        git(dir, &["add", "a.txt"]).await;
        git(dir, &["commit", "-q", "-m", "First"]).await;
    }

    /// "GitHub" (a repository with pull request 7's head) and a clone of it.
    async fn github_and_clone() -> (tempfile::TempDir, tempfile::TempDir) {
        let github = tempfile::tempdir().unwrap();
        repo(github.path()).await;
        git(github.path(), &["switch", "-q", "-c", "feature"]).await;
        std::fs::write(github.path().join("b.txt"), "two\n").unwrap();
        git(github.path(), &["add", "b.txt"]).await;
        git(github.path(), &["commit", "-q", "-m", "Add b\n\nBecause."]).await;
        git(
            github.path(),
            &["update-ref", "refs/pull/7/head", "feature"],
        )
        .await;
        git(github.path(), &["switch", "-q", "main"]).await;
        let clone = tempfile::tempdir().unwrap();
        let from = github.path().to_str().unwrap();
        git(clone.path(), &["clone", "-q", from, "."]).await;
        for (key, value) in [("user.name", "T"), ("user.email", "t@example.com")] {
            git(clone.path(), &["config", key, value]).await;
        }
        (github, clone)
    }

    /// With a change not committed, nothing happens; once it's committed,
    /// the pull request's head is checked out on its branch, which tracks
    /// GitHub's.
    #[tokio::test]
    async fn checkout_refuses_a_dirty_tree_and_switches_on_a_clean_one() {
        let (_github, clone) = github_and_clone().await;
        let top = clone.path();
        std::fs::write(top.join("a.txt"), "changed\n").unwrap();
        let refused = checkout(
            top,
            "origin",
            "refs/pull/7/head",
            "feature",
            Some("feature"),
        )
        .await
        .unwrap();
        assert_eq!(refused, Checkout::Uncommitted(vec!["a.txt".into()]));
        assert_eq!(git(top, &["branch", "--show-current"]).await.trim(), "main");

        git(top, &["commit", "-q", "-am", "Mine"]).await;
        let done = checkout(
            top,
            "origin",
            "refs/pull/7/head",
            "feature",
            Some("feature"),
        )
        .await
        .unwrap();
        assert_eq!(done, Checkout::Created("feature".into()));
        assert_eq!(
            git(top, &["branch", "--show-current"]).await.trim(),
            "feature"
        );
        assert!(top.join("b.txt").exists());
        let upstream = git(top, &["rev-parse", "--abbrev-ref", "feature@{upstream}"]).await;
        assert_eq!(upstream.trim(), "origin/feature");
        // Again: already there.
        let again = checkout(top, "origin", "refs/pull/7/head", "feature", None)
            .await
            .unwrap();
        assert_eq!(again, Checkout::Switched("feature".into()));
    }

    /// A branch made on GitHub (an issue's) is checked out as itself,
    /// tracking GitHub's.
    #[tokio::test]
    async fn a_branch_made_on_github_is_checked_out_tracking_it() {
        let (_github, clone) = github_and_clone().await;
        let top = clone.path();
        let done = checkout(
            top,
            "origin",
            "refs/heads/feature",
            "feature",
            Some("feature"),
        )
        .await
        .unwrap();
        assert_eq!(done, Checkout::Created("feature".into()));
        assert!(top.join("b.txt").exists());
        let upstream = git(top, &["rev-parse", "--abbrev-ref", "feature@{upstream}"]).await;
        assert_eq!(upstream.trim(), "origin/feature");
    }

    /// A branch GitHub doesn't have says so, and its commits (against the
    /// base fetched from GitHub) carry their whole messages; once pushed,
    /// GitHub has it as it is here.
    #[tokio::test]
    async fn a_branch_is_compared_with_github_and_pushed() {
        let (github, clone) = github_and_clone().await;
        let top = clone.path();
        git(top, &["switch", "-q", "-c", "topic"]).await;
        std::fs::write(top.join("c.txt"), "three\n").unwrap();
        git(top, &["add", "c.txt"]).await;
        git(top, &["commit", "-q", "-m", "Add c\n\nWith a body."]).await;
        let found = clone_of(top, "o", "r").await.unwrap();
        assert_eq!(found, None, "the remote isn't github.com");

        let b = branch(top, "origin", "topic").await.unwrap().unwrap();
        assert_eq!(
            (b.on_github.as_ref(), b.remote.as_str(), b.tracked),
            (None, "origin", false)
        );
        let commits = commits(top, "origin", "main", "topic").await.unwrap();
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].message, "Add c\n\nWith a body.");

        push(top, "origin", "topic", true).await.unwrap();
        let b = branch(top, "origin", "topic").await.unwrap().unwrap();
        assert!(b.pushed() && b.tracked, "{b:?}");
        assert_eq!((b.ahead, b.behind), (Some(0), Some(0)));
        let there = git(github.path(), &["rev-parse", "topic"]).await;
        assert_eq!(there.trim(), b.head.to_string());
    }
}
