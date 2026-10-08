//! Repository selection, fetching and object prefetching.
//!
//! A PR's diff is computed in the user's own clone when the current
//! directory is one (any remote pointing at the base repo counts, which covers
//! fork clones). There ghtui only writes refs under `refs/ghtui/` and never
//! touches branches, HEAD, the index or the working tree. Otherwise it keeps a
//! bare partial clone (`--filter=blob:none`) per repository in the cache.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;

use crate::blobs::BlobReader;
use crate::credentials::Credentials;
use crate::files::{ChangedFile, parse_raw};
use crate::{GitError, Oid, Version, git, github_repo_from_url, piped, stdout};

/// `GIT_NO_LAZY_FETCH` lets us ask which objects are missing without
/// fetching them.
const NO_LAZY_FETCH_VERSION: Version = Version(2, 44, 0);
/// `git check-attr --source` reads attributes from a commit (needed in bare
/// repositories).
const CHECK_ATTR_SOURCE_VERSION: Version = Version(2, 40, 0);

/// Progress lines from long-running git commands (clone, fetch).
pub type Progress<'a> = &'a (dyn Fn(String) + Send + Sync);

#[derive(Debug, Clone)]
pub struct Repo {
    pub path: PathBuf,
    pub remote: String,
    /// A promisor (partial clone) remote: blobs may be missing locally.
    pub partial: bool,
    version: Version,
    credentials: Credentials,
}

/// git's empty tree: what a root commit is diffed against.
pub const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// Refs fetched for a PR (or a commit and its parent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrRefs {
    pub head: Oid,
    pub base: Oid,
    /// `merge-base(base, head)`: the diff runs from here to `head`, like
    /// GitHub's three-dot comparison.
    pub merge_base: Oid,
}

/// A commit in a PR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub oid: Oid,
    pub subject: String,
}

/// One lock per repository path: concurrent fetches into one repository
/// conflict on ref and pack locks.
fn repo_lock(path: &Path) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    locks.entry(path.to_owned()).or_default().clone()
}

impl Repo {
    /// The user's clone containing `dir`, if it has a remote for
    /// `owner/name`.
    pub async fn find_local(dir: &Path, owner: &str, name: &str) -> Result<Option<Repo>, GitError> {
        let Ok(top) = crate::run(Some(dir), &["rev-parse", "--show-toplevel"]).await else {
            return Ok(None);
        };
        let top = PathBuf::from(top.trim());
        let remotes = crate::remotes(&top).await?;
        let Some(remote) = remotes.iter().find(|r| {
            github_repo_from_url(&r.url).is_some_and(|repo| {
                repo.owner.eq_ignore_ascii_case(owner) && repo.name.eq_ignore_ascii_case(name)
            })
        }) else {
            return Ok(None);
        };
        Self::open_local(&top, &remote.name).await.map(Some)
    }

    /// The user's clone at `top`, fetching from `remote`.
    pub async fn open_local(top: &Path, remote: &str) -> Result<Repo, GitError> {
        let version = crate::version().await?;
        Self::open(top.to_owned(), remote, version, Credentials::Ambient).await
    }

    /// Opens (cloning on first use) the cache clone of `url` at
    /// `<cache_root>/repos/<owner>/<name>.git`.
    pub async fn open_cache(
        cache_root: &Path,
        owner: &str,
        name: &str,
        url: &str,
        credentials: Credentials,
        progress: Progress<'_>,
    ) -> Result<Repo, GitError> {
        let path = cache_root
            .join("repos")
            .join(owner.to_ascii_lowercase())
            .join(format!("{}.git", name.to_ascii_lowercase()));
        let _guard = repo_lock(&path).lock_owned().await;
        let version = crate::version().await?;
        if !path.join("HEAD").exists() {
            // Clone next to the target and rename, so an interrupted clone
            // never leaves a half-made repository behind.
            let parent = path.parent().ok_or_else(|| {
                std::io::Error::other(format!("no parent for {}", path.display()))
            })?;
            std::fs::create_dir_all(parent)?;
            let tmp = parent.join(format!(".{name}.git.tmp-{}", std::process::id()));
            if tmp.exists() {
                std::fs::remove_dir_all(&tmp)?;
            }
            progress(format!("Cloning {owner}/{name}"));
            let mut cmd = git(None);
            credentials.apply(&mut cmd);
            cmd.args(["clone", "--bare", "--filter=blob:none", "--progress", url])
                .arg(&tmp);
            if let Err(err) = run_with_progress(cmd, "clone", None, progress, STALL).await {
                // The clone's error is the one to report; a half-made
                // directory left behind is removed on the next try.
                let _ = std::fs::remove_dir_all(&tmp);
                return Err(err);
            }
            std::fs::rename(&tmp, &path)?;
        }
        Self::open(path, "origin", version, credentials).await
    }

