//! Browsing: the data behind each page, and building pages from it.

use std::sync::Arc;

use std::collections::HashMap;

use ghtui_api::browse::{
    Blob, CommitInfo, IssueDetail, PrActivity, Profile, Refs, RepoOverview, RepoSummary, Results,
    SearchKind, SearchResults, TreeEntry,
};
use ghtui_api::model::{PrRef, RepoId};
use ghtui_ui::page::{Page, Role, Seg};
use ghtui_ui::pages::{self, Keys, PrTab};

use crate::keymap::Action;
use crate::route::Route;
use crate::state::State;

/// Widest a page gets; wider terminals center it, as GitHub does.
pub const MAX_WIDTH: u16 = 140;

/// One piece of GitHub data a page shows.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DataKey {
    Repo(RepoId),
    Tree(RepoId, String, String),
    Blob(RepoId, String, String),
    Search(SearchKind, String),
    Issue(RepoId, u64),
    PrActivity(PrRef),
    Profile(String),
    ViewerRepos,
    /// Every file path at a revision ("Go to file").
    Files(RepoId, String),
    /// Branches and tags.
    Refs(RepoId),
    /// The latest commit of each entry in a directory.
    LastCommits(RepoId, String, String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Data {
    Repo(Box<RepoOverview>),
    Tree(Vec<TreeEntry>),
    Blob(Box<Blob>),
    Search(Box<SearchResults>),
    /// `None`: the number is a pull request.
    Issue(Option<Box<IssueDetail>>),
    PrActivity(Box<PrActivity>),
    Profile(Box<Profile>),
    Repos(Vec<RepoSummary>),
    /// Paths, and whether GitHub cut the list short.
    Files(Arc<Vec<String>>, bool),
    Refs(Box<Refs>),
    LastCommits(Arc<HashMap<String, CommitInfo>>),
}

/// What a page needs fetched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Need {
    Inbox,
    Pr(PrRef),
    Data(DataKey),
}

pub fn needs(route: &Route) -> Vec<Need> {
    use DataKey as K;
    let header = |repo: &RepoId| Need::Data(K::Repo(repo.clone()));
    match route {
        Route::Home => vec![Need::Inbox, Need::Data(K::ViewerRepos)],
        Route::Repo(repo) => vec![
            header(repo),
            Need::Data(K::LastCommits(repo.clone(), "HEAD".into(), String::new())),
        ],
        Route::Tree { repo, rev, path } => vec![
            header(repo),
            Need::Data(K::Tree(repo.clone(), rev.clone(), path.clone())),
            Need::Data(K::LastCommits(repo.clone(), rev.clone(), path.clone())),
        ],
        Route::Blob { repo, rev, path } => vec![
            header(repo),
            Need::Data(K::Blob(repo.clone(), rev.clone(), path.clone())),
        ],
        Route::Issues { repo, .. } | Route::Pulls { repo, .. } => {
            let (kind, query) = route.search().expect("lists are searches");
            vec![header(repo), Need::Data(K::Search(kind, query))]
        }
        Route::Search { .. } => {
            let (kind, query) = route.search().expect("a search");
            vec![Need::Data(K::Search(kind, query))]
        }
        Route::Issue { repo, number } => {
            vec![header(repo), Need::Data(K::Issue(repo.clone(), *number))]
        }
        Route::Pr { pr, .. } => vec![Need::Pr(pr.clone()), Need::Data(K::PrActivity(pr.clone()))],
        Route::User { login, .. } => vec![Need::Data(K::Profile(login.to_lowercase()))],
    }
}

/// Whether `need` is a repository's header on one of its other pages
/// (fetched only when missing).
pub fn is_header(route: &Route, need: &Need) -> bool {
    matches!(need, Need::Data(DataKey::Repo(_))) && !matches!(route, Route::Repo(_))
}

/// The screen of one page: where it is, what's selected, and the page as
/// last built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageScreen {
    pub route: Route,
    /// The top row shown.
    pub scroll: usize,
    /// The selected item, if one is.
    pub selected: Option<usize>,
    pub page: Arc<Page>,
    /// The data generation and width `page` was built for.
    pub built: Option<(u64, u16)>,
    /// Nothing has been selected or scrolled yet: select the first visible
    /// item once the page has items.
    pub fresh: bool,
}

impl PageScreen {
    pub fn new(route: Route) -> Self {
        // Lists start on their first row; reading pages (issues, pull
        // requests, files) start with nothing selected.
        let list = !matches!(
            route,
            Route::Issue { .. }
                | Route::Blob { .. }
                | Route::Pr {
                    tab: PrTab::Conversation,
                    ..
                }
        );
        Self {
            route,
            scroll: 0,
            selected: None,
            page: Arc::new(Page::default()),
            built: None,
            fresh: list,
        }
    }

