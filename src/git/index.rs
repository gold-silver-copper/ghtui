//! What the code index asks of git: the commit a revision names (fetched
//! when it's GitHub's), a commit's files with their blobs and sizes, and
//! `git grep` for a whole word, at a commit or over a set of blobs.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::git::repo::Repo;
use crate::git::{GitError, Oid, piped, stdout};

/// A file in a commit's tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeFile {
    pub path: String,
    pub blob: String,
    /// Unknown in a partial clone until the blob is fetched.
    pub size: Option<u64>,
}

/// A line a word is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub path: String,
    pub line: u32,
    pub text: String,
}

/// What `git grep` found, up to a limit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hits {
    pub hits: Vec<Hit>,
    /// More lines matched than were kept.
    pub more: bool,
}

/// The commit `rev` names on GitHub. A commit ID is fetched if it isn't
/// here; a branch or tag is fetched into `refs/ghtui/at/<rev>`, so a
/// stale local branch is never taken for GitHub's. Offline, the last
/// fetch of it is used, and the error comes back with it.
pub async fn resolve(repo: &Repo, rev: &str) -> Result<(Oid, Option<GitError>), GitError> {
    let quiet = |_: String| {};
    if let Ok(oid) = Oid::parse(rev) {
        repo.ensure(&oid, &quiet).await?;
        return Ok((oid, None));
    }
    if rev.is_empty()
        || rev.starts_with('-')
        || rev.contains(|c: char| c == ':' || c.is_whitespace() || c.is_control())
    {
        return Err(GitError::Parse(format!("not a revision: {rev:?}")));
    }
    let local = format!("refs/ghtui/at/{rev}");
    let fetched = repo.fetch(&[&format!("+{rev}:{local}")], &quiet).await;
    match (fetched, repo.rev_parse(&local).await) {
        (Ok(()), Ok(oid)) => Ok((oid, None)),
        (Err(err), Ok(oid)) => Ok((oid, Some(err))),
        (Err(err), Err(_)) | (Ok(()), Err(err)) => Err(err),
    }
}

/// Every file (not submodule) in `commit`'s tree, with its blob and size.
/// A partial clone can lack some of its folders (trees): they're fetched
/// a level at a time in one request each, never one by one as listing
/// them would.
pub async fn tree_files(repo: &Repo, commit: &Oid) -> Result<Vec<TreeFile>, GitError> {
    if repo.partial {
        fetch_trees(repo, commit).await?;
    }
    let mut cmd = repo.cmd();
    cmd.env("GIT_NO_LAZY_FETCH", "1")
        .args(["ls-tree", "-r", "-z", "--full-tree"]);
    // Sizes are read from the blobs: a partial clone has few of them,
    // and looking for each missing one is slow.
    if !repo.partial {
        cmd.arg("-l");
    }
    cmd.arg(&**commit);
    let out = stdout(&cmd.output().await?, "ls-tree")?;
    Ok(parse_tree(&out))
}

/// Folders a tree goes down at most.
const DEPTH: usize = 64;

/// Fetches the trees of `commit` a partial clone lacks.
async fn fetch_trees(repo: &Repo, commit: &Oid) -> Result<(), GitError> {
    for _ in 0..DEPTH {
        let mut cmd = repo.cmd();
        cmd.env("GIT_NO_LAZY_FETCH", "1").args([
            "rev-list",
            "--objects",
            "--no-walk",
            "--missing=print",
            "--filter=blob:none",
            "--no-object-names",
            commit,
        ]);
        let out = stdout(&cmd.output().await?, "rev-list --missing=print")?;
        let missing: Vec<String> = (out.lines())
            .filter_map(|l| l.strip_prefix('?'))
            .map(str::to_owned)
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        repo.prefetch(&missing).await?;
    }
    Ok(())
}

/// `<mode> blob <oid> <size>\t<path>` records, NUL-terminated.
fn parse_tree(out: &str) -> Vec<TreeFile> {
    let file = |record: &str| {
        let (meta, path) = record.split_once('\t')?;
        let mut meta = meta.split_whitespace();
        let (_mode, kind, blob) = (meta.next()?, meta.next()?, meta.next()?);
        (kind == "blob").then(|| TreeFile {
            path: path.to_owned(),
            blob: blob.to_owned(),
            size: meta.next().and_then(|s| s.parse().ok()),
        })
    };
    out.split('\0').filter_map(file).collect()
}

