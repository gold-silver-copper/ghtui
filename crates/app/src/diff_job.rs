//! The background job behind a diff screen: pick the repository, fetch the
//! PR, list files, prefetch blobs, and compute per-file diffs, sending
//! results to the UI as they're ready.
//!
//! Prefetching runs in two batches: the blobs of the first file in the queue
//! (so something shows quickly), then all the rest in one request. Workers
//! wait for the batch that covers their file, so a partial clone never falls
//! back to fetching blobs one at a time.

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use ghtui_api::model::PrRef;
use ghtui_diff::FileDiff;
use ghtui_git::GitError;
use ghtui_git::blobs::BlobReader;
use ghtui_git::credentials::Credentials;
use ghtui_git::files::{ChangedFile, ZERO_OID, is_lockfile};
use ghtui_git::repo::{PrRefs, Repo};
use tokio::sync::{mpsc, watch};

use crate::state::Msg;

/// Where and how git runs.
#[derive(Debug, Clone)]
pub struct GitContext {
    pub cache_root: PathBuf,
    pub credentials: Credentials,
    /// Directory ghtui was started in; used to find the user's clone.
    pub cwd: PathBuf,
}

#[derive(Debug)]
pub struct DiffFiles {
    pub refs: PrRefs,
    pub files: Vec<ChangedFile>,
    pub generated: HashSet<String>,
}

/// Shared between the job's workers and the UI (for prioritizing, and for
/// later work on the same repository such as mapping outdated comments).
#[derive(Default)]
pub struct JobControl {
    queue: Mutex<VecDeque<usize>>,
    git: OnceLock<(Arc<Repo>, Arc<BlobReader>)>,
}

impl JobControl {
    /// Moves `files` to the front of the queue, in order.
    pub fn prioritize(&self, files: &[usize]) {
        let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        for file in files.iter().rev() {
            if let Some(pos) = queue.iter().position(|f| f == file) {
                queue.remove(pos);
                queue.push_front(*file);
            }
        }
    }

    fn pop(&self) -> Option<usize> {
        self.queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front()
    }

    fn fill(&self, files: impl IntoIterator<Item = usize>) {
        let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        queue.clear();
        queue.extend(files);
    }

    /// The repository and blob reader, once the job has set them up.
    pub fn git(&self) -> Option<(Arc<Repo>, Arc<BlobReader>)> {
        self.git.get().cloned()
    }

    fn front(&self) -> Option<usize> {
        self.queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .front()
            .copied()
    }
}

/// A sub-range of the PR's commits to diff instead of the whole PR.
pub type CommitRange = Option<(String, String)>;

pub async fn run(
    ctx: GitContext,
    pr: PrRef,
    base_ref: String,
    range: CommitRange,
    tx: mpsc::UnboundedSender<Msg>,
    control: Arc<JobControl>,
) {
    if let Err(err) = run_inner(&ctx, &pr, &base_ref, range, &tx, &control).await {
        tracing::warn!(%pr, %err, "diff job failed");
        let _ = tx.send(Msg::DiffFailed(pr, err.to_string()));
    }
}

