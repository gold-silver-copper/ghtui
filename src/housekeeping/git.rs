//! Housekeeping with git, in your clone, with your own git and its
//! credentials: a branch or tag made or deleted on GitHub with `git push`,
//! and files committed on GitHub's tip of a branch and pushed. Nothing
//! here touches the clone's branches, index or working tree: commits are
//! made from objects alone ([`crate::git::commit`]) and pushed by their
//! IDs, so whatever you're in the middle of stays as it is.

use std::path::Path;

use crate::api::housekeeping::RefKind;
use crate::git::repo::Repo;
use crate::git::{GitError, commit};

/// Where GitHub's tip of a branch is fetched to, to commit on.
const TIP: &str = "refs/ghtui/edit/tip";

/// Your clone at `top`, pushing to and fetching from `remote`.
async fn open(top: &Path, remote: &str) -> Result<Repo, String> {
    Repo::open_local(top, remote)
        .await
        .map_err(|e| format!("Couldn't open your clone: {e}"))
}

/// Fetches commit `sha` from GitHub unless the clone has it.
async fn ensure(repo: &Repo, sha: &str) -> Result<(), String> {
    let commit = format!("{sha}^{{commit}}");
    let have = repo.git(&["cat-file", "-e", &commit], &[], None).await;
    if have.is_ok() {
        return Ok(());
    }
    let args = [
        "fetch",
        "--no-tags",
        "--no-write-fetch-head",
        &repo.remote,
        sha,
    ];
    repo.git(&args, &[], None).await.map(drop).map_err(|e| {
        let sha = crate::ui::text::short_sha(sha);
        format!("Couldn't fetch {sha} from GitHub: {e}")
    })
}

/// Pushes to GitHub, quietly: `args` after the remote.
async fn push(repo: &Repo, args: &[&str]) -> Result<(), GitError> {
    let args = [&["push", "--quiet", repo.remote.as_str()], args].concat();
    repo.git(&args, &[], None).await.map(drop)
}

/// Why GitHub refused a push.
fn push_refused(err: &GitError) -> String {
    format!("GitHub refused the push: {}", refused(err))
}

/// Whether GitHub has the branch or tag `full` (`refs/heads/…`).
async fn remote_has(repo: &Repo, kind: RefKind, full: &str) -> Result<bool, String> {
    let all = match kind {
        RefKind::Branch => "branches",
        RefKind::Tag => "tags",
    };
    let there = (repo
        .git(&["ls-remote", &repo.remote, full], &[], None)
        .await)
        .map_err(|e| format!("Couldn't ask GitHub for its {all}: {e}"))?;
    Ok(there.lines().any(|l| l.ends_with(&format!("\t{full}"))))
}

/// What git said when GitHub refused a push, in its own words.
fn refused(err: &GitError) -> String {
    match err {
        GitError::Failed { stderr, .. } => {
            let lines: Vec<&str> = (stderr.lines())
                .map(str::trim)
                .filter(|l| l.starts_with("remote:") || l.starts_with("! ") || l.contains("error"))
                .map(|l| l.trim_start_matches("remote:").trim())
                .filter(|l| !l.is_empty())
                .collect();
            if lines.is_empty() {
                stderr.trim().to_owned()
            } else {
                lines.join(" ")
            }
        }
        err => err.to_string(),
    }
}

/// Makes branch or tag `name` on GitHub at `sha` with `git push`.
pub async fn push_new_ref(
    top: &Path,
    remote: &str,
    kind: RefKind,
    name: &str,
    sha: &str,
) -> Result<(), String> {
    let repo = open(top, remote).await?;
    ensure(&repo, sha).await?;
    let full = kind.full(name);
    // A push to a ref that's there already would move it: make sure it
    // isn't.
    if remote_has(&repo, kind, &full).await? {
        return Err(format!("GitHub has a {} named {name} already", kind.noun()));
    }
    let spec = format!("{sha}:{full}");
    push(&repo, &[&spec]).await.map_err(|e| push_refused(&e))
}

/// Deletes branch or tag `name` on GitHub with `git push --delete`; the
/// clone's own branch or tag of that name stays.
pub async fn push_delete(
    top: &Path,
    remote: &str,
    kind: RefKind,
    name: &str,
) -> Result<(), String> {
    let repo = open(top, remote).await?;
    let full = kind.full(name);
    (push(&repo, &["--delete", &full]).await).map_err(|e| push_refused(&e))
}

/// Files to commit on a branch from your clone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// The branch it goes on: an existing one, or a new one made at
    /// `from`.
    pub branch: String,
    pub from: Option<String>,
    /// Each path it changes, with its blob as you saw it (none: it wasn't
    /// there): one changed on GitHub since is never overwritten.
    pub seen: Vec<(String, Option<String>)>,
    /// Paths and their new text.
    pub additions: Vec<(String, String)>,
    pub deletions: Vec<String>,
    pub message: String,
    /// Your GitHub login: the commit's author when git has no name for
    /// you.
    pub login: Option<String>,
}

