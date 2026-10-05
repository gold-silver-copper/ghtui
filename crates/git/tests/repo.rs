//! Repository operations against throwaway repositories. A local "GitHub"
//! repo is served over `file://` with filtering enabled, so partial clones,
//! lazy fetching and prefetching behave as they do against github.com, with
//! no network access.

// Fixture helpers aren't `#[test]` functions, but a panic in them is still a
// test failure.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

use ghtui_git::credentials::Credentials;
use ghtui_git::files::{ChangedFile, FileStatus, MODE_EXECUTABLE, MODE_SUBMODULE, MODE_SYMLINK};
use ghtui_git::repo::Repo;

/// Runs git for fixture setup, isolated from the user's configuration.
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

fn write(dir: &Path, path: &str, contents: &[u8]) {
    let path = dir.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    origin: PathBuf,
    /// Where `main` was when the PR branched.
    branch_point: String,
}

fn numbered(n: u32) -> String {
    (1..=n).map(|i| format!("fn line_{i}() {{}}\n")).collect()
}

/// `main` with a few files; PR #7 (`feature`) branches off it, then `main`
/// moves on, so the three-dot base differs from `main`'s tip.
fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_owned();
    let origin = root.join("origin");
    std::fs::create_dir_all(&origin).unwrap();
    git(&origin, &["init", "-q", "-b", "main"]);
    git(&origin, &["config", "uploadpack.allowFilter", "true"]);
    git(
        &origin,
        &["config", "uploadpack.allowAnySHA1InWant", "true"],
    );

    write(&origin, "src/lib.rs", numbered(30).as_bytes());
    write(&origin, "src/old_name.rs", numbered(40).as_bytes());
    write(&origin, "gone.txt", b"bye\n");
    write(&origin, "script.sh", b"echo hi\n");
    write(&origin, "image.bin", b"\x89PNG\0\x01\x02");
    write(&origin, "crlf.txt", b"a\nb\n");
    write(&origin, "no_newline.txt", b"a\nb\n");
    git(&origin, &["add", "."]);
    git(&origin, &["commit", "-q", "-m", "base"]);
    let branch_point = git(&origin, &["rev-parse", "HEAD"]);

    git(&origin, &["checkout", "-q", "-b", "feature"]);
    write(
        &origin,
        "src/lib.rs",
        numbered(30).replace("line_5", "line_five").as_bytes(),
    );
    git(&origin, &["mv", "src/old_name.rs", "src/new_name.rs"]);
    write(
        &origin,
        "src/new_name.rs",
        numbered(40).replace("line_40", "line_forty").as_bytes(),
    );
    git(&origin, &["rm", "-q", "gone.txt"]);
    write(&origin, "added.md", b"# New\n");
    write(&origin, "image.bin", b"\x89PNG\0\x03\x04");
    write(&origin, "crlf.txt", b"a\r\nb\r\n");
    write(&origin, "no_newline.txt", b"a\nb");
    write(&origin, "gen/schema.rs", b"// generated\n");
    write(&origin, ".gitattributes", b"gen/** linguist-generated\n");
    std::os::unix::fs::symlink("src/lib.rs", origin.join("link")).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        let script = origin.join("script.sh");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    git(&origin, &["add", "-A"]);
    git(
        &origin,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{branch_point},vendor/sub"),
        ],
    );
    git(&origin, &["commit", "-q", "-m", "feature"]);
    git(&origin, &["update-ref", "refs/pull/7/head", "feature"]);

    git(&origin, &["checkout", "-q", "main"]);
    write(&origin, "later.txt", b"main moved on\n");
    git(&origin, &["add", "."]);
    git(&origin, &["commit", "-q", "-m", "later"]);

    Fixture {
        _tmp: tmp,
        root,
        origin,
        branch_point,
    }
}

fn url(f: &Fixture) -> String {
    format!("file://{}", f.origin.display())
}

