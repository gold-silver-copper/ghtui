//! Where each fact a page shows comes from: git or GitHub's API. The one
//! place that decides it ([`by_git`]).
//!
//! Whatever git can answer is read from a clone: yours, when ghtui starts
//! inside one of the repository, else a partial (blobless) clone in the
//! cache, made in the background the first time it's wanted. A name
//! (`main`, `v1.0`) is always asked of GitHub (`git ls-remote`, kept for
//! [`REFS_FRESH`]), never taken from a local branch, and the commit it
//! names is fetched when the clone doesn't have it. The API answers
//! instead, and the page says why, while the clone is being made, when
//! git fails, when a limit ([`crate::git::read`]'s, [`CLONE_MAX`]) is hit,
//! and when git takes longer than the API would.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::api::browse::{Refs, Results, SearchKind, SearchResults};
use crate::api::model::{Capped, RepoId};
use crate::api::{ApiError, GitHub};
use crate::data::{Data, DataKey};
use crate::diff_job::GitContext;
use crate::git::read::{self, Grep, ReadError, RemoteRefs};
use crate::git::repo::Repo;
use crate::git::{GitError, Oid};
use crate::lock;

/// How long what GitHub says its branches and tags are is taken as true.
const REFS_FRESH: Duration = Duration::from_secs(30);
/// The biggest clone the cache makes on its own; a bigger repository is
/// read through the API.
pub const CLONE_MAX: u64 = 512 * 1024 * 1024;
/// How long a clone found too big is left alone.
const TOO_BIG_FOR: Duration = Duration::from_secs(7 * 24 * 3600);
/// How long keeping a repository offline waits for its clone.
const KEEP_CLONE_WAIT: Duration = Duration::from_secs(600);
/// How many files a kept repository's clone fetches at once.
const KEEP_BATCH: usize = 1000;
/// How long the history walk for a directory's latest commits goes on;
/// GitHub is asked for the entries it didn't reach.
const WALK: Duration = Duration::from_secs(1);

/// Whose clone a page was read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clone {
    /// Yours, where ghtui started.
    Yours(PathBuf),
    /// ghtui's own, in its cache.
    Cache,
}

/// Where a page's data came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum From {
    /// A clone, at a commit (none for what isn't at one: branches, tags).
    Git {
        clone: Clone,
        at: Option<Oid>,
        /// GitHub couldn't be reached: its branches and tags are as it
        /// listed them then (Unix seconds).
        listed: Option<u64>,
    },
    /// GitHub's API, and why git didn't answer.
    Api(String),
}

impl From {
    /// One line saying where the page was read: the commit, and the clone;
    /// or that GitHub answered, and why.
    pub fn note(&self, now: u64) -> String {
        let note = self.read();
        match self {
            From::Git {
                listed: Some(at), ..
            } => format!(
                "{note} · offline: branches and tags as GitHub had them {}",
                crate::ui::time::ago(*at, now)
            ),
            _ => note,
        }
    }

    fn read(&self) -> String {
        match self {
            From::Git { clone, at, .. } => {
                let whose = match clone {
                    Clone::Yours(top) => match top.file_name() {
                        Some(name) => format!("your clone, {}", name.to_string_lossy()),
                        None => "your clone".to_owned(),
                    },
                    Clone::Cache => "ghtui's clone".to_owned(),
                };
                match at {
                    Some(at) => format!(
                        "At {} · read by git from {whose}",
                        crate::ui::text::short_sha(at)
                    ),
                    None => format!("Read by git from {whose}"),
                }
            }
            From::Api(why) => format!("From GitHub's API: {why}"),
        }
    }
}

/// Whether git answers `key` (or, with `after`, its next page). The API
/// answers what git can't know (who opened what, checks, reactions) and
/// what git couldn't read this time.
pub fn by_git(key: &DataKey, after: Option<&str>) -> bool {
    match key {
        DataKey::History(..) => after.is_none_or(|a| read::history_skip(a).is_some()),
        _ if after.is_some() => false,
        DataKey::Tree(..)
        | DataKey::Blob(..)
        | DataKey::Files(..)
        | DataKey::Commit(..)
        | DataKey::Blame(..)
        | DataKey::Compare(..)
        | DataKey::LastCommits(..)
        | DataKey::Refs(..)
        | DataKey::Tags(..)
        | DataKey::RefIn(..)
        | DataKey::Activity(_) => true,
        DataKey::Search(SearchKind::Code, query) => code_search(query).is_some(),
        _ => false,
    }
}

/// The repository a key reads.
fn repo_of(key: &DataKey) -> Option<RepoId> {
    match key {
        DataKey::Tree(repo, ..)
        | DataKey::Blob(repo, ..)
        | DataKey::Files(repo, _)
        | DataKey::Commit(repo, _)
        | DataKey::Blame(repo, ..)
        | DataKey::Compare(repo, _)
        | DataKey::LastCommits(repo, ..)
        | DataKey::History(repo, ..)
        | DataKey::Refs(repo)
        | DataKey::Tags(repo)
        | DataKey::RefIn(repo, _)
        | DataKey::Activity(repo) => Some(repo.clone()),
        DataKey::Search(SearchKind::Code, query) => code_search(query).map(|s| s.repo),
        _ => None,
    }
}

