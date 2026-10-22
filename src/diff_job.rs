//! The background job behind a diff screen: pick the repository, fetch the
//! PR, list files, prefetch blobs, and compute per-file diffs, sending
//! results to the UI as they're ready.
//!
//! Prefetching runs in two batches: the blobs of the first file in the queue
//! (so something shows quickly), then all the rest in one request. Workers
//! wait for the batch that covers their file, so a partial clone never falls
//! back to fetching blobs one at a time.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use crate::diff::FileDiff;
use crate::git::blobs::BlobReader;
use crate::git::credentials::Credentials;
use crate::git::files::{ChangedFile, blobs, is_lockfile};
use crate::git::repo::{PrRefs, Repo};
use crate::git::{GitError, Oid};
use crate::lock;
use futures::{StreamExt, TryStreamExt};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;

use crate::diff_screen::DiffOf;
use crate::state::{DiffMsg, Msg, OutdatedThread};
use crate::work::Failure;

/// Where and how git runs.
#[derive(Debug, Clone)]
pub struct GitContext {
    pub cache_root: PathBuf,
    pub credentials: Credentials,
    /// Directory ghtui was started in; used to find the user's clone.
    pub cwd: PathBuf,
}

impl GitContext {
    /// `owner/name`'s clone: yours, else the cache's (cloned on first use).
    pub async fn repo(
        &self,
        owner: &str,
        name: &str,
        progress: crate::git::repo::Progress<'_>,
    ) -> Result<Repo, GitError> {
        if let Some(repo) = Repo::find_local(&self.cwd, owner, name).await? {
            tracing::info!(path = %repo.path.display(), remote = repo.remote, "using local clone");
            return Ok(repo);
        }
        let url = crate::host::current().git_url(owner, name);
        let credentials = self.credentials.clone();
        Repo::open_cache(&self.cache_root, owner, name, &url, credentials, progress).await
    }
}

#[derive(Debug)]
pub struct DiffFiles {
    pub refs: PrRefs,
    pub files: Vec<ChangedFile>,
    pub generated: HashSet<String>,
}

/// What a diff job reports, tagged with its [`JobId`] so a restarted
/// job's stragglers can be told apart.
#[derive(Debug)]
pub enum JobMsg {
    Progress(String),
    Files(Box<DiffFiles>),
    File(usize, Arc<FileDiff>),
    Failed(Failure),
    Moves(Vec<crate::diff::moves::Move>),
    Mapped(Vec<(crate::api::model::NodeId, Option<u32>)>),
    Since(String, Result<HashSet<String>, Failure>),
}

/// Identifies one run of a diff job: a refresh or a new commit range gets
/// a new one.
pub type JobId = u64;

/// Sends a job's messages to the UI.
#[derive(Clone)]
pub struct JobTx {
    pub tx: mpsc::UnboundedSender<Msg>,
    pub of: DiffOf,
    pub job: JobId,
}

impl JobTx {
    /// False once the UI is gone.
    pub fn send(&self, msg: JobMsg) -> bool {
        self.tx
            .send(Msg::Diff(self.of.clone(), DiffMsg::Job(self.job, msg)))
            .is_ok()
    }
}

/// A job's repository and blob reader.
pub type RepoGit = (Arc<Repo>, Arc<BlobReader>);

/// Shared between the job's workers and the UI (for prioritizing, and for
/// later work on the same repository such as mapping outdated comments).
#[derive(Default)]
pub struct JobControl {
    /// The files still to diff.
    queue: Mutex<VecDeque<usize>>,
    git: OnceLock<RepoGit>,
}

impl JobControl {
    /// Moves `files` to the front of the queue, in order.
    pub fn prioritize(&self, files: &[usize]) {
        let mut queue = lock(&self.queue);
        for file in files.iter().rev() {
            if let Some(pos) = queue.iter().position(|f| f == file) {
                queue.remove(pos);
                queue.push_front(*file);
            }
        }
    }

