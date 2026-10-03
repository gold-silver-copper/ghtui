//! Browsing: the data behind each page, and building pages from it.

use std::sync::Arc;

use ghtui_api::browse::{
    Blob, IssueDetail, PrActivity, Profile, RepoOverview, RepoSummary, Results, SearchKind,
    SearchResults, TreeEntry,
};
use ghtui_api::model::{PrRef, RepoId};
use ghtui_ui::page::{Page, PageLine, Role, Seg};
use ghtui_ui::pages::{self, PrTab, RepoTab};

use crate::keymap::Action;
use crate::route::{OPEN, Route, Target};
use crate::state::{Remote, State};

/// Widest a page gets; wider terminals center it, as GitHub does.
pub const MAX_WIDTH: u16 = 120;

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
        Route::Repo(repo) => vec![header(repo)],
        Route::Tree { repo, rev, path } => vec![
            header(repo),
            Need::Data(K::Tree(repo.clone(), rev.clone(), path.clone())),
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
        Route::Issue { repo, number } => vec![Need::Data(K::Issue(repo.clone(), *number))],
        Route::Pr { pr, .. } => vec![Need::Pr(pr.clone()), Need::Data(K::PrActivity(pr.clone()))],
        Route::User(login) => vec![Need::Data(K::Profile(login.to_lowercase()))],
    }
}

/// The screen of one page: where it is, and the page as last built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageScreen {
    pub route: Route,
    pub cursor: usize,
    pub scroll: usize,
    pub page: Arc<Page>,
    /// The data generation and width `page` was built for.
    pub built: Option<(u64, u16)>,
}

impl PageScreen {
    pub fn new(route: Route) -> Self {
        Self {
            route,
            cursor: 0,
            scroll: 0,
            page: Arc::new(Page::default()),
            built: None,
        }
    }

    /// Distinct links on the cursor line, or failing that on the lines above
    /// it in the same block (a card's title links the whole card).
    pub fn links_at_cursor(&self) -> Vec<(String, String)> {
        let lines = &self.page.lines;
        let mut at = self.cursor.min(lines.len().saturating_sub(1));
        for _ in 0..4 {
            let Some(line) = lines.get(at) else {
                return Vec::new();
            };
            if line.is_blank() {
                return Vec::new();
            }
            let links = line_links(&self.page, line);
            if !links.is_empty() {
                return links;
            }
            let Some(prev) = at.checked_sub(1) else {
                return Vec::new();
            };
            at = prev;
        }
        Vec::new()
    }
}

fn line_links(page: &Page, line: &PageLine) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for seg in line.segs.iter().chain(&line.right) {
        let Some(link) = seg.link else { continue };
        let url = page.links[link as usize].clone();
        match out.iter_mut().find(|(u, _)| *u == url) {
            Some((_, text)) => text.push_str(&seg.text),
            None => out.push((url, seg.text.trim().to_owned())),
        }
    }
    out
}

impl State {
    pub fn remote(&self, key: &DataKey) -> Option<&Remote<Data>> {
        self.data.get(key)
    }

    fn get(&self, key: &DataKey) -> Option<&Data> {
        self.data.get(key).and_then(|r| r.data.as_ref())
    }

    pub fn overview(&self, repo: &RepoId) -> Option<&RepoOverview> {
        match self.get(&DataKey::Repo(repo.clone()))? {
            Data::Repo(o) => Some(o),
            _ => None,
        }
    }

