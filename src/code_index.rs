//! An index of a repository's definitions at a commit, read from a clone
//! (yours, or the cache's partial clone) with the grammars ghtui
//! highlights with: the functions, types, traits, classes and modules
//! [`scope::scopes`] names, file by file.
//!
//! It's built in the background and kept per commit in the cache
//! (`code-index/<owner>/<name>/<commit>`). Definitions are kept by blob,
//! so a commit that moved on reads only the files that changed. Limits
//! keep a very large repository cheap: files, bytes and time; a limit
//! hit is said, never hidden.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync::{OnceCell, Semaphore, watch};

use crate::api::model::RepoId;
use crate::diff::Language;
use crate::diff::scope;
use crate::diff::text::Text;
use crate::diff_job::GitContext;
use crate::git::blobs::BlobReader;
use crate::git::index::{TreeFile, resolve, tree_files};
use crate::git::repo::Repo;
use crate::git::{GitError, Oid};
use crate::runtime::blocking;

/// The limits of one build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Files indexed.
    pub files: usize,
    /// Bytes of source read (and, in a partial clone, fetched).
    pub bytes: u64,
    /// A file bigger than this is left out (it isn't highlighted either).
    pub file_bytes: u64,
    pub time: Duration,
}

pub const LIMITS: Limits = Limits {
    files: 20_000,
    bytes: 64 << 20,
    file_bytes: crate::diff::highlight::MAX_HIGHLIGHT_BYTES as u64,
    time: Duration::from_secs(120),
};

/// Blobs fetched in one request, and files read between progress reports.
const BATCH: usize = 1000;
/// Files' definitions kept in memory, per repository, before they're
/// let go (they're on disk).
const KEPT_BLOBS: usize = 200_000;
/// Commits whose index is kept on disk, per repository.
const KEEP: usize = 6;
/// How long a branch's commit is taken as GitHub's before it's fetched
/// again.
const FRESH: Duration = Duration::from_secs(120);

/// A definition in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Def {
    pub line: u32,
    /// As the symbols list it: `fn offset`.
    pub name: String,
    /// The name it defines (`offset`); an impl defines none.
    pub defines: Option<String>,
}

/// A file's definitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDefs {
    pub path: String,
    pub blob: String,
    pub defs: Arc<[Def]>,
}

/// The limit a build stopped at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cut {
    Files,
    Bytes,
    Time,
}

/// A commit's definitions, nearest the file it was asked from first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Index {
    pub files: Vec<FileDefs>,
    /// Files of a language ghtui reads, small enough to read.
    pub eligible: usize,
    pub cut: Option<Cut>,
    /// Files this build read; the rest were read for another commit.
    pub read: usize,
}

impl Index {
    /// Every definition of `word`, in files `keep` accepts the path of.
    pub fn definitions<'a>(
        &'a self,
        word: &'a str,
        keep: impl Fn(&str) -> bool + 'a,
    ) -> impl Iterator<Item = (&'a FileDefs, &'a Def)> + 'a {
        let files = self.files.iter().filter(move |f| keep(&f.path));
        let defs = files.flat_map(|f| f.defs.iter().map(move |d| (f, d)));
        defs.filter(move |(_, d)| d.defines.as_deref() == Some(word))
    }

    pub fn symbols(&self) -> impl Iterator<Item = (&FileDefs, &Def)> {
        (self.files.iter()).flat_map(|f| f.defs.iter().map(move |d| (f, d)))
    }
}

/// How a build is going.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    /// Opening (on first use, cloning) the repository, or finding the
    /// commit; how it's going.
    Opening(String),
    /// Fetching the files to read, in a partial clone.
    Fetching {
        done: usize,
        total: usize,
    },
    Reading {
        done: usize,
        total: usize,
    },
    Ready,
    Failed(String),
}

/// What's known of a revision's index so far.
#[derive(Clone)]
pub struct Snapshot {
    pub stage: Stage,
    pub repo: Option<Arc<Repo>>,
    pub commit: Option<Oid>,
    /// Why the revision may be behind GitHub's (it couldn't be fetched).
    pub stale: Option<String>,
    pub index: Arc<Index>,
    /// The limits it was built within.
    pub limits: Limits,
}