/// A code search git can run: in one repository (`repo:owner/name`), at a
/// revision (`rev:`, else the default branch), for words, in paths
/// (`path:`, `extension:`, `filename:`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeSearch {
    pub repo: RepoId,
    pub rev: Option<String>,
    pub grep: Grep,
}

/// `query` as a search git can run, if it is one: anything else GitHub's
/// code search understands (`language:`, `org:`, `OR`) is left to it.
pub fn code_search(query: &str) -> Option<CodeSearch> {
    let (mut repo, mut rev, mut grep) = (None, None, Grep::default());
    for term in terms(query) {
        let qualifier = term.split_once(':').filter(|(k, _)| !k.contains(' '));
        match qualifier {
            Some(("repo", r)) if repo.is_none() => repo = Some(RepoId::parse(r)?),
            Some(("rev", r)) if rev.is_none() && !r.is_empty() => rev = Some(r.to_owned()),
            Some(("path", p)) if !p.is_empty() => {
                grep.paths.push(p.trim_start_matches('/').to_owned());
            }
            Some(("extension", e)) if !e.is_empty() => grep.paths.push(format!("*.{e}")),
            Some(("filename", f)) if !f.is_empty() => {
                grep.paths.push(format!(":(glob)**/{f}"));
            }
            Some(_) => return None,
            None if matches!(term.as_str(), "AND" | "OR" | "NOT") => return None,
            None => grep.words.push(term),
        }
    }
    (!grep.words.is_empty()).then_some(CodeSearch {
        repo: repo?,
        rev,
        grep,
    })
}

/// A query's terms: words, and `"quoted phrases"` as one.
fn terms(query: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut rest = query.trim();
    while !rest.is_empty() {
        let quoted = rest.strip_prefix('"').and_then(|r| r.split_once('"'));
        let (term, after) = match quoted {
            Some((phrase, after)) => (phrase, after),
            None => rest.split_once(char::is_whitespace).unwrap_or((rest, "")),
        };
        if !term.is_empty() {
            terms.push(term.to_owned());
        }
        rest = after.trim_start();
    }
    terms
}

/// A repository's clone, as far as ghtui knows.
enum Slot {
    Ready(Arc<Repo>, Clone),
    /// The cache's clone is being made.
    Cloning,
    /// It can't be read with git, and why.
    Failed(String),
}

/// What git knows of each repository: its clone and GitHub's refs.
pub struct Local {
    git: GitContext,
    clones: Mutex<HashMap<String, Slot>>,
    /// GitHub's branches and tags, and its default branch, by repository,
    /// when asked; one ask at a time each.
    refs: Mutex<HashMap<String, Arc<tokio::sync::Mutex<Known>>>>,
    /// Each directory's latest commits, by commit and path: they never
    /// change, and a big repository's take seconds to find.
    memo: Mutex<HashMap<(Oid, String), LastCommits>>,
}

type LastCommits = Arc<HashMap<String, crate::api::browse::CommitInfo>>;

/// What a repository's remote said its refs were, and when.
#[derive(Default)]
struct Known {
    listed: Option<(Instant, RemoteRefs)>,
    head: Option<(Instant, Oid)>,
    /// GitHub couldn't be reached last time: when it listed what was
    /// used instead.
    old: Option<u64>,
}

/// What GitHub last listed of a repository's branches and tags, or its
/// default branch, kept in the cache to read by while it can't be
/// reached.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Kept {
    branches: Vec<(String, String)>,
    tags: Vec<(String, String)>,
}

impl Kept {
    fn of(refs: &RemoteRefs) -> Self {
        let names = |list: &[(String, Oid)]| {
            (list.iter())
                .map(|(name, oid)| (name.clone(), oid.to_string()))
                .collect()
        };
        Self {
            branches: names(&refs.branches),
            tags: names(&refs.tags),
        }
    }

    /// The default branch, as the one branch.
    fn of_head(name: &str, oid: &Oid) -> Self {
        Self {
            branches: vec![(name.to_owned(), oid.to_string())],
            tags: Vec::new(),
        }
    }

    fn refs(&self) -> RemoteRefs {
        let oids = |list: &[(String, String)]| {
            (list.iter())
                .filter_map(|(name, oid)| Some((name.clone(), Oid::parse(oid).ok()?)))
                .collect()
        };
        RemoteRefs {
            branches: oids(&self.branches),
            tags: oids(&self.tags),
        }
    }

    fn head(&self) -> Option<Oid> {
        self.refs().branches.into_iter().next().map(|(_, oid)| oid)
    }
}

/// Where what GitHub listed of `repo` (`what`: its refs or its head) is
/// kept.
fn kept_key(what: &str, repo: &RepoId) -> String {
    format!("git-{what}:{}", slug(repo))
}

/// Why GitHub wasn't asked.
fn offline(repo: &RepoId) -> ReadError {
    ReadError::Limit(format!(
        "GitHub can't be reached, and ghtui hasn't seen {repo}'s branches before"
    ))
}

/// A repository's key in the maps: names are case-insensitive.
fn slug(repo: &RepoId) -> String {
    format!("{}/{}", repo.owner, repo.name).to_ascii_lowercase()
}

