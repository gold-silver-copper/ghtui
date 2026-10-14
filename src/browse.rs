//! Building pages: the frame every page has (its width, a banner when
//! what's shown is a cached copy, where it was read), around what its
//! family builds ([`crate::route::Family`]).

use std::collections::BTreeSet;

use crate::api::browse::{CommitInfo, PrActivity, RepoOverview};
use crate::api::model::{PrDetail, PrRef, RepoId};
use crate::ui::Fetched;
use crate::ui::page::{Page, PageLine, Role, Seg};
use crate::ui::pages::{self, Keys};

use crate::data::{Data, DataKey, Picked};
use crate::diff_screen::{DiffOf, DiffState};
use crate::keymap::Action;
use crate::route::{Route, code};
use crate::state::{Remote, State};

/// Widest a page of reading (a conversation, a Markdown file) gets, its
/// sidebar included; wider terminals center it, as GitHub does.
const READING: u16 = 160;
/// Widest a page of lists gets: room for their columns, without a row's
/// two ends drifting apart.
const LISTS: u16 = 200;

/// The pull request whose review a page shows at a glance: its Review
/// tab's.
pub(crate) fn glanced(route: &Route) -> Option<&PrRef> {
    match route {
        Route::Pulls(p) => p.glanced(),
        _ => None,
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

    /// `key`'s data once it's in, or why it couldn't be read: `None`
    /// while it's on its way (or wasn't asked for).
    pub fn answer<T: Picked + ?Sized>(&self, key: &DataKey) -> Option<Result<&T, &str>> {
        let remote = self.remote(key)?;
        match (remote.data.as_ref().and_then(T::pick), &remote.error) {
            (Some(data), _) => Some(Ok(data)),
            (None, Some(why)) if !remote.loading => Some(Err(why)),
            _ => None,
        }
    }

    /// The clone ghtui runs in, if it's `repo`'s, once git has said
    /// ([`DataKey::Clone`]): `Some(None)` when it isn't, or git couldn't
    /// tell.
    pub fn clone_of(&self, repo: &RepoId) -> Option<Option<&crate::git::local::Clone>> {
        let found =
            self.answer::<Option<Box<crate::git::local::Clone>>>(&DataKey::Clone(repo.clone()));
        found.map(|c| c.ok().and_then(|c| c.as_deref()))
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

    /// Which versions of GitHub's data a page is built from: its needs',
    /// and what else it reads. One not asked for, or gone, is 0.
    pub(crate) fn data_version(&self, route: &Route) -> u64 {
        let version = |remote: Option<&Remote<Data>>| remote.map_or(0, |r| r.version);
        let needs = (self.needs(route).iter()).fold(17, |h: u64, key| {
            h.wrapping_mul(31).wrapping_add(version(self.data.get(key)))
        });
        // Read in no order: versions are never repeated, so a sum changes
        // when any of them does, or one goes.
        let family = route.family();
        let also = (self.data.iter())
            .filter(|(key, _)| family.also_reads(key))
            .fold(0, |sum: u64, (_, r)| sum.wrapping_add(r.version));
        needs.wrapping_mul(31).wrapping_add(also)
    }

    /// How a page's fetches are going (the ones that have started).
    pub(crate) fn fetches(&self, route: &Route) -> impl Iterator<Item = &Remote<Data>> {
        (self.needs(route).into_iter()).filter_map(|key| self.data.get(&key))
    }

    /// Why the page's needs that show a copy couldn't refresh it, each
    /// reason once (one outage fails them all alike).
    fn stale_errors(&self, route: &Route) -> Option<String> {
        let fetches = self.fetches(route).filter(|r| r.data.is_some());
        let errors = BTreeSet::from_iter(fetches.filter_map(|r| r.error.as_deref()));
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
        let mut page = match route.family().width() {
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
        let source = route.family().shown_source();
        if let Some(from) = source.and_then(|k| self.remote(&k)?.source.as_ref()) {
            let note = vec![Seg::new(from.note(now), Role::Meta)];
            page.wrapped(note, 0, crate::ui::page::Frame::None);
        }
        // Writes made here, waiting for GitHub.
        crate::offline::queued_here(self, route, &mut page, now);
        let banner = page.lines.len();
        let retry = self.first_key(Action::Refresh);
        let b = Build {
            state: self,
            cx,
            retry: &retry,
        };
        route.family().build(b, &mut page);
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
