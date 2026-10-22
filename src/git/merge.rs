//! A pull request's base merged into its head with git alone, to resolve
//! their conflicts: `git merge-tree` merges the two commits without a
//! working tree, a conflicted file's text is cut at its conflicts
//! ([`Part`]), and the merge is committed from objects through an index
//! of its own, to push by its ID ([`crate::git::branch`]). A clone's
//! branches, index and working tree stay as they are; until it's pushed,
//! nothing points at what was made.

use crate::git::repo::Repo;
use crate::git::{GitError, Oid, Version, checked, commit};

/// `git merge-tree --write-tree` came with this release.
pub const MERGE_TREE: Version = Version(2, 38, 0);

/// A path's mode and blob on one side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub mode: String,
    pub oid: Oid,
}

/// A path the merge left conflicted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflicted {
    pub path: String,
    /// The head's side (ours) and the base's (theirs), where each has it.
    pub ours: Option<Entry>,
    pub theirs: Option<Entry>,
    /// What git merged it to, conflict markers and all, where the merged
    /// tree has it.
    pub merged: Option<Entry>,
}

/// What merging made: its tree, with the conflicted paths as git left
/// them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merge {
    pub tree: Oid,
    pub conflicted: Vec<Conflicted>,
}

/// Merges `theirs` into `ours`, as `git merge` would, in objects alone.
pub async fn merge(repo: &Repo, ours: &Oid, theirs: &Oid) -> Result<Merge, GitError> {
    // Plain markers, whatever conflict style git is set to.
    let args = [
        "-c",
        "merge.conflictStyle=merge",
        "merge-tree",
        "--write-tree",
        "-z",
        "--no-messages",
        ours,
        theirs,
    ];
    let output = repo.cmd().args(args).output().await?;
    // 1: it merged, with conflicts.
    let out = if output.status.code() == Some(1) {
        output.stdout.as_slice()
    } else {
        checked(&output, "merge-tree")?
    };
    let out = String::from_utf8_lossy(out);
    let mut fields = out.split('\0').filter(|f| !f.is_empty());
    let tree = Oid::parse(fields.next().unwrap_or_default().trim())?;
    let mut conflicted: Vec<Conflicted> = Vec::new();
    for field in fields {
        let Some((meta, path)) = field.split_once('\t') else { continue };
        let mut meta = meta.split_whitespace();
        let (Some(mode), Some(oid), Some(stage)) = (meta.next(), meta.next(), meta.next()) else {
            continue;
        };
        let entry = Entry {
            mode: mode.to_owned(),
            oid: Oid::parse(oid)?,
        };
        let at = match conflicted.iter().position(|c| c.path == path) {
            Some(at) => at,
            None => {
                conflicted.push(Conflicted {
                    path: path.to_owned(),
                    ours: None,
                    theirs: None,
                    merged: None,
                });
                conflicted.len() - 1
            }
        };
        let Some(c) = conflicted.get_mut(at) else { continue };
        match stage {
            "2" => c.ours = Some(entry),
            "3" => c.theirs = Some(entry),
            _ => {}
        }
    }
    for c in &mut conflicted {
        c.merged = entry_at(repo, &tree, &c.path).await?;
    }
    Ok(Merge { tree, conflicted })
}

/// The entry for `path` in `tree`, if it has one.
async fn entry_at(repo: &Repo, tree: &Oid, path: &str) -> Result<Option<Entry>, GitError> {
    let out = repo.run(&["ls-tree", "-z", tree, "--", path]).await?;
    let found = (out.split('\0'))
        .filter_map(|l| l.split_once('\t'))
        .find(|(_, p)| *p == path)
        .and_then(|(meta, _)| {
            let mut meta = meta.split_whitespace();
            let (mode, kind, oid) = (meta.next()?, meta.next()?, meta.next()?);
            let oid = Oid::parse(oid).ok()?;
            (kind == "blob").then(|| Entry {
                mode: mode.to_owned(),
                oid,
            })
        });
    Ok(found)
}