impl Snapshot {
    fn new(stage: Stage, limits: Limits) -> Snapshot {
        Snapshot {
            stage,
            limits,
            repo: None,
            commit: None,
            stale: None,
            index: Arc::default(),
        }
    }

    pub fn done(&self) -> bool {
        matches!(self.stage, Stage::Ready | Stage::Failed(_))
    }

    /// The commit, short, for saying where it's read.
    pub fn at(&self) -> String {
        (self.commit.as_ref()).map_or_else(String::new, |c| c.get(..7).unwrap_or(c).to_owned())
    }

    /// What isn't in the index, in a sentence, if anything: it's still
    /// being built, or it stopped at a limit.
    pub fn short_of(&self, repo: &RepoId) -> Option<String> {
        let index = &self.index;
        let at = self.at();
        let still = |what: &str, done: usize, total: usize| {
            Some(format!(
                "Still indexing {repo} at {at}: {what} {done} of {total} files"
            ))
        };
        match (&self.stage, index.cut) {
            (Stage::Opening(how), _) => Some(format!("Still indexing {repo}: {how}")),
            (Stage::Fetching { done, total }, _) => still("fetched", *done, *total),
            (Stage::Reading { done, total }, _) => still("read", *done, *total),
            (Stage::Failed(why), _) => Some(format!("Couldn't index {repo}: {why}")),
            (Stage::Ready, None) => None,
            (Stage::Ready, Some(cut)) => {
                let limit = match cut {
                    Cut::Files => format!("{} files", self.limits.files),
                    Cut::Bytes => format!("{} MiB", self.limits.bytes >> 20),
                    Cut::Time => format!("{} seconds", self.limits.time.as_secs()),
                };
                Some(format!(
                    "The index of {repo} at {at} stopped at its limit of {limit}: it holds {} of {} files, nearest this one first",
                    index.files.len(),
                    index.eligible
                ))
            }
        }
    }
}

/// One repository's builds, and the definitions read so far, by blob.
#[derive(Default)]
struct RepoIndexes {
    repo: OnceCell<Arc<Repo>>,
    blobs: Mutex<HashMap<String, Arc<[Def]>>>,
    /// The latest build of each revision, and when it started.
    builds: Mutex<HashMap<String, (Instant, watch::Receiver<Snapshot>)>>,
}

