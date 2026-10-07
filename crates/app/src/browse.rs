//! Browsing: the data behind each page, and building pages from it.

use std::sync::Arc;

use std::collections::HashMap;

use ghtui_api::browse::{
    Blame, Blob, BranchInfo, Checks, CommitDetail, CommitInfo, Comparison, DeploymentList,
    DiscussionDetail, DiscussionList, DiscussionsOf, IssueDetail, Job, MilestoneDetail,
    MilestoneList, PrActivity, Profile, Refs, Release, RepoOverview, RepoSort, RepoSummary,
    Results, RunSummary, SearchKind, SearchResults, TagInfo, TreeEntry, UserList, UserSummary,
    Workflow, WorkflowRun,
};
use ghtui_api::model::{PrRef, RepoId};
use ghtui_ui::page::{Link, Page, Role, Seg};
use ghtui_ui::pages::{self, Keys, PrTab, ProfileList, ProfileTab};

use crate::keymap::Action;
use crate::route::Route;
use crate::state::{Remote, State};

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
    /// A commit, by the revision linked.
    Commit(RepoId, String),
    /// A revision's commits (of a path, if it isn't empty).
    History(RepoId, String, String),
    /// The checks on a pull request's head.
    PrChecks(PrRef),
    /// The checks on a repository's default branch.
    BranchChecks(RepoId),
    Users(UserList),
    Forks(RepoId),
    Releases(RepoId),
    Release(RepoId, String),
    Tags(RepoId),
    Branches(RepoId),
    Blame(RepoId, String, String),
    Compare(RepoId, String),
    Deployments(RepoId, Option<String>),
    Milestones(RepoId, bool),
    Milestone(RepoId, u64),
    Discussions(DiscussionsOf, Option<String>),
    Discussion(DiscussionsOf, u64),
    /// A workflow run (an attempt of it, or the latest).
    Run(RepoId, u64, Option<u64>),
    Job(RepoId, u64),
    /// A job's log (not cached: it's big).
    JobLog(RepoId, u64),
    Workflow(RepoId, String),
    /// A workflow's runs.
    WorkflowRuns(RepoId, String),
    CommitChecks(RepoId, String),
    /// Someone's own repositories, in an order.
    OwnerRepos(String, RepoSort),
    /// What someone starred.
    Stars(String),
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
    Commit(Box<CommitDetail>),
    History(Box<Results<CommitInfo>>),
    Checks(Box<Checks>),
    Users(Box<Results<UserSummary>>),
    /// A page of a list of repositories.
    RepoPage(Box<Results<RepoSummary>>),
    Releases(Box<Results<Release>>),
    Discussions(Box<DiscussionList>),
    Discussion(Box<DiscussionDetail>),
    Run(Box<WorkflowRun>),
    Job(Box<Job>),
    Log(Arc<String>),
    Workflow(Box<Workflow>),
    Runs(Box<Results<RunSummary>>),
    Release(Box<Release>),
    Tags(Box<Results<TagInfo>>),
    Branches(Box<Results<BranchInfo>>),
    Blame(Box<Blame>),
    Compare(Box<Comparison>),
    Deployments(Box<DeploymentList>),
    Milestones(Box<MilestoneList>),
    Milestone(Box<MilestoneDetail>),
}

impl Data {
    /// Where a list's next page starts, if there's more.
    pub fn next_cursor(&self) -> Option<&str> {
        match self {
            Data::Search(r) => next_cursor(r),
            Data::History(r) => r.next.as_deref(),
            Data::Users(r) => r.next.as_deref(),
            Data::RepoPage(r) => r.next.as_deref(),
            Data::Releases(r) => r.next.as_deref(),
            Data::Runs(r) => r.next.as_deref(),
            Data::Discussions(d) => d.results.next.as_deref(),
            Data::Tags(r) => r.next.as_deref(),
            Data::Branches(r) => r.next.as_deref(),
            Data::Deployments(d) => d.results.next.as_deref(),
            Data::Milestones(m) => m.results.next.as_deref(),
            Data::Milestone(m) => m.items.next.as_deref(),
            _ => None,
        }
    }

