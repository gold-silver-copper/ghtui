//! The changes git's own diff tests cover (t4*: binary files, a rename
//! with changes, a mode-only change, a submodule, CRLF, no newline at the
//! end, Unicode paths, a symlink), made in a real repository, with git as
//! the oracle: ghtui reads each change's status and paths as git lists
//! them, counts the lines git counts, and, where GitHub's patch is missing,
//! makes the commentable ranges git's own hunks have.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "tests: failing loudly is the point"
)]

use std::path::Path;
use std::process::Command;

use ghtui_diff::anchor::{Commentable, parse_patch_headers};
use ghtui_diff::{Content, FileDiff, Text};
use ghtui_git::files::{FileStatus, MODE_SUBMODULE, MODE_SYMLINK, parse_raw};

fn git_bytes(dir: &Path, args: &[&str]) -> Vec<u8> {
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
    out.stdout
}

fn git(dir: &Path, args: &[&str]) -> String {
    String::from_utf8(git_bytes(dir, args))
        .unwrap()
        .trim()
        .to_owned()
}

fn write(dir: &Path, path: &str, bytes: &[u8]) {
    let path = dir.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn lines(n: u32, f: impl Fn(u32) -> String) -> String {
    (1..=n).map(|i| format!("{}\n", f(i))).collect()
}

const UNICODE: &str = "päth/日本語 ✨.txt";

/// Two commits: every kind of change from the first to the second.
fn history(dir: &Path) -> (String, String) {
    git(dir, &["init", "-q", "-b", "main"]);
    write(dir, "bin.dat", b"\x89PNG\0\x01\x02\x03");
    write(
        dir,
        "old_name.txt",
        lines(40, |i| format!("line {i}")).as_bytes(),
    );
    write(dir, "mode.sh", b"echo hi\n");
    write(dir, "crlf.txt", b"a\nb\nc\n");
    write(dir, "noeol.txt", b"one\ntwo\nthree\n");
    write(dir, UNICODE, "こんにちは\nwörld\n".as_bytes());
    git(dir, &["add", "."]);
    git(
        dir,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{},sub", "1".repeat(40)),
        ],
    );
    git(dir, &["commit", "-q", "-m", "before"]);
    let before = git(dir, &["rev-parse", "HEAD"]);

    write(dir, "bin.dat", b"\x89PNG\0\x04\x05");
    git(dir, &["mv", "old_name.txt", "new_name.txt"]);
    write(
        dir,
        "new_name.txt",
        lines(40, |i| match i {
            7 => "seven".to_owned(),
            33 => "thirty-three".to_owned(),
            _ => format!("line {i}"),
        })
        .as_bytes(),
    );
    git(dir, &["update-index", "--chmod=+x", "mode.sh"]);
    write(dir, "crlf.txt", b"a\r\nb\r\nc\r\n");
    write(dir, "noeol.txt", b"one\ntwo\nTHREE");
    write(dir, UNICODE, "こんにちは\nworld ✨\n".as_bytes());
    std::os::unix::fs::symlink("new_name.txt", dir.join("link")).unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["update-index", "--chmod=+x", "mode.sh"]);
    git(
        dir,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{},sub", "2".repeat(40)),
        ],
    );
    git(dir, &["commit", "-q", "-m", "after"]);
    let after = git(dir, &["rev-parse", "HEAD"]);
    (before, after)
}

#[test]
fn every_kind_of_change_matches_git() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let (before, after) = history(dir);
    let raw = String::from_utf8(git_bytes(
        dir,
        &["diff", "--raw", "-z", "--no-abbrev", "-M", &before, &after],
    ))
    .unwrap();
    let files = parse_raw(&raw).unwrap();
    let by_path = |p: &str| {
        files
            .iter()
            .find(|f| f.path() == p)
            .unwrap_or_else(|| panic!("{p} not in {files:#?}"))
    };
    assert_eq!(files.len(), 8, "{files:#?}");
    let renamed = by_path("new_name.txt");
    assert_eq!(
        (renamed.status, renamed.old_path.as_deref()),
        (FileStatus::Renamed, Some("old_name.txt"))
    );
    let mode = by_path("mode.sh");
    assert_eq!((mode.old_mode, mode.new_mode), (0o100_644, 0o100_755));
    assert_eq!(mode.old_oid, mode.new_oid, "a mode-only change");
    assert_eq!(by_path("sub").new_mode, MODE_SUBMODULE);
    assert_eq!(by_path("link").new_mode, MODE_SYMLINK);
    by_path(UNICODE);

    for file in &files {
        if file.is_submodule() || file.status == FileStatus::Added {
            continue;
        }
        let blob = |oid: &str| git_bytes(dir, &["cat-file", "blob", oid]);
        let (old, new) = (blob(&file.old_oid), blob(&file.new_oid));
        let path = file.path();
        let diff = FileDiff::compute(path, Some(&old), Some(&new));
        // git's numbers for this file: `-` for binary.
        let old_path = file.old_path.as_deref().unwrap_or(path);
        let numstat = git(
            dir,
            &[
                "diff",
                "--numstat",
                "-M",
                &before,
                &after,
                "--",
                old_path,
                path,
            ],
        );
        let counts: Vec<&str> = numstat.split_whitespace().take(2).collect();
        if counts == ["-", "-"] {
            assert!(matches!(diff.content, Content::Binary { .. }), "{path}");
            continue;
        }
        let git_counts = (counts[0].parse().unwrap(), counts[1].parse().unwrap());
        assert_eq!((diff.additions, diff.deletions), git_counts, "{path}");
        // GitHub's patch is git's: its hunks are what ghtui falls back to.
        let patch = git(
            dir,
            &["diff", "-U3", "-M", &before, &after, "--", old_path, path],
        );
        let local = Commentable::local(&Text::new(&old), &Text::new(&new));
        assert_eq!(local.ranges, parse_patch_headers(&patch), "{path}");
    }
}