impl RepoIndexes {
    fn blobs(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<[Def]>>> {
        self.blobs.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Every repository's index: builds run once, in the background, and are
/// shared by whoever asks.
pub struct Indexer {
    git: GitContext,
    limits: Limits,
    repos: Mutex<HashMap<RepoId, Arc<RepoIndexes>>>,
    /// One build reads and fetches at a time; the others wait.
    one: Arc<Semaphore>,
}

impl Indexer {
    pub fn new(git: GitContext) -> Indexer {
        Indexer::with_limits(git, LIMITS)
    }

    pub fn with_limits(git: GitContext, limits: Limits) -> Indexer {
        Indexer {
            git,
            limits,
            repos: Mutex::default(),
            one: Arc::new(Semaphore::new(1)),
        }
    }

    /// A file of its own for git's scratch work (a temporary index).
    pub fn scratch(&self) -> PathBuf {
        let dir = self.git.cache_root.join("code-index");
        let _ = std::fs::create_dir_all(&dir);
        dir.join(format!(".scratch-{}-{}", std::process::id(), unique()))
    }

    /// The index of `repo` at `rev`, starting (or refreshing) its build;
    /// `near` is the path asked from, read first.
    pub fn get(&self, repo: &RepoId, rev: &str, near: &str) -> watch::Receiver<Snapshot> {
        let indexes = {
            let mut repos = self.repos.lock().unwrap_or_else(PoisonError::into_inner);
            repos.entry(repo.clone()).or_default().clone()
        };
        let mut builds = (indexes.builds.lock()).unwrap_or_else(PoisonError::into_inner);
        if let Some((started, rx)) = builds.get(rev) {
            let fixed = Oid::parse(rev).is_ok();
            let failed = matches!(rx.borrow().stage, Stage::Failed(_));
            if !failed && (fixed || started.elapsed() < FRESH) {
                return rx.clone();
            }
        }
        let (tx, rx) = watch::channel(Snapshot::new(
            Stage::Opening("opening its clone".to_owned()),
            self.limits,
        ));
        builds.insert(rev.to_owned(), (Instant::now(), rx.clone()));
        let build = Build {
            git: self.git.clone(),
            limits: self.limits,
            id: repo.clone(),
            indexes: indexes.clone(),
            one: self.one.clone(),
            tx,
        };
        let (rev, near) = (rev.to_owned(), near.to_owned());
        tokio::spawn(async move { build.run(&rev, &near).await });
        rx
    }
}

/// Waits for `rx` to finish, or for `within`, whichever comes first, and
/// says how far it got.
pub async fn settle(
    rx: &mut watch::Receiver<Snapshot>,
    within: Duration,
    enough: impl Fn(&Snapshot) -> bool,
) -> Snapshot {
    let wait = rx.wait_for(|s| s.done() || enough(s));
    #[expect(
        clippy::disallowed_methods,
        reason = "how long an answer may wait on the index; the build itself goes on"
    )]
    let _ = tokio::time::timeout(within, wait).await;
    rx.borrow().clone()
}

/// One build of one revision's index.
struct Build {
    git: GitContext,
    limits: Limits,
    id: RepoId,
    indexes: Arc<RepoIndexes>,
    one: Arc<Semaphore>,
    tx: watch::Sender<Snapshot>,
}

impl Build {
    async fn run(&self, rev: &str, near: &str) {
        if let Err(why) = self.build(rev, near).await {
            tracing::warn!(repo = %self.id, rev, why, "code index failed");
            self.tx.send_modify(|s| s.stage = Stage::Failed(why));
        }
    }

    fn dir(&self) -> PathBuf {
        let (owner, name) = (&self.id.owner, &self.id.name);
        (self.git.cache_root.join("code-index"))
            .join(owner.to_ascii_lowercase())
            .join(name.to_ascii_lowercase())
    }

    async fn open(&self) -> Result<Arc<Repo>, String> {
        let git = &self.git;
        let (owner, name) = (&self.id.owner, &self.id.name);
        let open = async {
            if let Some(repo) = Repo::find_local(&git.cwd, owner, name).await? {
                return Ok(Arc::new(repo));
            }
            let url = crate::host::current().git_url(owner, name);
            let progress = |line: String| self.tx.send_modify(|s| s.stage = Stage::Opening(line));
            let credentials = git.credentials.clone();
            let repo = Repo::open_cache(&git.cache_root, owner, name, &url, credentials, &progress);
            repo.await.map(Arc::new)
        };
        // A failure is tried again next time.
        let opened = self.indexes.repo.get_or_try_init(|| open);
        opened.await.cloned().map_err(|e: GitError| e.to_string())
    }