/// Commits `c` on GitHub's tip of its branch (or at `from`, for a new
/// branch) and pushes it: its commit's ID. A path changed on GitHub since
/// it was seen isn't overwritten; the commit isn't made.
pub async fn commit(top: &Path, remote: &str, c: &Commit) -> Result<String, String> {
    let repo = open(top, remote).await?;
    // Once more if GitHub's branch moves on between looking and pushing.
    for attempt in 0..2 {
        let parent = match &c.from {
            Some(from) => {
                let full = RefKind::Branch.full(&c.branch);
                if remote_has(&repo, RefKind::Branch, &full).await? {
                    return Err(format!(
                        "GitHub has a branch named {} already: choose another name",
                        c.branch
                    ));
                }
                ensure(&repo, from).await?;
                from.clone()
            }
            None => tip(&repo, &c.branch).await?,
        };
        for (path, seen) in &c.seen {
            if commit::blob(&repo, &parent, path).await != *seen {
                return Err(super::files::not_as_seen(&c.branch, path, seen.as_ref()));
            }
        }
        let change = commit::Change {
            additions: &c.additions,
            deletions: &c.deletions,
            message: &c.message,
            login: c.login.as_deref(),
        };
        let made = commit::make(&repo, &parent, &change)
            .await
            .map_err(|e| format!("Couldn't make the commit: {e}"))?;
        let spec = format!("{made}:{}", RefKind::Branch.full(&c.branch));
        match push(&repo, &[&spec]).await {
            Ok(()) => return Ok(made),
            Err(err) if attempt == 0 && c.from.is_none() && commit::moved_on(&err) => {}
            Err(err) => return Err(push_refused(&err)),
        }
    }
    Err(format!(
        "{} kept changing while committing: try again",
        c.branch
    ))
}

/// GitHub's tip of `branch`, fetched.
async fn tip(repo: &Repo, branch: &str) -> Result<String, String> {
    let spec = format!("+{}:{TIP}", RefKind::Branch.full(branch));
    let args = [
        "fetch",
        "--no-tags",
        "--no-write-fetch-head",
        &repo.remote,
        &spec,
    ];
    repo.git(&args, &[], None)
        .await
        .map_err(|e| format!("Couldn't fetch {branch} from GitHub: {e}"))?;
    let out = repo.git(&["rev-parse", TIP], &[], None).await;
    out.map(|o| o.trim().to_owned())
        .map_err(|e| format!("Couldn't read {branch}: {e}"))
}