async fn run_inner(
    ctx: &GitContext,
    pr: &PrRef,
    base_ref: &str,
    range: CommitRange,
    tx: &mpsc::UnboundedSender<Msg>,
    control: &Arc<JobControl>,
) -> Result<(), GitError> {
    let progress = {
        let tx = tx.clone();
        let pr = pr.clone();
        move |line: String| {
            let _ = tx.send(Msg::DiffProgress(pr.clone(), line));
        }
    };
    let (owner, name) = (&pr.repo.owner, &pr.repo.name);
    let repo = match Repo::find_local(&ctx.cwd, owner, name).await? {
        Some(repo) => {
            tracing::info!(path = %repo.path.display(), remote = repo.remote, "using local clone");
            repo
        }
        None => {
            Repo::open_cache(
                &ctx.cache_root,
                owner,
                name,
                &format!("https://github.com/{owner}/{name}.git"),
                ctx.credentials.clone(),
                &progress,
            )
            .await?
        }
    };
    let mut refs = repo.fetch_pr(pr.number, base_ref, &progress).await?;
    if let Some((from, to)) = range {
        refs.merge_base = from;
        refs.head = to;
    }
    if let Err(err) = repo.pin_seen(pr.number, &refs.head).await {
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
    let order: Vec<usize> = (0..files.len())
        .filter(|i| !collapsed(&files[*i]))
        .chain((0..files.len()).filter(|i| collapsed(&files[*i])))
        .collect();
    control.fill(order);
    let _ = tx.send(Msg::DiffFiles(
        pr.clone(),
        Box::new(DiffFiles {
            refs,
            files: files.clone(),
            generated,
        }),
    ));

    let files = Arc::new(files);
    let repo = Arc::new(repo);
    // The first batch covers the head of the queue: the first file, which
    // is where the diff screen opens.
    let first: HashSet<usize> = control.front().into_iter().collect();
    let (stage_tx, stage_rx) = watch::channel(0u8);
    {
        let repo = repo.clone();
        let files = files.clone();
        let first = first.clone();
        tokio::spawn(async move {
            let oids = |pick: &dyn Fn(usize) -> bool| -> Vec<String> {
                files
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| pick(*i))
                    .flat_map(|(_, f)| f.blob_oids().map(str::to_owned).collect::<Vec<_>>())
                    .collect()
            };
            for (stage, batch) in [(1u8, oids(&|i| first.contains(&i))), (2, oids(&|_| true))] {
                match repo.prefetch(&batch).await {
                    Ok(n) if n > 0 => tracing::info!(n, stage, "prefetched blobs"),
                    Ok(_) => {}
                    // Reads still work through lazy fetching, just slower.
                    Err(err) => tracing::warn!(%err, "blob prefetch failed"),
                }
                let _ = stage_tx.send(stage);
            }
        });
    }

    let reader = Arc::new(repo.blob_reader()?);
    let _ = control.git.set((repo.clone(), reader.clone()));
    let workers = std::thread::available_parallelism().map_or(2, |n| n.get().clamp(2, 4));
    let mut handles = Vec::new();
    for _ in 0..workers {
        let control = control.clone();
        let files = files.clone();
        let reader = reader.clone();
        let tx = tx.clone();
        let pr = pr.clone();
        let first = first.clone();
        let mut stage = stage_rx.clone();
        handles.push(tokio::spawn(async move {
            while let Some(index) = control.pop() {
                let needed = if first.contains(&index) { 1 } else { 2 };
                if stage.wait_for(|s| *s >= needed).await.is_err() {
                    return;
                }
                let diff = diff_file(&reader, &files[index]).await;
                if tx
                    .send(Msg::FileDiff(pr.clone(), index, Arc::new(diff)))
                    .is_err()
                {
                    return;
                }
            }
        }));
    }
    for handle in handles {
        let _ = handle.await;
    }
    Ok(())
}

async fn diff_file(reader: &BlobReader, file: &ChangedFile) -> FileDiff {
    if file.is_submodule() {
        let side = |oid: &str| (oid != ZERO_OID).then(|| oid.to_owned());
        return FileDiff::submodule(side(&file.old_oid), side(&file.new_oid));
    }
    let read = |oid: String| async move {
        if oid == ZERO_OID {
            return Ok(None);
        }
        match reader.read(&oid).await {
            Ok(Some(bytes)) => Ok(Some(bytes)),
            Ok(None) => Err(format!(
                "object {} is missing",
                ghtui_ui::text::short_sha(&oid)
            )),
            Err(err) => Err(err.to_string()),
        }
    };
    let (old, new) = match (
        read(file.old_oid.clone()).await,
        read(file.new_oid.clone()).await,
    ) {
        (Ok(old), Ok(new)) => (old, new),
        (Err(err), _) | (_, Err(err)) => return FileDiff::error(err),
    };
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
    if !repo.has(old_head).await {
        repo.fetch_commit(old_head).await?;
    }
    let base = repo
        .rev_parse(&format!("refs/ghtui/pr/{number}/base"))
        .await?;
    let merge_base = repo.merge_base(&base, old_head).await?;
    let files = repo.changed_files(&merge_base, old_head).await?;
    let oids: Vec<String> = files
        .iter()
        .flat_map(|f| f.blob_oids().map(str::to_owned).collect::<Vec<_>>())
        .collect();
    if let Err(err) = repo.prefetch(&oids).await {
        tracing::warn!(%err, "prefetch for the earlier diff failed");
    }
    let mut hashes = HashSet::new();
    for file in files.iter().filter(|f| !f.is_submodule()) {
        let read = |oid: &str| {
            let oid = oid.to_owned();
            async move {
                if oid == ZERO_OID {
                    Ok(None)
                } else {
                    reader.read(&oid).await
                }
            }
        };
        let (old, new) = (read(&file.old_oid).await?, read(&file.new_oid).await?);
        let path = file.path().to_owned();
        let diff = tokio::task::spawn_blocking(move || {
            let diff = FileDiff::compute_plain(&path, old.as_deref(), new.as_deref());
            match &diff.content {
                ghtui_diff::Content::Text(text) => ghtui_diff::blocks::change_blocks(
                    &path,
                    text,
                    text.lines(ghtui_diff::Whitespace::Exact),
                )
                .into_iter()
                .map(|b| b.hash)
                .collect(),
                _ => Vec::new(),
            }
        })
        .await
        .unwrap_or_default();
        hashes.extend(diff);
    }
    Ok(hashes)
}