/// A git read's answer, or why there's none.
enum Answer {
    Data(Data, Option<Oid>),
    /// Git, at a commit GitHub named, says it isn't there.
    Absent(String),
}

impl Local {
    pub fn new(git: GitContext) -> Arc<Local> {
        Arc::new(Local {
            git,
            clones: Mutex::default(),
            refs: Mutex::default(),
            memo: Mutex::default(),
        })
    }

    /// Where clones and git's credentials are.
    pub fn git(&self) -> &GitContext {
        &self.git
    }

    /// `key`'s data and where it came from: git when it can, else the API
    /// (see [`by_git`]). Git gets `wait`; past it, the API answers, and
    /// git goes on in the background, so the clone is ready next time.
    pub async fn fetch(
        self: &Arc<Self>,
        gh: &GitHub,
        key: &DataKey,
        after: Option<String>,
    ) -> (Result<Data, ApiError>, From) {
        let Some(repo) = repo_of(key) else {
            let result = crate::data::load::fetch(gh, &self.git, key, after, None).await;
            return (result, From::Api("git doesn't know it".into()));
        };
        let why = match self.clone_of(gh, &repo).await {
            Err(why) => why,
            Ok((clone, whose)) => {
                let (local, key2, after2) = (self.clone(), key.clone(), after.clone());
                let (gh2, repo2) = (gh.clone(), repo.clone());
                let task =
                    tokio::spawn(
                        async move { local.read(&gh2, &clone, &repo2, &key2, after2).await },
                    );
                #[expect(
                    clippy::disallowed_methods,
                    reason = "git races the API: past this, the API answers, and the read carries on"
                )]
                let done = tokio::time::timeout(wait(key), task).await;
                match done {
                    Ok(Ok(Ok(Answer::Data(data, at)))) => {
                        let listed = self.listed(&repo).await;
                        return (
                            Ok(data),
                            From::Git {
                                clone: whose,
                                at,
                                listed,
                            },
                        );
                    }
                    Ok(Ok(Ok(Answer::Absent(what)))) => {
                        let from = From::Git {
                            clone: whose,
                            at: None,
                            listed: self.listed(&repo).await,
                        };
                        return (Err(ApiError::NotFound(what)), from);
                    }
                    Ok(Ok(Err(ReadError::Limit(limit)))) => limit,
                    Ok(Ok(Err(ReadError::Git(err)))) => format!("git couldn't read it: {err}"),
                    Ok(Err(join)) if join.is_panic() => {
                        std::panic::resume_unwind(join.into_panic())
                    }
                    Ok(Err(join)) => format!("git stopped: {join}"),
                    Err(_) => format!(
                        "git took over {}s (it goes on in the background)",
                        wait(key).as_secs()
                    ),
                }
            }
        };
        tracing::info!(?key, %why, "read through the API");
        let result = crate::data::load::fetch(gh, &self.git, key, after, None).await;
        (result, From::Api(why))
    }

    /// The clone to read `repo` from: yours, or the cache's. Without
    /// either, the cache's is made in the background, and the API answers
    /// meanwhile. While GitHub can't be reached, none is made.
    async fn clone_of(
        self: &Arc<Self>,
        gh: &GitHub,
        repo: &RepoId,
    ) -> Result<(Arc<Repo>, Clone), String> {
        let slug = slug(repo);
        match lock(&self.clones).get(&slug) {
            Some(Slot::Ready(clone, whose)) => return Ok((clone.clone(), whose.clone())),
            Some(Slot::Cloning) => return Err(format!("ghtui is still cloning {repo} for git")),
            Some(Slot::Failed(why)) => return Err(why.clone()),
            None => {}
        }
        let ready = match Repo::find_local(&self.git.cwd, &repo.owner, &repo.name).await {
            Ok(Some(clone)) => Some((clone.path.clone(), clone, true)),
            _ => match Repo::open_cached(
                &self.git.cache_root,
                &repo.owner,
                &repo.name,
                self.git.credentials.clone(),
            )
            .await
            {
                Ok(Some(clone)) => Some((clone.path.clone(), clone, false)),
                Ok(None) => None,
                Err(err) => {
                    let why = format!("ghtui's clone of {repo} doesn't open: {err}");
                    lock(&self.clones).insert(slug, Slot::Failed(why.clone()));
                    return Err(why);
                }
            },
        };
        let mut clones = lock(&self.clones);
        if let Some((top, clone, yours)) = ready {
            let whose = if yours {
                Clone::Yours(top)
            } else {
                Clone::Cache
            };
            let clone = Arc::new(clone);
            clones.insert(slug, Slot::Ready(clone.clone(), whose.clone()));
            return Ok((clone, whose));
        }
        if let Some(why) = too_big(&self.git.cache_root, repo) {
            clones.insert(slug, Slot::Failed(why.clone()));
            return Err(why);
        }
        if matches!(clones.get(&slug), Some(Slot::Cloning)) {
            return Err(format!("ghtui is still cloning {repo} for git"));
        }
        // Not marked failed: it's made once GitHub is back.
        if gh.offline_since().is_some() {
            return Err(format!(
                "GitHub can't be reached, and there's no clone of {repo} to read"
            ));
        }
        clones.insert(slug.clone(), Slot::Cloning);
        drop(clones);
        let (local, repo2) = (self.clone(), repo.clone());
        tokio::spawn(async move {
            let slot = match local.make_clone(&repo2).await {
                Ok(clone) => Slot::Ready(Arc::new(clone), Clone::Cache),
                Err(why) => Slot::Failed(why),
            };
            lock(&local.clones).insert(slug, slot);
        });
        Err(format!(
            "ghtui is cloning {repo} for git, in the background"
        ))
    }

    /// Makes the cache's partial clone of `repo`, unless it grows past
    /// [`CLONE_MAX`]: then it's given up, and noted, so it isn't tried
    /// again for a week.
    async fn make_clone(&self, repo: &RepoId) -> Result<Repo, String> {
        let over = Arc::new(tokio::sync::Notify::new());
        let progress = {
            let over = over.clone();
            move |line: String| {
                if received(&line).is_some_and(|bytes| bytes > CLONE_MAX) {
                    over.notify_one();
                }
            }
        };
        let url = crate::host::current().git_url(&repo.owner, &repo.name);
        let clone = Repo::open_cache(
            &self.git.cache_root,
            &repo.owner,
            &repo.name,
            &url,
            self.git.credentials.clone(),
            &progress,
        );
        tokio::select! {
            made = clone => made.map_err(|err| format!("ghtui couldn't clone {repo}: {err}")),
            () = over.notified() => {
                let why = format!(
                    "{repo} is too big for ghtui's clone (over {})",
                    crate::ui::text::size(CLONE_MAX)
                );
                let marker = marker(&self.git.cache_root, repo);
                if let Err(err) = std::fs::write(&marker, &why) {
                    tracing::warn!(%err, "couldn't note a clone too big");
                }
                tracing::info!(%repo, "clone given up: too big");
                Err(why)
            }
        }
    }

    /// GitHub's branches and tags for `repo`, asked again once they're
    /// [`REFS_FRESH`] old.
    /// While GitHub can't be reached (or git can't reach it), they're as
    /// GitHub last listed them, kept in the cache, and that's noted.
    async fn remote_refs(
        &self,
        gh: &GitHub,
        repo: &RepoId,
        clone: &Repo,
    ) -> Result<RemoteRefs, ReadError> {
        let entry = self.refs_of(repo);
        let mut refs = entry.lock().await;
        if let Some((at, listed)) = &refs.listed
            && at.elapsed() < REFS_FRESH
        {
            return Ok(listed.clone());
        }
        let asked = match gh.offline_since() {
            None => read::remote_refs(clone).await,
            Some(_) => Err(offline(repo)),
        };
        match asked {
            Ok(listed) => {
                refs.listed = Some((Instant::now(), listed.clone()));
                refs.old = None;
                gh.remember(&kept_key("refs", repo), &Kept::of(&listed));
                Ok(listed)
            }
            Err(err) => {
                let kept = gh.cached::<Kept>(&kept_key("refs", repo)).ok_or(err)?;
                tracing::info!(%repo, "branches and tags as GitHub last listed them");
                refs.old = Some(kept.fetched_at);
                Ok(kept.value.refs())
            }
        }
    }

    /// When GitHub listed the branches and tags `repo`'s last read went
    /// by, if that was before it went away.
    async fn listed(&self, repo: &RepoId) -> Option<u64> {
        self.refs_of(repo).lock().await.old
    }

    /// Makes `repo` readable offline ([`crate::offline::keep`]): its clone
    /// (the cache's is made, and waited for, where there's none), at
    /// GitHub's default branch, with that commit's files fetched (into
    /// the cache's partial clone, all at once) while they fit in
    /// `budget` bytes, and its branches and tags listed (and kept). The
    /// clone's size on disk.
    pub async fn keep(
        self: &Arc<Self>,
        gh: &GitHub,
        repo: &RepoId,
        budget: u64,
    ) -> Result<u64, String> {
        let started = Instant::now();
        let (clone, _) = loop {
            match self.clone_of(gh, repo).await {
                Ok(found) => break found,
                Err(_) if matches!(lock(&self.clones).get(&slug(repo)), Some(Slot::Cloning)) => {
                    if started.elapsed() > KEEP_CLONE_WAIT {
                        return Err(format!("{repo} is still being cloned"));
                    }
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                Err(why) => return Err(why),
            }
        };
        let fail = |err: ReadError| err.to_string();
        self.remote_refs(gh, repo, &clone).await.map_err(fail)?;
        let head = self.at(gh, repo, &clone, "HEAD").await.map_err(fail)?;
        if let Some(head) = head.filter(|_| clone.partial) {
            let out = (clone
                .run(&["ls-tree", "-r", "-l", "-z", head.as_ref()])
                .await)
                .map_err(|e| e.to_string())?;
            let mut room = budget;
            let blobs: Vec<String> = (out.split('\0'))
                .filter_map(|line| {
                    let (meta, _) = line.split_once('\t')?;
                    let mut meta = meta.split_whitespace();
                    let (_, kind, oid, size) =
                        (meta.next()?, meta.next()?, meta.next()?, meta.next()?);
                    let size: u64 = size.parse().ok()?;
                    (kind == "blob" && size <= room).then(|| {
                        room -= size;
                        oid.to_owned()
                    })
                })
                .collect();
            for batch in blobs.chunks(KEEP_BATCH) {
                clone.prefetch(batch).await.map_err(|e| e.to_string())?;
            }
        }
        let path = clone.path.clone();
        let size = tokio::task::spawn_blocking(move || crate::offline::keep::disk_size(&path))
            .await
            .map_err(|e| e.to_string())?;
        Ok(size)
    }

    fn refs_of(&self, repo: &RepoId) -> Arc<tokio::sync::Mutex<Known>> {
        let mut all = lock(&self.refs);
        all.entry(slug(repo)).or_default().clone()
    }

    /// The commit GitHub's `rev` names (a branch, a tag, its default
    /// branch for `HEAD`, a commit ID), here: fetched when the clone
    /// doesn't have it. `None` when GitHub has nothing by that name and
    /// the clone no commit.
    async fn at(
        &self,
        gh: &GitHub,
        repo: &RepoId,
        clone: &Repo,
        rev: &str,
    ) -> Result<Option<Oid>, ReadError> {
        let quiet = |_: String| {};
        if let Ok(oid) = Oid::parse(rev) {
            return match clone.ensure(&oid, &quiet).await {
                Ok(()) => Ok(Some(oid)),
                Err(GitError::Failed { .. }) => Ok(None),
                Err(err) => Err(err.into()),
            };
        }
        let named = if rev == "HEAD" {
            let entry = self.refs_of(repo);
            let mut refs = entry.lock().await;
            match &refs.head {
                Some((at, oid)) if at.elapsed() < REFS_FRESH => Some(oid.clone()),
                _ => {
                    let asked = match gh.offline_since() {
                        None => read::remote_head(clone).await,
                        Some(_) => Err(offline(repo)),
                    };
                    match asked {
                        Ok((name, oid)) => {
                            refs.head = Some((Instant::now(), oid.clone()));
                            refs.old = None;
                            let kept = Kept::of_head(&name, &oid);
                            gh.remember(&kept_key("head", repo), &kept);
                            Some(oid)
                        }
                        Err(err) => {
                            let key = kept_key("head", repo);
                            let kept = gh.cached::<Kept>(&key).ok_or(err)?;
                            refs.old = Some(kept.fetched_at);
                            kept.value.head()
                        }
                    }
                }
            }
        } else {
            self.remote_refs(gh, repo, clone).await?.find(rev).cloned()
        };
        match named {
            Some(oid) => {
                clone.ensure(&oid, &quiet).await?;
                Ok(Some(oid))
            }
            // A short commit ID the clone knows.
            None => Ok(clone.rev_parse(rev).await.ok()),
        }
    }

    /// `key` read from `clone`.
    async fn read(
        &self,
        gh: &GitHub,
        clone: &Repo,
        repo: &RepoId,
        key: &DataKey,
        after: Option<String>,
    ) -> Result<Answer, ReadError> {
        let absent = |what: String| Ok(Answer::Absent(what));
        let at = |rev: String| async move {
            let oid = self.at(gh, repo, clone, &rev).await?;
            oid.ok_or_else(|| {
                ReadError::Limit(format!("git has no branch, tag or commit {rev} of {repo}"))
            })
        };
        Ok(match key {
            DataKey::Tree(_, rev, path) => {
                let oid = at(rev.clone()).await?;
                match read::tree(clone, &oid, path).await? {
                    Some(entries) => Answer::Data(Data::Tree(entries), Some(oid)),
                    None => return absent(format!("{repo}/{path} at {rev}")),
                }
            }
            DataKey::Blob(_, rev, path) => {
                let oid = at(rev.clone()).await?;
                match read::blob(clone, &oid, path).await? {
                    Some(blob) => Answer::Data(Data::Blob(Box::new(blob)), Some(oid)),
                    None => return absent(format!("{repo}/{path} at {rev}")),
                }
            }
            DataKey::Files(_, rev) => {
                let oid = at(rev.clone()).await?;
                let (files, cut) = read::files(clone, &oid).await?;
                Answer::Data(Data::Files(Arc::new(files), cut), Some(oid))
            }
            DataKey::History(_, rev, path) => {
                let oid = at(rev.clone()).await?;
                let skip = after.as_deref().and_then(read::history_skip).unwrap_or(0);
                let page = read::history(clone, &oid, path, skip).await?;
                Answer::Data(Data::History(Box::new(page)), Some(oid))
            }
            DataKey::Commit(_, rev) => {
                let Some(oid) = self.at(gh, repo, clone, rev).await? else {
                    return Err(ReadError::Limit(format!("git doesn't have commit {rev}")));
                };
                match read::commit(clone, &oid).await? {
                    Some(c) => Answer::Data(Data::Commit(Box::new(c)), Some(oid)),
                    None => return absent(format!("{repo}@{rev}")),
                }
            }
            DataKey::Blame(_, rev, path) => {
                let oid = at(rev.clone()).await?;
                match read::blame(clone, &oid, path).await? {
                    Some(b) => Answer::Data(Data::Blame(Box::new(b)), Some(oid)),
                    None => return absent(format!("{repo}/{path} at {rev}")),
                }
            }
            DataKey::Compare(_, spec) => {
                let (base, head, direct) = match spec.split_once("...") {
                    Some((base, head)) => (base.to_owned(), head.to_owned(), false),
                    None => match spec.split_once("..") {
                        Some((base, head)) => (base.to_owned(), head.to_owned(), true),
                        None => ("HEAD".to_owned(), spec.clone(), false),
                    },
                };
                // A fork's branch (`owner:branch`) is in another repository.
                if base.contains(':') || head.contains(':') {
                    return Err(ReadError::Limit(
                        "a fork's branch isn't in this clone".into(),
                    ));
                }
                let (base, head) = (at(base).await?, at(head).await?);
                let c = read::compare(clone, &base, &head, direct).await?;
                Answer::Data(Data::Compare(Box::new(c)), Some(head))
            }
            DataKey::LastCommits(_, rev, path) => {
                let oid = at(rev.clone()).await?;
                // At one commit, the answer never changes.
                let memo_key = (oid.clone(), path.clone());
                if let Some(found) = lock(&self.memo).get(&memo_key) {
                    return Ok(Answer::Data(Data::LastCommits(found.clone()), Some(oid)));
                }
                let Some(entries) = read::tree(clone, &oid, path).await? else {
                    return absent(format!("{repo}/{path} at {rev}"));
                };
                let names: Vec<String> = entries.into_iter().map(|e| e.name).collect();
                let names = names
                    .get(..crate::api::browse::LAST_COMMIT_ENTRIES)
                    .unwrap_or(&names);
                let started = Instant::now();
                let mut found =
                    read::last_commits(clone, &oid, path, names, || started.elapsed() > WALK)
                        .await?;
                let rest: Vec<String> = (names.iter())
                    .filter(|n| !found.contains_key(*n))
                    .cloned()
                    .collect();
                if !rest.is_empty() {
                    // The oldest entries are GitHub's to find, at the same
                    // commit.
                    tracing::info!(n = rest.len(), "last commits past git's walk");
                    let more = gh.last_commits(repo, &oid, path, &rest).await;
                    found.extend(more.map_err(|e| ReadError::Limit(e.to_string()))?);
                }
                let found = Arc::new(found);
                lock(&self.memo).insert(memo_key, found.clone());
                Answer::Data(Data::LastCommits(found), Some(oid))
            }
            DataKey::Refs(_) => {
                let listed = self.remote_refs(gh, repo, clone).await?;
                let names = |list: Vec<(String, Oid)>| {
                    Capped::from(list.into_iter().map(|(n, _)| n).collect::<Vec<_>>())
                };
                let mut tags = listed.tags;
                // Newest versions first, as near as names say.
                tags.sort_by(|a, b| version_order(&b.0, &a.0));
                let refs = Refs {
                    branches: names(listed.branches),
                    tags: names(tags),
                };
                Answer::Data(Data::Refs(Box::new(refs)), None)
            }
            DataKey::Tags(_) => {
                let listed = self.remote_refs(gh, repo, clone).await?;
                let tags = read::tags(clone, &listed.tags).await?;
                let tags = Results {
                    total: tags.len() as u64,
                    items: tags,
                    next: None,
                };
                Answer::Data(Data::Tags(Box::new(tags)), None)
            }
            DataKey::RefIn(_, spot) => {
                let listed = self.remote_refs(gh, repo, clone).await?;
                Answer::Data(Data::RefIn(listed.ref_in(spot)), None)
            }
            DataKey::Search(SearchKind::Code, query) => {
                let Some(search) = code_search(query) else {
                    return Err(ReadError::Limit("not a search git runs".into()));
                };
                let oid = at(search.rev.clone().unwrap_or_else(|| "HEAD".into())).await?;
                let hits = read::grep(clone, repo, &oid, &search.grep).await?;
                let results = Results {
                    total: hits.len() as u64,
                    items: hits,
                    next: None,
                };
                let found = SearchResults::Code(results);
                Answer::Data(Data::Search(Box::new(found)), Some(oid))
            }
            DataKey::Activity(_) => {
                let oid = at("HEAD".into()).await?;
                let now = crate::store::now();
                let activity = crate::site::insights::activity(clone, &oid, now).await?;
                Answer::Data(Data::Activity(Box::new(activity)), Some(oid))
            }
            _ => return Err(ReadError::Limit("git doesn't know it".into())),
        })
    }
}