/// Clones `url` into `dir` with your own git.
pub async fn clone(url: &str, dir: &Path) -> Result<(), String> {
    if dir.exists() {
        return Err(format!("{} is there already", dir.display()));
    }
    let target = dir.to_string_lossy().into_owned();
    crate::git::run(None, &["clone", "--quiet", url, &target])
        .await
        .map(drop)
        .map_err(|e| format!("Couldn't clone it: {}", refused(&e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::git::set_up as git;

    /// "GitHub" (a bare repository with `a.txt` and `run.sh` on main),
    /// your clone of it, and someone else's.
    async fn setup() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let github = dir.path().join("github.git");
        git(
            dir.path(),
            &["init", "-q", "--bare", "-b", "main", "github.git"],
        )
        .await;
        let url = format!("file://{}", github.display());
        let mut clones = Vec::new();
        for name in ["mine", "theirs"] {
            let clone = dir.path().join(name);
            std::fs::create_dir_all(&clone).unwrap();
            git(&clone, &["init", "-q", "-b", "main"]).await;
            for (key, value) in [("user.name", name), ("user.email", "x@example.com")] {
                git(&clone, &["config", key, value]).await;
            }
            git(&clone, &["remote", "add", "origin", &url]).await;
            clones.push(clone);
        }
        let theirs = clones.pop().unwrap();
        let mine = clones.pop().unwrap();
        std::fs::write(theirs.join("a.txt"), "one\n").unwrap();
        std::fs::write(theirs.join("run.sh"), "echo\n").unwrap();
        git(&theirs, &["add", "a.txt", "run.sh"]).await;
        git(&theirs, &["update-index", "--chmod=+x", "run.sh"]).await;
        git(&theirs, &["commit", "-q", "-m", "First"]).await;
        git(&theirs, &["push", "-q", "origin", "main"]).await;
        git(&mine, &["pull", "-q", "origin", "main"]).await;
        (dir, mine, theirs)
    }

    fn edit(seen: Option<String>, text: &str) -> Commit {
        Commit {
            branch: "main".into(),
            from: None,
            seen: vec![("a.txt".into(), seen)],
            additions: vec![("a.txt".into(), text.into())],
            deletions: Vec::new(),
            message: "Update a.txt\n\nWhy.".into(),
            login: None,
        }
    }

    async fn on_github(theirs: &Path, rev: &str) -> String {
        git(theirs, &["fetch", "-q", "origin"]).await;
        git(theirs, &["show", rev]).await
    }

    /// A file is committed on GitHub's latest of its branch and pushed;
    /// another change there meanwhile doesn't stop it, and your clone's
    /// branch and working tree stay as they were.
    #[tokio::test]
    async fn a_file_is_committed_on_githubs_tip_and_pushed() {
        let (_dir, mine, theirs) = setup().await;
        let seen = git(&mine, &["rev-parse", "HEAD:a.txt"])
            .await
            .trim()
            .to_owned();
        let before = git(&mine, &["rev-parse", "HEAD"]).await;
        std::fs::write(theirs.join("b.txt"), "b\n").unwrap();
        git(&theirs, &["add", "b.txt"]).await;
        git(&theirs, &["commit", "-q", "-m", "B"]).await;
        git(&theirs, &["push", "-q", "origin", "main"]).await;
        let made = commit(&mine, "origin", &edit(Some(seen), "two\n"))
            .await
            .unwrap();
        assert_eq!(on_github(&theirs, "origin/main:a.txt").await, "two\n");
        assert_eq!(on_github(&theirs, "origin/main:b.txt").await, "b\n");
        let message = git(&theirs, &["log", "-1", "--format=%B", "origin/main"]).await;
        assert_eq!(message.trim(), "Update a.txt\n\nWhy.");
        assert_eq!(
            git(&theirs, &["rev-parse", "origin/main"]).await.trim(),
            made
        );
        assert_eq!(git(&mine, &["rev-parse", "HEAD"]).await, before);
        assert_eq!(
            std::fs::read_to_string(mine.join("a.txt")).unwrap(),
            "one\n"
        );
        assert_eq!(git(&mine, &["status", "--porcelain"]).await, "");
    }

    /// A file changed on GitHub since it was seen isn't overwritten: no
    /// commit is made, and GitHub keeps its text.
    #[tokio::test]
    async fn a_file_changed_meanwhile_is_never_overwritten() {
        let (_dir, mine, theirs) = setup().await;
        let seen = git(&mine, &["rev-parse", "HEAD:a.txt"])
            .await
            .trim()
            .to_owned();
        std::fs::write(theirs.join("a.txt"), "theirs\n").unwrap();
        git(&theirs, &["commit", "-q", "-am", "Theirs"]).await;
        git(&theirs, &["push", "-q", "origin", "main"]).await;
        let refused = commit(&mine, "origin", &edit(Some(seen), "mine\n"))
            .await
            .unwrap_err();
        assert!(refused.contains("a.txt changed on GitHub"), "{refused}");
        assert_eq!(on_github(&theirs, "origin/main:a.txt").await, "theirs\n");
    }

    /// A rename on a new branch keeps the file's mode; the branch is made
    /// at the commit asked for, and only once.
    #[tokio::test]
    async fn a_rename_goes_on_a_new_branch() {
        let (_dir, mine, theirs) = setup().await;
        let base = git(&mine, &["rev-parse", "HEAD"]).await.trim().to_owned();
        let seen = git(&mine, &["rev-parse", "HEAD:run.sh"])
            .await
            .trim()
            .to_owned();
        let rename = Commit {
            branch: "rename".into(),
            from: Some(base),
            seen: vec![("run.sh".into(), Some(seen)), ("go.sh".into(), None)],
            additions: vec![("go.sh".into(), "echo go\n".into())],
            deletions: vec!["run.sh".into()],
            message: "Rename run.sh to go.sh".into(),
            login: Some("octocat".into()),
        };
        commit(&mine, "origin", &rename).await.unwrap();
        let tree = on_github(&theirs, "origin/rename^{tree}").await;
        assert!(tree.contains("go.sh") && !tree.contains("run.sh"), "{tree}");
        let ls = git(&theirs, &["ls-tree", "origin/rename", "go.sh"]).await;
        assert!(ls.starts_with("100755"), "{ls}");
        let again = commit(&mine, "origin", &rename).await.unwrap_err();
        assert!(again.contains("has a branch named rename"), "{again}");
    }

    /// A branch and a tag are made on GitHub at a commit, and deleted, with
    /// git; your clone gets neither.
    #[tokio::test]
    async fn branches_and_tags_are_pushed_and_deleted() {
        let (_dir, mine, theirs) = setup().await;
        let sha = git(&theirs, &["rev-parse", "HEAD"]).await.trim().to_owned();
        push_new_ref(&mine, "origin", RefKind::Branch, "topic", &sha)
            .await
            .unwrap();
        push_new_ref(&mine, "origin", RefKind::Tag, "v1", &sha)
            .await
            .unwrap();
        let refs = git(&theirs, &["ls-remote", "origin"]).await;
        assert!(refs.contains(&format!("{sha}\trefs/heads/topic")), "{refs}");
        assert!(refs.contains(&format!("{sha}\trefs/tags/v1")), "{refs}");
        let again = push_new_ref(&mine, "origin", RefKind::Branch, "topic", &sha).await;
        assert!(again.unwrap_err().contains("has a branch named topic"));
        assert_eq!(git(&mine, &["branch", "--list", "topic"]).await, "");
        push_delete(&mine, "origin", RefKind::Branch, "topic")
            .await
            .unwrap();
        push_delete(&mine, "origin", RefKind::Tag, "v1")
            .await
            .unwrap();
        let refs = git(&theirs, &["ls-remote", "origin"]).await;
        assert!(!refs.contains("topic") && !refs.contains("v1"), "{refs}");
    }
}
