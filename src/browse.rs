//! Browsing: the data behind each page, and building pages from it.

use crate::route::Family;
use crate::route::code;

use std::collections::BTreeSet;

use crate::api::browse::{CommitInfo, PrActivity, RepoOverview};
use crate::api::model::{PrDetail, PrRef, RepoId};
use crate::api::project::{Project, ProjectsOf};
use crate::api::security::AlertRef;
use crate::ui::Fetched;
use crate::ui::page::{Page, PageLine, Role, Seg};
use crate::ui::pages::{self, Keys};

use crate::data::{Data, DataKey, Picked};
use crate::diff_screen::{DiffOf, DiffState};
use crate::keymap::Action;
use crate::route::Route;
use crate::state::{Remote, State};

/// Widest a page of reading (a conversation, a Markdown file) gets, its
/// sidebar included; wider terminals center it, as GitHub does.
const READING: u16 = 160;
/// Widest a page of lists gets: room for their columns, without a row's
/// two ends drifting apart.
const LISTS: u16 = 200;

/// The list a page loads more of.
pub fn paged(route: &Route) -> Option<DataKey> {
    match route {
        Route::Advisories(repo) => Some(DataKey::Advisories(repo.clone())),
        Route::Notifications { query } => Some(DataKey::Notifications(query.clone())),
        Route::Projects(of) => Some(DataKey::Projects(of.clone())),
        Route::Project { project, view } | Route::ProjectItem { project, view, .. } => {
            Some(DataKey::Project(project.clone(), *view))
        }
        Route::Alerts { repo, kind, query } => {
            Some(DataKey::Alerts(repo.clone(), *kind, query.clone()))
        }
        Route::App(p) => p.paged(),
        Route::Code(p) => p.paged(),
        Route::Wiki(p) => p.paged(),
        Route::Discussions(p) => p.paged(),
        Route::Gists(p) => p.paged(),
        Route::People(p) => p.paged(),
        Route::Releases(p) => p.paged(),
        Route::Runs(p) => p.paged(),
        Route::Issues(p) => p.paged(),
        Route::Pulls(p) => p.paged(),
        Route::Site(p) => p.paged(),
        Route::Housekeeping(p) => p.paged(),
        _ => route
            .search()
            .map(|(kind, query)| DataKey::Search(kind, query)),
    }
}

/// The pull request whose review a page shows at a glance: its Review
/// tab's.
pub(crate) fn glanced(route: &Route) -> Option<&PrRef> {
    match route {
        Route::Pulls(p) => p.glanced(),
        _ => None,
    }
}

/// What a page needs fetched, but Home's (its sections are the state's:
/// [`State::needs`]).
pub(crate) fn needs(route: &Route) -> Vec<DataKey> {
    use DataKey as K;
    let header = |repo: &RepoId| K::Repo(repo.clone());
    let list = route.search().map(|(kind, query)| K::Search(kind, query));
    match route {
        Route::App(_)
        | Route::Code(_)
        | Route::Issues(_)
        | Route::Pulls(_)
        | Route::Gists(_)
        | Route::Discussions(_)
        | Route::Wiki(_)
        | Route::People(_)
        | Route::Releases(_)
        | Route::Runs(_)
        | Route::Site(_)
        | Route::Housekeeping(_) => Vec::new(),
        Route::Search { .. } => list.into_iter().collect(),
        Route::Advisories(repo) | Route::Advisory { repo, .. } => {
            let own = match route {
                Route::Advisory { ghsa, .. } => K::Advisory(repo.clone(), ghsa.clone()),
                _ => K::Advisories(repo.clone()),
            };
            repo.iter().map(header).chain([own]).collect()
        }
        Route::Notifications { .. }
        | Route::Project { .. }
        | Route::ProjectItem { .. }
        | Route::Projects(ProjectsOf::Owner(_)) => paged(route).into_iter().collect(),
        Route::Alerts { repo, .. } | Route::Projects(ProjectsOf::Repo(repo)) => {
            std::iter::once(header(repo)).chain(paged(route)).collect()
        }
        Route::Alert { repo, kind, number } => {
            let a = AlertRef {
                repo: repo.clone(),
                kind: *kind,
                number: *number,
            };
            vec![header(repo), K::Alert(a)]
        }
        Route::Settings { repo, page } => {
            vec![header(repo), K::Settings(repo.clone(), *page)]
        }
    }
}