/// A blob's bytes.
pub async fn read(repo: &Repo, oid: &Oid) -> Result<Vec<u8>, GitError> {
    repo.run_bytes(&["cat-file", "blob", oid]).await
}

/// A conflicted file's text, cut at its conflicts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    /// What both sides agree on, or git merged.
    Merged(String),
    /// Where they don't: each side's lines, and what was chosen in their
    /// place, once something is.
    Conflict {
        ours: String,
        theirs: String,
        chosen: Option<String>,
    },
}

/// Whether `line` is a conflict marker of `c`: seven of it, then a label
/// or nothing (an inner merge's longer markers aren't).
fn marker(line: &str, c: char, label: bool) -> bool {
    let line = line.trim_end_matches(['\n', '\r']);
    let rest = line.strip_prefix(&*c.to_string().repeat(7));
    match rest {
        Some("") => true,
        Some(rest) => label && rest.starts_with(' '),
        None => false,
    }
}

/// `text` cut at its conflict markers; none when it has none (or they
/// don't close).
pub fn parts(text: &str) -> Option<Vec<Part>> {
    #[derive(PartialEq)]
    enum In {
        Merged,
        Ours,
        Base,
        Theirs,
    }
    let (mut parts, mut at) = (Vec::new(), In::Merged);
    let (mut merged, mut ours, mut theirs) = (String::new(), String::new(), String::new());
    for line in text.split_inclusive('\n') {
        match at {
            In::Merged if marker(line, '<', true) => {
                if !merged.is_empty() {
                    parts.push(Part::Merged(std::mem::take(&mut merged)));
                }
                at = In::Ours;
            }
            In::Merged => merged.push_str(line),
            In::Ours | In::Base if marker(line, '=', false) => at = In::Theirs,
            // diff3's base, between ours and theirs: what neither is.
            In::Ours if marker(line, '|', true) => at = In::Base,
            In::Ours => ours.push_str(line),
            In::Base => {}
            In::Theirs if marker(line, '>', true) => {
                parts.push(Part::Conflict {
                    ours: std::mem::take(&mut ours),
                    theirs: std::mem::take(&mut theirs),
                    chosen: None,
                });
                at = In::Merged;
            }
            In::Theirs => theirs.push_str(line),
        }
    }
    if !merged.is_empty() {
        parts.push(Part::Merged(merged));
    }
    let conflicts = parts.iter().any(|p| matches!(p, Part::Conflict { .. }));
    (at == In::Merged && conflicts).then_some(parts)
}

/// Lines with a newline at the end, so a marker after them starts its own.
fn ended(text: &str) -> String {
    let mut text = text.to_owned();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// The text of `parts`: what was chosen, and the conflicts that wait
/// between markers labelled `ours` and `theirs`.
pub fn render(parts: &[Part], ours_label: &str, theirs_label: &str) -> String {
    let mut text = String::new();
    for part in parts {
        match part {
            Part::Merged(t)
            | Part::Conflict {
                chosen: Some(t), ..
            } => text.push_str(t),
            Part::Conflict { ours, theirs, .. } => {
                text.push_str(&format!("<<<<<<< {ours_label}\n"));
                text.push_str(&ended(ours));
                text.push_str("=======\n");
                text.push_str(&ended(theirs));
                text.push_str(&format!(">>>>>>> {theirs_label}\n"));
            }
        }
    }
    text
}

/// What a path holds in a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Text(String),
    /// A side's blob, as it is.
    Entry(Entry),
    Gone,
}