    fn pop(&self) -> Option<usize> {
        lock(&self.queue).pop_front()
    }

    fn fill(&self, files: impl IntoIterator<Item = usize>) {
        let mut queue = lock(&self.queue);
        queue.clear();
        queue.extend(files);
    }

    /// The repository and blob reader, once the job has set them up.
    pub fn git(&self) -> Option<RepoGit> {
        self.git.get().cloned()
    }

    fn front(&self) -> Option<usize> {
        lock(&self.queue).front().copied()
    }
}

/// A sub-range of the PR's commits to diff instead of the whole PR.
pub type CommitRange = Option<(String, String)>;

/// What a pull request's diff is from: its base branch, and the base it
/// had once that's no longer the branch's tip (merged or closed).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrBase {
    pub branch: String,
    pub oid: Option<Oid>,
}

/// Runs the job. Its helper tasks belong to it: dropping (aborting) the
/// job stops them too.
pub async fn run(
    ctx: GitContext,
    base: PrBase,
    range: CommitRange,
    out: JobTx,
    control: Arc<JobControl>,
) {
    if let Err(err) = run_inner(&ctx, &base, range, &out, &control).await {
        tracing::warn!(diff = %out.of, %err, "diff job failed");
        out.send(JobMsg::Failed(err.into()));
    }
}

async fn run_inner(
    ctx: &GitContext,
    base: &PrBase,
    range: CommitRange,
    out: &JobTx,
    control: &Arc<JobControl>,
) -> Result<(), GitError> {
    let of = &out.of;
    let progress = {
        let out = out.clone();
        move |line: String| {
            out.send(JobMsg::Progress(line));
        }
    };
    let repo = ctx
        .repo(&of.repo().owner, &of.repo().name, &progress)
        .await?;
    let mut refs = match of {
        DiffOf::Pr(pr) => {
            repo.fetch_pr(pr.number, &base.branch, base.oid.as_ref(), &progress)
                .await?
        }
        DiffOf::Commit(_, oid) => repo.commit_refs(oid, &progress).await?,
        DiffOf::Range(_, from, to) => repo.range_refs(from, to, &progress).await?,
    };
    if let Some((from, to)) = range {
        // A commit a push left behind (a review's) is fetched by its SHA.
        if let Ok(oid) = Oid::parse(&to) {
            repo.ensure(&oid, &progress).await?;
        }
        refs.head = repo.rev_parse(&to).await?;
        // No start: the pull request as it was at `to`, from where it
        // left its base.
        refs.merge_base = if from.is_empty() {
            repo.merge_base(&refs.base, &refs.head).await?
        } else {
            repo.rev_parse(&from).await?
        };
    }
    if let DiffOf::Pr(pr) = of
        && let Err(err) = repo.pin_seen(pr.number, &refs.head).await
    {
        tracing::warn!(%err, "could not pin head");
    }
    progress("Listing changed files".into());
    let files = repo.changed_files(&refs.merge_base, &refs.head).await?;
    let paths: Vec<&str> = files.iter().map(ChangedFile::path).collect();
    let generated = repo
        .generated_paths(&refs.head, &paths)
        .await
        .unwrap_or_else(|err| {
            tracing::warn!(%err, "could not read linguist attributes");
            HashSet::new()
        });

    // Files shown expanded first; collapsed (generated, lockfiles) last.
    let collapsed = |f: &ChangedFile| generated.contains(f.path()) || is_lockfile(f.path());
    let (expanded, collapsed): (Vec<_>, Vec<_>) = (0..files.len())
        .zip(&files)
        .partition(|(_, f)| !collapsed(f));
    let order = expanded.into_iter().chain(collapsed).map(|(i, _)| i);
    control.fill(order);
    // Git is there for the work that waits on the listing (if it doesn't
    // open, the job fails once the files are shown).
    let repo = Arc::new(repo);
    let reader = repo.blob_reader().map(Arc::new);
    if let Ok(reader) = &reader {
        let _ = control.git.set((repo.clone(), reader.clone()));
    }
    out.send(JobMsg::Files(Box::new(DiffFiles {
        refs,
        files: files.clone(),
        generated,
    })));

    let files = Arc::new(files);
    // The first batch covers the head of the queue: the first file, which
    // is where the diff screen opens.
    let first = control.front();
    let (stage_tx, stage_rx) = watch::channel(0u8);
    let mut tasks = JoinSet::new();
    {
        let repo = repo.clone();
        let files = files.clone();
        tasks.spawn(async move {
            let firsts = first.and_then(|i| files.get(i));
            for (stage, batch) in [(1u8, blobs(firsts)), (2, blobs(files.iter()))] {
                match repo.prefetch(batch).await {
                    Ok(n) if n > 0 => tracing::info!(n, stage, "prefetched blobs"),
                    Ok(_) => {}
                    // Reads still work through lazy fetching, just slower.
                    Err(err) => tracing::warn!(%err, "blob prefetch failed"),
                }
                let _ = stage_tx.send(stage);
            }
        });
    }

    let reader = reader?;
    for _ in 0..workers() {
        let control = control.clone();
        let files = files.clone();
        let reader = reader.clone();
        let out = out.clone();
        let mut stage = stage_rx.clone();
        tasks.spawn(async move {
            while let Some(index) = control.pop() {
                let Some(file) = files.get(index) else { continue };
                let needed = if first == Some(index) { 1 } else { 2 };
                if stage.wait_for(|s| *s >= needed).await.is_err() {
                    return;
                }
                let diff = diff_file(&reader, file).await;
                if !out.send(JobMsg::File(index, Arc::new(diff))) {
                    return;
                }
            }
        });
    }
    // A helper's panic is the job's: the runtime reports it.
    while let Some(done) = tasks.join_next().await {
        if let Err(err) = done
            && err.is_panic()
        {
            std::panic::resume_unwind(err.into_panic());
        }
    }
    Ok(())
}