    async fn open(
        path: PathBuf,
        remote: &str,
        version: Version,
        credentials: Credentials,
    ) -> Result<Repo, GitError> {
        let mut repo = Repo {
            path,
            remote: remote.to_owned(),
            partial: false,
            version,
            credentials,
        };
        repo.partial = repo.is_promisor().await;
        Ok(repo)
    }

    fn cmd(&self) -> Command {
        let mut cmd = git(Some(&self.path));
        self.credentials.apply(&mut cmd);
        cmd
    }

    async fn run(&self, args: &[&str]) -> Result<String, GitError> {
        stdout(&self.cmd().args(args).output().await?, &args.join(" "))
    }

    async fn is_promisor(&self) -> bool {
        let key = format!("remote.{}.promisor", self.remote);
        self.run(&["config", "--get", &key])
            .await
            .is_ok_and(|v| v.trim() == "true")
    }

    fn ref_name(number: u64, what: &str) -> String {
        format!("refs/ghtui/pr/{number}/{what}")
    }

    /// Fetches the PR head and its base branch into `refs/ghtui/pr/<N>/`
    /// and computes the merge base.
    ///
    /// `base_oid` is the base the PR had, when it's no longer its branch's
    /// tip: once merged with a merge commit, a PR's head is in its base
    /// branch, so the tip would leave nothing to diff.
    pub async fn fetch_pr(
        &self,
        number: u64,
        base_branch: &str,
        base_oid: Option<&Oid>,
        progress: Progress<'_>,
    ) -> Result<PrRefs, GitError> {
        let head_ref = Self::ref_name(number, "head");
        let base_ref = Self::ref_name(number, "base");
        progress(format!("Fetching #{number}"));
        let head_spec = format!("+refs/pull/{number}/head:{head_ref}");
        let base_spec = format!("+refs/heads/{base_branch}:{base_ref}");
        self.fetch(&[&head_spec, &base_spec], progress).await?;
        let head = self.rev_parse(&head_ref).await?;
        let base = match base_oid {
            Some(oid) => self.ensure(oid, progress).await.map(|()| oid.clone())?,
            None => self.rev_parse(&base_ref).await?,
        };
        let merge_base = self.merge_base(&base, &head).await?;
        Ok(PrRefs {
            head,
            base,
            merge_base,
        })
    }

    pub async fn merge_base(&self, a: &str, b: &str) -> Result<Oid, GitError> {
        Oid::parse(self.run(&["merge-base", a, b]).await?.trim())
    }

    /// Commits in `from..to`, oldest first.
    pub async fn commits(&self, from: &str, to: &str) -> Result<Vec<Commit>, GitError> {
        let out = self
            .run(&[
                "log",
                "--reverse",
                "--format=%H%x1f%s",
                &format!("{from}..{to}"),
            ])
            .await?;
        Ok(out
            .lines()
            .filter_map(|l| l.split_once('\u{1f}'))
            .filter_map(|(sha, subject)| {
                Some(Commit {
                    oid: Oid::parse(sha).ok()?,
                    subject: subject.to_owned(),
                })
            })
            .collect())
    }

