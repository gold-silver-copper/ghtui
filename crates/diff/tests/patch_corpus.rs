//! go-gitdiff's patches (see `go-gitdiff/README.md`): every one's hunk
//! headers parse, malformed ones included; and for each pair of files it
//! has (a source and what a patch made of it), ghtui's diff adds and
//! removes as many lines as git's (`git diff --no-index --numstat`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests: failing loudly is the point"
)]

use std::path::{Path, PathBuf};

use ghtui_diff::FileDiff;
use ghtui_diff::anchor::parse_patch_headers;

fn dir(sub: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/go-gitdiff")
        .join(sub)
}

fn patches(sub: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = std::fs::read_dir(dir(sub))
        .unwrap()
        .filter_map(|e| {
            let path = e.unwrap().path();
            let name = path.file_name()?.to_str()?.to_owned();
            let text = String::from_utf8_lossy(&std::fs::read(&path).ok()?).into_owned();
            name.ends_with(".patch").then_some((name, text))
        })
        .collect();
    out.sort();
    out
}

/// Lines added and removed from `old` to `new`, by git.
fn git_numstat(old: &Path, new: &Path) -> (u32, u32) {
    let out = std::process::Command::new("git")
        .args(["diff", "--no-index", "--numstat", "--"])
        .arg(old)
        .arg(new)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let mut fields = text.split_whitespace();
    let mut next = || fields.next().map_or(0, |n| n.parse().unwrap());
    let added = next();
    (added, next())
}

#[test]
fn every_patchs_hunk_headers_parse() {
    let all: Vec<(String, String)> = patches("apply")
        .into_iter()
        .chain(patches("string"))
        .collect();
    assert!(all.len() > 30, "{}", all.len());
    for (name, patch) in all {
        let headers = patch.lines().filter(|l| l.starts_with("@@ -")).count();
        let ranges = parse_patch_headers(&patch);
        // A header that doesn't parse is skipped, never a panic; the
        // well-formed ones all parse.
        if !name.contains("error") {
            assert_eq!(ranges.len(), headers, "{name}");
        }
        for r in ranges {
            assert!(r.old.0 <= r.old.1 && r.new.0 <= r.new.1, "{name}: {r:?}");
        }
    }
}

#[test]
fn ghtuis_diff_counts_what_gits_does() {
    let mut checked = 0;
    for (name, _) in patches("apply") {
        let stem = name.trim_end_matches(".patch");
        // file_text_* cases share one source.
        let src = if stem.starts_with("file_text") {
            dir("apply").join("file_text.src")
        } else {
            dir("apply").join(format!("{stem}.src"))
        };
        let out = dir("apply").join(format!("{stem}.out"));
        let (Ok(old), Ok(new)) = (std::fs::read(&src), std::fs::read(&out)) else {
            continue;
        };
        let diff = FileDiff::compute("file.txt", Some(&old), Some(&new));
        assert_eq!(
            (diff.additions, diff.deletions),
            git_numstat(&src, &out),
            "{stem}"
        );
        checked += 1;
    }
    assert!(checked >= 15, "{checked}");
}