/// The commit of `tree` with `files` put in it, on `parents`, saying
/// `message`. A text keeps the mode its path has in `tree`.
pub async fn commit(
    repo: &Repo,
    tree: &str,
    parents: &[&str],
    files: &[(String, Content)],
    message: &str,
    login: Option<&str>,
) -> Result<Oid, GitError> {
    let index = commit::own_index(repo, "merge").await?;
    let index_path = index.to_string_lossy().into_owned();
    let env = [("GIT_INDEX_FILE", index_path.as_str())];
    let made = async {
        repo.git(&["read-tree", tree], &env, None).await?;
        for (path, content) in files {
            let entry = match content {
                Content::Gone => {
                    // Mode 0 takes it out; `--force-remove` would want a
                    // working tree, which a bare clone hasn't.
                    let gone = format!("0 {}\t{path}\n", "0".repeat(40));
                    repo.git(&["update-index", "--index-info"], &env, Some(gone))
                        .await?;
                    continue;
                }
                Content::Entry(entry) => entry.clone(),
                Content::Text(text) => {
                    let blob =
                        (repo.git(&["hash-object", "-w", "--stdin"], &[], Some(text.clone())))
                            .await?;
                    Entry {
                        mode: commit::mode(repo, tree, path).await,
                        oid: Oid::parse(blob.trim())?,
                    }
                }
            };
            let info = format!("{},{},{path}", entry.mode, entry.oid);
            repo.git(&["update-index", "--add", "--cacheinfo", &info], &env, None)
                .await?;
        }
        // A partial clone lacks the blobs it didn't need.
        repo.git(&["write-tree", "--missing-ok"], &env, None).await
    }
    .await;
    let _ = std::fs::remove_file(&index);
    let made = made?;
    let oid = commit::commit_tree(repo, made.trim(), parents, message, login).await?;
    Oid::parse(&oid)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::git::branch;
    use crate::git::fixture::{cache_clone, commit as commit_files, origin};
    use crate::git::set_up as git;

    #[test]
    fn markers_cut_a_text_at_its_conflicts_and_put_it_back() {
        let text = "a\n<<<<<<< ours\nmine\n=======\nyours\nmore\n>>>>>>> theirs\nb\n";
        let mut cut = parts(text).unwrap();
        assert_eq!(
            cut,
            [
                Part::Merged("a\n".into()),
                Part::Conflict {
                    ours: "mine\n".into(),
                    theirs: "yours\nmore\n".into(),
                    chosen: None
                },
                Part::Merged("b\n".into()),
            ]
        );
        assert_eq!(render(&cut, "ours", "theirs"), text);
        if let Some(Part::Conflict { chosen, .. }) = cut.get_mut(1) {
            *chosen = Some("both\n".into());
        }
        assert_eq!(render(&cut, "ours", "theirs"), "a\nboth\nb\n");
        // diff3's base is neither side; a marker that doesn't close, or
        // none, isn't a conflict.
        let diff3 = "<<<<<<< o\nx\n||||||| base\nw\n=======\ny\n>>>>>>> t\n";
        assert!(
            matches!(&parts(diff3).unwrap()[..], [Part::Conflict { ours, theirs, .. }] if ours == "x\n" && theirs == "y\n")
        );
        assert_eq!(parts("<<<<<<< o\nx\n=======\n"), None);
        assert_eq!(parts("a\n========\nb\n"), None);
    }

    /// "GitHub", with `feature` and `main` changing a line each their own
    /// way and `main` adding a file; the cache's clone of it, and the two
    /// tips.
    async fn diverged() -> (tempfile::TempDir, tempfile::TempDir, Repo, Oid, Oid) {
        let github = tempfile::tempdir().unwrap();
        let dir = github.path();
        origin(dir).await;
        commit_files(dir, &[("a.txt", "1\n2\n3\n"), ("gone.txt", "x\n")], "First").await;
        git(dir, &["switch", "-q", "-c", "feature"]).await;
        commit_files(
            dir,
            &[("a.txt", "1\ntwo\n3\n"), ("gone.txt", "y\n")],
            "Ours",
        )
        .await;
        git(dir, &["switch", "-q", "main"]).await;
        commit_files(dir, &[("a.txt", "1\nTWO\n3\n"), ("b.txt", "b\n")], "Theirs").await;
        git(dir, &["rm", "-q", "gone.txt"]).await;
        git(dir, &["commit", "-q", "-m", "Gone"]).await;
        let (cache, repo) = cache_clone(dir).await;
        let ours = branch::tip(&repo, "origin", "feature", "refs/ghtui/t/ours").await;
        let theirs = branch::tip(&repo, "origin", "main", "refs/ghtui/t/theirs").await;
        (github, cache, repo, ours.unwrap(), theirs.unwrap())
    }

    /// The base merges into the head in objects alone: a changed line is
    /// a conflict with each side's text, a file changed on one side and
    /// deleted on the other is one with no text, what merged cleanly is
    /// in the tree; committed with both parents and pushed, GitHub's
    /// branch has the merge, and nothing else moved.
    #[tokio::test]
    async fn the_base_merges_into_the_head_and_the_resolution_is_pushed() {
        let (github, _cache, repo, ours, theirs) = diverged().await;
        let merged = merge(&repo, &ours, &theirs).await.unwrap();
        let paths: Vec<&str> = merged.conflicted.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(paths, ["a.txt", "gone.txt"]);
        let [a, gone] = &merged.conflicted[..] else { panic!() };
        let text = read(&repo, &a.merged.as_ref().unwrap().oid).await.unwrap();
        let cut = parts(&String::from_utf8(text).unwrap()).unwrap();
        assert!(
            matches!(&cut[1], Part::Conflict { ours, theirs, .. } if ours == "two\n" && theirs == "TWO\n")
        );
        assert!(gone.ours.is_some() && gone.theirs.is_none());

        let files = [
            ("a.txt".to_owned(), Content::Text("1\nboth\n3\n".into())),
            ("gone.txt".to_owned(), Content::Gone),
        ];
        let message = "Merge branch 'main' into feature";
        let made = commit(
            &repo,
            &merged.tree,
            &[&*ours, &*theirs],
            &files,
            message,
            None,
        )
        .await
        .unwrap();
        branch::push(&repo, "origin", &made, "feature")
            .await
            .unwrap();
        let dir = github.path();
        assert_eq!(git(dir, &["rev-parse", "feature"]).await.trim(), &*made);
        assert_eq!(git(dir, &["show", "feature:a.txt"]).await, "1\nboth\n3\n");
        assert_eq!(git(dir, &["show", "feature:b.txt"]).await, "b\n");
        let tree = git(dir, &["ls-tree", "--name-only", "feature"]).await;
        assert!(!tree.contains("gone.txt"), "{tree}");
        let parents = git(dir, &["log", "-1", "--format=%P", "feature"]).await;
        assert_eq!(parents.trim(), format!("{ours} {theirs}"));
        assert_eq!(git(dir, &["rev-parse", "main"]).await.trim(), &*theirs);
    }

    /// Merging, and a commit to look at, made and left: GitHub's branches
    /// and the clone's refs are as they were, so there's nothing to undo.
    #[tokio::test]
    async fn a_merge_given_up_on_leaves_nothing_behind() {
        let (github, _cache, repo, ours, theirs) = diverged().await;
        let refs = repo.run(&["for-each-ref"]).await.unwrap();
        let merged = merge(&repo, &ours, &theirs).await.unwrap();
        let files = [("a.txt".to_owned(), Content::Text("mine\n".into()))];
        commit(&repo, &ours, &[&*ours], &files, "Look", None)
            .await
            .unwrap();
        assert!(!merged.conflicted.is_empty());
        assert_eq!(repo.run(&["for-each-ref"]).await.unwrap(), refs);
        let dir = github.path();
        assert_eq!(git(dir, &["rev-parse", "feature"]).await.trim(), &*ours);
        // A merge that's clean has nothing to resolve.
        let clean = merge(&repo, &theirs, &theirs).await.unwrap();
        assert!(clean.conflicted.is_empty());
    }
}