    fn search_results(&self, route: &Route) -> Option<&SearchResults> {
        let (kind, query) = route.search()?;
        match self.get(&DataKey::Search(kind, query))? {
            Data::Search(r) => Some(r),
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

    pub fn first_key(&self, action: Action) -> String {
        self.keymap
            .keys_for(action)
            .into_iter()
            .next()
            .unwrap_or_else(|| format!(":{}", action.name()))
    }

    /// Builds the page for `route`.
    pub fn build_page(&self, route: &Route, width: u16, now: u64) -> Page {
        let mut page = Page::new(width);
        let icons = self.icons;
        let error = self.page_error(route);
        let comment = self.first_key(Action::Comment);
        let filter = self.first_key(Action::Filter);
        let missing = |page: &mut Page, what: &str| match &error {
            Some((err, false)) => {
                page.line(vec![Seg::new(
                    format!("Couldn't load {what}: {err}"),
                    Role::Error,
                )]);
                page.line(vec![Seg::new("Press r to retry.", Role::Meta)]);
            }
            _ => page.line(vec![Seg::new("Loading…", Role::Meta)]),
        };
        match route {
            Route::Home => {
                let repos = match self.get(&DataKey::ViewerRepos) {
                    Some(Data::Repos(r)) => Some(r.as_slice()),
                    _ => None,
                };
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
                pages::repo_header(&mut page, repo, overview, RepoTab::Code);
                match overview {
                    Some(o) => pages::repo_code(&mut page, repo, o, now),
                    None => missing(&mut page, "the repository"),
                }
            }
            Route::Tree { repo, rev, path } => {
                pages::repo_header(&mut page, repo, self.overview(repo), RepoTab::Code);
                let key = DataKey::Tree(repo.clone(), rev.clone(), path.clone());
                match self.get(&key) {
                    Some(Data::Tree(entries)) => {
                        pages::repo_dir(&mut page, repo, rev, path, Some(entries))
                    }
                    _ => {
                        pages::repo_dir(&mut page, repo, rev, path, Some(&[]));
                        missing(&mut page, "the directory");
                    }
                }
            }
            Route::Blob { repo, rev, path } => {
                pages::repo_header(&mut page, repo, self.overview(repo), RepoTab::Code);
                let key = DataKey::Blob(repo.clone(), rev.clone(), path.clone());
                match self.get(&key) {
                    Some(Data::Blob(blob)) => pages::file(&mut page, repo, rev, path, blob),
                    _ => missing(&mut page, "the file"),
                }
            }
            Route::Issues { repo, query } | Route::Pulls { repo, query } => {
                let tab = if matches!(route, Route::Issues { .. }) {
                    RepoTab::Issues
                } else {
                    RepoTab::Pulls
                };
                pages::repo_header(&mut page, repo, self.overview(repo), tab);
                let results = match self.search_results(route) {
                    Some(SearchResults::Issues(r)) => Some(r),
                    _ => None,
                };
                if results.is_none() && matches!(error, Some((_, false))) {
                    missing(&mut page, "the list");
                } else {
                    pages::issue_list(&mut page, query, results, false, &filter, icons, now);
                }
            }
            Route::Search { kind, query } => {
                let results = self.search_results(route);
                if results.is_none() && matches!(error, Some((_, false))) {
                    pages::search(&mut page, *kind, query, None, icons, now);
                    page.lines.pop();
                    missing(&mut page, "results");
                } else {
                    pages::search(&mut page, *kind, query, results, icons, now);
                }
            }
            Route::Issue { repo, number } => {
                match self.get(&DataKey::Issue(repo.clone(), *number)) {
                    Some(Data::Issue(Some(issue))) => pages::issue(&mut page, issue, &comment, now),
                    // Redirecting to the pull request.
                    Some(Data::Issue(None)) => page.line(vec![Seg::new("Loading…", Role::Meta)]),
                    _ => missing(&mut page, &format!("{repo}#{number}")),
                }
            }
            Route::Pr { pr, tab } => {
                let detail = self.prs.get(pr).and_then(|r| r.data.as_ref());
                let activity = match self.get(&DataKey::PrActivity(pr.clone())) {
                    Some(Data::PrActivity(a)) => Some(&**a),
                    _ => None,
                };
                match (detail, tab) {
                    (Some(d), PrTab::Conversation) => {
                        pages::pr_conversation(&mut page, pr, d, activity, &comment, now)
                    }
                    (Some(d), PrTab::Commits) => pages::pr_commits(&mut page, pr, d, activity, now),
                    (None, _) => missing(&mut page, &pr.to_string()),
                }
            }
            Route::User(login) => match self.get(&DataKey::Profile(login.to_lowercase())) {
                Some(Data::Profile(p)) => pages::profile(&mut page, p, now),
                _ => missing(&mut page, &format!("@{login}")),
            },
        }
        if let Some((err, true)) = error {
            page.lines.insert(
                0,
                PageLine {
                    segs: vec![Seg::new(format!("Couldn't refresh: {err}"), Role::Error)],
                    ..PageLine::default()
                },
            );
        }
        page
    }

    /// Where the page's numbered tab `n` (1-based) leads.
    pub fn tab_target(&self, route: &Route, n: usize) -> Option<Target> {
        let page = |route| Some(Target::Page(route));
        match route {
            Route::Repo(repo)
            | Route::Tree { repo, .. }
            | Route::Blob { repo, .. }
            | Route::Issues { repo, .. }
            | Route::Pulls { repo, .. } => {
                let has_issues = self.overview(repo).is_none_or(|o| o.has_issues);
                let issues = Route::Issues {
                    repo: repo.clone(),
                    query: OPEN.into(),
                };
                let pulls = Route::Pulls {
                    repo: repo.clone(),
                    query: OPEN.into(),
                };
                let tabs: Vec<Route> = if has_issues {
                    vec![Route::Repo(repo.clone()), issues, pulls]
                } else {
                    vec![Route::Repo(repo.clone()), pulls]
                };
                tabs.into_iter().nth(n.checked_sub(1)?).map(Target::Page)
            }
            Route::Pr { pr, .. } => match n {
                1 => page(Route::Pr {
                    pr: pr.clone(),
                    tab: PrTab::Conversation,
                }),
                2 => page(Route::Pr {
                    pr: pr.clone(),
                    tab: PrTab::Commits,
                }),
                3 => Some(Target::Files(pr.clone())),
                _ => None,
            },
            Route::Search { query, .. } => {
                let kind = [SearchKind::Repos, SearchKind::Issues, SearchKind::Users]
                    .get(n.checked_sub(1)?)?;
                page(Route::Search {
                    kind: *kind,
                    query: query.clone(),
                })
            }
            Route::Home | Route::Issue { .. } | Route::User(_) => None,
        }
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