    /// Appends a list's next page.
    pub fn append(&mut self, more: Data) {
        match (self, more) {
            (Data::Search(a), Data::Search(b)) => append(a, *b),
            (Data::History(a), Data::History(b)) => extend(a, *b),
            (Data::Users(a), Data::Users(b)) => extend(a, *b),
            (Data::RepoPage(a), Data::RepoPage(b)) => extend(a, *b),
            (Data::Releases(a), Data::Releases(b)) => extend(a, *b),
            (Data::Runs(a), Data::Runs(b)) => extend(a, *b),
            (Data::Discussions(a), Data::Discussions(b)) => extend(&mut a.results, b.results),
            (Data::Tags(a), Data::Tags(b)) => extend(a, *b),
            (Data::Branches(a), Data::Branches(b)) => extend(a, *b),
            (Data::Deployments(a), Data::Deployments(b)) => extend(&mut a.results, b.results),
            (Data::Milestones(a), Data::Milestones(b)) => extend(&mut a.results, b.results),
            (Data::Milestone(a), Data::Milestone(b)) => extend(&mut a.items, b.items),
            _ => {}
        }
    }
}

/// The list a page loads more of.
pub fn paged(route: &Route) -> Option<DataKey> {
    match route {
        Route::Commits { repo, rev, path } => {
            Some(DataKey::History(repo.clone(), rev.clone(), path.clone()))
        }
        Route::Stargazers(repo) => Some(DataKey::Users(UserList::Stargazers(repo.clone()))),
        Route::Watchers(repo) => Some(DataKey::Users(UserList::Watchers(repo.clone()))),
        Route::Forks(repo) => Some(DataKey::Forks(repo.clone())),
        Route::Releases(repo) => Some(DataKey::Releases(repo.clone())),
        Route::Discussions { of, category } => {
            Some(DataKey::Discussions(of.clone(), category.clone()))
        }
        Route::Workflow { repo, file } => Some(DataKey::WorkflowRuns(repo.clone(), file.clone())),
        Route::User { login, tab } => {
            let login = login.to_lowercase();
            Some(match tab {
                ProfileTab::Overview => return None,
                ProfileTab::Repositories(sort) => DataKey::OwnerRepos(login, *sort),
                ProfileTab::Stars => DataKey::Stars(login),
                ProfileTab::Followers => DataKey::Users(UserList::Followers(login)),
                ProfileTab::Following => DataKey::Users(UserList::Following(login)),
                ProfileTab::People => DataKey::Users(UserList::People(login)),
            })
        }
        Route::Tags(repo) => Some(DataKey::Tags(repo.clone())),
        Route::Branches(repo) => Some(DataKey::Branches(repo.clone())),
        Route::Compare { repo, spec } => Some(DataKey::Compare(repo.clone(), spec.clone())),
        Route::Deployments { repo, environment } => {
            Some(DataKey::Deployments(repo.clone(), environment.clone()))
        }
        Route::Milestones { repo, closed } => Some(DataKey::Milestones(repo.clone(), *closed)),
        Route::Milestone { repo, number } => Some(DataKey::Milestone(repo.clone(), *number)),
        _ => route
            .search()
            .map(|(kind, query)| DataKey::Search(kind, query)),
    }
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
    let list = route
        .search()
        .map(|(kind, query)| Need::Data(K::Search(kind, query)));
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
        Route::Blob {
            repo, rev, path, ..
        } => vec![
            header(repo),
            Need::Data(K::Blob(repo.clone(), rev.clone(), path.clone())),
        ],
        Route::Blame {
            repo, rev, path, ..
        } => vec![
            header(repo),
            Need::Data(K::Blob(repo.clone(), rev.clone(), path.clone())),
            Need::Data(K::Blame(repo.clone(), rev.clone(), path.clone())),
        ],
        Route::Issues { repo, .. } | Route::Pulls { repo, .. } => {
            std::iter::once(header(repo)).chain(list).collect()
        }
        Route::Search { .. } => list.into_iter().collect(),
        Route::Issue { repo, number } => {
            vec![header(repo), Need::Data(K::Issue(repo.clone(), *number))]
        }
        Route::Pr {
            pr,
            tab: PrTab::Checks,
        } => vec![
            Need::Pr(pr.clone()),
            Need::Data(K::PrActivity(pr.clone())),
            Need::Data(K::PrChecks(pr.clone())),
        ],
        Route::Pr { pr, .. } => vec![Need::Pr(pr.clone()), Need::Data(K::PrActivity(pr.clone()))],
        Route::Actions(repo) => vec![header(repo), Need::Data(K::BranchChecks(repo.clone()))],
        Route::Discussions { of, .. } | Route::Discussion { of, .. } => {
            let header = match of {
                DiscussionsOf::Repo(repo) => Some(header(repo)),
                DiscussionsOf::Org(_) => None,
            };
            let own = match route {
                Route::Discussion { number, .. } => Some(K::Discussion(of.clone(), *number)),
                _ => paged(route),
            };
            header.into_iter().chain(own.map(Need::Data)).collect()
        }
        Route::WorkflowRun { repo, run, attempt } => {
            vec![
                header(repo),
                Need::Data(K::Run(repo.clone(), *run, *attempt)),
            ]
        }
        Route::Job { repo, job, .. } => vec![
            header(repo),
            Need::Data(K::Job(repo.clone(), *job)),
            Need::Data(K::JobLog(repo.clone(), *job)),
        ],
        Route::Workflow { repo, file } => vec![
            header(repo),
            Need::Data(K::Workflow(repo.clone(), file.clone())),
            Need::Data(K::WorkflowRuns(repo.clone(), file.clone())),
        ],
        Route::CommitChecks { repo, oid } => vec![
            header(repo),
            Need::Data(K::Commit(repo.clone(), oid.clone())),
            Need::Data(K::CommitChecks(repo.clone(), oid.clone())),
        ],
        Route::Release { repo, tag } => {
            vec![
                header(repo),
                Need::Data(K::Release(repo.clone(), tag.clone())),
            ]
        }
        Route::Stargazers(repo)
        | Route::Watchers(repo)
        | Route::Forks(repo)
        | Route::Releases(repo)
        | Route::Tags(repo)
        | Route::Branches(repo)
        | Route::Compare { repo, .. }
        | Route::Deployments { repo, .. }
        | Route::Milestones { repo, .. }
        | Route::Milestone { repo, .. } => std::iter::once(header(repo))
            .chain(paged(route).map(Need::Data))
            .collect(),
        Route::Commit { repo, oid } => {
            vec![
                header(repo),
                Need::Data(K::Commit(repo.clone(), oid.clone())),
            ]
        }
        Route::Commits { repo, rev, path } => vec![
            header(repo),
            Need::Data(K::History(repo.clone(), rev.clone(), path.clone())),
        ],
        Route::User { login, .. } => std::iter::once(Need::Data(K::Profile(login.to_lowercase())))
            .chain(paged(route).map(Need::Data))
            .collect(),
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
    /// The data generation, width and density `page` was built for.
    pub built: Option<(u64, u16, bool)>,
    /// Nothing has been selected or scrolled yet: select the first visible
    /// item once the page has items.
    pub fresh: bool,
    /// The page has been scrolled to its [`Page::jump`] or anchor (once).
    pub jumped: bool,
    /// The `#fragment` of the link that opened it: a comment to scroll to.
    pub anchor: Option<String>,
}

