//! ghtui's view of a pull request checked against git's own: for each way
//! a PR's history can go (merged with a merge commit, squashed, rebased,
//! force-pushed, criss-crossed with its base), the merge base must be one
//! `git merge-base --all` gives, the commits as many as `git rev-list
//! --count` counts and the PR made, and the files the ones the PR changed
//! from where it branched (`git diff --raw`).

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::string_slice,
    reason = "fixture helpers aren't #[test] functions, but a panic in them is still a test failure"
)]

use std::path::{Path, PathBuf};
use std::process::Command;

use ghtui_git::credentials::Credentials;
use ghtui_git::repo::{PrRefs, Repo};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// "GitHub": a repository with `main`, served over `file://`.
struct Origin {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    dir: PathBuf,
}

impl Origin {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_owned();
        let dir = root.join("origin");
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q", "-b", "main"]);
        git(&dir, &["config", "uploadpack.allowFilter", "true"]);
        git(&dir, &["config", "uploadpack.allowAnySHA1InWant", "true"]);
        let origin = Self {
            _tmp: tmp,
            root,
            dir,
        };
        origin.commit("README.md", "base\n", "base");
        origin
    }

    fn git(&self, args: &[&str]) -> String {
        git(&self.dir, args)
    }

    /// Writes `path` and commits it; the new commit's ID.
    fn commit(&self, path: &str, text: &str, message: &str) -> String {
        let file = self.dir.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", message]);
        self.git(&["rev-parse", "HEAD"])
    }

    /// Points PR `number`'s head at `rev`, as GitHub keeps it.
    fn pr_head(&self, number: u64, rev: &str) {
        self.git(&["update-ref", &format!("refs/pull/{number}/head"), rev]);
    }

    async fn cache(&self) -> Repo {
        Repo::open_cache(
            &self.root.join("cache"),
            "o",
            "r",
            &format!("file://{}", self.dir.display()),
            Credentials::Ambient,
            &|_| {},
        )
        .await
        .unwrap()
    }

    /// `git diff --raw` between two revisions: status and paths, sorted.
    fn diff(&self, from: &str, to: &str) -> Vec<String> {
        let out = self.git(&["diff", "--raw", "--no-abbrev", "-M", from, to]);
        let mut files: Vec<String> = out
            .lines()
            .map(|l| {
                let (_, rest) = l.split_once(' ').unwrap();
                let fields: Vec<&str> = rest.split_whitespace().collect();
                // mode mode oid oid status path [path]
                let status = &fields[3][..1];
                format!("{status} {}", fields[4..].join(" -> "))
            })
            .collect();
        files.sort();
        files
    }
}

/// ghtui's files for `refs`, in the same form as [`Origin::diff`].
async fn ghtui_files(repo: &Repo, refs: &PrRefs) -> Vec<String> {
    use ghtui_git::files::FileStatus as S;
    let mut files: Vec<String> = repo
        .changed_files(&refs.merge_base, &refs.head)
        .await
        .unwrap()
        .iter()
        .map(|f| {
            let status = match f.status {
                S::Added => "A",
                S::Deleted => "D",
                S::Modified => "M",
                S::Renamed => "R",
                S::Copied => "C",
                S::TypeChanged => "T",
            };
            match (&f.old_path, &f.new_path) {
                (Some(old), Some(new)) if old != new => format!("{status} {old} -> {new}"),
                _ => format!("{status} {}", f.path()),
            }
        })
        .collect();
    files.sort();
    files
}

/// Checks ghtui's PR against git's: the merge base, the commits, the files
/// against the known branch point and commit count.
async fn check(origin: &Origin, repo: &Repo, refs: &PrRefs, branched: &str, commits: usize) {
    let bases = origin.git(&["merge-base", "--all", &refs.base, &refs.head]);
    assert!(
        bases.lines().any(|b| b == refs.merge_base.to_string()),
        "{} isn't a merge base of {bases}",
        refs.merge_base
    );
    let counted: usize = origin
        .git(&[
            "rev-list",
            "--count",
            &format!("{}..{}", refs.merge_base, refs.head),
        ])
        .parse()
        .unwrap();
    let listed = repo.commits(&refs.merge_base, &refs.head).await.unwrap();
    assert_eq!((listed.len(), counted), (commits, commits));
    assert_eq!(
        ghtui_files(repo, refs).await,
        origin.diff(branched, &refs.head)
    );
}

/// A PR of two commits off `main`, which then moves on.
fn pull_request(origin: &Origin) -> String {
    let branched = origin.git(&["rev-parse", "HEAD"]);
    origin.git(&["checkout", "-q", "-b", "feature"]);
    origin.commit("src/a.rs", "fn a() {}\n", "a");
    origin.commit("src/b.rs", "fn b() {}\n", "b");
    origin.pr_head(7, "feature");
    origin.git(&["checkout", "-q", "main"]);
    origin.commit("later.txt", "main moved on\n", "later");
    branched
}

