//! Repositories kept to read offline: their pages fetched ahead into the
//! cache, and kept fresh while GitHub can be reached, within plain
//! limits. Kept are the repository's page, its Issues, Pull requests
//! and Discussions tabs as they open, and its issues, pull requests and
//! discussions updated most recently (up to [`Limits::items`] of each,
//! updated in the last [`Limits::days`]), each as its page shows it; and
//! its clone, at the default branch, with that commit's files. Fetching
//! stops while the cache is over [`Limits::disk`]. Every half hour (and
//! when GitHub is back) what changed since is fetched again: an item
//! updated since it was kept, and the lists.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::api::browse::{DiscussionsOf, IssueKind, SearchKind, SearchResults};
use crate::api::model::{PrRef, RepoId};
use crate::api::{ApiError, GitHub};
use crate::browse::{DataKey, Need};
use crate::route::Route;
use crate::ui::pages::PrTab;

/// How much is kept, from `config.toml`'s `[offline]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    /// Of each kind (issues, pull requests, discussions), how many.
    pub items: u64,
    /// Only those updated in this many days.
    pub days: u64,
    /// No more is fetched while the cache and the kept clones together
    /// are over this many MiB.
    pub disk: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            items: 100,
            days: 90,
            disk: 2048,
        }
    }
}

impl Limits {
    fn bytes(self) -> u64 {
        self.disk.saturating_mul(1 << 20)
    }

    /// As people say it.
    pub fn said(self) -> String {
        format!(
            "up to {} issues, {} pull requests and {} discussions each, updated in the last {} days, while the cache and clones take under {} MiB",
            self.items, self.items, self.items, self.days, self.disk
        )
    }
}

/// A repository kept offline, and how keeping it went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Kept {
    pub repo: RepoId,
    /// When it was last fetched ahead to the end.
    pub refreshed: Option<u64>,
    pub issues: u64,
    pub pulls: u64,
    pub discussions: u64,
    /// How big its clone is.
    pub clone: Option<u64>,
    /// What stopped the last refresh short, or went wrong in it.
    pub problem: Option<String>,
    /// Each item's `updated_at` when it was kept (`i12`, `p3`, `d7`), so
    /// only what changed is fetched again.
    #[serde(default)]
    pub seen: BTreeMap<String, String>,
    /// It's being fetched now.
    #[serde(skip)]
    pub busy: bool,
}

impl Kept {
    fn new(repo: RepoId) -> Self {
        Self {
            repo,
            refreshed: None,
            issues: 0,
            pulls: 0,
            discussions: 0,
            clone: None,
            problem: None,
            seen: BTreeMap::new(),
            busy: false,
        }
    }
}

/// What the keeper is asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// Keep a repository, or stop keeping it.
    Toggle(RepoId),
    /// Fetch them all again now (GitHub is back).
    Now,
}

/// Where the kept repositories are listed, in the cache.
const KEY: &str = "offline:kept";
/// How often what's kept is refreshed.
const EVERY: Duration = Duration::from_secs(30 * 60);
/// How many pages are fetched at once.
const AT_ONCE: usize = 4;

/// Runs the keeper: one task, refreshing one repository at a time.
pub fn start(
    gh: GitHub,
    local: Arc<crate::source::Local>,
    limits: Limits,
    tx: mpsc::UnboundedSender<crate::state::Msg>,
) -> mpsc::UnboundedSender<Op> {
    let (ops, mut rx) = mpsc::unbounded_channel::<Op>();
    tokio::spawn(async move {
        let tell = |kept: &[Kept]| {
            let msg = crate::offline::Msg::Kept(kept.to_vec(), gh.store().size());
            let _ = tx.send(crate::state::Msg::Offline(msg));
        };
        let mut keeper = Keeper {
            kept: gh
                .cached::<Vec<Kept>>(KEY)
                .map(|c| c.value)
                .unwrap_or_default(),
            due: Vec::new(),
        };
        tell(&keeper.kept);
        // Each is due once ghtui has settled in.
        let mut wait = Duration::from_secs(20);
        loop {
            #[expect(
                clippy::disallowed_methods,
                reason = "the keeper's own pace: nothing waits on it"
            )]
            let op = tokio::time::timeout(wait, rx.recv()).await;
            match op {
                Ok(None) => return,
                Ok(Some(op)) => keeper.take(op),
                Err(_) => keeper.take(Op::Now),
            }
            wait = EVERY;
            gh.remember(KEY, &keeper.kept);
            tell(&keeper.kept);
            while gh.offline_since().is_none()
                && let Some(one) = keeper.next()
            {
                tell(&keeper.kept);
                let clones = keeper.kept.iter().filter_map(|k| k.clone).sum();
                let done = refresh(&gh, &local, limits, clones, one).await;
                keeper.done(done);
                // What was asked meanwhile, first.
                while let Ok(op) = rx.try_recv() {
                    keeper.take(op);
                }
                gh.remember(KEY, &keeper.kept);
                tell(&keeper.kept);
            }
        }
    });
    ops
}

/// The kept repositories, and those due a refresh.
struct Keeper {
    kept: Vec<Kept>,
    due: Vec<RepoId>,
}

