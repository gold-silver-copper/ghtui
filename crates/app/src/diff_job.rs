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

pub async fn run(
    ctx: GitContext,
    pr: PrRef,
    base_ref: String,
    tx: mpsc::UnboundedSender<Msg>,
    control: Arc<JobControl>,
) {
    if let Err(err) = run_inner(&ctx, &pr, &base_ref, &tx, &control).await {
        tracing::warn!(%pr, %err, "diff job failed");
        let _ = tx.send(Msg::DiffFailed(pr, err.to_string()));
    }
}

async fn run_inner(
    ctx: &GitContext,
    pr: &PrRef,
    base_ref: &str,
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
    let refs = repo.fetch_pr(pr.number, base_ref, &progress).await?;
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
            Ok(None) => Err(format!("object {} is missing", &oid[..7.min(oid.len())])),
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
}