    async fn build(&self, rev: &str, near: &str) -> Result<(), String> {
        let repo = self.open().await?;
        let (commit, stale) = resolve(&repo, rev).await.map_err(|e| e.to_string())?;
        self.tx.send_modify(|s| {
            s.repo = Some(repo.clone());
            s.commit = Some(commit.clone());
            s.stale = stale.map(|e| e.to_string());
        });
        let dir = self.dir();
        let file = dir.join(&*commit);
        if let Some(index) = blocking(move || load(&file)).await {
            self.remember(&index);
            return self.ready(index);
        }
        let _turn = self.one.acquire().await.map_err(|e| e.to_string())?;
        {
            let mut blobs = self.indexes.blobs();
            if blobs.len() > KEPT_BLOBS {
                blobs.clear();
            }
        }
        if self.indexes.blobs().is_empty() {
            let dir = dir.clone();
            if let Some(index) = blocking(move || newest(&dir).and_then(|f| load(&f))).await {
                self.remember(&index);
            }
        }
        let files = tree_files(&repo, &commit)
            .await
            .map_err(|e| e.to_string())?;
        let (chosen, eligible, mut cut) = choose(files, near, &self.limits);
        // The clock is the index's: opening (cloning) and fetching the
        // revision went before.
        let started = Instant::now();
        let todo: Vec<&TreeFile> = {
            let blobs = self.indexes.blobs();
            (chosen.iter())
                .filter(|f| !blobs.contains_key(&f.blob))
                .collect()
        };
        let late = |cut: &mut Option<Cut>| {
            let late = started.elapsed() > self.limits.time;
            if late {
                *cut = Some(Cut::Time);
            }
            late
        };
        let reader = {
            let repo = repo.clone();
            // Only what's fetched a batch at a time below is read: nothing
            // one by one.
            BlobReader::spawn(move || {
                let mut cmd = repo.cmd();
                cmd.env("GIT_NO_LAZY_FETCH", "1");
                cmd
            })
            .map_err(|e| e.to_string())?
        };
        // A partial clone doesn't know the sizes of files it hasn't
        // fetched: what's read is counted as it comes.
        let (mut read, mut bytes) = (0usize, 0u64);
        let total = todo.len();
        for (i, batch) in todo.chunks(BATCH).enumerate() {
            let done = i.saturating_mul(BATCH);
            if late(&mut cut) {
                break;
            }
            if bytes > self.limits.bytes {
                cut = Some(Cut::Bytes);
                break;
            }
            if repo.partial {
                self.tx
                    .send_modify(|s| s.stage = Stage::Fetching { done, total });
                let oids: Vec<String> = batch.iter().map(|f| f.blob.clone()).collect();
                repo.prefetch(&oids).await.map_err(|e| e.to_string())?;
            }
            self.tx
                .send_modify(|s| s.stage = Stage::Reading { done, total });
            let mut sources = Vec::new();
            for f in batch {
                let source = match reader.read(&f.blob).await {
                    Ok(Some(source)) => source,
                    // Unreadable: indexed as defining nothing.
                    _ => Vec::new(),
                };
                bytes = bytes.saturating_add(source.len() as u64);
                sources.push((f.path.clone(), f.blob.clone(), source));
            }
            let file_bytes = self.limits.file_bytes;
            let parsed = blocking(move || {
                let parse = |(path, blob, bytes): (String, String, Vec<u8>)| {
                    if bytes.len() as u64 > file_bytes {
                        return (blob, Arc::from([]));
                    }
                    let text = Text::new(&bytes);
                    let scopes = scope::scopes(Language::from_path(&path), &text);
                    let defs = scopes.into_iter().map(|s| Def {
                        line: *s.lines.start(),
                        name: s.name,
                        defines: s.defines,
                    });
                    (blob, defs.collect::<Arc<[Def]>>())
                };
                sources.into_iter().map(parse).collect::<Vec<_>>()
            })
            .await;
            read = read.saturating_add(parsed.len());
            self.indexes.blobs().extend(parsed);
            let index = self.assemble(&chosen, eligible, None);
            self.tx.send_modify(|s| s.index = Arc::new(index));
        }
        let mut index = self.assemble(&chosen, eligible, cut);
        index.read = read;
        // An index cut short by time isn't kept: the next build goes on
        // from what it read.
        let (file, saved) = (dir.join(&*commit), index.clone());
        if index.cut != Some(Cut::Time)
            && let Err(err) = blocking(move || save(&file, &saved)).await
        {
            tracing::warn!(%err, "couldn't save the code index");
        }
        self.ready(index)
    }

    /// The index of the files of `chosen` read so far.
    fn assemble(&self, chosen: &[TreeFile], eligible: usize, cut: Option<Cut>) -> Index {
        let blobs = self.indexes.blobs();
        let files = chosen.iter().filter_map(|f| {
            Some(FileDefs {
                path: f.path.clone(),
                blob: f.blob.clone(),
                defs: blobs.get(&f.blob)?.clone(),
            })
        });
        let files: Vec<FileDefs> = files.collect();
        // A file missing from what was chosen is one a limit left out.
        let cut = cut.or((files.len() < chosen.len()).then_some(Cut::Time));
        Index {
            files,
            eligible,
            cut,
            read: 0,
        }
    }

    fn remember(&self, index: &Index) {
        let mut blobs = self.indexes.blobs();
        for f in &index.files {
            blobs.insert(f.blob.clone(), f.defs.clone());
        }
    }