    pub async fn rev_parse(&self, rev: &str) -> Result<Oid, GitError> {
        let out = self
            .run(&[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{rev}^{{commit}}"),
            ])
            .await?;
        Oid::parse(out.trim())
    }

    /// Pins `sha` as `refs/ghtui/pr/<N>/seen/<sha>` so force-pushes and
    /// `git gc` can't remove a head we've shown.
    pub async fn pin_seen(&self, number: u64, sha: &str) -> Result<(), GitError> {
        let name = Self::ref_name(number, &format!("seen/{sha}"));
        self.run(&["update-ref", &name, sha]).await.map(drop)
    }

    /// Whether `rev` (e.g. `<sha>:<path>`) resolves locally, without lazy
    /// fetching.
    pub async fn has(&self, rev: &str) -> bool {
        self.cmd()
            .env("GIT_NO_LAZY_FETCH", "1")
            .args(["cat-file", "-e", rev])
            .status()
            .await
            .is_ok_and(|s| s.success())
    }

    /// A commit against its first parent, the empty tree for a root commit;
    /// fetched by SHA if it isn't here.
    pub async fn commit_refs(&self, oid: &Oid, progress: Progress<'_>) -> Result<PrRefs, GitError> {
        self.ensure(oid, progress).await?;
        let parent = match self.rev_parse(&format!("{oid}^")).await {
            Ok(parent) => parent,
            Err(_) => Oid::parse(EMPTY_TREE)?,
        };
        Ok(PrRefs {
            head: oid.clone(),
            base: parent.clone(),
            merge_base: parent,
        })
    }

    /// The diff from one commit to another, each fetched by SHA if it
    /// isn't here.
    pub async fn range_refs(
        &self,
        from: &Oid,
        to: &Oid,
        progress: Progress<'_>,
    ) -> Result<PrRefs, GitError> {
        self.ensure(from, progress).await?;
        self.ensure(to, progress).await?;
        Ok(PrRefs {
            head: to.clone(),
            base: from.clone(),
            merge_base: from.clone(),
        })
    }
    /// Fetches the remote's default branch into `into` (a ref name).
    pub async fn fetch_head(&self, into: &str, progress: Progress<'_>) -> Result<(), GitError> {
        self.fetch(&[&format!("+HEAD:{into}")], progress).await
    }

    /// Fetches `refspecs` (refs, or commits by SHA) from the remote,
    /// reporting git's progress. Every fetch of refs or commits goes
    /// through here, so each is given up on only when it goes quiet.
    async fn fetch(&self, refspecs: &[&str], progress: Progress<'_>) -> Result<(), GitError> {
        let _guard = repo_lock(&self.path).lock_owned().await;
        let mut cmd = self.cmd();
        cmd.args([
            "fetch",
            "--no-tags",
            "--no-write-fetch-head",
            "--progress",
            &self.remote,
        ])
        .args(refspecs);
        run_with_progress(cmd, "fetch", None, progress, STALL)
            .await
            .map(drop)
    }

    /// Every file's path at `rev`.
    pub async fn file_names(&self, rev: &str) -> Result<Vec<String>, GitError> {
        let out = self
            .run(&["ls-tree", "-r", "-z", "--name-only", rev])
            .await?;
        Ok(out
            .split('\0')
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
            .collect())
    }

    /// A file's text at `rev` (fetched, in a partial clone, if it isn't
    /// here).
    pub async fn file_text(&self, rev: &str, path: &str) -> Result<String, GitError> {
        self.run(&["cat-file", "blob", &format!("{rev}:{path}")])
            .await
    }
    /// Makes sure a commit is here, fetching it by SHA if it isn't (e.g. a
    /// head that was force-pushed away). GitHub serves commits it still has
    /// even when no ref points at them.
    pub async fn ensure(&self, oid: &Oid, progress: Progress<'_>) -> Result<(), GitError> {
        if self.has(oid).await {
            return Ok(());
        }
        progress(format!("Fetching {}", oid.get(..7).unwrap_or(oid)));
        self.fetch(&[oid], progress).await
    }