    /// The selected item's link.
    pub fn selected_url(&self) -> Option<&str> {
        let item = self.page.items.get(self.selected?)?;
        self.page.links.get(item.link as usize).map(String::as_str)
    }
}

impl State {
    pub fn get(&self, key: &DataKey) -> Option<&Data> {
        self.data.get(key).and_then(|r| r.data.as_ref())
    }

    pub fn overview(&self, repo: &RepoId) -> Option<&RepoOverview> {
        match self.get(&DataKey::Repo(repo.clone()))? {
            Data::Repo(o) => Some(o),
            _ => None,
        }
    }

    pub fn search_results(&self, route: &Route) -> Option<&SearchResults> {
        let (kind, query) = route.search()?;
        match self.get(&DataKey::Search(kind, query))? {
            Data::Search(r) => Some(r),
            _ => None,
        }
    }

    pub fn activity(&self, pr: &PrRef) -> Option<&PrActivity> {
        match self.get(&DataKey::PrActivity(pr.clone()))? {
            Data::PrActivity(a) => Some(a),
            _ => None,
        }
    }

    /// The latest commit of each entry in a directory, once loaded.
    pub fn last_commits(
        &self,
        repo: &RepoId,
        rev: &str,
        path: &str,
    ) -> Option<&HashMap<String, CommitInfo>> {
        match self.get(&DataKey::LastCommits(
            repo.clone(),
            rev.to_owned(),
            path.to_owned(),
        ))? {
            Data::LastCommits(m) => Some(m),
            _ => None,
        }
    }

    pub fn profile(&self, login: &str) -> Option<&Profile> {
        match self.get(&DataKey::Profile(login.to_lowercase()))? {
            Data::Profile(p) => Some(p),
            _ => None,
        }
    }

    /// The first refresh error among a page's needs, and whether the page
    /// has data despite it.
    fn page_error(&self, route: &Route) -> Option<(String, bool)> {
        needs(route).into_iter().find_map(|need| match need {
            Need::Inbox => self
                .inbox
                .error
                .clone()
                .map(|e| (e, self.inbox.data.is_some())),
            Need::Pr(pr) => {
                let r = self.prs.get(&pr)?;
                r.error.clone().map(|e| (e, r.data.is_some()))
            }
            Need::Data(key) => {
                let r = self.data.get(&key)?;
                r.error.clone().map(|e| (e, r.data.is_some()))
            }
        })
    }

    pub fn page_loading(&self, route: &Route) -> bool {
        needs(route).into_iter().any(|need| match need {
            Need::Inbox => self.inbox.loading,
            Need::Pr(pr) => self.prs.get(&pr).is_some_and(|r| r.loading),
            Need::Data(key) => self.data.get(&key).is_some_and(|r| r.loading),
        })
    }

    /// The first key that runs `action` here, as people write it.
    pub fn first_key(&self, action: Action) -> String {
        self.key_here(action)
            .unwrap_or_else(|| format!(":{}", action.name().replace('_', " ")))
    }