/// Whether `need` is a repository's header on one of its other pages
/// (fetched only when missing).
pub fn is_header(route: &Route, key: &DataKey) -> bool {
    matches!(key, DataKey::Repo(_)) && !matches!(route, Route::Code(code::Repo(_)))
}

/// How wide a page gets: reading stays narrow enough to follow, lists
/// get room for their columns, and output and tables take the whole
/// width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    Reading,
    Lists,
    Full,
    /// Reading, with a sidebar.
    ReadingAside,
    /// Lists, with a sidebar.
    ListsAside,
}

/// How wide `route`'s page gets.
fn width_of(route: &Route) -> Width {
    match route {
        Route::Advisory { .. } | Route::Alert { .. } | Route::ProjectItem { .. } => Width::Reading,
        Route::Project { .. } => Width::Full,
        Route::App(p) => p.width(),
        Route::Code(p) => p.width(),
        Route::Wiki(p) => p.width(),
        Route::Discussions(p) => p.width(),
        Route::Gists(p) => p.width(),
        Route::People(p) => p.width(),
        Route::Releases(p) => p.width(),
        Route::Runs(p) => p.width(),
        Route::Issues(p) => p.width(),
        Route::Pulls(p) => p.width(),
        Route::Site(p) => p.width(),
        Route::Housekeeping(p) => p.width(),
        _ => Width::Lists,
    }
}

/// What a page is built from: the state, and what every page builder
/// needs.
#[derive(Clone, Copy)]
pub struct Build<'a> {
    pub state: &'a State,
    pub cx: pages::PageCtx<'a>,
    /// The key that tries again.
    pub retry: &'a str,
}

impl<'a> Build<'a> {
    /// `key`'s data, as the page shows it: loading, failed (and why) or
    /// ready.
    pub fn get<T: Picked + ?Sized>(&self, key: &DataKey) -> Fetched<'a, T> {
        Remote::fetched(self.state.remote(key), self.retry).pick(T::pick)
    }

    /// The list the page loads more of.
    pub fn list<T: Picked + ?Sized>(&self, key: Option<DataKey>) -> Fetched<'a, T> {
        let remote = key.and_then(|key| self.state.remote(&key));
        Remote::fetched(remote, self.retry).pick(T::pick)
    }
}

/// A page's needs as the page shows them, failing with the key that
/// tries again.
struct Needs<'a> {
    state: &'a State,
    retry: &'a str,
}

impl<'a> Needs<'a> {
    fn of<T>(&self, remote: Option<&'a Remote<T>>) -> Fetched<'a, T> {
        Remote::fetched(remote, self.retry)
    }

    fn get<T: Picked + ?Sized>(&self, key: &DataKey) -> Fetched<'a, T> {
        self.of(self.state.remote(key)).pick(T::pick)
    }

    /// The list the route loads more of.
    fn paged<T: Picked + ?Sized>(&self, route: &Route) -> Fetched<'a, T> {
        let remote = paged(route).and_then(|key| self.state.remote(&key));
        self.of(remote).pick(T::pick)
    }
}

impl State {
    /// How fetching `key` is going, once it's started.
    pub fn remote(&self, key: &DataKey) -> Option<&Remote<Data>> {
        #[cfg(test)]
        reads::record(key);
        self.data.get(key)
    }

    pub fn get(&self, key: &DataKey) -> Option<&Data> {
        self.remote(key).and_then(|r| r.data.as_ref())
    }

    /// `key`'s data, if it's there: for what isn't a page.
    pub fn picked<T: Picked + ?Sized>(&self, key: &DataKey) -> Option<&T> {
        self.get(key).and_then(T::pick)
    }

    /// Where commit `oid` is among pull request `number`'s commits, once
    /// they're in, with links to the ones either side.
    pub fn pr_commit_at(&self, repo: &RepoId, number: u64, oid: &str) -> Option<pages::PrCommitAt> {
        let pr = PrRef {
            repo: repo.clone(),
            number,
        };
        let commits = &self
            .picked::<PrActivity>(&DataKey::PrActivity(pr.clone()))?
            .commits;
        let i = commits
            .items
            .iter()
            .position(|c| c.oid.starts_with(oid) || oid.starts_with(&c.oid))?;
        let link = |c: &CommitInfo| {
            let route = Route::Code(code::Commit {
                repo: repo.clone(),
                oid: c.oid.clone(),
                pr: Some(number),
            });
            route.url()
        };
        Some(pages::PrCommitAt {
            n: commits.left_out() + i as u64 + 1,
            of: commits.total(),
            pr: pages::url::pull(&pr),
            number,
            previous: i
                .checked_sub(1)
                .and_then(|i| commits.items.get(i))
                .map(link),
            next: commits.items.get(i + 1).map(link),
        })
    }