    /// Files changed from `from` to `to`, with rename detection. In a
    /// partial clone git fetches the blobs rename detection needs in one
    /// batch.
    pub async fn changed_files(&self, from: &str, to: &str) -> Result<Vec<ChangedFile>, GitError> {
        let out = self
            .run(&["diff", "--raw", "-z", "--no-abbrev", "-M", from, to])
            .await?;
        parse_raw(&out)
    }

    /// Which of `oids` aren't present locally. Without `GIT_NO_LAZY_FETCH`
    /// (git < 2.44) this can't be asked safely, so everything counts as
    /// missing.
    pub async fn missing(&self, oids: &[String]) -> Result<Vec<String>, GitError> {
        if oids.is_empty() {
            return Ok(Vec::new());
        }
        if self.version < NO_LAZY_FETCH_VERSION {
            return Ok(oids.to_vec());
        }
        let mut cmd = self.cmd();
        cmd.env("GIT_NO_LAZY_FETCH", "1")
            .args(["cat-file", "--batch-check=%(objectname)"]);
        let out = run_with_stdin(cmd, "cat-file --batch-check", oids.join("\n") + "\n").await?;
        Ok(out
            .lines()
            .filter_map(|l| l.strip_suffix(" missing"))
            .map(str::to_owned)
            .collect())
    }

    /// Fetches the given blobs in a single request if any are missing.
    /// Returns how many were fetched. No-op outside partial clones.
    pub async fn prefetch(&self, oids: &[String]) -> Result<usize, GitError> {
        if !self.partial {
            return Ok(0);
        }
        let mut unique: Vec<String> = oids.to_vec();
        unique.sort();
        unique.dedup();
        let missing = self.missing(&unique).await?;
        if missing.is_empty() {
            return Ok(0);
        }
        let _guard = repo_lock(&self.path).lock_owned().await;
        // The same invocation git uses for its own lazy fetches, but for all
        // the blobs at once.
        let mut cmd = self.cmd();
        cmd.args([
            "-c",
            "fetch.negotiationAlgorithm=noop",
            "fetch",
            "--no-tags",
            "--no-write-fetch-head",
            "--recurse-submodules=no",
            "--filter=blob:none",
            "--progress",
            "--stdin",
            &self.remote,
        ]);
        let input = Some(missing.join("\n") + "\n");
        run_with_progress(cmd, "fetch --stdin", input, &|_| {}, STALL).await?;
        Ok(missing.len())
    }

    /// Paths marked `linguist-generated` or `linguist-vendored` in the
    /// `.gitattributes` at `commit`. Empty on git < 2.40.
    pub async fn generated_paths(
        &self,
        commit: &str,
        paths: &[&str],
    ) -> Result<HashSet<String>, GitError> {
        if paths.is_empty() || self.version < CHECK_ATTR_SOURCE_VERSION {
            return Ok(HashSet::new());
        }
        let mut cmd = self.cmd();
        cmd.args([
            "check-attr",
            "-z",
            "--stdin",
            &format!("--source={commit}"),
            "linguist-generated",
            "linguist-vendored",
        ]);
        let mut input = paths.join("\0");
        input.push('\0');
        let out = run_with_stdin(cmd, "check-attr", input).await?;
        let fields: Vec<&str> = out.split('\0').collect();
        let (triples, _) = fields.as_chunks::<3>();
        Ok(triples
            .iter()
            .filter(|[_, _, value]| matches!(*value, "set" | "true"))
            .map(|[path, _, _]| (*path).to_owned())
            .collect())
    }

    pub fn blob_reader(&self) -> Result<BlobReader, GitError> {
        let repo = self.clone();
        BlobReader::spawn(move || repo.cmd())
    }
}

async fn run_with_stdin(mut cmd: Command, what: &str, input: String) -> Result<String, GitError> {
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let writer = feed(&mut child, input)?;
    let output = child.wait_with_output().await?;
    let _ = writer.await;
    stdout(&output, what)
}