/// How many files are diffed at once.
fn workers() -> usize {
    std::thread::available_parallelism().map_or(2, |n| n.get().clamp(2, 4))
}

/// `file`'s old and new contents, read together: `None` for a side it
/// doesn't have, or git doesn't.
async fn sides(
    reader: &BlobReader,
    file: &ChangedFile,
) -> Result<(Option<Vec<u8>>, Option<Vec<u8>>), GitError> {
    let names: Vec<&str> = file.blob_oids().collect();
    let mut read = reader.read_each(&names).await?.into_iter();
    let mut side = |oid: &Oid| {
        if oid.is_zero() {
            None
        } else {
            read.next().flatten()
        }
    };
    Ok((side(&file.old_oid), side(&file.new_oid)))
}

async fn diff_file(reader: &BlobReader, file: &ChangedFile) -> FileDiff {
    if file.is_submodule() {
        let side = |oid: &Oid| (!oid.is_zero()).then(|| oid.to_string());
        return FileDiff::submodule(side(&file.old_oid), side(&file.new_oid));
    }
    let (old, new) = or_return!(sides(reader, file).await, |err| {
        FileDiff::error(err.to_string())
    });
    let sides = [(&file.old_oid, &old), (&file.new_oid, &new)];
    if let Some((oid, _)) = sides
        .iter()
        .find(|(oid, read)| !oid.is_zero() && read.is_none())
    {
        let short = crate::ui::text::short_sha(oid);
        return FileDiff::error(format!("object {short} is missing"));
    }
    let path = file.path().to_owned();
    tokio::task::spawn_blocking(move || FileDiff::compute(&path, old.as_deref(), new.as_deref()))
        .await
        .unwrap_or_else(|err| FileDiff::error(format!("diff failed: {err}")))
}