impl Keeper {
    fn take(&mut self, op: Op) {
        match op {
            Op::Toggle(repo) => match self.kept.iter().position(|k| k.repo == repo) {
                Some(at) => {
                    self.kept.remove(at);
                    self.due.retain(|r| *r != repo);
                }
                None => {
                    self.kept.push(Kept::new(repo.clone()));
                    self.due.push(repo);
                }
            },
            Op::Now => self.due = self.kept.iter().map(|k| k.repo.clone()).collect(),
        }
    }

    /// The next to refresh, marked busy.
    fn next(&mut self) -> Option<Kept> {
        while !self.due.is_empty() {
            let repo = self.due.remove(0);
            if let Some(k) = self.kept.iter_mut().find(|k| k.repo == repo) {
                k.busy = true;
                return Some(k.clone());
            }
        }
        None
    }

    /// A refresh is done; unless it stopped being kept meanwhile.
    fn done(&mut self, done: Kept) {
        if let Some(slot) = self.kept.iter_mut().find(|k| k.repo == done.repo) {
            *slot = done;
        }
    }
}

/// Fetches `kept`'s repository ahead, within `limits`.
/// `clones` is how much the kept clones take.
async fn refresh(
    gh: &GitHub,
    local: &Arc<crate::source::Local>,
    limits: Limits,
    clones: u64,
    mut kept: Kept,
) -> Kept {
    kept.busy = false;
    kept.problem = None;
    let repo = kept.repo.clone();
    let cutoff = crate::store::now().saturating_sub(limits.days.saturating_mul(86_400));
    let room = || {
        let used = gh.store().size().unwrap_or(0).saturating_add(clones);
        limits.bytes().saturating_sub(used)
    };
    // The clone first: code reads from it.
    tracing::info!(%repo, "keeping offline: its clone");
    match local.keep(gh, &repo, room()).await {
        Ok(size) => kept.clone = Some(size),
        Err(why) => kept.problem = Some(format!("The clone: {why}")),
    }
    let tabs = [
        Route::Repo(repo.clone()),
        Route::Issues {
            repo: repo.clone(),
            query: crate::route::OPEN.into(),
        },
        Route::Pulls {
            repo: repo.clone(),
            query: crate::route::OPEN.into(),
        },
        Route::Discussions {
            of: DiscussionsOf::Repo(repo.clone()),
            category: None,
        },
    ];
    let pages = tabs.iter().flat_map(crate::browse::needs);
    tracing::info!(%repo, "keeping offline: its tabs");
    if let Err(err) = fetch_all(gh, pages.collect()).await {
        kept.problem = Some(err.to_string());
        return kept;
    }
    let mut current = std::collections::BTreeSet::new();
    for kind in [Kind::Issue, Kind::Pull, Kind::Discussion] {
        if room() == 0 {
            kept.problem = Some(format!("Stopped at the disk limit ({} MiB)", limits.disk));
            return kept;
        }
        tracing::info!(%repo, ?kind, "keeping offline: its items");
        let items = match recent(gh, &repo, kind, limits.items, cutoff).await {
            Ok(items) => items,
            Err(err) => {
                kept.problem = Some(err.to_string());
                return kept;
            }
        };
        let count = items.len() as u64;
        current.extend(items.iter().map(|(key, ..)| key.clone()));
        let changed: Vec<(String, String, Route)> = items
            .into_iter()
            .filter(|(key, updated, _)| kept.seen.get(key) != Some(updated))
            .collect();
        let needs: Vec<Need> = changed
            .iter()
            .flat_map(|(_, _, r)| crate::browse::needs(r))
            .collect();
        if let Err(err) = fetch_all(gh, needs).await {
            kept.problem = Some(err.to_string());
            return kept;
        }
        for (key, updated, _) in changed {
            kept.seen.insert(key, updated);
        }
        match kind {
            Kind::Issue => kept.issues = count,
            Kind::Pull => kept.pulls = count,
            Kind::Discussion => kept.discussions = count,
        }
    }
    // Only what's still kept is remembered.
    kept.seen.retain(|key, _| current.contains(key));
    kept.refreshed = Some(crate::store::now());
    kept
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    Issue,
    Pull,
    Discussion,
}