/// A tree of `files` (path and blob, all of them here), written to
/// `repo` through a scratch index at `scratch`, so `git grep` reads
/// those and nothing it would have to fetch.
pub async fn tree_of(repo: &Repo, files: &[(&str, &str)], scratch: &Path) -> Result<Oid, GitError> {
    let _ = std::fs::remove_file(scratch);
    let mut input = String::new();
    for (path, blob) in files {
        if !path.contains(['\n', '\0']) {
            input.push_str(&format!("100644 {blob}\t{path}\n"));
        }
    }
    let mut cmd = repo.cmd();
    cmd.env("GIT_INDEX_FILE", scratch)
        .env("GIT_NO_LAZY_FETCH", "1")
        .args(["update-index", "--index-info"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let mut stdin = piped(child.stdin.take(), "stdin")?;
    let writer = tokio::spawn(async move {
        let _ = stdin.write_all(input.as_bytes()).await;
    });
    let output = child.wait_with_output().await?;
    let _ = writer.await;
    stdout(&output, "update-index --index-info")?;
    let mut cmd = repo.cmd();
    cmd.env("GIT_INDEX_FILE", scratch)
        .env("GIT_NO_LAZY_FETCH", "1")
        .arg("write-tree");
    let tree = stdout(&cmd.output().await?, "write-tree");
    let _ = std::fs::remove_file(scratch);
    Oid::parse(tree?.trim())
}

/// The lines of `tree` (a commit or a tree) that have `word` as a whole
/// word, in text files, at most `limit` of them; it stops at `within`.
/// Nothing is fetched: in a partial clone, `tree` must be of blobs here.
pub async fn grep_word(
    repo: &Repo,
    tree: &Oid,
    word: &str,
    limit: usize,
    within: Duration,
) -> Result<Hits, GitError> {
    let mut cmd = repo.cmd();
    cmd.env("GIT_NO_LAZY_FETCH", "1")
        .args(["grep", "-n", "-w", "-I", "-F", "-z", "--no-color", "-e"])
        .arg(word)
        .arg(&**tree)
        .arg("--")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let mut lines = BufReader::new(piped(child.stdout.take(), "stdout")?).lines();
    let prefix = format!("{tree}:");
    let mut found = Hits::default();
    let read = async {
        while let Some(line) = lines.next_line().await? {
            if found.hits.len() >= limit {
                found.more = true;
                break;
            }
            if let Some(hit) = parse_hit(&line, &prefix) {
                found.hits.push(hit);
            }
        }
        Ok::<_, GitError>(())
    };
    #[expect(clippy::disallowed_methods, reason = "a local read with a deadline")]
    let timed = tokio::time::timeout(within, read).await;
    match timed {
        Ok(read) => read?,
        Err(_) => found.more = true,
    }
    if found.more {
        // Enough: the rest isn't wanted.
        let _ = child.start_kill();
        return Ok(found);
    }
    let output = child.wait_with_output().await?;
    // 1: no line matched.
    if !output.status.success() && output.status.code() != Some(1) {
        return Err(GitError::Failed {
            args: "grep".to_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(found)
}

/// `<tree>:<path>\0<line>\0<text>`.
fn parse_hit(line: &str, prefix: &str) -> Option<Hit> {
    let mut parts = line.strip_prefix(prefix)?.splitn(3, '\0');
    let (path, number, text) = (parts.next()?, parts.next()?, parts.next()?);
    Some(Hit {
        path: path.to_owned(),
        line: number.parse().ok()?,
        text: text.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::git::set_up as git;

    /// A repository with a commit of two Rust files and a binary one.
    async fn repo() -> (tempfile::TempDir, Repo, Oid) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        git(path, &["init", "-q", "-b", "main"]).await;
        std::fs::create_dir(path.join("src")).unwrap();
        std::fs::write(path.join("src/a.rs"), "fn run() {}\nfn rung() { run() }\n").unwrap();
        std::fs::write(path.join("b.rs"), "// run, run\nfn other() {}\n").unwrap();
        std::fs::write(path.join("bin"), b"run\0\x01").unwrap();
        git(path, &["add", "."]).await;
        git(
            path,
            &[
                "-c",
                "user.name=a",
                "-c",
                "user.email=a@b",
                "commit",
                "-qm",
                "x",
            ],
        )
        .await;
        let repo = Repo::open_local(path, "origin").await.unwrap();
        let head = repo.rev_parse("HEAD").await.unwrap();
        (dir, repo, head)
    }

    /// A commit's files come with their blobs and sizes; a word is found
    /// whole, in text files only, at that commit or over a tree of blobs.
    #[tokio::test]
    async fn lists_files_and_greps_whole_words() {
        let (dir, repo, head) = repo().await;
        let files = tree_files(&repo, &head).await.unwrap();
        let paths: Vec<(&str, Option<u64>)> =
            files.iter().map(|f| (f.path.as_str(), f.size)).collect();
        assert_eq!(
            paths,
            [("b.rs", Some(26)), ("bin", Some(5)), ("src/a.rs", Some(32))]
        );

        let second = Duration::from_secs(5);
        let found = grep_word(&repo, &head, "run", 10, second).await.unwrap();
        let at: Vec<(&str, u32)> = (found.hits.iter())
            .map(|h| (h.path.as_str(), h.line))
            .collect();
        assert_eq!(at, [("b.rs", 1), ("src/a.rs", 1), ("src/a.rs", 2)]);
        assert_eq!(found.hits.last().unwrap().text, "fn rung() { run() }");
        assert!(!found.more);
        let capped = grep_word(&repo, &head, "run", 2, second).await.unwrap();
        assert_eq!((capped.hits.len(), capped.more), (2, true));
        let none = grep_word(&repo, &head, "nothing", 10, second)
            .await
            .unwrap();
        assert_eq!(none, Hits::default());

        let a = files.iter().find(|f| f.path == "src/a.rs").unwrap();
        let scratch = dir.path().join(".git/ghtui-scratch-index");
        let tree = tree_of(&repo, &[("src/a.rs", &a.blob)], &scratch)
            .await
            .unwrap();
        let found = grep_word(&repo, &tree, "run", 10, second).await.unwrap();
        assert_eq!(found.hits.len(), 2, "only the tree's file");
        assert!(!scratch.exists());

        let (oid, stale) = resolve(&repo, &head).await.unwrap();
        assert_eq!((oid, stale.is_none()), (head, true));
        resolve(&repo, "-x").await.unwrap_err();
    }
}