/// How long git gets for `key` before the API answers.
fn wait(key: &DataKey) -> Duration {
    Duration::from_secs(match key {
        DataKey::Blame(..) | DataKey::Compare(..) | DataKey::Search(..) | DataKey::Activity(_) => {
            20
        }
        DataKey::LastCommits(..) => 12,
        _ => 6,
    })
}

/// Tag names in version order: `v1.10` after `v1.9`.
fn version_order(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |s: &str| -> Vec<(u64, String)> {
        (s.split(|c: char| !c.is_ascii_alphanumeric()))
            .map(|p| {
                let digits: String = p.chars().take_while(char::is_ascii_digit).collect();
                (digits.parse().unwrap_or(0), p.to_owned())
            })
            .collect()
    };
    parts(a).cmp(&parts(b))
}

/// How much a clone has received, from its progress line (`Receiving
/// objects:  45% (1/2), 123.45 MiB | 1 MiB/s`).
fn received(line: &str) -> Option<u64> {
    let rest = line.strip_prefix("Receiving objects:")?;
    let amount = rest.split_once("), ")?.1.split(" |").next()?.trim();
    let (number, unit) = amount.split_once(' ')?;
    let scale: u64 = match unit.trim_end_matches(',') {
        "bytes" => 1,
        "KiB" => 1 << 10,
        "MiB" => 1 << 20,
        "GiB" => 1 << 30,
        _ => return None,
    };
    let number: f64 = number.parse().ok()?;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "a size to compare with a limit"
    )]
    Some((number * scale as f64) as u64)
}