    fn ready(&self, index: Index) -> Result<(), String> {
        self.tx.send_modify(|s| {
            s.index = Arc::new(index);
            s.stage = Stage::Ready;
        });
        Ok(())
    }
}

/// The files to index: of a language ghtui reads, within the limits,
/// nearest `near` first. Also how many there were, and the limit that
/// left some out.
fn choose(
    files: Vec<TreeFile>,
    near: &str,
    limits: &Limits,
) -> (Vec<TreeFile>, usize, Option<Cut>) {
    let readable = |f: &TreeFile| {
        let named = Language::from_path(&f.path).is_some_and(has_scopes);
        let size = f.size.unwrap_or(1);
        named && size > 0 && size <= limits.file_bytes && !f.path.contains(['\t', '\n'])
    };
    let mut files: Vec<TreeFile> = files.into_iter().filter(readable).collect();
    let eligible = files.len();
    // Nearest: the most leading folders in common with `near`.
    let folders = |p: &str| p.rsplit_once('/').map_or("", |(dir, _)| dir).to_owned();
    let here = folders(near);
    let shared = |p: &str| {
        let there = folders(p);
        let pairs = here.split('/').zip(there.split('/'));
        pairs.take_while(|(a, b)| a == b && !a.is_empty()).count()
    };
    files.sort_by_cached_key(|f| std::cmp::Reverse(shared(&f.path)));
    let (mut bytes, mut cut) = (0u64, None);
    let mut chosen = Vec::new();
    for f in files {
        if chosen.len() >= limits.files {
            cut = Some(Cut::Files);
            break;
        }
        // Unknown (not fetched yet): counted as it's read.
        bytes = bytes.saturating_add(f.size.unwrap_or(0));
        if bytes > limits.bytes {
            cut = Some(Cut::Bytes);
            break;
        }
        chosen.push(f);
    }
    (chosen, eligible, cut)
}

/// Languages whose scopes ghtui names.
fn has_scopes(lang: Language) -> bool {
    use Language::{Go, JavaScript, Python, Rust, Tsx, TypeScript};
    matches!(lang, Rust | TypeScript | Tsx | JavaScript | Python | Go)
}

const MAGIC: &str = "ghtui code index 1";

/// Saves `index` as `file`, keeping only the newest [`KEEP`] beside it.
fn save(file: &Path, index: &Index) -> std::io::Result<()> {
    use std::fmt::Write;
    let Some(dir) = file.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(dir)?;
    let cut = match index.cut {
        None => "-",
        Some(Cut::Files) => "files",
        Some(Cut::Bytes) => "bytes",
        Some(Cut::Time) => "time",
    };
    let mut out = format!("{MAGIC}\n{}\t{cut}\n", index.eligible);
    for f in &index.files {
        let _ = writeln!(out, "{}\t{}", f.blob, f.path);
        for d in f.defs.iter() {
            let defines = d.defines.as_deref().unwrap_or("");
            let _ = writeln!(out, "\t{}\t{defines}\t{}", d.line, d.name);
        }
    }
    let tmp = dir.join(format!(".tmp-{}-{}", std::process::id(), unique()));
    std::fs::write(&tmp, out)?;
    std::fs::rename(&tmp, file)?;
    let mut kept: Vec<(std::time::SystemTime, PathBuf)> = (std::fs::read_dir(dir)?)
        .filter_map(Result::ok)
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    kept.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (_, old) in kept.iter().skip(KEEP) {
        let _ = std::fs::remove_file(old);
    }
    Ok(())
}

fn unique() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The most recently saved index in `dir`.
fn newest(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?.filter_map(Result::ok);
    let saved = entries.filter(|e| !e.file_name().to_string_lossy().starts_with('.'));
    let dated = saved.filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())));
    dated.max_by_key(|(at, _)| *at).map(|(_, path)| path)
}