/// Writes `input` to `child`'s piped stdin from its own task, so git can
/// fill its output pipes while it reads.
fn feed(child: &mut Child, input: String) -> Result<JoinHandle<()>, GitError> {
    let mut stdin = piped(child.stdin.take(), "stdin")?;
    Ok(tokio::spawn(async move {
        let _ = stdin.write_all(input.as_bytes()).await;
    }))
}

/// How long a network operation may go without printing progress before
/// it's given up on.
const STALL: Duration = Duration::from_secs(180);

/// Runs `cmd` (writing `input` to its stdin, if any), reporting the
/// latest `--progress` line from stderr, and gives up if it goes quiet for
/// `stall` (normally [`STALL`]).
#[expect(clippy::disallowed_methods, reason = "times silence, not the run")]
async fn run_with_progress(
    mut cmd: Command,
    what: &str,
    input: Option<String>,
    progress: Progress<'_>,
    stall: Duration,
) -> Result<String, GitError> {
    if input.is_some() {
        cmd.stdin(Stdio::piped());
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let writer = input.map(|input| feed(&mut child, input)).transpose()?;
    let mut stderr = piped(child.stderr.take(), "stderr")?;
    let mut stdout = piped(child.stdout.take(), "stdout")?;
    let out_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf).await;
        buf
    });
    let mut all = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut line = Vec::new();
    loop {
        let Ok(read) = tokio::time::timeout(stall, stderr.read(&mut chunk)).await else {
            let _ = child.kill().await;
            return Err(GitError::Stalled {
                what: what.to_owned(),
                secs: stall.as_secs(),
            });
        };
        let n = read?;
        let Some(read) = chunk.get(..n).filter(|read| !read.is_empty()) else {
            break;
        };
        all.extend_from_slice(read);
        for &b in read {
            if b == b'\r' || b == b'\n' {
                let text = String::from_utf8_lossy(&line).trim().to_owned();
                if !text.is_empty() {
                    progress(text);
                }
                line.clear();
            } else {
                line.push(b);
            }
        }
    }
    if let Some(writer) = writer {
        let _ = writer.await;
    }
    let status = child.wait().await?;
    let stdout = out_task.await.unwrap_or_default();
    if !status.success() {
        let stderr = String::from_utf8_lossy(&all);
        // Progress output is noise in an error; keep the real messages.
        let message: Vec<&str> = stderr
            .split(['\r', '\n'])
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.contains('%'))
            .collect();
        return Err(GitError::Failed {
            args: what.to_owned(),
            stderr: message.join("; "),
        });
    }
    Ok(String::from_utf8_lossy(&stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A network operation that goes quiet is given up on, not waited on
    /// forever.
    #[tokio::test]
    async fn quiet_commands_stall_out() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo 'Receiving objects: 1%' >&2; sleep 30"]);
        let seen = Mutex::new(Vec::new());
        let started = std::time::Instant::now();
        let result = run_with_progress(
            cmd,
            "fetch",
            None,
            &|line| seen.lock().unwrap().push(line),
            Duration::from_millis(300),
        )
        .await;
        assert!(
            matches!(result, Err(GitError::Stalled { .. })),
            "{result:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(*seen.lock().unwrap(), ["Receiving objects: 1%"]);
    }

    /// One that keeps printing progress runs as long as it needs, however
    /// much longer than the silence it would be given up after.
    #[tokio::test]
    async fn moving_commands_outlast_the_stall_window() {
        let mut cmd = Command::new("sh");
        let script = "for i in 1 2 3 4 5 6 7 8 9 10; do echo \"Receiving objects: ${i}0%\" >&2; sleep 0.1; done";
        cmd.args(["-c", script]);
        let result =
            run_with_progress(cmd, "fetch", None, &|_| {}, Duration::from_millis(300)).await;
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn git_gives_up_on_dead_connections() {
        let cmd = git(None);
        let envs: HashMap<_, _> = cmd.as_std().get_envs().collect();
        assert!(envs.contains_key(std::ffi::OsStr::new("GIT_HTTP_LOW_SPEED_TIME")));
    }
}