/// Where a clone given up as too big is noted.
fn marker(cache_root: &Path, repo: &RepoId) -> PathBuf {
    let path = Repo::cache_path(cache_root, &repo.owner, &repo.name);
    path.with_extension("too-big")
}

/// Why `repo` isn't cloned: it was found too big within the last week.
fn too_big(cache_root: &Path, repo: &RepoId) -> Option<String> {
    let marker = marker(cache_root, repo);
    let age = std::fs::metadata(&marker)
        .ok()?
        .modified()
        .ok()?
        .elapsed()
        .ok()?;
    (age < TOO_BIG_FOR).then(|| std::fs::read_to_string(&marker).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_search_in_one_repository_is_git_s() {
        let s = code_search(r#"repo:o/r rev:v1 "fn main" extension:rs path:src/"#).unwrap();
        assert_eq!(s.repo, RepoId::new("o", "r"));
        assert_eq!(s.rev.as_deref(), Some("v1"));
        assert_eq!(s.grep.words, ["fn main"]);
        assert_eq!(s.grep.paths, ["*.rs", "src/"]);
        for github in [
            "needle",
            "repo:o/r",
            "repo:o/r language:rust needle",
            "repo:o/r repo:o/s needle",
            "repo:o/r a OR b",
        ] {
            assert_eq!(code_search(github), None, "{github}");
        }
        assert!(by_git(
            &DataKey::Search(SearchKind::Code, "repo:o/r x".into()),
            None
        ));
        assert!(!by_git(
            &DataKey::Search(SearchKind::Code, "org:o x".into()),
            None
        ));
        let history = DataKey::History(RepoId::new("o", "r"), "main".into(), String::new());
        assert!(by_git(&history, Some("git:30")));
        assert!(!by_git(&history, Some("Y3Vyc29y")), "GitHub's own cursor");
    }

    use crate::git::set_up as git;

    /// Pages read from the cache's clone at the commit GitHub's branch is
    /// at, fetching it when GitHub's moved on; what isn't there is "not
    /// found", from git.
    #[tokio::test]
    async fn pages_read_from_the_clone_at_github_s_commit() {
        let github = tempfile::tempdir().unwrap();
        let g = github.path();
        git(g, &["init", "-q", "-b", "main"]).await;
        for (key, value) in [
            ("user.name", "Ann"),
            ("user.email", "ann@example.com"),
            ("uploadpack.allowFilter", "true"),
            ("uploadpack.allowAnySHA1InWant", "true"),
        ] {
            git(g, &["config", key, value]).await;
        }
        std::fs::write(g.join("a.txt"), "one\n").unwrap();
        git(g, &["add", "a.txt"]).await;
        git(g, &["commit", "-q", "-m", "First"]).await;
        let cache = tempfile::tempdir().unwrap();
        let url = format!("file://{}", g.display());
        let credentials = crate::git::credentials::Credentials::Ambient;
        Repo::open_cache(cache.path(), "o", "r", &url, credentials.clone(), &|_| {})
            .await
            .unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let local = Local::new(GitContext {
            cache_root: cache.path().to_owned(),
            credentials,
            cwd: elsewhere.path().to_owned(),
        });
        let gh = GitHub::new(
            crate::api::auth::Token::new("t"),
            crate::store::Store::disabled(),
            crate::host::Host::GITHUB_COM,
        );
        let repo = RepoId::new("o", "r");
        let blob = |path: &str| DataKey::Blob(repo.clone(), "main".into(), path.into());

        // GitHub moved on since the clone was made.
        std::fs::write(g.join("b.txt"), "two\n").unwrap();
        git(g, &["add", "b.txt"]).await;
        git(g, &["commit", "-q", "-m", "Second"]).await;
        let head = Oid::parse(git(g, &["rev-parse", "main"]).await.trim()).unwrap();
        let (data, from) = local.fetch(&gh, &blob("b.txt"), None).await;
        let Ok(Data::Blob(b)) = data else {
            panic!("{data:?}")
        };
        assert_eq!(b.text.as_deref(), Some("two\n"));
        let at_head = From::Git {
            clone: Clone::Cache,
            at: Some(head.clone()),
            listed: None,
        };
        assert_eq!(from, at_head);
        assert!(
            from.note(0)
                .starts_with(&format!("At {}", crate::ui::text::short_sha(&head)))
        );

        let (data, from) = local.fetch(&gh, &blob("c.txt"), None).await;
        assert!(matches!(data, Err(ApiError::NotFound(_))), "{data:?}");
        assert!(matches!(from, From::Git { .. }));

        let history = DataKey::History(repo.clone(), "main".into(), String::new());
        let (data, _) = local.fetch(&gh, &history, None).await;
        let Ok(Data::History(h)) = data else {
            panic!("{data:?}")
        };
        assert_eq!(h.items.len(), 2);
        let refs = DataKey::Refs(repo.clone());
        let Ok(Data::Refs(refs)) = local.fetch(&gh, &refs, None).await.0 else {
            panic!("no refs")
        };
        assert_eq!(refs.branches.items, ["main"]);

        // Tags made since (one annotated), at their commits, newest first.
        git(g, &["tag", "-a", "v1", "-m", "One", "main~1"]).await;
        git(g, &["tag", "v2", "main"]).await;
        let fresh = Local::new(local.git.clone());
        let tags = DataKey::Tags(repo.clone());
        let Ok(Data::Tags(tags)) = fresh.fetch(&gh, &tags, None).await.0 else {
            panic!("no tags")
        };
        let named: Vec<_> = (tags.items.iter())
            .map(|t| {
                (
                    t.name.as_str(),
                    t.oid.as_deref().map(crate::ui::text::short_sha),
                )
            })
            .collect();
        let first = git(g, &["rev-parse", "main~1"]).await;
        assert_eq!(
            named,
            [
                ("v2", Some(crate::ui::text::short_sha(&head))),
                ("v1", Some(crate::ui::text::short_sha(&first)))
            ]
        );
        assert!(tags.items.iter().all(|t| t.date.is_some()));
    }

    /// Without GitHub, the clone still answers, at the commit GitHub's
    /// branch was at when it was last listed, and says so; a repository
    /// without a clone isn't cloned (nor given up on).
    #[tokio::test]
    async fn offline_the_clone_reads_at_the_branch_github_last_listed() {
        let github = tempfile::tempdir().unwrap();
        let g = github.path().join("r.git");
        std::fs::create_dir(&g).unwrap();
        git(&g, &["init", "-q", "-b", "main"]).await;
        git(&g, &["config", "uploadpack.allowFilter", "true"]).await;
        std::fs::write(g.join("a.txt"), "one\n").unwrap();
        git(&g, &["add", "a.txt"]).await;
        let who = ["-c", "user.name=Ann", "-c", "user.email=ann@example.com"];
        git(&g, &[&who[..], &["commit", "-q", "-m", "First"]].concat()).await;
        let cache = tempfile::tempdir().unwrap();
        let url = format!("file://{}", g.display());
        let credentials = crate::git::credentials::Credentials::Ambient;
        Repo::open_cache(cache.path(), "o", "r", &url, credentials.clone(), &|_| {})
            .await
            .unwrap();
        let context = GitContext {
            cache_root: cache.path().to_owned(),
            credentials,
            cwd: cache.path().to_owned(),
        };
        let store = crate::store::Store::open(&cache.path().join("cache.redb"));
        let gh = GitHub::new(
            crate::api::auth::Token::new("t"),
            store,
            crate::host::Host::GITHUB_COM,
        );
        let repo = RepoId::new("o", "r");
        let blob = DataKey::Blob(repo.clone(), "main".into(), "a.txt".into());
        let (data, from) = Local::new(context.clone()).fetch(&gh, &blob, None).await;
        assert!(data.is_ok(), "{data:?}");
        assert!(matches!(from, From::Git { listed: None, .. }), "{from:?}");

        // GitHub goes away.
        std::fs::rename(&g, github.path().join("gone")).unwrap();
        gh.reach().lost();
        let local = Local::new(context);
        let (data, from) = local.fetch(&gh, &blob, None).await;
        let Ok(Data::Blob(b)) = data else {
            panic!("{data:?}")
        };
        assert_eq!(b.text.as_deref(), Some("one\n"));
        assert!(
            matches!(
                from,
                From::Git {
                    listed: Some(_),
                    ..
                }
            ),
            "{from:?}"
        );
        assert!(
            from.note(crate::store::now()).contains("offline"),
            "{}",
            from.note(0)
        );
        let other = RepoId::new("o", "elsewhere");
        let tree = DataKey::Tree(other.clone(), "main".into(), String::new());
        let (data, from) = local.fetch(&gh, &tree, None).await;
        assert!(matches!(data, Err(ApiError::Offline(_))), "{data:?}");
        assert!(
            matches!(&from, From::Api(why) if why.contains("no clone")),
            "{from:?}"
        );
        assert!(lock(&local.clones).get(&slug(&other)).is_none());
    }

    #[test]
    fn a_clone_s_progress_says_how_much_came() {
        let line = "Receiving objects:  45% (1/2), 123.50 MiB | 1.00 MiB/s";
        assert_eq!(received(line), Some(123 * (1 << 20) + (1 << 19)));
        assert_eq!(received("Receiving objects: 100% (5/5), done."), None);
        assert_eq!(received("Resolving deltas: 1%"), None);
    }

    #[test]
    fn tags_go_newest_version_first() {
        let mut tags = vec!["v1.9", "v1.10", "v1.2"];
        tags.sort_by(|a, b| version_order(b, a));
        assert_eq!(tags, ["v1.10", "v1.9", "v1.2"]);
    }
}