    pub fn overview(&self, repo: &RepoId) -> Option<&RepoOverview> {
        self.picked(&DataKey::Repo(repo.clone()))
    }

    pub fn pr(&self, pr: &PrRef) -> Option<&PrDetail> {
        self.picked(&DataKey::Pr(pr.clone()))
    }

    /// How fetching pull request `pr` is going.
    pub fn pr_remote(&self, pr: &PrRef) -> Option<&Remote<Data>> {
        self.remote(&DataKey::Pr(pr.clone()))
    }

    pub fn activity(&self, pr: &PrRef) -> Option<&PrActivity> {
        self.picked(&DataKey::PrActivity(pr.clone()))
    }

    /// Which state of its diff's inputs a page shows: a pull request's
    /// Review tab shows the diff's threads and drafts.
    pub(crate) fn diff_version(&self, route: &Route) -> Option<u64> {
        let of = DiffOf::Pr(glanced(route)?.clone());
        self.diffs.get(&of).map(DiffState::version)
    }

    /// How a page's fetches are going (the ones that have started).
    pub(crate) fn fetches(&self, route: &Route) -> impl Iterator<Item = Remote<()>> {
        (self.needs(route).into_iter()).filter_map(|key| self.data.get(&key).map(Remote::status))
    }

    /// Why the page's needs that show a copy couldn't refresh it, each
    /// reason once (one outage fails them all alike).
    fn stale_errors(&self, route: &Route) -> Option<String> {
        let fetches = self.fetches(route).filter(|r| r.data.is_some());
        let errors = BTreeSet::from_iter(fetches.filter_map(|r| r.error));
        (!errors.is_empty()).then(|| Vec::from_iter(errors).join("; "))
    }

    /// When the oldest cached copy on the page was fetched, while one is
    /// shown in place of fresh data.
    pub fn page_cached_at(&self, route: &Route) -> Option<u64> {
        self.fetches(route).filter_map(|r| r.cached_at).min()
    }

    /// One row per list item.
    pub fn compact(&self) -> bool {
        match self.density {
            crate::config::Density::Compact => true,
            crate::config::Density::Comfortable => false,
            crate::config::Density::Auto => self.size.1 < 30,
        }
    }

    pub fn page_loading(&self, route: &Route) -> bool {
        self.fetches(route).any(|r| r.loading)
    }

    /// The first key that runs `action` here, as people write it.
    pub fn first_key(&self, action: Action) -> String {
        self.key_here(action)
            .unwrap_or_else(|| format!(":{}", action.name().replace('_', " ")))
    }