/// `repo`'s items of `kind` updated most recently, newest first, up to
/// `max` and back to `cutoff`: each as its key in [`Kept::seen`], when
/// it was updated, and its page.
async fn recent(
    gh: &GitHub,
    repo: &RepoId,
    kind: Kind,
    max: u64,
    cutoff: u64,
) -> Result<Vec<(String, String, Route)>, ApiError> {
    let mut found = Vec::new();
    let mut after = None;
    let recent =
        |updated: &str| crate::ui::time::parse_iso8601(updated).is_none_or(|t| t >= cutoff);
    loop {
        let (page, next) = match kind {
            Kind::Issue | Kind::Pull => {
                let (search, is) = match kind {
                    Kind::Issue => (SearchKind::Issues, "issue"),
                    _ => (SearchKind::Pulls, "pr"),
                };
                let query = format!("repo:{repo} is:{is} sort:updated-desc");
                let SearchResults::Issues(results) = gh.search(search, &query, after).await? else {
                    return Ok(found);
                };
                let page: Vec<_> = (results.items.into_iter())
                    .map(|i| {
                        let (key, route) = match i.kind {
                            IssueKind::Pr { .. } => (
                                format!("p{}", i.number),
                                Route::Pr {
                                    pr: PrRef {
                                        repo: repo.clone(),
                                        number: i.number,
                                    },
                                    tab: PrTab::Conversation,
                                },
                            ),
                            IssueKind::Issue(_) => (
                                format!("i{}", i.number),
                                Route::Issue {
                                    repo: repo.clone(),
                                    number: i.number,
                                },
                            ),
                        };
                        (key, i.updated_at, route)
                    })
                    .collect();
                (page, results.next)
            }
            Kind::Discussion => {
                let of = DiscussionsOf::Repo(repo.clone());
                let list = match gh.discussions(&of, None, after).await {
                    Ok(list) => list,
                    // Discussions turned off: none to keep.
                    Err(ApiError::NotFound(_)) => return Ok(found),
                    Err(err) => return Err(err),
                };
                let page: Vec<_> = (list.results.items.into_iter())
                    .map(|d| {
                        let route = Route::Discussion {
                            of: of.clone(),
                            number: d.number,
                        };
                        (format!("d{}", d.number), d.updated_at, route)
                    })
                    .collect();
                (page, list.results.next)
            }
        };
        let mut old = false;
        for item in page {
            if found.len() as u64 >= max {
                return Ok(found);
            }
            if !recent(&item.1) {
                old = true;
                break;
            }
            found.push(item);
        }
        match next {
            Some(next) if !old => after = Some(next),
            _ => return Ok(found),
        }
    }
}

/// Fetches what the pages need into the cache, a few at a time; stops at
/// the first sign GitHub's gone. Other failures (an item since deleted)
/// are left for the page to say.
async fn fetch_all(gh: &GitHub, needs: Vec<Need>) -> Result<(), ApiError> {
    let mut needs = needs;
    needs.dedup();
    let mut asks = futures::stream::iter(needs.into_iter().filter(worth_keeping))
        .map(|need| async move {
            match need {
                Need::Pr(pr) => gh.pull_request(&pr).await.map(drop),
                Need::Data(key) => crate::runtime::fetch(gh, &key, None, None).await.map(drop),
            }
        })
        .buffer_unordered(AT_ONCE);
    while let Some(result) = asks.next().await {
        match result {
            Err(
                err @ (ApiError::Offline(_) | ApiError::RateLimited(_) | ApiError::Unauthorized),
            ) => {
                return Err(err);
            }
            Err(err) => tracing::debug!(%err, "a page kept offline didn't load"),
            Ok(()) => {}
        }
    }
    Ok(())
}

/// Whether a page's need is kept in the cache. What git answers comes
/// from the clone; some data isn't cached at all.
fn worth_keeping(need: &Need) -> bool {
    match need {
        Need::Pr(_) => true,
        Need::Data(key) if crate::source::by_git(key, None) => false,
        Need::Data(key) => !matches!(
            key,
            DataKey::InProjects(..)
                | DataKey::Sidebar(..)
                | DataKey::NewForm(..)
                | DataKey::Subscription(..)
                | DataKey::CommitComments(..)
                | DataKey::JobLog(..)
                | DataKey::Lines(..)
                | DataKey::Wiki(..)
        ),
    }
}

/// How many bytes the files under `path` take.
pub fn disk_size(path: &std::path::Path) -> u64 {
    let mut total = 0u64;
    let mut dirs = vec![path.to_owned()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            match entry.file_type() {
                Ok(t) if t.is_dir() => dirs.push(entry.path()),
                Ok(t) if t.is_file() => {
                    total = total.saturating_add(entry.metadata().map_or(0, |m| m.len()));
                }
                _ => {}
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_kept_is_what_the_cache_keeps() {
        let repo = RepoId::new("o", "r");
        let issue = crate::browse::needs(&Route::Issue {
            repo: repo.clone(),
            number: 1,
        });
        let kept: Vec<&Need> = issue.iter().filter(|n| worth_keeping(n)).collect();
        assert_eq!(
            kept,
            [
                &Need::Data(DataKey::Repo(repo.clone())),
                &Need::Data(DataKey::Issue(repo.clone(), 1))
            ]
        );
        // The code comes from the clone.
        let tree = Need::Data(DataKey::Tree(repo, "main".into(), String::new()));
        assert!(!worth_keeping(&tree));
    }

    #[test]
    fn limits_come_from_the_config_and_say_themselves() {
        let limits: Limits = toml_edit::de::from_str("items = 20\ndisk = 100").unwrap();
        assert_eq!(
            limits,
            Limits {
                items: 20,
                days: 90,
                disk: 100
            }
        );
        assert!(
            limits.said().contains("up to 20 issues"),
            "{}",
            limits.said()
        );
        toml_edit::de::from_str::<Limits>("item = 20").unwrap_err();
    }
}