#[tokio::test]
async fn an_open_pr() {
    let origin = Origin::new();
    let branched = pull_request(&origin);
    let repo = origin.cache().await;
    let refs = repo.fetch_pr(7, "main", None, &|_| {}).await.unwrap();
    assert_eq!(refs.merge_base.to_string(), branched);
    check(&origin, &repo, &refs, &branched, 2).await;
}

/// Merged with a merge commit, squashed, or rebased onto `main`: the PR's
/// diff is from the base it had, not `main` after.
#[tokio::test]
async fn a_merged_pr_in_every_way() {
    for how in ["merge commit", "squash", "rebase"] {
        let origin = Origin::new();
        let branched = pull_request(&origin);
        let base = origin.git(&["rev-parse", "main"]);
        match how {
            "merge commit" => {
                origin.git(&["merge", "-q", "--no-ff", "-m", "Merge #7", "feature"]);
            }
            "squash" => {
                origin.git(&["merge", "-q", "--squash", "feature"]);
                origin.git(&["commit", "-q", "-m", "Squashed #7"]);
            }
            _ => {
                // GitHub's rebase-and-merge: copies on main; the PR's head
                // stays its own commits.
                origin.git(&["cherry-pick", "main..feature"]);
            }
        }
        let repo = origin.cache().await;
        let refs = repo
            .fetch_pr(7, "main", Some(&base), &|_| {})
            .await
            .unwrap();
        check(&origin, &repo, &refs, &branched, 2).await;
        assert!(!ghtui_files(&repo, &refs).await.is_empty(), "{how}");
        // From main's tip instead, a merge commit leaves nothing: the bug.
        let tip = repo.fetch_pr(7, "main", None, &|_| {}).await.unwrap();
        if how == "merge commit" {
            assert!(ghtui_files(&repo, &tip).await.is_empty());
        }
    }
}

/// Force-pushed: the PR is its new commits, the old ones forgotten.
#[tokio::test]
async fn a_force_pushed_pr() {
    let origin = Origin::new();
    let branched = pull_request(&origin);
    let repo = origin.cache().await;
    let before = repo.fetch_pr(7, "main", None, &|_| {}).await.unwrap();
    origin.git(&["checkout", "-q", "feature"]);
    origin.git(&["reset", "-q", "--hard", &branched]);
    origin.commit("src/c.rs", "fn c() {}\n", "c, instead");
    origin.pr_head(7, "feature");
    origin.git(&["checkout", "-q", "main"]);
    let after = repo.fetch_pr(7, "main", None, &|_| {}).await.unwrap();
    assert_ne!(before.head, after.head);
    check(&origin, &repo, &after, &branched, 1).await;
    assert_eq!(ghtui_files(&repo, &after).await, ["A src/c.rs"]);
}

/// Criss-crossed: main merged into the PR and the PR into another branch
/// merged into main, so two merge bases. Either is one git would use.
#[tokio::test]
async fn a_criss_crossed_pr() {
    let origin = Origin::new();
    let root = origin.git(&["rev-parse", "HEAD"]);
    origin.git(&["checkout", "-q", "-b", "feature"]);
    let f1 = origin.commit("f.txt", "f1\n", "f1");
    origin.git(&["checkout", "-q", "main"]);
    let m1 = origin.commit("m.txt", "m1\n", "m1");
    // Each side merges the other's first commit.
    origin.git(&["merge", "-q", "--no-ff", "-m", "main takes f1", &f1]);
    origin.git(&["checkout", "-q", "feature"]);
    origin.git(&["merge", "-q", "--no-ff", "-m", "feature takes m1", &m1]);
    origin.commit("f.txt", "f2\n", "f2");
    origin.pr_head(7, "feature");
    origin.git(&["checkout", "-q", "main"]);
    let bases = origin.git(&["merge-base", "--all", "main", "feature"]);
    assert_eq!(
        bases.lines().count(),
        2,
        "a criss-cross has two merge bases"
    );

    let repo = origin.cache().await;
    let refs = repo.fetch_pr(7, "main", None, &|_| {}).await.unwrap();
    let base = refs.merge_base.to_string();
    // The PR's own work since that base: what git says, whichever it is.
    let counted: usize = origin
        .git(&["rev-list", "--count", &format!("{base}..{}", refs.head)])
        .parse()
        .unwrap();
    check(&origin, &repo, &refs, &base, counted).await;
    assert_ne!(base, root);
}