async fn cache_repo(f: &Fixture) -> Repo {
    Repo::open_cache(
        &f.root.join("cache"),
        "Owner",
        "Repo",
        &url(f),
        Credentials::Ambient,
        &|_| {},
    )
    .await
    .unwrap()
}

fn find<'a>(files: &'a [ChangedFile], path: &str) -> &'a ChangedFile {
    files
        .iter()
        .find(|f| f.path() == path)
        .unwrap_or_else(|| panic!("{path} not in {files:#?}"))
}

#[tokio::test]
async fn partial_cache_clone_fetches_pr_with_three_dot_base() {
    let f = fixture();
    let repo = cache_repo(&f).await;
    assert!(repo.partial);
    assert!(repo.path.ends_with("repos/owner/repo.git"));

    let refs = repo.fetch_pr(7, "main", &|_| {}).await.unwrap();
    assert_eq!(&*refs.head, git(&f.origin, &["rev-parse", "feature"]));
    assert_eq!(&*refs.base, git(&f.origin, &["rev-parse", "main"]));
    assert_eq!(
        &*refs.merge_base, f.branch_point,
        "diff base is the merge base, not main's tip"
    );

    // Reopening reuses the clone.
    let again = cache_repo(&f).await;
    assert_eq!(again.path, repo.path);
}

#[tokio::test]
async fn lists_every_kind_of_change() {
    let f = fixture();
    let repo = cache_repo(&f).await;
    let refs = repo.fetch_pr(7, "main", &|_| {}).await.unwrap();
    let files = repo
        .changed_files(&refs.merge_base, &refs.head)
        .await
        .unwrap();

    assert_eq!(find(&files, "src/lib.rs").status, FileStatus::Modified);
    let renamed = find(&files, "src/new_name.rs");
    assert_eq!(renamed.status, FileStatus::Renamed);
    assert_eq!(renamed.old_path.as_deref(), Some("src/old_name.rs"));
    assert!(renamed.similarity.unwrap() > 90);
    assert_eq!(find(&files, "gone.txt").status, FileStatus::Deleted);
    assert_eq!(find(&files, "added.md").status, FileStatus::Added);
    let script = find(&files, "script.sh");
    assert!(script.mode_changed());
    assert_eq!(script.new_mode, MODE_EXECUTABLE);
    assert_eq!(find(&files, "link").new_mode, MODE_SYMLINK);
    assert_eq!(find(&files, "vendor/sub").new_mode, MODE_SUBMODULE);
    assert!(find(&files, "vendor/sub").is_submodule());
    find(&files, "image.bin");
    find(&files, "crlf.txt");
    find(&files, "no_newline.txt");
    assert!(
        files.iter().all(|f| f.path() != "later.txt"),
        "changes on main after the branch point aren't part of the PR"
    );
}