/// Where an outdated thread's line is now: reads the file at the thread's
/// original commit and at `head`, and maps the line if it survived
/// unchanged. Fetches the original commit by SHA if it isn't local.
pub async fn map_outdated(
    repo: &Repo,
    reader: &BlobReader,
    head: &str,
    path: &str,
    original_commit: &str,
    original_line: u32,
) -> Option<u32> {
    let original = format!("{original_commit}:{path}");
    if !repo.has(original_commit).await
        && let Err(err) = repo.fetch_commit(original_commit).await
    {
        tracing::info!(%err, original_commit, "original commit unavailable");
        return None;
    }
    let old = reader.read(&original).await.ok().flatten()?;
    let new = reader
        .read(&format!("{head}:{path}"))
        .await
        .ok()
        .flatten()?;
    let (old, new) = (ghtui_diff::Text::new(&old), ghtui_diff::Text::new(&new));
    ghtui_diff::anchor::map_line(&old, &new, original_line)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// "Since my last review" against a real force-push and rebase: the
    /// PR is rebased onto a `main` that moved on, and gains one change.
    /// Only that change is new; the rebase and the old change aren't.
    #[tokio::test]
    async fn since_review_survives_force_push_and_rebase() {
        use std::path::Path;
        use std::process::Command;

        fn git(dir: &Path, args: &[&str]) -> String {
            let out = Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_AUTHOR_NAME", "T")
                .env("GIT_AUTHOR_EMAIL", "t@example.invalid")
                .env("GIT_COMMITTER_NAME", "T")
                .env("GIT_COMMITTER_EMAIL", "t@example.invalid")
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).trim().to_owned()
        }
        let numbered = |n: u32| (1..=n).map(|i| format!("line {i}\n")).collect::<String>();

        let tmp = tempfile::tempdir().unwrap();
        let origin = tmp.path().join("origin");
        std::fs::create_dir_all(&origin).unwrap();
        git(&origin, &["init", "-q", "-b", "main"]);
        git(&origin, &["config", "uploadpack.allowFilter", "true"]);
        git(
            &origin,
            &["config", "uploadpack.allowAnySHA1InWant", "true"],
        );
        std::fs::write(origin.join("a.txt"), numbered(60)).unwrap();
        git(&origin, &["add", "."]);
        git(&origin, &["commit", "-q", "-m", "base"]);

        // v1 of the PR: one change near the top.
        git(&origin, &["checkout", "-q", "-b", "feature"]);
        std::fs::write(
            origin.join("a.txt"),
            numbered(60).replace("line 5\n", "five\n"),
        )
        .unwrap();
        git(&origin, &["commit", "-qam", "v1"]);
        git(&origin, &["update-ref", "refs/pull/1/head", "feature"]);

        let repo = Repo::open_cache(
            &tmp.path().join("cache"),
            "o",
            "r",
            &format!("file://{}", origin.display()),
            Credentials::Ambient,
            &|_| {},
        )
        .await
        .unwrap();
        let reviewed = repo.fetch_pr(1, "main", &|_| {}).await.unwrap().head;

        // main moves on (unrelated change), and the PR is rebased onto it,
        // keeps its first change and adds a second: force-pushed.
        git(&origin, &["checkout", "-q", "main"]);
        std::fs::write(
            origin.join("a.txt"),
            numbered(60).replace("line 30\n", "thirty\n"),
        )
        .unwrap();
        git(&origin, &["commit", "-qam", "upstream"]);
        git(&origin, &["checkout", "-q", "-B", "feature", "main"]);
        let rebased = numbered(60)
            .replace("line 30\n", "thirty\n")
            .replace("line 5\n", "five\n")
            .replace("line 55\n", "fifty-five\n");
        std::fs::write(origin.join("a.txt"), rebased).unwrap();
        git(&origin, &["commit", "-qam", "v2"]);
        git(&origin, &["update-ref", "refs/pull/1/head", "feature"]);
        let now = repo.fetch_pr(1, "main", &|_| {}).await.unwrap();
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
        let ghtui_diff::Content::Text(text) = &diff.content else {
            panic!()
        };
        let blocks = ghtui_diff::blocks::change_blocks(
            "a.txt",
            text,
            text.lines(ghtui_diff::Whitespace::Exact),
        );
        let new_blocks: Vec<String> = blocks
            .iter()
            .filter(|b| !seen.contains(&b.hash))
            .map(|b| {
                text.text(&text.lines[b.entries.end as usize - 1])
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