impl PageScreen {
    pub fn new(route: Route) -> Self {
        // Lists start on their first row; reading pages (issues, pull
        // requests, files) start with nothing selected.
        let list = !matches!(
            route,
            Route::Issue { .. }
                | Route::Blob { .. }
                | Route::Blame { .. }
                | Route::Commit { .. }
                | Route::Release { .. }
                | Route::Discussion { .. }
                | Route::Job { .. }
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
            jumped: false,
            anchor: None,
        }
    }

    /// The selected item's link.
    pub fn selected_link(&self) -> Option<&Link> {
        let item = self.page.items.get(self.selected?)?;
        self.page.target(item.link)
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

    pub fn checks(&self, key: &DataKey) -> Option<&Checks> {
        match self.get(key)? {
            Data::Checks(c) => Some(c),
            _ => None,
        }
    }

    pub fn commit(&self, repo: &RepoId, oid: &str) -> Option<&CommitDetail> {
        match self.get(&DataKey::Commit(repo.clone(), oid.to_owned()))? {
            Data::Commit(c) => Some(c),
            _ => None,
        }
    }

    pub fn profile(&self, login: &str) -> Option<&Profile> {
        match self.get(&DataKey::Profile(login.to_lowercase()))? {
            Data::Profile(p) => Some(p),
            _ => None,
        }
    }

    /// How a page's fetches are going (the ones that have started).
    fn fetches(&self, route: &Route) -> impl Iterator<Item = Remote<()>> {
        needs(route).into_iter().filter_map(|need| match need {
            Need::Inbox => Some(self.inbox.status()),
            Need::Pr(pr) => self.prs.get(&pr).map(Remote::status),
            Need::Data(key) => self.data.get(&key).map(Remote::status),
        })
    }

    /// The first refresh error among a page's needs, and whether the page
    /// has data despite it.
    fn page_error(&self, route: &Route) -> Option<(String, bool)> {
        self.fetches(route)
            .find_map(|r| Some((r.error?, r.data.is_some())))
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

    pub fn page_loading_more(&self, route: &Route) -> bool {
        self.fetches(route).any(|r| r.loading_more)
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
        let cx = pages::PageCtx { icons, keys, now };
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
        page.compact = self.compact();
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
                        pages::repo_code(&mut page, repo, o, commits, aside, cx);
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
                let dir = pages::Listing {
                    repo,
                    rev,
                    path,
                    entries,
                    commits,
                };
                pages::repo_dir(&mut page, dir, cx);
                if entries.is_none() && matches!(error, Some((_, false))) {
                    missing(&mut page, "the directory");
                }
            }
            Route::Blob {
                repo,
                rev,
                path,
                lines,
            } => {
                let key = DataKey::Blob(repo.clone(), rev.clone(), path.clone());
                match self.get(&key) {
                    Some(Data::Blob(blob)) => {
                        let file = pages::FileAt {
                            repo,
                            rev,
                            path,
                            lines: *lines,
                        };
                        pages::file(&mut page, file, blob, keys);
                    }
                    _ => missing(&mut page, "the file"),
                }
            }
            Route::Blame {
                repo,
                rev,
                path,
                lines,
            } => {
                let blob = match self.get(&DataKey::Blob(repo.clone(), rev.clone(), path.clone())) {
                    Some(Data::Blob(b)) => Some(&**b),
                    _ => None,
                };
                let blame = match self.get(&DataKey::Blame(repo.clone(), rev.clone(), path.clone()))
                {
                    Some(Data::Blame(b)) => Some(&**b),
                    _ => None,
                };
                let file = pages::FileAt {
                    repo,
                    rev,
                    path,
                    lines: *lines,
                };
                if blob.is_none() && matches!(error, Some((_, false))) {
                    missing(&mut page, "the file");
                } else {
                    pages::blame(&mut page, file, blob, blame, keys, now);
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
                    pages::issue_list(&mut page, query, counts, is_pr, results, cx);
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
                    pages::issue(&mut page, issue, icons, keys, aside, now);
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
                        pages::pr_conversation(&mut page, pr, d, activity, aside, cx);
                    }
                    (Some(d), PrTab::Commits) => pages::pr_commits(&mut page, pr, d, activity, now),
                    (Some(d), PrTab::Checks) => {
                        let checks = self.checks(&DataKey::PrChecks(pr.clone()));
                        pages::pr_checks(&mut page, pr, d, checks, now);
                    }
                    (None, _) => missing(&mut page, &pr.to_string()),
                }
            }
            Route::Stargazers(_) | Route::Watchers(_) | Route::Forks(_) => {
                let data = paged(route).and_then(|key| self.get(&key));
                let title = match route {
                    Route::Stargazers(_) => "Stargazers",
                    Route::Watchers(_) => "Watchers",
                    _ => "Forks",
                };
                match data {
                    None if matches!(error, Some((_, false))) => {
                        missing(&mut page, &title.to_lowercase());
                    }
                    Some(Data::RepoPage(r)) => pages::repos(&mut page, title, Some(r), now),
                    Some(Data::Users(r)) => pages::people_list(&mut page, title, Some(r)),
                    _ => pages::people_list(&mut page, title, None),
                }
            }
            Route::Releases(repo) | Route::Tags(repo) => {
                let data = paged(route).and_then(|key| self.get(&key));
                match data {
                    None if matches!(error, Some((_, false))) => missing(&mut page, "the list"),
                    Some(Data::Tags(t)) => pages::tags(&mut page, repo, Some(t), now),
                    Some(Data::Releases(r)) => pages::releases(&mut page, repo, Some(r), now),
                    _ if matches!(route, Route::Tags(_)) => pages::tags(&mut page, repo, None, now),
                    _ => pages::releases(&mut page, repo, None, now),
                }
            }
            Route::Branches(repo) => match self.get(&DataKey::Branches(repo.clone())) {
                Some(Data::Branches(b)) => pages::branches(&mut page, repo, Some(b), icons, now),
                None if matches!(error, Some((_, false))) => missing(&mut page, "the branches"),
                _ => pages::branches(&mut page, repo, None, icons, now),
            },
            Route::Milestones { repo, closed } => {
                match self.get(&DataKey::Milestones(repo.clone(), *closed)) {
                    Some(Data::Milestones(m)) => {
                        pages::milestones(&mut page, repo, Some(m), *closed, now);
                    }
                    None if matches!(error, Some((_, false))) => {
                        missing(&mut page, "the milestones");
                    }
                    _ => pages::milestones(&mut page, repo, None, *closed, now),
                }
            }
            Route::Milestone { repo, number } => {
                match self.get(&DataKey::Milestone(repo.clone(), *number)) {
                    Some(Data::Milestone(m)) => pages::milestone(&mut page, repo, m, icons, now),
                    _ => missing(&mut page, &route.title()),
                }
            }
            Route::Compare { repo, spec } => {
                match self.get(&DataKey::Compare(repo.clone(), spec.clone())) {
                    Some(Data::Compare(c)) => pages::compare(&mut page, repo, spec, c, now),
                    _ => missing(&mut page, &route.title()),
                }
            }
            Route::Deployments { repo, environment } => {
                let list = match self.get(&DataKey::Deployments(repo.clone(), environment.clone()))
                {
                    Some(Data::Deployments(d)) => Some(&**d),
                    _ => None,
                };
                if list.is_none() && matches!(error, Some((_, false))) {
                    missing(&mut page, "the deployments");
                } else {
                    pages::deployments(&mut page, repo, list, environment.as_deref(), now);
                }
            }
            Route::Discussions { of, category } => {
                let list = match self.get(&DataKey::Discussions(of.clone(), category.clone())) {
                    Some(Data::Discussions(d)) => Some(&**d),
                    _ => None,
                };
                if list.is_none() && matches!(error, Some((_, false))) {
                    missing(&mut page, "the discussions");
                } else {
                    let base = Route::Discussions {
                        of: of.clone(),
                        category: None,
                    };
                    pages::discussions(&mut page, &base.url(), list, category.as_deref(), now);
                }
            }
            Route::Discussion { of, number } => {
                match self.get(&DataKey::Discussion(of.clone(), *number)) {
                    Some(Data::Discussion(d)) => pages::discussion(&mut page, d, now),
                    _ => missing(&mut page, &route.title()),
                }
            }
            Route::WorkflowRun { repo, run, attempt } => {
                match self.get(&DataKey::Run(repo.clone(), *run, *attempt)) {
                    Some(Data::Run(r)) => pages::workflow_run(&mut page, repo, r, now),
                    _ => missing(&mut page, &route.title()),
                }
            }
            Route::Job {
                repo,
                job,
                step,
                query,
                ..
            } => match self.get(&DataKey::Job(repo.clone(), *job)) {
                Some(Data::Job(j)) => {
                    let log = match self.get(&DataKey::JobLog(repo.clone(), *job)) {
                        Some(Data::Log(log)) => Some(log.as_str()),
                        _ => None,
                    };
                    let at = pages::JobAt {
                        step: *step,
                        query,
                        keys,
                    };
                    pages::job(&mut page, repo, j, log, at, now);
                }
                _ => missing(&mut page, &route.title()),
            },
            Route::Workflow { repo, file } => {
                let wf = match self.get(&DataKey::Workflow(repo.clone(), file.clone())) {
                    Some(Data::Workflow(w)) => Some(&**w),
                    _ => None,
                };
                let runs = match self.get(&DataKey::WorkflowRuns(repo.clone(), file.clone())) {
                    Some(Data::Runs(r)) => Some(&**r),
                    _ => None,
                };
                if wf.is_none() && runs.is_none() && matches!(error, Some((_, false))) {
                    missing(&mut page, &route.title());
                } else {
                    pages::workflow(&mut page, repo, wf, runs, now);
                }
            }
            Route::CommitChecks { repo, oid } => {
                let checks = self.checks(&DataKey::CommitChecks(repo.clone(), oid.clone()));
                if checks.is_none() && matches!(error, Some((_, false))) {
                    missing(&mut page, "the checks");
                } else {
                    pages::commit_checks(&mut page, checks, now);
                }
            }
            Route::Release { repo, tag } => {
                match self.get(&DataKey::Release(repo.clone(), tag.clone())) {
                    Some(Data::Release(r)) => pages::release(&mut page, repo, r, now),
                    _ => missing(&mut page, &route.title()),
                }
            }
            Route::Actions(repo) => {
                let checks = self.checks(&DataKey::BranchChecks(repo.clone()));
                if checks.is_none() && matches!(error, Some((_, false))) {
                    missing(&mut page, "the checks");
                } else {
                    let branch = self
                        .overview(repo)
                        .and_then(|o| o.default_branch.as_deref());
                    pages::actions(&mut page, branch, checks, now);
                }
            }
            Route::Commits { repo, rev, path } => {
                let key = DataKey::History(repo.clone(), rev.clone(), path.clone());
                let history = match self.get(&key) {
                    Some(Data::History(h)) => Some(&**h),
                    _ => None,
                };
                if history.is_none() && matches!(error, Some((_, false))) {
                    missing(&mut page, "the commits");
                } else {
                    pages::commit_history(&mut page, repo, rev, path, history, now);
                }
            }
            Route::Commit { repo, oid } => match self.commit(repo, oid) {
                Some(c) => {
                    let files = Route::Commit {
                        repo: repo.clone(),
                        oid: c.oid.clone(),
                    };
                    let files = format!("{}#files", files.url());
                    pages::commit(&mut page, repo, c, &files, now);
                }
                None => missing(&mut page, &route.title()),
            },
            Route::User { login, tab } => match self.profile(login) {
                Some(p) => {
                    let list = match paged(route).map(|key| (self.get(&key), key)) {
                        Some((Some(Data::RepoPage(r)), _)) => ProfileList::Repos(Some(r)),
                        Some((Some(Data::Users(r)), _)) => ProfileList::People(Some(r)),
                        Some((_, DataKey::Users(_))) => ProfileList::People(None),
                        Some(_) => ProfileList::Repos(None),
                        None => ProfileList::None,
                    };
                    pages::profile(&mut page, p, *tab, list, now);
                }
                _ => missing(&mut page, &format!("@{login}")),
            },
        }
        if let Some((err, true)) = error {
            // A flash banner on top; what's below is the cached copy.
            let when = self
                .page_cached_at(route)
                .map_or_else(|| "before".to_owned(), |at| ghtui_ui::time::ago(at, now));
            let mut banner = Page::new(page.width);
            pages::flash(
                &mut banner,
                &format!("Couldn't refresh: {err}. Showing what was loaded {when}."),
            );
            banner.blank();
            let n = banner.lines.len();
            page.lines.splice(0..0, banner.lines);
            for item in &mut page.items {
                item.start += n;
                item.end += n;
            }
            if let Some(jump) = &mut page.jump {
                *jump += n;
            }
            for line in page.anchors.values_mut() {
                *line += n;
            }
        }
        page
    }
}

fn extend<T>(a: &mut Results<T>, b: Results<T>) {
    a.items.extend(b.items);
    a.next = b.next;
    a.total = b.total;
}

/// Appends a page of results to what's shown.
pub fn append(results: &mut SearchResults, more: SearchResults) {
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