/// Block hashes of the PR's diff as it was at `old_head` (your last
/// review). Fetches that commit by SHA if it was force-pushed away.
pub async fn since_review_hashes(
    repo: &Repo,
    reader: &BlobReader,
    number: u64,
    old_head: &str,
) -> Result<HashSet<String>, GitError> {
    repo.ensure(&Oid::parse(old_head)?, &|_| {}).await?;
    let base = repo
        .rev_parse(&format!("refs/ghtui/pr/{number}/base"))
        .await?;
    let merge_base = repo.merge_base(&base, old_head).await?;
    let files = repo.changed_files(&merge_base, old_head).await?;
    if let Err(err) = repo.prefetch(blobs(&files)).await {
        tracing::warn!(%err, "prefetch for the earlier diff failed");
    }
    // Diffed a few at once, as the job does.
    let files = files.into_iter().filter(|f| !f.is_submodule());
    let diffs = futures::stream::iter(files)
        .map(|file| async move {
            let (old, new) = sides(reader, &file).await?;
            let path = file.path().to_owned();
            let plain = move || FileDiff::compute_plain(&path, old.as_deref(), new.as_deref());
            let diff = tokio::task::spawn_blocking(plain).await;
            Ok::<_, GitError>(diff.ok().map(|d| d.content))
        })
        .buffer_unordered(workers());
    let mut diffs = std::pin::pin!(diffs);
    let mut hashes = HashSet::new();
    while let Some(content) = diffs.try_next().await? {
        if let Some(crate::diff::Content::Text(text)) = content {
            let exact = text.alignment(crate::diff::Whitespace::Exact);
            hashes.extend(exact.blocks.iter().map(|b| b.hash.clone()));
        }
    }
    Ok(hashes)
}

/// Where outdated threads' lines are now: reads each file at a thread's
/// original commit and at `head`, and maps the line if it survived
/// unchanged. Fetches an original commit by SHA if it isn't local. Threads
/// on the same file and commit share the reads and the diff.
pub async fn map_outdated(
    repo: &Repo,
    reader: &BlobReader,
    head: &str,
    threads: &[OutdatedThread],
) -> Vec<(crate::api::model::NodeId, Option<u32>)> {
    let mut maps = HashMap::new();
    let mut mapped = Vec::with_capacity(threads.len());
    for t in threads {
        let map = match maps.entry((t.commit.as_str(), t.path.as_str())) {
            Entry::Occupied(known) => known.into_mut(),
            Entry::Vacant(new) => {
                new.insert(line_map(repo, reader, head, &t.path, &t.commit).await)
            }
        };
        let to = map.as_ref().and_then(|m| m.get(&t.line).copied());
        mapped.push((t.thread.clone(), to));
    }
    mapped
}