    /// Builds the page for `route`, `width` columns wide in all.
    pub fn build_page(&self, route: &Route, width: u16, now: u64) -> Page {
        let icons = self.icons;
        let (comment, find_file, filter) = (
            self.first_key(Action::Comment),
            self.first_key(Action::FindFile),
            self.first_key(Action::Search),
        );
        let archive = self.first_key(Action::ArchiveItem);
        let keys = Keys {
            comment: &comment,
            find_file: &find_file,
            filter: &filter,
            archive: &archive,
        };
        let cx = pages::PageCtx { icons, keys, now };
        // Reading stays narrow enough to follow, lists get room for their
        // columns, and output and tables take the whole width.
        let (reading, lists) = (width.min(READING), width.min(LISTS));
        let mut page = match width_of(route) {
            Width::Reading => Page::new(reading),
            Width::Lists => Page::new(lists),
            Width::Full => Page::new(width),
            Width::ReadingAside => Page::with_aside(reading),
            Width::ListsAside => Page::with_aside(lists),
        };
        page.compact = self.compact();
        if let Some(err) = self.stale_errors(route) {
            // A flash banner on top; what's below is the cached copy.
            let when = self
                .page_cached_at(route)
                .map_or_else(|| "before".to_owned(), |at| crate::ui::time::ago(at, now));
            let why = crate::offline::banner(self, &when).unwrap_or_else(|| {
                format!("Couldn't refresh: {err}. Showing what was loaded {when}.")
            });
            pages::flash(&mut page, &why);
            page.blank();
        }
        // Where the page was read: a clone at a commit, or GitHub's API.
        if let Some(from) = crate::source::shown(route).and_then(|k| self.sources.get(&k)) {
            let note = vec![Seg::new(from.note(now), Role::Meta)];
            page.wrapped(note, 0, crate::ui::page::Frame::None);
        }
        // Writes made here, waiting for GitHub.
        crate::offline::queued_here(self, route, &mut page, now);
        let banner = page.lines.len();
        let retry = self.first_key(Action::Refresh);
        let f = Needs {
            state: self,
            retry: &retry,
        };
        let title = route.title();
        let b = Build {
            state: self,
            cx,
            retry: &retry,
        };
        match route {
            Route::App(p) => p.build(b, &mut page),
            Route::Code(p) => p.build(b, &mut page),
            Route::Wiki(p) => p.build(b, &mut page),
            Route::Discussions(p) => p.build(b, &mut page),
            Route::Gists(p) => p.build(b, &mut page),
            Route::People(p) => p.build(b, &mut page),
            Route::Releases(p) => p.build(b, &mut page),
            Route::Runs(p) => p.build(b, &mut page),
            Route::Issues(p) => p.build(b, &mut page),
            Route::Pulls(p) => p.build(b, &mut page),
            Route::Search { kind, query } => {
                let results = f.paged(route);
                pages::search(&mut page, *kind, query, results, icons, keys, now);
            }
            Route::Advisories(repo) => {
                if let Some(repo) = repo {
                    pages::security_nav(&mut page, repo, None);
                }
                pages::advisories(&mut page, repo.as_ref(), f.paged(route), now);
            }
            Route::Alerts { repo, kind, query } => {
                pages::security_nav(&mut page, repo, Some(*kind));
                pages::alerts(&mut page, repo, *kind, query, f.paged(route), cx);
            }
            Route::Alert { repo, kind, number } => {
                let key = DataKey::Alert(AlertRef {
                    repo: repo.clone(),
                    kind: *kind,
                    number: *number,
                });
                if let Some(a) = f.get(&key).show(&mut page, &title) {
                    pages::alert(&mut page, repo, a, cx);
                }
            }
            Route::Settings { repo, page: at } => {
                let key = DataKey::Settings(repo.clone(), *at);
                crate::settings::build(self, &mut page, repo, *at, f.get(&key), now);
            }
            Route::Advisory { repo, ghsa } => {
                let key = DataKey::Advisory(repo.clone(), ghsa.clone());
                if let Some(a) = f.get(&key).show(&mut page, ghsa) {
                    pages::advisory(&mut page, a, now);
                }
            }
            Route::Notifications { query } => {
                pages::notifications(&mut page, query, f.paged(route), cx);
            }
            Route::Projects(of) => pages::projects(&mut page, of, f.paged(route), now),
            Route::Project { project, view } => {
                if let Some(p) = f.paged(route).show(&mut page, &title) {
                    pages::project_view(&mut page, project, *view, p, cx);
                }
            }
            Route::ProjectItem {
                project,
                view,
                item,
            } => {
                if let Some(p) = f.paged::<Project>(route).show(&mut page, &title) {
                    pages::project_item(&mut page, project, *view, p, *item, cx);
                }
            }
            Route::Site(p) => p.build(b, &mut page),
            Route::Housekeeping(p) => p.build(b, &mut page),
        }
        // The sidebar starts beside the page, below the banner.
        if let Some(a) = page.aside.as_mut().filter(|a| !a.lines.is_empty()) {
            a.lines.splice(0..0, vec![PageLine::default(); banner]);
        }
        page
    }
}

/// What building a page reads, recorded in tests to check a page reads
/// only what it needs.
#[cfg(test)]
pub(crate) mod reads {
    use std::cell::RefCell;

    use crate::data::DataKey;

    thread_local! {
        static READS: RefCell<Option<Vec<DataKey>>> = const { RefCell::new(None) };
    }

    pub(crate) fn record(key: &DataKey) {
        READS.with_borrow_mut(|reads| {
            if let Some(reads) = reads {
                reads.push(key.clone());
            }
        });
    }

    /// What `f` reads.
    pub(crate) fn during<R>(f: impl FnOnce() -> R) -> (R, Vec<DataKey>) {
        READS.set(Some(Vec::new()));
        let out = f();
        (out, READS.take().unwrap_or_default())
    }
}