    /// Builds the page for `route`, `width` columns wide in all.
    pub fn build_page(&self, route: &Route, width: u16, now: u64) -> Page {
        let icons = self.icons;
        let error = self.page_error(route);
        let (comment, find_file, filter) = (
            self.first_key(Action::Comment),
            self.first_key(Action::FindFile),
            self.first_key(Action::Search),
        );
        let keys = Keys {
            comment: &comment,
            find_file: &find_file,
            filter: &filter,
        };
        let aside = match route {
            Route::Repo(_)
            | Route::Issue { .. }
            | Route::Pr {
                tab: PrTab::Conversation,
                ..
            } => pages::aside_width(width),
            _ => None,
        };
        let mut page = Page::new(pages::main_width(width, aside));
        let retry = self.first_key(Action::Refresh);
        let missing = |page: &mut Page, what: &str| match &error {
            Some((err, false)) => {
                pages::flash(
                    page,
                    &format!("Couldn't load {what}: {err}. {retry} tries again."),
                );
            }
            _ => pages::loading_box(page, vec![Seg::new("Loading…", Role::Meta)]),
        };
        match route {
            Route::Home => {
                let repos = match self.get(&DataKey::ViewerRepos) {
                    Some(Data::Repos(r)) => Some(r.as_slice()),
                    _ => None,
                };
                // A failed first load (nothing cached) says why, instead of
                // empty boxes that look like they're still loading.
                if matches!(error, Some((_, false))) {
                    missing(&mut page, "your home page");
                    if self.inbox.data.is_none() {
                        return page;
                    }
                }
                pages::home(
                    &mut page,
                    self.inbox.data.as_ref(),
                    repos,
                    self.viewer.as_deref(),
                    icons,
                    now,
                );
            }
            Route::Repo(repo) => {
                let overview = self.overview(repo);
                pages::repo_title(&mut page, repo, overview);
                match overview {
                    Some(o) => {
                        let commits = self.last_commits(repo, "HEAD", "");
                        pages::repo_code(&mut page, repo, o, commits, icons, keys, aside, now)
                    }
                    None => missing(&mut page, "the repository"),
                }
            }
            Route::Tree { repo, rev, path } => {
                let key = DataKey::Tree(repo.clone(), rev.clone(), path.clone());
                let entries = match self.get(&key) {
                    Some(Data::Tree(entries)) => Some(entries.as_slice()),
                    _ => None,
                };
                let commits = self.last_commits(repo, rev, path);
                pages::repo_dir(
                    &mut page, repo, rev, path, entries, commits, icons, keys, now,
                );
                if entries.is_none() && matches!(error, Some((_, false))) {
                    missing(&mut page, "the directory");
                }
            }
            Route::Blob { repo, rev, path } => {
                let key = DataKey::Blob(repo.clone(), rev.clone(), path.clone());
                match self.get(&key) {
                    Some(Data::Blob(blob)) => pages::file(&mut page, repo, rev, path, blob, keys),
                    _ => missing(&mut page, "the file"),
                }
            }
            Route::Issues { repo, query } | Route::Pulls { repo, query } => {
                let is_pr = matches!(route, Route::Pulls { .. });
                let counts = self.overview(repo).map(|o| {
                    if is_pr {
                        (o.open_prs, o.closed_prs)
                    } else {
                        (o.open_issues, o.closed_issues)
                    }
                });
                let results = match self.search_results(route) {
                    Some(SearchResults::Issues(r)) => Some(r),
                    _ => None,
                };
                if results.is_none() && matches!(error, Some((_, false))) {
                    missing(&mut page, "the list");
                } else {
                    pages::issue_list(&mut page, query, counts, is_pr, results, icons, keys, now);
                }
            }
            Route::Search { kind, query } => {
                let results = self.search_results(route);
                if results.is_none() && matches!(error, Some((_, false))) {
                    missing(&mut page, "results");
                } else {
                    pages::search(&mut page, *kind, query, results, icons, keys, now);
                }
            }
            Route::Issue { repo, number } => match self.get(&DataKey::Issue(repo.clone(), *number))
            {
                Some(Data::Issue(Some(issue))) => {
                    pages::issue(&mut page, issue, icons, keys, aside, now)
                }
                // Redirecting to the pull request.
                Some(Data::Issue(None)) => page.line(vec![Seg::new("Loading…", Role::Meta)]),
                _ => missing(&mut page, &format!("{repo}#{number}")),
            },
            Route::Pr { pr, tab } => {
                let detail = self.prs.get(pr).and_then(|r| r.data.as_ref());
                let activity = self.activity(pr);
                match (detail, tab) {
                    (Some(d), PrTab::Conversation) => {
                        pages::pr_conversation(&mut page, pr, d, activity, icons, keys, aside, now)
                    }
                    (Some(d), PrTab::Commits) => pages::pr_commits(&mut page, pr, d, activity, now),
                    (None, _) => missing(&mut page, &pr.to_string()),
                }
            }
            Route::User { login, tab } => match self.profile(login) {
                Some(p) => pages::profile(&mut page, p, *tab, now),
                _ => missing(&mut page, &format!("@{login}")),
            },
        }
        if let Some((err, true)) = error {
            // A flash banner on top; what's below is the cached copy.
            let mut banner = Page::new(page.width);
            pages::flash(
                &mut banner,
                &format!("Couldn't refresh: {err}. Showing what was loaded before."),
            );
            banner.blank();
            let n = banner.lines.len();
            page.lines.splice(0..0, banner.lines);
            for item in &mut page.items {
                item.start += n;
                item.end += n;
            }
        }
        page
    }
}

/// Appends a page of results to what's shown.
pub fn append(results: &mut SearchResults, more: SearchResults) {
    fn extend<T>(a: &mut Results<T>, b: Results<T>) {
        a.items.extend(b.items);
        a.next = b.next;
        a.total = b.total;
    }
    match (results, more) {
        (SearchResults::Repos(a), SearchResults::Repos(b)) => extend(a, b),
        (SearchResults::Issues(a), SearchResults::Issues(b)) => extend(a, b),
        (SearchResults::Users(a), SearchResults::Users(b)) => extend(a, b),
        _ => {}
    }
}

pub fn next_cursor(results: &SearchResults) -> Option<&str> {
    match results {
        SearchResults::Repos(r) => r.next.as_deref(),
        SearchResults::Issues(r) => r.next.as_deref(),
        SearchResults::Users(r) => r.next.as_deref(),
    }
}