/// [`crate::diff::anchor::line_map`] of `path` from `original_commit` to
/// `head`; `None` when either side can't be read.
async fn line_map(
    repo: &Repo,
    reader: &BlobReader,
    head: &str,
    path: &str,
    original_commit: &str,
) -> Option<HashMap<u32, u32>> {
    if let Err(err) = async { repo.ensure(&Oid::parse(original_commit)?, &|_| {}).await }.await {
        tracing::info!(%err, original_commit, "original commit unavailable");
        return None;
    }
    // Both sides asked together: one wait on git.
    let names = [
        format!("{original_commit}:{path}"),
        format!("{head}:{path}"),
    ];
    let read = reader
        .read_each(&names.each_ref().map(String::as_str))
        .await;
    let [Some(old), Some(new)] = <[_; 2]>::try_from(read.ok()?).ok()? else {
        return None;
    };
    let diff = move || {
        let (old, new) = (crate::diff::Text::new(&old), crate::diff::Text::new(&new));
        crate::diff::anchor::line_map(&old, &new)
    };
    tokio::task::spawn_blocking(diff).await.ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::fixture::{cache_clone, commit, origin};
    use crate::git::set_up as git;

    #[test]
    fn prioritize_moves_files_to_front_in_order() {
        let control = JobControl::default();
        control.fill([0, 1, 2, 3, 4]);
        control.prioritize(&[3, 1]);
        let order: Vec<usize> = std::iter::from_fn(|| control.pop()).collect();
        assert_eq!(order, [3, 1, 0, 2, 4]);
        control.prioritize(&[9]);
        assert_eq!(control.pop(), None);
    }

    /// The whole job, for a PR merged with a merge commit: its file list
    /// is the PR's, from the base it had, not empty from `main` after.
    #[tokio::test]
    async fn a_merged_prs_job_lists_its_files() {
        let tmp = tempfile::tempdir().unwrap();
        let o = tmp.path().join("origin");
        origin(&o).await;
        let base = commit(&o, &[("a.txt", "a\n")], "base").await;
        git(&o, &["checkout", "-q", "-b", "feature"]).await;
        commit(&o, &[("b.txt", "b\n")], "b").await;
        git(&o, &["update-ref", "refs/pull/7/head", "feature"]).await;
        git(&o, &["checkout", "-q", "main"]).await;
        git(&o, &["merge", "-q", "--no-ff", "-m", "Merge #7", "feature"]).await;

        // The user's clone of github.com/o/r, which git fetches from the
        // local "GitHub" instead.
        let local = tmp.path().join("local");
        let url = format!("file://{}", o.display());
        let github = "https://github.com/o/r.git";
        git(tmp.path(), &["clone", "-q", &url, "local"]).await;
        git(&local, &["remote", "set-url", "origin", github]).await;
        let instead = format!("url.{url}.insteadOf");
        git(&local, &["config", &instead, github]).await;

        let ctx = GitContext {
            cache_root: tmp.path().join("cache"),
            credentials: Credentials::Ambient,
            cwd: local,
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        let of = DiffOf::Pr(crate::api::model::PrRef::parse("o/r#7").unwrap());
        let out = JobTx { tx, of, job: 1 };
        let base = PrBase {
            branch: "main".into(),
            oid: Some(Oid::parse(&base).unwrap()),
        };
        run(ctx, base, None, out, Arc::default()).await;
        let mut listed = None;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                Msg::Diff(_, DiffMsg::Job(_, JobMsg::Files(files))) => listed = Some(files),
                Msg::Diff(_, DiffMsg::Job(_, JobMsg::Failed(err))) => panic!("{err}"),
                _ => {}
            }
        }
        let files = listed.expect("the job listed files");
        let paths: Vec<&str> = files.files.iter().map(ChangedFile::path).collect();
        assert_eq!(paths, ["b.txt"]);
    }

    /// An outdated comment's line, found again where it moved: from the
    /// commit it was written on (fetched by SHA once force-pushed away) to
    /// the head. A line that changed stays outdated.
    #[tokio::test]
    async fn outdated_lines_map_to_where_they_moved() {
        let lines = |n: u32| (1..=n).map(|i| format!("line {i}\n")).collect::<String>();

        let tmp = tempfile::tempdir().unwrap();
        let o = tmp.path();
        origin(o).await;
        commit(o, &[("a.txt", &lines(20))], "base").await;
        git(o, &["checkout", "-q", "-b", "feature"]).await;
        let v1 = lines(20).replace("line 3\n", "three\n");
        let written_on = commit(o, &[("a.txt", &v1)], "v1").await;
        // Force-pushed: v1 is gone from every branch; v2 adds two lines on
        // top and changes line 15.
        git(o, &["reset", "-q", "--hard", "main"]).await;
        let v2 = format!("new 1\nnew 2\n{}", v1.replace("line 15\n", "fifteen\n"));
        commit(o, &[("a.txt", &v2)], "v2").await;
        git(o, &["update-ref", "refs/pull/1/head", "feature"]).await;

        let (_cache, repo) = cache_clone(o).await;
        let head = repo.fetch_pr(1, "main", None, &|_| {}).await.unwrap().head;
        let reader = repo.blob_reader().unwrap();
        let thread = |n: u32, commit: &str, line| OutdatedThread {
            thread: crate::api::model::NodeId::new(n.to_string()),
            path: "a.txt".into(),
            commit: commit.into(),
            line,
        };
        let threads = [
            thread(1, &written_on, 10),
            thread(2, &written_on, 3),
            thread(3, &written_on, 15),
            thread(4, &"f".repeat(40), 1),
        ];
        let mapped = map_outdated(&repo, &reader, &head, &threads).await;
        let to: Vec<Option<u32>> = mapped.into_iter().map(|(_, to)| to).collect();
        // "three" is unchanged since; line 15 changed; the last commit
        // doesn't exist.
        assert_eq!(to, [Some(12), Some(5), None, None]);
    }

    /// "Since my last review" against a real force-push and rebase: the
    /// PR is rebased onto a `main` that moved on, and gains one change.
    /// Only that change is new; the rebase and the old change aren't.
    #[tokio::test]
    async fn since_review_survives_force_push_and_rebase() {
        let numbered = |n: u32| (1..=n).map(|i| format!("line {i}\n")).collect::<String>();

        let tmp = tempfile::tempdir().unwrap();
        let o = tmp.path();
        origin(o).await;
        commit(o, &[("a.txt", &numbered(60))], "base").await;

        // v1 of the PR: one change near the top.
        git(o, &["checkout", "-q", "-b", "feature"]).await;
        let v1 = numbered(60).replace("line 5\n", "five\n");
        commit(o, &[("a.txt", &v1)], "v1").await;
        git(o, &["update-ref", "refs/pull/1/head", "feature"]).await;

        let (_cache, repo) = cache_clone(o).await;
        let reviewed = repo.fetch_pr(1, "main", None, &|_| {}).await.unwrap().head;

        // main moves on (unrelated change), and the PR is rebased onto it,
        // keeps its first change and adds a second: force-pushed.
        git(o, &["checkout", "-q", "main"]).await;
        let upstream = numbered(60).replace("line 30\n", "thirty\n");
        commit(o, &[("a.txt", &upstream)], "upstream").await;
        git(o, &["checkout", "-q", "-B", "feature", "main"]).await;
        let rebased = upstream
            .replace("line 5\n", "five\n")
            .replace("line 55\n", "fifty-five\n");
        commit(o, &[("a.txt", &rebased)], "v2").await;
        git(o, &["update-ref", "refs/pull/1/head", "feature"]).await;
        let now = repo.fetch_pr(1, "main", None, &|_| {}).await.unwrap();
        assert_ne!(now.head, reviewed);

        let reader = repo.blob_reader().unwrap();
        let seen = since_review_hashes(&repo, &reader, 1, &reviewed)
            .await
            .unwrap();

        // The current diff's blocks: "five" (seen before) and "fifty-five".
        let files = repo
            .changed_files(&now.merge_base, &now.head)
            .await
            .unwrap();
        assert_eq!(files.len(), 1);
        let old = reader.read(&files[0].old_oid).await.unwrap().unwrap();
        let new = reader.read(&files[0].new_oid).await.unwrap().unwrap();
        let diff = FileDiff::compute_plain("a.txt", Some(&old), Some(&new));
        let crate::diff::Content::Text(text) = &diff.content else { panic!() };
        let exact = text.alignment(crate::diff::Whitespace::Exact);
        let blocks = &exact.blocks;
        let new_blocks: Vec<String> = blocks
            .iter()
            .filter(|b| !seen.contains(&b.hash))
            .map(|b| {
                text.line(exact.lines[b.entries.end as usize - 1].shown())
                    .to_owned()
            })
            .collect();
        assert_eq!(
            blocks.len(),
            2,
            "upstream's change isn't part of the PR diff"
        );
        assert_eq!(new_blocks, ["fifty-five"]);
    }
}