/// An index saved by [`save`]; `None` if there's none or it doesn't read.
fn load(file: &Path) -> Option<Index> {
    let text = std::fs::read_to_string(file).ok()?;
    let mut lines = text.lines();
    if lines.next()? != MAGIC {
        return None;
    }
    let (eligible, cut) = lines.next()?.split_once('\t')?;
    let cut = match cut {
        "files" => Some(Cut::Files),
        "bytes" => Some(Cut::Bytes),
        "time" => Some(Cut::Time),
        _ => None,
    };
    let mut files: Vec<(String, String, Vec<Def>)> = Vec::new();
    for line in lines {
        if let Some(def) = line.strip_prefix('\t') {
            let mut parts = def.splitn(3, '\t');
            let (at, defines, name) = (parts.next()?, parts.next()?, parts.next()?);
            files.last_mut()?.2.push(Def {
                line: at.parse().ok()?,
                name: name.to_owned(),
                defines: (!defines.is_empty()).then(|| defines.to_owned()),
            });
        } else {
            let (blob, path) = line.split_once('\t')?;
            files.push((path.to_owned(), blob.to_owned(), Vec::new()));
        }
    }
    let files = files.into_iter().map(|(path, blob, defs)| FileDefs {
        path,
        blob,
        defs: defs.into(),
    });
    Some(Index {
        files: files.collect(),
        eligible: eligible.parse().ok()?,
        cut,
        read: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::credentials::Credentials;

    use crate::git::set_up as git;

    async fn commit(dir: &Path, files: &[(&str, &str)]) -> String {
        for (path, text) in files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        git(dir, &["add", "."]).await;
        let who = ["-c", "user.name=a", "-c", "user.email=a@b"];
        git(dir, &[&who[..], &["commit", "-qm", "x"]].concat()).await;
        git(dir, &["rev-parse", "HEAD"]).await.trim().to_owned()
    }

    /// Indexes your clone at `clone`, kept in `cache`.
    fn indexer(cache: &Path, clone: &Path, limits: Limits) -> Indexer {
        let git = GitContext {
            cache_root: cache.to_owned(),
            credentials: Credentials::Ambient,
            cwd: clone.to_owned(),
        };
        Indexer::with_limits(git, limits)
    }

    async fn built(indexer: &Indexer, rev: &str, near: &str) -> Snapshot {
        let mut rx = indexer.get(&RepoId::new("o", "r"), rev, near);
        settle(&mut rx, Duration::from_secs(30), |_| false).await
    }

    fn found(snap: &Snapshot, word: &str) -> Vec<(String, u32, String)> {
        let defs = snap.index.definitions(word, |_| true);
        defs.map(|(f, d)| (f.path.clone(), d.line, d.name.clone()))
            .collect()
    }

    fn git_partial(snap: &Snapshot) -> bool {
        snap.repo.as_ref().is_some_and(|r| r.partial)
    }

    /// Indexes a real repository through a partial clone in a cache of
    /// its own, within the limits, and says how it went (by hand):
    /// `GHTUI_INDEX_REPO=rust-lang/rust GHTUI_INDEX_REV=main
    /// GHTUI_INDEX_NEAR=compiler/rustc_middle/src/ty/mod.rs
    /// GHTUI_INDEX_CACHE=dir cargo test --lib -- --ignored
    /// index_a_real_repository --nocapture`
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "clones from GitHub"]
    #[expect(clippy::print_stdout, reason = "run by hand, to read")]
    async fn index_a_real_repository() {
        let var = |k: &str| std::env::var(k).unwrap();
        let (owner, name) = var("GHTUI_INDEX_REPO")
            .split_once('/')
            .map(|(o, n)| (o.to_owned(), n.to_owned()))
            .unwrap();
        let cache = PathBuf::from(var("GHTUI_INDEX_CACHE"));
        let indexer = Indexer::new(GitContext {
            cache_root: cache.clone(),
            credentials: Credentials::Ambient,
            // Your clone, if `GHTUI_INDEX_CLONE` names one.
            cwd: std::env::var_os("GHTUI_INDEX_CLONE").map_or(cache, PathBuf::from),
        });
        let repo = RepoId::new(owner, name);
        let start = Instant::now();
        let mut rx = indexer.get(&repo, &var("GHTUI_INDEX_REV"), &var("GHTUI_INDEX_NEAR"));
        loop {
            let snap = settle(&mut rx, Duration::from_secs(10), |_| false).await;
            let stage = format!("{:?}", snap.stage);
            println!(
                "{:>4}s {stage} {:?}",
                start.elapsed().as_secs(),
                snap.short_of(&repo)
            );
            if snap.done() {
                let defs: usize = snap.index.files.iter().map(|f| f.defs.len()).sum();
                let (files, read) = (snap.index.files.len(), snap.index.read);
                println!(
                    "{files} of {} files ({read} read), {defs} definitions",
                    snap.index.eligible
                );
                let word = var("GHTUI_INDEX_WORD");
                let found = snap.index.definitions(&word, |_| true).count();
                println!("{found} definitions of {word}");
                let (Some(git), true) = (&snap.repo, git_partial(&snap)) else {
                    break;
                };
                let at = Instant::now();
                let files: Vec<(&str, &str)> = (snap.index.files.iter())
                    .map(|f| (f.path.as_str(), f.blob.as_str()))
                    .collect();
                let tree = crate::git::index::tree_of(git, &files, &indexer.scratch())
                    .await
                    .unwrap();
                let hits =
                    crate::git::index::grep_word(git, &tree, &word, 1000, Duration::from_secs(10))
                        .await
                        .unwrap();
                println!("{} uses of {word} ({:?})", hits.hits.len(), at.elapsed());
                let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
                println!(
                    "{}",
                    status
                        .lines()
                        .filter(|l| l.starts_with("VmHWM"))
                        .collect::<String>()
                );
                break;
            }
        }
    }

    /// Your clone's definitions are indexed at a commit and kept in the
    /// cache; a commit that moved on reads only the files that changed;
    /// a limit is said, keeping the files nearest the one asked from.
    #[tokio::test]
    async fn indexes_a_clone_at_a_commit_and_moves_on_incrementally() {
        let tmp = tempfile::tempdir().unwrap();
        let (clone, cache) = (tmp.path().join("clone"), tmp.path().join("cache"));
        std::fs::create_dir(&clone).unwrap();
        git(&clone, &["init", "-q", "-b", "main"]).await;
        git(
            &clone,
            &["remote", "add", "origin", "git@github.com:o/r.git"],
        )
        .await;
        let first = commit(
            &clone,
            &[
                ("src/a.rs", "struct Beta;\n\nfn alpha() {}\n"),
                ("tools/b.py", "def gamma():\n    pass\n"),
                ("README.md", "# alpha\n"),
            ],
        )
        .await;

        let snap = built(&indexer(&cache, &clone, LIMITS), &first, "src/a.rs").await;
        assert_eq!(snap.stage, Stage::Ready);
        assert_eq!(snap.short_of(&RepoId::new("o", "r")), None);
        let alpha = found(&snap, "alpha");
        assert_eq!(alpha, [("src/a.rs".into(), 3, "fn alpha".into())]);
        assert_eq!(found(&snap, "gamma").len(), 1);
        assert_eq!(
            (snap.index.eligible, snap.index.read),
            (2, 2),
            "not the README"
        );
        assert!(cache.join("code-index/o/r").join(&first).exists());

        // Another run of ghtui, a commit later: one file read.
        let second = commit(&clone, &[("tools/b.py", "def delta():\n    pass\n")]).await;
        let snap = built(&indexer(&cache, &clone, LIMITS), &second, "src/a.rs").await;
        assert_eq!(snap.index.read, 1);
        assert_eq!(found(&snap, "gamma"), []);
        assert_eq!(found(&snap, "delta").len(), 1);
        assert_eq!(found(&snap, "Beta").len(), 1, "kept from the first");

        // One file allowed: the nearest.
        let one = Limits { files: 1, ..LIMITS };
        let other = tmp.path().join("other");
        let snap = built(&indexer(&other, &clone, one), &second, "tools/x.py").await;
        let paths: Vec<&str> = snap.index.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["tools/b.py"]);
        assert_eq!(snap.index.cut, Some(Cut::Files));
        let said = snap.short_of(&RepoId::new("o", "r")).unwrap();
        assert!(said.contains("limit of 1 files: it holds 1 of 2"), "{said}");
    }
}