#[tokio::test]
async fn prefetch_fetches_missing_blobs_in_one_batch() {
    let f = fixture();
    let repo = cache_repo(&f).await;
    let refs = repo.fetch_pr(7, "main", &|_| {}).await.unwrap();
    let files = repo
        .changed_files(&refs.merge_base, &refs.head)
        .await
        .unwrap();
    let mut oids: Vec<String> = files
        .iter()
        .flat_map(|f| f.blob_oids().map(str::to_owned).collect::<Vec<_>>())
        .collect();
    oids.sort();
    oids.dedup();

    let missing = repo.missing(&oids).await.unwrap();
    assert!(
        !missing.is_empty(),
        "a blob:none clone starts without blobs"
    );
    let fetched = repo.prefetch(&oids).await.unwrap();
    assert_eq!(fetched, missing.len());
    assert!(repo.missing(&oids).await.unwrap().is_empty());
    assert_eq!(
        repo.prefetch(&oids).await.unwrap(),
        0,
        "second prefetch is a no-op"
    );

    // With everything local, reads must not need the network: forbid lazy
    // fetches and read every blob.
    let lib = find(&files, "src/lib.rs");
    let check = Command::new("git")
        .arg("-C")
        .arg(&repo.path)
        .args(["cat-file", "-e", &lib.new_oid])
        .env("GIT_NO_LAZY_FETCH", "1")
        .status()
        .unwrap();
    assert!(check.success());

    let reader = repo.blob_reader().unwrap();
    let contents = reader.read(&lib.new_oid).await.unwrap().unwrap();
    assert!(String::from_utf8(contents).unwrap().contains("line_five"));
    let image = reader
        .read(&find(&files, "image.bin").new_oid)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(image, b"\x89PNG\0\x03\x04");
    let no_newline = reader
        .read(&find(&files, "no_newline.txt").new_oid)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(no_newline, b"a\nb");
    assert_eq!(
        reader
            .read("0123456789012345678901234567890123456789")
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn blob_reader_lazily_fetches_without_prefetch() {
    let f = fixture();
    let repo = cache_repo(&f).await;
    let refs = repo.fetch_pr(7, "main", &|_| {}).await.unwrap();
    let files = repo
        .changed_files(&refs.merge_base, &refs.head)
        .await
        .unwrap();
    let reader = repo.blob_reader().unwrap();
    let added = reader
        .read(&find(&files, "added.md").new_oid)
        .await
        .unwrap();
    assert_eq!(added.as_deref(), Some(&b"# New\n"[..]));
}

#[tokio::test]
async fn reads_linguist_attributes_from_the_head_commit() {
    let f = fixture();
    let repo = cache_repo(&f).await;
    let refs = repo.fetch_pr(7, "main", &|_| {}).await.unwrap();
    let generated = repo
        .generated_paths(&refs.head, &["gen/schema.rs", "src/lib.rs"])
        .await
        .unwrap();
    assert!(generated.contains("gen/schema.rs"));
    assert!(!generated.contains("src/lib.rs"));
}

#[tokio::test]
async fn pins_seen_heads() {
    let f = fixture();
    let repo = cache_repo(&f).await;
    let refs = repo.fetch_pr(7, "main", &|_| {}).await.unwrap();
    repo.pin_seen(7, &refs.head).await.unwrap();
    let pinned = git(
        &repo.path,
        &["rev-parse", &format!("refs/ghtui/pr/7/seen/{}", refs.head)],
    );
    assert_eq!(pinned, &*refs.head);
}

#[tokio::test]
async fn users_clone_only_gains_ghtui_refs() {
    let f = fixture();
    let local = f.root.join("local");
    let out = Command::new("git")
        .args(["clone", "-q", "-o", "upstream"])
        .arg(url(&f))
        .arg(&local)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(out.status.success());
    write(&local, "uncommitted.txt", b"work in progress\n");

    let refs_before = git(
        &local,
        &["for-each-ref", "--format=%(refname) %(objectname)"],
    );
    let head_before = git(&local, &["symbolic-ref", "HEAD"]);
    let status_before = git(&local, &["status", "--porcelain"]);

    let repo = Repo::open_local(&local, "upstream").await.unwrap();
    assert!(!repo.partial);
    let refs = repo.fetch_pr(7, "main", &|_| {}).await.unwrap();
    assert_eq!(&*refs.merge_base, f.branch_point);

    let refs_after = git(
        &local,
        &["for-each-ref", "--format=%(refname) %(objectname)"],
    );
    let added: Vec<&str> = refs_after
        .lines()
        .filter(|l| !refs_before.lines().any(|b| b == *l))
        .collect();
    assert!(!added.is_empty());
    assert!(
        added.iter().all(|l| l.starts_with("refs/ghtui/")),
        "only refs/ghtui/ may change: {added:?}"
    );
    assert!(
        refs_before.lines().all(|l| refs_after.contains(l)),
        "no ref removed or moved"
    );
    assert_eq!(git(&local, &["symbolic-ref", "HEAD"]), head_before);
    assert_eq!(git(&local, &["status", "--porcelain"]), status_before);
}

#[tokio::test]
async fn failed_clone_leaves_nothing_behind() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("cache");
    let err = Repo::open_cache(
        &cache,
        "o",
        "r",
        &format!("file://{}/does-not-exist", tmp.path().display()),
        Credentials::Ambient,
        &|_| {},
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("clone"), "{err}");
    let leftovers: Vec<_> = std::fs::read_dir(cache.join("repos/o"))
        .map(|d| d.flatten().map(|e| e.file_name()).collect())
        .unwrap_or_default();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[tokio::test]
async fn fetches_force_pushed_commits_by_sha() {
    let f = fixture();
    let repo = cache_repo(&f).await;
    let refs = repo.fetch_pr(7, "main", &|_| {}).await.unwrap();
    // Rewrite the PR so its old head is unreferenced on the server.
    let old_head = refs.head.clone();
    git(&f.origin, &["checkout", "-q", "feature"]);
    git(&f.origin, &["commit", "-q", "--amend", "-m", "rewritten"]);
    git(&f.origin, &["update-ref", "refs/pull/7/head", "feature"]);
    git(&f.origin, &["checkout", "-q", "main"]);
    let new = repo.fetch_pr(7, "main", &|_| {}).await.unwrap();
    assert_ne!(new.head, old_head);
    // Still local thanks to the earlier fetch: `has` sees it without fetching.
    assert!(repo.has(&old_head).await);
    assert!(!repo.has("0123456789012345678901234567890123456789").await);
    assert!(repo.fetch_commit("not-a-sha").await.is_err());
    // Fetching a known commit by SHA works (the server allows any SHA here),
    // and its files read through the blob reader (blobs fetch lazily).
    repo.fetch_commit(&old_head).await.unwrap();
    let reader = repo.blob_reader().unwrap();
    let lib = reader
        .read(&format!("{old_head}:src/lib.rs"))
        .await
        .unwrap()
        .unwrap();
    assert!(String::from_utf8(lib).unwrap().contains("line_five"));
}

#[tokio::test]
async fn lists_pr_commits_oldest_first() {
    let f = fixture();
    let repo = cache_repo(&f).await;
    let refs = repo.fetch_pr(7, "main", &|_| {}).await.unwrap();
    let commits = repo.commits(&refs.merge_base, &refs.head).await.unwrap();
    assert_eq!(commits.len(), 1);
    assert_eq!(
        (&commits[0].oid, commits[0].subject.as_str()),
        (&refs.head, "feature")
    );
}

/// A read cancelled halfway (its task aborted) must not leave the rest of
/// its reply for the next read to parse. Paths with spaces work too.
#[tokio::test]
async fn blob_reader_survives_cancelled_reads() {
    let f = fixture();
    let big: Vec<u8> = (0..4_000_000u32).map(|i| b'a' + (i % 26) as u8).collect();
    write(&f.origin, "big.txt", &big);
    write(&f.origin, "with space.txt", b"spaced\n");
    git(&f.origin, &["add", "."]);
    git(&f.origin, &["commit", "-q", "-m", "more"]);
    git(&f.origin, &["update-ref", "refs/pull/7/head", "HEAD"]);
    let repo = cache_repo(&f).await;
    let refs = repo.fetch_pr(7, "main", &|_| {}).await.unwrap();
    let reader = repo.blob_reader().unwrap();
    let head = refs.head;

    let cancelled = tokio::time::timeout(
        std::time::Duration::ZERO,
        reader.read(&format!("{head}:big.txt")),
    )
    .await;
    assert!(cancelled.is_err(), "the big read was cut short");
    let spaced = reader.read(&format!("{head}:with space.txt")).await;
    assert_eq!(spaced.unwrap().as_deref(), Some(&b"spaced\n"[..]));
    let again = reader.read(&format!("{head}:big.txt")).await;
    assert_eq!(again.unwrap().map(|b| b.len()), Some(big.len()));
}
