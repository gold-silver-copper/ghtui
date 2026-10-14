//! Routes: the pages ghtui shows, addressed by their GitHub URLs.
//!
//! Pages come in families, each one enum in its own file that says, in
//! one place, what each of its pages is: its URL and title, the page above
//! it, what it needs fetched, and how it's built ([`Family`]). [`Route`]
//! is one of them, and asks the family.

pub mod app;
pub mod code;
pub mod issues;
pub mod pulls;

use crate::api::browse::{DiscussionsOf, RepoSort, SearchKind};
use crate::api::model::{PrRef, RepoId};
use crate::api::project::{ProjectOwner, ProjectRef, ProjectsOf};
use crate::api::security::AlertKind;
use crate::ui::pages::url as links;
use crate::ui::pages::{PrTab, ProfileTab};

use crate::browse::{Build, Width};
use crate::chrome::{Chrome, Section};
use crate::data::DataKey;
use crate::diff_screen::DiffOf;
use crate::state::State;

/// The default filter of a repository's issue and pull request lists.
pub const OPEN: &str = "is:open";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Route {
    /// Home, and the Offline page.
    App(app::Page),
    /// A repository's code, its history, branches and tags.
    Code(code::Page),
    /// Its issues, labels and milestones.
    Issues(issues::Page),
    /// Its pull requests.
    Pulls(pulls::Page),
    /// Workflow runs, filtered as GitHub's Actions tab filters them
    /// (`branch:main is:failure`).
    Actions {
        repo: RepoId,
        query: String,
    },
    /// Discussions, in a category (by its slug) if given.
    Discussions {
        of: DiscussionsOf,
        category: Option<String>,
    },
    Discussion {
        of: DiscussionsOf,
        number: u64,
    },
    /// A workflow run, or one attempt of it.
    WorkflowRun {
        repo: RepoId,
        run: u64,
        attempt: Option<u64>,
    },
    /// A job: its steps and its log. `step` is the step and line a link
    /// points at (expanded and scrolled to); `query` filters the log.
    Job {
        repo: RepoId,
        run: Option<u64>,
        job: u64,
        step: Option<(u32, u32)>,
        query: String,
    },
    /// A workflow, by its file name, and its runs.
    Workflow {
        repo: RepoId,
        file: String,
    },
    /// The form that runs a workflow by hand (GitHub's "Run workflow"
    /// menu, on the workflow's page).
    Dispatch {
        repo: RepoId,
        file: String,
    },
    Stargazers(RepoId),
    Watchers(RepoId),
    Forks(RepoId),
    Releases(RepoId),
    Release {
        repo: RepoId,
        tag: String,
    },
    /// A wiki page (its Home without one; its pages for `_pages`).
    Wiki {
        repo: RepoId,
        page: Option<String>,
    },
    /// A repository's security advisories, or GitHub's newest.
    Advisories(Option<RepoId>),
    /// A security advisory: a repository's, or from GitHub's database.
    Advisory {
        repo: Option<RepoId>,
        ghsa: String,
    },
    /// A repository's Dependabot, code scanning or secret scanning
    /// alerts, filtered as GitHub's lists are (`is:open severity:high`).
    Alerts {
        repo: RepoId,
        kind: AlertKind,
        query: String,
    },
    /// One security alert.
    Alert {
        repo: RepoId,
        kind: AlertKind,
        number: u64,
    },
    /// A page of a repository's Settings tab ([`crate::settings`]).
    Settings {
        repo: RepoId,
        page: crate::api::settings::SettingsPage,
    },
    /// An organization's teams.
    Teams(String),
    Team {
        org: String,
        slug: String,
    },
    /// A gist, by its ID (its owner, when the URL names them).
    Gist {
        owner: Option<String>,
        id: String,
    },
    /// Someone's gists.
    Gists(String),
    /// Deployments, to one environment if given.
    Deployments {
        repo: RepoId,
        environment: Option<String>,
    },
    User {
        login: String,
        tab: ProfileTab,
    },
    Search {
        kind: SearchKind,
        query: String,
    },
    /// Your inbox, filtered as GitHub's is (`is:unread reason:mention`).
    Notifications {
        query: String,
    },
    /// An organization's or someone's projects, or a repository's.
    Projects(ProjectsOf),
    /// A project at one of its views (its first, if none is named).
    Project {
        project: ProjectRef,
        view: Option<u64>,
    },
    /// One of a project's items, as the view's side pane shows it, by the
    /// number its URL names it by (`?pane=issue&itemId=…`).
    ProjectItem {
        project: ProjectRef,
        view: Option<u64>,
        item: u64,
    },
    /// A new release's form.
    NewRelease(RepoId),
    /// The form editing a release, by its tag.
    EditRelease {
        repo: RepoId,
        tag: String,
    },
    /// A new gist's form.
    NewGist,
    /// The form changing one of your gists.
    EditGist {
        owner: String,
        id: String,
    },
    /// The form writing a wiki page (a new one without `page`).
    WikiEdit {
        repo: RepoId,
        page: Option<String>,
    },
    /// Insights, Explore and topics, starred lists, packages
    /// ([`crate::site`]).
    Site(crate::site::Page),
    /// One of housekeeping's forms: a label, a milestone, a fork, a new
    /// repository, a file.
    Housekeeping(crate::housekeeping::FormOf),
}

/// Where a link leads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Page(Route),
    /// The diff screen: a pull request's Files changed tab, or a commit's.
    Files(DiffOf),
    /// Anything ghtui doesn't show itself.
    External(String),
}

/// Where a link leads, and the spot there its `#fragment` names: a
/// comment, a file in a diff, a review comment's thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dest {
    pub target: Target,
    pub anchor: Option<String>,
}

impl From<Target> for Dest {
    fn from(target: Target) -> Dest {
        Dest {
            target,
            anchor: None,
        }
    }
}

/// What a family of pages says about each of its pages.
pub trait Family {
    fn url(&self) -> String;
    /// The short name shown in the top bar.
    fn title(&self) -> String;
    /// The repository the page belongs to.
    fn repo(&self) -> Option<&RepoId> {
        None
    }
    /// The page above this one, like GitHub's breadcrumbs.
    fn up(&self, state: &State) -> Option<Route>;
    /// What the page needs fetched: everything [`Family::build`] reads.
    fn needs(&self) -> Vec<DataKey>;
    /// What the page reads besides its needs, already fetched for it by
    /// something else.
    fn also_reads(&self, _key: &DataKey) -> bool {
        false
    }
    /// The list the page loads more of.
    fn paged(&self) -> Option<DataKey> {
        None
    }
    /// How wide the page gets.
    fn width(&self) -> Width {
        Width::Lists
    }
    /// Whether the page opens with its first row selected: lists do;
    /// reading pages start with nothing selected.
    fn starts_on_row(&self) -> bool {
        true
    }
    /// The data the page shows that git can answer, whose source it says.
    fn shown_source(&self) -> Option<DataKey> {
        None
    }
    /// A list's filter.
    fn query(&self) -> Option<&str> {
        None
    }
    /// The same list with another filter.
    fn with_query(&self, _query: String) -> Option<Route> {
        None
    }
    /// Which of its repository's tabs the page is under.
    fn section(&self) -> Section {
        Section::Code
    }
    /// The chrome around the page: its crumbs, and its tabs. A
    /// repository's page is under its repository's tabs.
    fn chrome(&self, state: &State, c: &mut Chrome) {
        if let Some(repo) = self.repo() {
            state.repo_chrome(repo, self.section(), c);
        }
    }
    /// Builds the page from what's loaded.
    fn build(&self, b: Build<'_>, page: &mut crate::ui::page::Page);
}

impl Route {
    /// What the page reads besides its needs, already fetched for it by
    /// something else.
    pub fn also_reads(&self, key: &DataKey) -> bool {
        match self {
            Route::App(p) => p.also_reads(key),
            Route::Code(p) => p.also_reads(key),
            Route::Issues(p) => p.also_reads(key),
            Route::Pulls(p) => p.also_reads(key),
            Route::Site(p) => p.also_reads(key),
            Route::Housekeeping(p) => p.also_reads(key),
            _ => false,
        }
    }

    /// What the page needs fetched (Home's are the state's:
    /// [`State::needs`]).
    pub fn needs(&self) -> Vec<DataKey> {
        match self {
            Route::App(p) => p.needs(),
            Route::Code(p) => p.needs(),
            Route::Issues(p) => p.needs(),
            Route::Pulls(p) => p.needs(),
            Route::Site(p) => p.needs(),
            Route::Housekeeping(p) => p.needs(),
            _ => crate::browse::needs(self),
        }
    }

    /// Whether the page opens with its first row selected.
    pub fn starts_on_row(&self) -> bool {
        match self {
            Route::App(p) => p.starts_on_row(),
            Route::Code(p) => p.starts_on_row(),
            Route::Issues(p) => p.starts_on_row(),
            Route::Pulls(p) => p.starts_on_row(),
            Route::Site(p) => p.starts_on_row(),
            Route::Housekeeping(p) => p.starts_on_row(),
            _ => true,
        }
    }

    /// A pull request's conversation.
    pub fn pr(pr: PrRef) -> Route {
        Route::Pulls(pulls::Pr {
            pr,
            tab: PrTab::Conversation,
        })
    }

    /// A file, from the top.
    pub fn blob(repo: RepoId, rev: String, path: String) -> Route {
        Route::Code(code::Blob {
            repo,
            rev,
            path,
            lines: None,
        })
    }

    /// An unsplit page once its ref is known ([`code::Page::split`]).
    pub fn split(&self, rev: Option<&str>) -> Option<Route> {
        match self {
            Route::Code(p) => p.split(rev),
            _ => None,
        }
    }

    /// The revision a page is at.
    pub fn rev(&self) -> Option<&str> {
        match self {
            Route::Code(p) => p.rev(),
            _ => None,
        }
    }

    /// A profile's overview.
    pub fn user(login: &str) -> Route {
        Route::User {
            login: login.to_owned(),
            tab: ProfileTab::Overview,
        }
    }

    /// A repository's Actions tab: all its runs.
    pub fn actions(repo: RepoId) -> Route {
        Route::Actions {
            repo,
            query: String::new(),
        }
    }

    pub fn url(&self) -> String {
        match self {
            Route::App(p) => p.url(),
            Route::Code(p) => p.url(),
            Route::Issues(p) => p.url(),
            Route::Pulls(p) => p.url(),
            Route::Actions { repo, query } if query.is_empty() => {
                format!("{}/actions", links::repo(repo))
            }
            Route::Actions { repo, query } => {
                format!(
                    "{}/actions?query={}",
                    links::repo(repo),
                    links::encode(query)
                )
            }
            Route::WorkflowRun { repo, run, attempt } => {
                let url = format!("{}/actions/runs/{run}", links::repo(repo));
                match attempt {
                    Some(n) => format!("{url}/attempts/{n}"),
                    None => url,
                }
            }
            Route::Job {
                repo,
                run,
                job,
                step,
                query,
            } => {
                let mut url = match run {
                    Some(run) => format!("{}/actions/runs/{run}/job/{job}", links::repo(repo)),
                    None => format!("{}/runs/{job}", links::repo(repo)),
                };
                if !query.is_empty() {
                    url.push_str(&format!("?q={}", links::encode(query)));
                }
                if let Some((step, line)) = step {
                    url.push_str(&format!("#step:{step}:{line}"));
                }
                url
            }
            Route::Workflow { repo, file } | Route::Dispatch { repo, file } => {
                format!(
                    "{}/actions/workflows/{}",
                    links::repo(repo),
                    links::encode_path(file)
                )
            }
            Route::Stargazers(repo) => format!("{}/stargazers", links::repo(repo)),
            Route::Watchers(repo) => format!("{}/watchers", links::repo(repo)),
            Route::Forks(repo) => format!("{}/forks", links::repo(repo)),
            Route::Releases(repo) => format!("{}/releases", links::repo(repo)),
            Route::Release { repo, tag } => links::release(repo, tag),
            Route::Discussions { of, category } => match category {
                Some(slug) => format!(
                    "{}/categories/{}",
                    discussions_url(of),
                    links::encode_path(slug)
                ),
                None => discussions_url(of),
            },
            Route::Discussion { of, number } => format!("{}/{number}", discussions_url(of)),
            Route::Wiki { repo, page } => match page {
                Some(page) => format!("{}/wiki/{}", links::repo(repo), links::encode_path(page)),
                None => format!("{}/wiki", links::repo(repo)),
            },
            Route::Advisories(repo) => match repo {
                Some(repo) => format!("{}/security/advisories", links::repo(repo)),
                None => format!("{}/advisories", links::base()),
            },
            Route::Advisory { repo, ghsa } => match repo {
                Some(repo) => format!("{}/security/advisories/{ghsa}", links::repo(repo)),
                None => format!("{}/advisories/{ghsa}", links::base()),
            },
            Route::Alerts { repo, kind, query } => {
                let base = format!("{}/security/{}", links::repo(repo), kind.slug());
                if query == OPEN {
                    base
                } else {
                    format!("{base}?{}={}", kind.query_param(), links::encode(query))
                }
            }
            Route::Alert { repo, kind, number } => {
                format!("{}/security/{}/{number}", links::repo(repo), kind.slug())
            }
            Route::Settings { repo, page } => {
                format!("{}/settings{}", links::repo(repo), page.slug())
            }
            Route::Teams(org) => format!("{}/orgs/{org}/teams", links::base()),
            Route::Team { org, slug } => format!("{}/orgs/{org}/teams/{slug}", links::base()),
            Route::Gist { owner, id } => match owner {
                Some(owner) => format!("{}/{owner}/{id}", links::gists()),
                None => format!("{}/{id}", links::gists()),
            },
            Route::Gists(login) => format!("{}/{login}", links::gists()),
            Route::Deployments { repo, environment } => match environment {
                Some(env) => format!(
                    "{}/deployments/activity_log?environments_filter={}",
                    links::repo(repo),
                    links::encode(env)
                ),
                None => format!("{}/deployments", links::repo(repo)),
            },
            Route::NewRelease(repo) => format!("{}/releases/new", links::repo(repo)),
            Route::EditRelease { repo, tag } => format!(
                "{}/releases/edit/{}",
                links::repo(repo),
                links::encode_path(tag)
            ),
            Route::NewGist => format!("{}/", links::gists()),
            Route::EditGist { owner, id } => format!("{}/{owner}/{id}/edit", links::gists()),
            Route::WikiEdit { repo, page } => match page {
                Some(page) => format!(
                    "{}/wiki/{}/_edit",
                    links::repo(repo),
                    links::encode_path(page)
                ),
                None => format!("{}/wiki/_new", links::repo(repo)),
            },
            Route::Site(page) => page.url(),
            Route::User { login, tab } => match tab {
                ProfileTab::Overview => links::user(login),
                ProfileTab::Repositories(sort) => {
                    let sort = match sort {
                        RepoSort::Updated => "",
                        RepoSort::Name => "&sort=name",
                        RepoSort::Stars => "&sort=stargazers",
                    };
                    format!("{}?tab=repositories{sort}", links::user(login))
                }
                ProfileTab::Stars => format!("{}?tab=stars", links::user(login)),
                ProfileTab::Followers => format!("{}?tab=followers", links::user(login)),
                ProfileTab::Following => format!("{}?tab=following", links::user(login)),
                ProfileTab::People => format!("{}/orgs/{login}/people", links::base()),
            },
            Route::Search { kind, query } => links::search(*kind, query),
            Route::Notifications { query } if query.is_empty() => {
                format!("{}/notifications", links::base())
            }
            Route::Notifications { query } => {
                format!(
                    "{}/notifications?query={}",
                    links::base(),
                    links::encode(query)
                )
            }
            Route::Projects(ProjectsOf::Owner(owner)) => format!("{}/projects", owner.url()),
            Route::Projects(ProjectsOf::Repo(repo)) => format!("{}/projects", links::repo(repo)),
            Route::Project { project, view } => view_url(project, *view),
            Route::ProjectItem {
                project,
                view,
                item,
            } => format!("{}?pane=issue&itemId={item}", view_url(project, *view)),
            Route::Housekeeping(of) => of.url(),
        }
    }

    /// The short name shown in the top bar.
    pub fn title(&self) -> String {
        match self {
            Route::App(p) => p.title(),
            Route::Code(p) => p.title(),
            Route::Issues(p) => p.title(),
            Route::Pulls(p) => p.title(),
            Route::Actions { repo, .. } => format!("{repo} · Actions"),
            Route::WorkflowRun { repo, run, .. } => format!("{repo} · Run {run}"),
            Route::Job { repo, job, .. } => format!("{repo} · Job {job}"),
            Route::Workflow { repo, file } => format!("{repo} · {file}"),
            Route::Dispatch { repo, file } => format!("{repo} · Run {file}"),
            Route::Stargazers(repo) => format!("{repo} · Stargazers"),
            Route::Watchers(repo) => format!("{repo} · Watchers"),
            Route::Forks(repo) => format!("{repo} · Forks"),
            Route::Releases(repo) => format!("{repo} · Releases"),
            Route::Release { repo, tag } => format!("{repo} {tag}"),
            Route::Discussions { of, .. } => format!("{} · Discussions", of_title(of)),
            Route::Discussion { of, number } => format!("{} · Discussion {number}", of_title(of)),
            Route::Wiki { repo, page } => match page {
                Some(page) => format!("{repo} · {}", page.replace('-', " ")),
                None => format!("{repo} · Wiki"),
            },
            Route::Advisories(Some(repo)) => format!("{repo} · Security"),
            Route::Advisories(None) => "Advisories".to_owned(),
            Route::Advisory { ghsa, .. } => ghsa.clone(),
            Route::Alerts { repo, kind, .. } => format!("{repo} · {}", kind.name()),
            Route::Alert { repo, kind, number } => {
                format!("{repo} · {} alert {number}", kind.name())
            }
            Route::Settings {
                repo,
                page: crate::api::settings::SettingsPage::Hook(id),
            } => format!("{repo} · Webhook {id}"),
            Route::Settings { repo, page } => format!("{repo} · Settings · {}", page.name()),
            Route::Teams(org) => format!("@{org} · Teams"),
            Route::Team { org, slug } => format!("@{org}/{slug}"),
            Route::Gist { id, .. } => format!("Gist {}", id.get(..7).unwrap_or(id)),
            Route::Gists(login) => format!("@{login} · Gists"),
            Route::Deployments { repo, .. } => format!("{repo} · Deployments"),
            Route::NewRelease(repo) => format!("{repo} · New release"),
            Route::EditRelease { repo, tag } => format!("{repo} · Editing {tag}"),
            Route::NewGist => "New gist".to_owned(),
            Route::EditGist { id, .. } => format!("Editing gist {}", id.get(..7).unwrap_or(id)),
            Route::WikiEdit { repo, page: None } => format!("{repo} · New wiki page"),
            Route::WikiEdit {
                repo,
                page: Some(page),
            } => format!("{repo} · Editing {page}"),
            Route::Site(page) => page.title(),
            Route::User { login, .. } => format!("@{login}"),
            Route::Search { query, .. } => format!("Search “{query}”"),
            Route::Notifications { .. } => "Notifications".to_owned(),
            Route::Projects(ProjectsOf::Owner(owner)) => format!("@{} · Projects", owner.login()),
            Route::Projects(ProjectsOf::Repo(repo)) => format!("{repo} · Projects"),
            Route::Project { project, .. } => {
                format!("@{} · Project {}", project.owner.login(), project.number)
            }
            Route::ProjectItem { project, item, .. } => format!(
                "@{} · Project {} · Item {item}",
                project.owner.login(),
                project.number
            ),
            Route::Housekeeping(of) => of.title(),
        }
    }

    /// The repository the page belongs to.
    pub fn repo(&self) -> Option<&RepoId> {
        match self {
            Route::Actions { repo, .. }
            | Route::WorkflowRun { repo, .. }
            | Route::Job { repo, .. }
            | Route::Workflow { repo, .. }
            | Route::Stargazers(repo)
            | Route::Watchers(repo)
            | Route::Forks(repo)
            | Route::Releases(repo)
            | Route::Release { repo, .. }
            | Route::Discussions {
                of: DiscussionsOf::Repo(repo),
                ..
            }
            | Route::Discussion {
                of: DiscussionsOf::Repo(repo),
                ..
            }
            | Route::Wiki { repo, .. }
            | Route::Advisories(Some(repo))
            | Route::Advisory {
                repo: Some(repo), ..
            }
            | Route::Alerts { repo, .. }
            | Route::Alert { repo, .. }
            | Route::Settings { repo, .. }
            | Route::Deployments { repo, .. }
            | Route::NewRelease(repo)
            | Route::EditRelease { repo, .. }
            | Route::WikiEdit { repo, .. }
            | Route::Dispatch { repo, .. }
            | Route::Projects(ProjectsOf::Repo(repo)) => Some(repo),
            Route::Code(p) => p.repo(),
            Route::Issues(p) => p.repo(),
            Route::Pulls(p) => p.repo(),
            Route::Site(page) => page.repo(),
            Route::Housekeeping(of) => of.repo(),
            Route::Projects(ProjectsOf::Owner(_))
            | Route::Project { .. }
            | Route::ProjectItem { .. }
            | Route::App(_)
            | Route::User { .. }
            | Route::Search { .. }
            | Route::Notifications { .. }
            | Route::Gist { .. }
            | Route::Gists(_)
            | Route::NewGist
            | Route::EditGist { .. }
            | Route::Teams(_)
            | Route::Team { .. }
            | Route::Advisories(None)
            | Route::Advisory { repo: None, .. }
            | Route::Discussions {
                of: DiscussionsOf::Org(_),
                ..
            }
            | Route::Discussion {
                of: DiscussionsOf::Org(_),
                ..
            } => None,
        }
    }

    /// A list's filter: issues, pull requests, search results, runs,
    /// notifications.
    pub fn list_query(&self) -> Option<&str> {
        match self {
            Route::Issues(p) => p.query(),
            Route::Pulls(p) => p.query(),
            Route::Actions { query, .. }
            | Route::Alerts { query, .. }
            | Route::Search { query, .. }
            | Route::Notifications { query } => Some(query),
            _ => None,
        }
    }

    /// The same list with another filter.
    pub fn with_query(&self, query: String) -> Option<Route> {
        Some(match self {
            Route::Issues(p) => return p.with_query(query),
            Route::Pulls(p) => return p.with_query(query),
            Route::Search { kind, .. } => Route::Search { kind: *kind, query },
            Route::Actions { repo, .. } => Route::Actions {
                repo: repo.clone(),
                query,
            },
            Route::Notifications { .. } => Route::Notifications { query },
            Route::Alerts { repo, kind, .. } => Route::Alerts {
                repo: repo.clone(),
                kind: *kind,
                query,
            },
            _ => return None,
        })
    }

    /// The search behind a list page.
    pub fn search(&self) -> Option<(SearchKind, String)> {
        let key = match self {
            Route::Issues(p @ issues::Issues { .. }) => p.paged(),
            Route::Pulls(p @ pulls::Pulls { .. }) => p.paged(),
            Route::Search { kind, query } => return Some((*kind, query.clone())),
            _ => None,
        };
        match key? {
            DataKey::Search(kind, query) => Some((kind, query)),
            _ => None,
        }
    }
}

impl Dest {
    /// Parses a URL of the GitHub ghtui works with (see [`crate::host`]),
    /// with or without its scheme. Other URLs, another GitHub's too, are
    /// external.
    pub fn from_url(s: &str) -> Dest {
        let with_scheme = if has_host_prefix(s) {
            format!("https://{s}")
        } else {
            s.to_owned()
        };
        let parsed = url::Url::parse(&with_scheme).ok();
        let anchor = parsed
            .as_ref()
            .and_then(url::Url::fragment)
            .map(str::to_owned);
        match parsed.as_ref().and_then(parse) {
            Some(target) => Dest { target, anchor },
            None => Target::External(s.to_owned()).into(),
        }
    }
}

/// Whether `s` starts with the host's name, scheme left off
/// (`github.com/o/r`).
fn has_host_prefix(s: &str) -> bool {
    s.split_once('/')
        .is_some_and(|(host, _)| crate::host::current().serves(host))
}

/// A URL being parsed: its path's segments, decoded, and its query and
/// fragment.
pub struct Url<'a> {
    pub segments: &'a [&'a str],
    url: &'a url::Url,
}

impl Url<'_> {
    /// A query parameter's value.
    pub fn param(&self, name: &str) -> Option<String> {
        param(self.url, name)
    }

    pub fn fragment(&self) -> Option<&str> {
        self.url.fragment()
    }
}

/// What a family makes of the path after `/<owner>/<repo>`.
type RepoParse = fn(&RepoId, &[&str], &Url<'_>) -> Option<Target>;

/// What the families make of a URL's path, asked in this order (after
/// [`crate::site::parse`]); the first to know the page says.
const TOP: &[fn(&Url<'_>) -> Option<Target>] = &[app::parse, crate::housekeeping::parse, top];

/// What they make of the path after `/<owner>/<repo>`, asked in this
/// order: housekeeping's forms (`milestones/new`) before the milestones.
const REPO: &[RepoParse] = &[
    crate::housekeeping::parse_repo,
    code::parse_repo,
    issues::parse_repo,
    pulls::parse_repo,
    repo_page,
];

/// What a URL opens in ghtui, or `None` for the browser.
fn parse(parsed: &url::Url) -> Option<Target> {
    if !matches!(parsed.scheme(), "https" | "http") {
        return None;
    }
    let host = crate::host::current();
    let name = crate::host::url_host(parsed)?;
    if host.serves_gists(&name) {
        return gist(parsed.path_segments()?).map(Target::Page);
    }
    if !host.serves(&name) {
        return None;
    }
    // An Enterprise Server's gists are under `/gist`.
    if host.is_enterprise()
        && let Some(mut segments) = parsed.path_segments()
        && segments.next() == Some("gist")
    {
        return gist(segments).map(Target::Page);
    }
    let segments: Vec<String> = parsed
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).map(decode).collect())
        .unwrap_or_default();
    let segments: Vec<&str> = segments.iter().map(String::as_str).collect();
    let url = Url {
        segments: &segments,
        url: parsed,
    };
    if let Some(page) = crate::site::parse(&segments, |name| url.param(name)) {
        return Some(Target::Page(Route::Site(page)));
    }
    // Names that can't be GitHub's are pages GitHub doesn't have.
    if let ["stars" | "orgs" | "users", login, ..] = segments.as_slice()
        && !valid_login(login)
    {
        return None;
    }
    if let Some(target) = TOP.iter().find_map(|parse| parse(&url)) {
        return Some(target);
    }
    // GitHub's own pages: settings, apps, the marketplace…
    let [owner, name, rest @ ..] = segments.as_slice() else {
        return None;
    };
    if is_reserved(owner) {
        return None;
    }
    let repo = RepoId::parse(&format!("{owner}/{name}"))?;
    REPO.iter().find_map(|parse| parse(&repo, rest, &url))
}

/// The pages at the top of GitHub's paths.
fn top(url: &Url<'_>) -> Option<Target> {
    let param = |name: &str| url.param(name);
    let (query, kind, tab) = (param("q"), param("type"), param("tab"));
    let repos = || {
        ProfileTab::Repositories(match param("sort").as_deref() {
            Some("name") => RepoSort::Name,
            Some("stargazers") => RepoSort::Stars,
            _ => RepoSort::Updated,
        })
    };
    let route = match url.segments {
        ["search"] => Route::Search {
            kind: match kind.as_deref().map(str::to_ascii_lowercase).as_deref() {
                None | Some("repositories") => SearchKind::Repos,
                Some("issues") => SearchKind::Issues,
                Some("pullrequests") => SearchKind::Pulls,
                Some("users") => SearchKind::Users,
                Some("discussions") => SearchKind::Discussions,
                Some("commits") => SearchKind::Commits,
                Some("code") => SearchKind::Code,
                // Topics: the repositories tagged with one.
                Some("topics") => {
                    let topic = query.unwrap_or_default();
                    let topic = topic.trim();
                    let query = if topic.is_empty() {
                        String::new()
                    } else {
                        format!("topic:{topic}")
                    };
                    return Some(Target::Page(Route::Search {
                        kind: SearchKind::Repos,
                        query,
                    }));
                }
                // Wikis, packages, the marketplace…
                Some(_) => return None,
            },
            query: query.unwrap_or_default(),
        },
        // The dashboards of your pull requests and issues: searches.
        [list @ ("pulls" | "issues"), rest @ ..] => {
            let (kind, is) = if *list == "pulls" {
                (SearchKind::Pulls, "pr")
            } else {
                (SearchKind::Issues, "issue")
            };
            let whose = match rest {
                [] => "author:@me",
                ["assigned"] => "assignee:@me",
                ["mentioned"] => "mentions:@me",
                ["review-requested"] if *list == "pulls" => "review-requested:@me",
                _ => return None,
            };
            Route::Search {
                kind,
                query: query.unwrap_or_else(|| format!("is:open is:{is} {whose} archived:false")),
            }
        }
        [kind @ ("orgs" | "users"), login, "projects", rest @ ..] => {
            let owner = if *kind == "orgs" {
                ProjectOwner::Org((*login).to_owned())
            } else {
                ProjectOwner::User((*login).to_owned())
            };
            project(owner, rest, url)?
        }
        ["stars", login] => Route::User {
            login: (*login).to_owned(),
            tab: ProfileTab::Stars,
        },
        ["orgs", login] => Route::user(login),
        ["notifications"] => Route::Notifications {
            query: param("query")
                .map(|q| q.trim().to_owned())
                .unwrap_or_default(),
        },
        ["advisories"] => Route::Advisories(None),
        ["advisories", ghsa] if valid_ghsa(ghsa) => Route::Advisory {
            repo: None,
            ghsa: (*ghsa).to_owned(),
        },
        ["orgs", org, "teams"] => Route::Teams((*org).to_owned()),
        // GitHub's form for a new team.
        ["orgs", _, "teams", "new"] => return None,
        // Its members, repositories and child teams are on its page.
        ["orgs", org, "teams", slug, ..] if valid_slug(slug) => Route::Team {
            org: (*org).to_owned(),
            slug: (*slug).to_owned(),
        },
        ["orgs", org, "discussions", rest @ ..] => {
            discussions(DiscussionsOf::Org((*org).to_owned()), rest)?
        }
        ["orgs", login, page @ ("repositories" | "people")] => Route::User {
            login: (*login).to_owned(),
            tab: if *page == "people" {
                ProfileTab::People
            } else {
                repos()
            },
        },
        // A profile's projects are an organization's at `/orgs/…/projects`.
        [login]
            if tab.as_deref() == Some("projects")
                && !RESERVED.contains(login)
                && valid_login(login) =>
        {
            Route::Projects(ProjectsOf::Owner(ProjectOwner::User((*login).to_owned())))
        }
        [login] if !RESERVED.contains(login) && valid_login(login) => Route::User {
            login: (*login).to_owned(),
            tab: match tab.as_deref() {
                None | Some("overview") => ProfileTab::Overview,
                Some("repositories") => repos(),
                Some("stars") => ProfileTab::Stars,
                Some("followers") => ProfileTab::Followers,
                Some("following") => ProfileTab::Following,
                // Packages need a scope gh doesn't grant.
                Some(_) => return None,
            },
        },
        _ => return None,
    };
    Some(Target::Page(route))
}

/// A query parameter's value.
fn param(url: &url::Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
}

/// A repository's page, from what follows `/<owner>/<repo>` in its URL.
fn repo_page(repo: &RepoId, rest: &[&str], url: &Url<'_>) -> Option<Target> {
    let repo = repo.clone();
    let param = |name: &str| url.param(name);
    let fragment = url.fragment();
    let route = match rest {
        ["actions"] => Route::Actions {
            repo,
            query: param("query")
                .map(|q| q.trim().to_owned())
                .unwrap_or_default(),
        },
        // Its linked projects; a classic project (`projects/1`) is gone.
        ["projects"] => Route::Projects(ProjectsOf::Repo(repo)),
        ["stargazers"] => Route::Stargazers(repo),
        ["watchers"] => Route::Watchers(repo),
        ["forks"] | ["network", "members"] => Route::Forks(repo),
        ["releases", "tag", tag @ ..] if !tag.is_empty() => Route::Release {
            repo,
            tag: tag.join("/"),
        },
        ["releases", "new"] => Route::NewRelease(repo),
        ["releases", "edit", tag @ ..] if !tag.is_empty() => Route::EditRelease {
            repo,
            tag: tag.join("/"),
        },
        ["wiki", "_new"] => Route::WikiEdit { repo, page: None },
        ["wiki", page, "_edit"] if !page.contains(char::is_control) => Route::WikiEdit {
            repo,
            page: Some((*page).to_owned()),
        },
        // The network graph is a picture. Assets are downloads; a wiki's
        // comparisons, and the form for a new advisory, are on GitHub.
        ["network"]
        | ["releases", "download", ..]
        | ["wiki", .., "_compare"]
        | ["security", "advisories", "new"] => return None,
        // A wiki's own pages but its list (history, comparisons).
        ["wiki", special, ..] if special.starts_with('_') && *special != "_pages" => return None,
        ["releases", ..] => Route::Releases(repo),
        // The security overview's part ghtui can show: advisories.
        ["security"] | ["security", "advisories"] => Route::Advisories(Some(repo)),
        ["security", "advisories", ghsa] if valid_ghsa(ghsa) => Route::Advisory {
            repo: Some(repo),
            ghsa: (*ghsa).to_owned(),
        },
        // Dependabot's, code scanning's and secret scanning's alerts.
        ["security", slug] if AlertKind::from_slug(slug).is_some() => {
            let kind = AlertKind::from_slug(slug)?;
            let query = param(kind.query_param())
                .or_else(|| param("q").or_else(|| param("query")))
                .map(|q| q.trim().to_owned())
                .filter(|q| !q.is_empty())
                .unwrap_or_else(|| OPEN.to_owned());
            Route::Alerts { repo, kind, query }
        }
        // The Settings tab's pages ghtui has; the rest are on GitHub.
        ["settings", rest @ ..] => {
            use crate::api::settings::SettingsPage as P;
            let page = match rest {
                [] => P::General,
                ["branches" | "branch_protection_rules" | "rules", ..] => P::Branches,
                ["access" | "collaboration"] => P::Access,
                ["hooks"] | ["hooks", "new"] => P::Hooks,
                ["hooks", id] => P::Hook(id.parse().ok().filter(|&n| n > 0)?),
                ["keys", ..] => P::Keys,
                _ => return None,
            };
            Route::Settings { repo, page }
        }
        ["security", slug, n] if AlertKind::from_slug(slug).is_some() => Route::Alert {
            repo,
            kind: AlertKind::from_slug(slug)?,
            number: n.parse().ok().filter(|&n| n > 0)?,
        },
        // A page's revisions and history: the page.
        ["wiki", rest @ ..] if !rest.iter().any(|p| p.contains(char::is_control)) => Route::Wiki {
            repo,
            page: rest.first().map(|p| (*p).to_owned()),
        },
        ["deployments", rest @ ..] => {
            let environment = match rest {
                [] => None,
                ["activity_log"] => param("environments_filter"),
                [env] if !env.contains(char::is_control) => Some((*env).to_owned()),
                _ => return None,
            };
            Route::Deployments { repo, environment }
        }
        ["discussions", rest @ ..] => discussions(DiscussionsOf::Repo(repo), rest)?,
        ["actions", "runs", run, rest @ ..] => {
            let run = run.parse().ok()?;
            match rest {
                // The run's workflow file shows on the run's page.
                [] | ["workflow"] => Route::WorkflowRun {
                    repo,
                    run,
                    attempt: None,
                },
                ["attempts", n] => Route::WorkflowRun {
                    repo,
                    run,
                    attempt: Some(n.parse().ok()?),
                },
                ["job", job] => Route::Job {
                    repo,
                    run: Some(run),
                    job: job.parse().ok()?,
                    step: fragment.and_then(step_line),
                    query: param("q").unwrap_or_default(),
                },
                _ => return None,
            }
        }
        // A check run's URL, which is its job's for Actions.
        ["runs", job] | ["runs", _, "jobs", job] => Route::Job {
            repo,
            run: None,
            job: job.parse().ok()?,
            step: fragment.and_then(step_line),
            query: param("q").unwrap_or_default(),
        },
        ["actions", "workflows", file]
            if !file.contains(|c: char| c.is_control() || c.is_whitespace()) =>
        {
            Route::Workflow {
                repo,
                file: (*file).to_owned(),
            }
        }
        _ => return None,
    };
    Some(Target::Page(route))
}

/// The gist website's own pages, not people's gists.
const GIST_PAGES: &[&str] = &[
    "discover", "starred", "search", "auth", "login", "join", "mine",
];

/// Whether a gist path segment after an owner is a gist's ID
/// (numbers for old gists, 20 or 32 hex digits for newer).
fn is_gist_id(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_digit())
        || (matches!(s.len(), 20 | 32) && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Whether a lone gist segment is a gist rather than someone's
/// gists. GitHub tries a login first; ghtui can't ask, so it takes short
/// numbers for logins (`/1`, `/2000`) and long ones for old gists
/// (`/1162032`, which GitHub redirects to its owner's).
fn is_lone_gist_id(s: &str) -> bool {
    is_gist_id(s) && !(s.len() < 5 && s.bytes().all(|b| b.is_ascii_digit()))
}

/// A gist website's page, from its path: a gist (`/<id>`, `/<owner>/<id>`)
/// or someone's gists (`/<owner>`). Raw files and embeds are downloads.
fn gist<'a>(segments: impl Iterator<Item = &'a str>) -> Option<Route> {
    let segments: Vec<&str> = segments.filter(|s| !s.is_empty()).collect();
    Some(match segments.as_slice() {
        // The gist website's front page is the form for a new gist.
        [] => Route::NewGist,
        [first, ..] if GIST_PAGES.contains(first) => return None,
        [owner, id, "edit"] if is_gist_id(id) && valid_login(owner) => Route::EditGist {
            owner: (*owner).to_owned(),
            id: (*id).to_owned(),
        },
        [id] if is_lone_gist_id(id) => Route::Gist {
            owner: None,
            id: (*id).to_owned(),
        },
        [login] if valid_login(login) => Route::Gists((*login).to_owned()),
        [owner, id] if is_gist_id(id) && valid_login(owner) => Route::Gist {
            owner: Some((*owner).to_owned()),
            id: (*id).to_owned(),
        },
        // Revisions, forks, stars: the gist.
        [owner, id, "revisions" | "forks" | "stargazers"]
            if is_gist_id(id) && valid_login(owner) =>
        {
            Route::Gist {
                owner: Some((*owner).to_owned()),
                id: (*id).to_owned(),
            }
        }
        _ => return None,
    })
}
/// A discussions page from what follows `/discussions` in its URL.
fn discussions(of: DiscussionsOf, rest: &[&str]) -> Option<Route> {
    Some(match rest {
        [] => Route::Discussions { of, category: None },
        ["categories", slug] if valid_slug(slug) => Route::Discussions {
            of,
            category: Some((*slug).to_owned()),
        },
        [n] => Route::Discussion {
            of,
            number: n.parse().ok()?,
        },
        _ => return None,
    })
}

/// An owner's projects page, from what follows `/projects` in its URL: the
/// list, a project, one of its views, or an item in a view's side pane.
/// Its settings, insights and workflows are on GitHub.
fn project(owner: ProjectOwner, rest: &[&str], url: &Url<'_>) -> Option<Route> {
    let [number, rest @ ..] = rest else {
        return Some(Route::Projects(ProjectsOf::Owner(owner)));
    };
    let project = ProjectRef {
        owner,
        number: number.parse().ok()?,
    };
    let view = match rest {
        [] => None,
        ["views", n] => Some(n.parse().ok()?),
        _ => return None,
    };
    Some(match url.param("itemId") {
        Some(item) => Route::ProjectItem {
            project,
            view,
            item: item.parse().ok()?,
        },
        None => Route::Project { project, view },
    })
}

/// A project's view's URL (the project's, without one).
fn view_url(project: &ProjectRef, view: Option<u64>) -> String {
    match view {
        Some(n) => format!("{}/views/{n}", project.url()),
        None => project.url(),
    }
}

fn discussions_url(of: &DiscussionsOf) -> String {
    match of {
        DiscussionsOf::Repo(repo) => format!("{}/discussions", links::repo(repo)),
        DiscussionsOf::Org(org) => format!("{}/orgs/{org}/discussions", links::base()),
    }
}

fn of_title(of: &DiscussionsOf) -> String {
    match of {
        DiscussionsOf::Repo(repo) => repo.to_string(),
        DiscussionsOf::Org(org) => format!("@{org}"),
    }
}

/// `step:3:12`, as GitHub links a line of a job's log.
fn step_line(fragment: &str) -> Option<(u32, u32)> {
    let mut parts = fragment.strip_prefix("step:")?.split(':');
    let step = parts.next()?.parse().ok()?;
    let line = parts.next().and_then(|l| l.parse().ok()).unwrap_or(1);
    Some((step, line))
}

/// Top-level GitHub paths that aren't users.
const RESERVED: &[&str] = &[
    "about",
    "account",
    "advisories",
    "apps",
    "codespaces",
    "collections",
    "contact",
    "copilot",
    "customer-stories",
    "enterprise",
    "enterprises",
    "events",
    "explore",
    "features",
    "github-copilot",
    "issues",
    "join",
    "licenses",
    "login",
    "logout",
    "marketplace",
    "mcp",
    "new",
    "notifications",
    "open-source",
    "organizations",
    "partners",
    "pricing",
    "pulls",
    "readme",
    "repos",
    "resources",
    "security",
    "sessions",
    "settings",
    "signup",
    "site",
    "solutions",
    "sponsors",
    "stars",
    "team",
    "topics",
    "trending",
    "trust-center",
    "user-attachments",
    "users",
    "why-github",
];

fn decode(segment: &str) -> String {
    let mut out = Vec::with_capacity(segment.len());
    let mut rest = segment.as_bytes();
    while let Some((&b, tail)) = rest.split_first() {
        let escaped = match tail {
            [hi, lo, ..] if b == b'%' => std::str::from_utf8(&[*hi, *lo])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok()),
            _ => None,
        };
        match (escaped, tail.get(2..)) {
            (Some(byte), Some(after)) => {
                out.push(byte);
                rest = after;
            }
            _ => {
                out.push(b);
                rest = tail;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// What the command palette makes of its input. A web link ghtui
/// doesn't show (another GitHub's, say) is the browser's.
pub fn parse_input(input: &str, context: Option<&RepoId>) -> Option<Dest> {
    let input = input.trim();
    let url = |url: &str| {
        let dest = Dest::from_url(url);
        let web = url.starts_with("https://") || url.starts_with("http://");
        (web || !matches!(dest.target, Target::External(_))).then_some(dest)
    };
    if input.contains("://") || has_host_prefix(input) {
        return url(input);
    }
    // A path as GitHub writes it, without the host: `o/r/pull/5/files`.
    let path = input.trim_start_matches('/');
    if path.split('/').count() > 2 && !path.contains(char::is_whitespace) {
        return url(&format!("{}/{path}", crate::host::current().web()));
    }
    if let Some(login) = input.strip_prefix('@') {
        return valid_login(login).then(|| Target::Page(Route::user(login)).into());
    }
    let issue = |repo: RepoId, n: &str| {
        let number = n.trim_start_matches('#').parse().ok()?;
        Some(Target::Page(Route::Issues(issues::Issue { repo, number })).into())
    };
    if let Some((repo, n)) = input.split_once('#')
        && !repo.is_empty()
    {
        return issue(RepoId::parse(repo)?, n);
    }
    if input
        .trim_start_matches('#')
        .chars()
        .all(|c| c.is_ascii_digit())
    {
        return issue(context?.clone(), input);
    }
    if input.contains('/') {
        return RepoId::parse(input).map(|r| Target::Page(Route::Code(code::Repo(r))).into());
    }
    None
}

/// Whether `s` is one of GitHub's own first path segments (settings,
/// the marketplace…), never an owner.
pub(crate) fn is_reserved(s: &str) -> bool {
    RESERVED.contains(&s)
}

pub(crate) fn valid_login(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// A revision as git allows ref names and SHAs to be written (roughly
/// `git check-ref-format`): no spaces or control characters, none of
/// `~^:?*[\`, no `..` or `@{`, and no `/`-separated part that is empty,
/// starts with `.` or ends with `.lock`.
fn valid_rev(s: &str) -> bool {
    s.split('/')
        .all(|c| !c.is_empty() && !c.starts_with('.') && !c.ends_with(".lock"))
        && !s.contains("..")
        && !s.contains("@{")
        && !s.ends_with('.')
        && !s
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || "~^:?*[\\".contains(c))
}

/// A security advisory's ID: `GHSA-xxxx-xxxx-xxxx`.
fn valid_ghsa(s: &str) -> bool {
    s.strip_prefix("GHSA-").is_some_and(|rest| {
        !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

/// A team's slug: its name, lowercased, its other characters dashes.
fn valid_slug(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Strategies for the parts of routes, for the families' own.
#[cfg(test)]
pub(crate) mod arb {
    use crate::api::model::RepoId;
    use proptest::prelude::*;

    pub(crate) fn segment() -> impl Strategy<Value = String> {
        "[A-Za-z0-9][A-Za-z0-9_.-]{0,15}".prop_filter("not . or .. or .git", |s| {
            s != "." && s != ".." && !s.ends_with(".git")
        })
    }

    /// A user's or organization's name, as GitHub allows them.
    pub(crate) fn login() -> impl Strategy<Value = String> {
        "[A-Za-z0-9][A-Za-z0-9-]{0,15}"
    }

    pub(crate) fn repo() -> impl Strategy<Value = RepoId> {
        (segment(), segment()).prop_map(|(o, r)| RepoId::new(o, r))
    }

    /// Any text a path segment can hold, spaces and non-ASCII included
    /// (git has no `.` or `..` segments).
    pub(crate) fn path() -> impl Strategy<Value = String> {
        let part = "[^/\\x00-\\x1f]{1,12}".prop_filter("not . or ..", |p| p != "." && p != "..");
        prop::collection::vec(part, 0..4).prop_map(|parts| parts.join("/"))
    }

    /// Filter words (an `is:` word would be dropped by design).
    pub(crate) fn query() -> impl Strategy<Value = String> {
        prop::collection::vec("[a-z0-9:#&+%\"]{1,8}", 1..4)
            .prop_map(|words| words.join(" "))
            .prop_filter("no is: words", |q| {
                !q.split(' ').any(|w| w.starts_with("is:"))
            })
    }

    /// A valid git ref name, of one to three segments (`feature/x`), with
    /// characters a URL must escape (`fix#1`, `50%`, `é`).
    pub(crate) fn rev() -> impl Strategy<Value = String> + Clone {
        prop::collection::vec("[A-Za-z0-9_#%+@&;é-][A-Za-z0-9._#%+@&;é-]{0,7}", 1..=3)
            .prop_map(|s| s.join("/"))
            .prop_filter("valid ref name", |r| {
                // git's rules, written apart from `valid_rev` so the round
                // trip catches it rejecting a legal name.
                !r.contains("..")
                    && !r.ends_with('.')
                    && !r.split('/').any(|c| c.ends_with(".lock"))
            })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::git::Oid;

    /// Names GitHub can't have aren't pages (found by fuzzing: a user
    /// named `https:` and NULs opened, and its URL led elsewhere).
    #[test]
    fn impossible_names_open_the_browser() {
        for url in [
            "https://github.com/https:%00%00/",
            "https://github.com/a%20b",
            "https://github.com/stars/a:b",
            "https://github.com/orgs/a%2Fb/teams",
            "https://github.com/orgs/a/teams/b%20c",
            "https://gist.github.com/a:b",
            "https://github.com/o/r/commit/o%20[[x]]%20",
            "https://github.com/o/r/commit/a..b",
            "https://github.com/o/r/labels/a%0Bb",
        ] {
            assert!(
                matches!(Dest::from_url(url).target, Target::External(_)),
                "{url}"
            );
        }
        assert_eq!(page("https://github.com/octo-cat"), Route::user("octo-cat"));
        assert!(matches!(
            page("https://github.com/o/r/commit/main"),
            Route::Code(code::Commit { .. })
        ));
    }

    /// Each name a URL holds is checked as what it names (pinned after
    /// mutation testing found the checks untested).
    #[test]
    fn names_in_urls_are_checked() {
        let sha = "0123456789abcdef0123456789abcdef01234567";
        for (url, opens) in [
            (
                "https://github.com/advisories/GHSA-abcd-efgh-2345".to_owned(),
                true,
            ),
            ("https://github.com/advisories/GHSB-1".to_owned(), false),
            ("https://github.com/advisories/GHSA-".to_owned(), false),
            (
                "https://github.com/o/r/security/advisories/GHSA-a%20b".to_owned(),
                false,
            ),
            (
                "https://github.com/o/r/blame/main/src/a.rs".to_owned(),
                true,
            ),
            (
                "https://github.com/o/r/blame/a..b/src/a.rs".to_owned(),
                false,
            ),
            ("https://github.com/o/r/blame/main".to_owned(), false),
            ("https://github.com/o/r/wiki/Some%20Page".to_owned(), true),
            ("https://github.com/o/r/wiki/a%01b".to_owned(), false),
            (
                "https://github.com/o/r/actions/workflows/ci.yml".to_owned(),
                true,
            ),
            (
                "https://github.com/o/r/actions/workflows/c%20i.yml".to_owned(),
                false,
            ),
            (
                "https://github.com/o/r/deployments/staging%20eu".to_owned(),
                true,
            ),
            ("https://github.com/o/r/deployments/a%01".to_owned(), false),
            ("https://github.com/o/r/commits/main/src".to_owned(), true),
            ("https://github.com/o/r/commits/a@{1}".to_owned(), false),
            (format!("https://github.com/o/r/pull/1/commits/{sha}"), true),
            (
                "https://github.com/o/r/discussions/categories/q-a".to_owned(),
                true,
            ),
            (
                "https://github.com/o/r/discussions/categories/q%20a".to_owned(),
                false,
            ),
            (
                "https://gist.github.com/octocat/0123456789abcdef0123".to_owned(),
                true,
            ),
            (
                "https://gist.github.com/octo.cat/0123456789abcdef0123".to_owned(),
                false,
            ),
            (
                "https://gist.github.com/octocat/0123456789abcdef0123/revisions".to_owned(),
                true,
            ),
            (
                "https://gist.github.com/oc%20t/0123456789abcdef0123/revisions".to_owned(),
                false,
            ),
        ] {
            let target = Dest::from_url(&url).target;
            assert_eq!(
                matches!(target, Target::Page(_)),
                opens,
                "{url}: {target:?}"
            );
        }
        // Not a commit under a PR: the PR's commits.
        assert!(matches!(
            Dest::from_url("https://github.com/o/r/pull/1/commits/a%20b").target,
            Target::Page(Route::Pulls(pulls::Pr {
                tab: PrTab::Commits,
                ..
            }))
        ));
        // A full SHA's files open the diff; a short one, its page.
        assert!(matches!(
            Dest::from_url(&format!("https://github.com/o/r/commit/{sha}#diff-1")).target,
            Target::Files(_)
        ));
        assert!(matches!(
            Dest::from_url("https://github.com/o/r/commit/0123456#diff-1").target,
            Target::Page(Route::Code(code::Commit { .. }))
        ));
        assert!(!valid_rev("a b") && !valid_rev("a~1") && !valid_rev("a.") && valid_rev("v1.2/x"));
        // A page at a revision git can't name opens the browser.
        for view in ["tree", "blob", "blame", "commits"] {
            for rev in ["a..b", "a~1", "%2E%2Fx", "a%2F%2Fb", "%2F", "x.lock%2Fy"] {
                let url = format!("https://github.com/o/r/{view}/{rev}/src/lib.rs");
                assert!(
                    matches!(Dest::from_url(&url).target, Target::External(_)),
                    "{url}"
                );
            }
        }
        assert!(valid_ghsa("GHSA-2345") && !valid_ghsa("ghsa-1") && !valid_ghsa("GHSA-1 2"));
    }

    /// What `route`'s URL opens, once GitHub says where its ref ends.
    fn reopened(route: &Route) -> Target {
        match Dest::from_url(&route.url()).target {
            Target::Page(r) => Target::Page(r.split(route.rev()).unwrap_or(r)),
            target => target,
        }
    }

    fn page(url: &str) -> Route {
        match Dest::from_url(url).target {
            Target::Page(route) => route,
            other => panic!("{url}: {other:?}"),
        }
    }

    #[test]
    fn parses_github_urls() {
        let repo = RepoId::new("o", "r");
        assert_eq!(page("https://github.com/"), Route::App(app::Home));
        assert_eq!(page("https://github.com/octocat"), Route::user("octocat"));
        assert_eq!(
            page("https://github.com/o/r"),
            Route::Code(code::Repo(repo.clone()))
        );
        assert_eq!(
            page("https://github.com/o/r/tree/main/src/ui").split(Some("main")),
            Some(Route::Code(code::Tree {
                repo: repo.clone(),
                rev: "main".into(),
                path: "src/ui".into()
            }))
        );
        assert_eq!(
            page("https://github.com/o/r/blob/v1.0/a%20b.md#readme").split(None),
            Some(Route::blob(repo.clone(), "v1.0".into(), "a b.md".into()))
        );
        for (fragment, lines) in [("L7", (7, 7)), ("L20-L10", (10, 20)), ("L0", (0, 0))] {
            let url = format!("https://github.com/o/r/blob/main/a.rs#{fragment}");
            let expected = (lines.0 > 0).then_some(lines);
            assert!(
                matches!(page(&url).split(None), Some(Route::Code(code::Blob { lines, .. })) if lines == expected),
                "{fragment}"
            );
        }
        assert_eq!(
            page("https://github.com/o/r/issues?q=is%3Aissue+is%3Aclosed"),
            Route::Issues(issues::Issues {
                repo: repo.clone(),
                query: "is:closed".into()
            })
        );
        assert_eq!(
            page("https://github.com/o/r/pulls"),
            Route::Pulls(pulls::Pulls {
                repo: repo.clone(),
                query: OPEN.into()
            })
        );
        assert_eq!(
            page("https://github.com/o/r/issues/5"),
            Route::Issues(issues::Issue {
                repo: repo.clone(),
                number: 5
            })
        );
        let pr = PrRef { repo, number: 7 };
        assert_eq!(
            page("https://github.com/o/r/pull/7/commits"),
            Route::Pulls(pulls::Pr {
                pr: pr.clone(),
                tab: PrTab::Commits
            })
        );
        assert_eq!(
            Dest::from_url("https://github.com/o/r/pull/7/files").target,
            Target::Files(DiffOf::Pr(pr.clone()))
        );
        let sha = "0123456789abcdef0123456789abcdef01234567";
        let commit = |oid: &str| {
            Route::Code(code::Commit {
                repo: pr.repo.clone(),
                oid: oid.into(),
                pr: None,
            })
        };
        assert_eq!(
            page(&format!("https://github.com/o/r/commit/{sha}")),
            commit(sha)
        );
        let commits = |rev: &str, path: &str| {
            Route::Code(code::Commits {
                repo: pr.repo.clone(),
                rev: rev.into(),
                path: path.into(),
            })
        };
        assert_eq!(
            page("https://github.com/o/r/commits/main/src/ui").split(None),
            Some(commits("main", "src/ui"))
        );
        assert_eq!(page("https://github.com/o/r/commits"), commits("HEAD", ""));
        assert_eq!(
            page("https://github.com/o/r/actions"),
            Route::actions(pr.repo.clone())
        );
        assert_eq!(
            page("https://github.com/o/r/actions?query=branch%3Amain+is%3Afailure"),
            Route::Actions {
                repo: pr.repo.clone(),
                query: "branch:main is:failure".into()
            }
        );
        assert_eq!(
            page("https://github.com/o/r/stargazers"),
            Route::Stargazers(pr.repo.clone())
        );
        assert_eq!(
            page("https://github.com/o/r/network/members"),
            Route::Forks(pr.repo.clone())
        );
        assert_eq!(
            page("https://github.com/o/r/releases/tag/v1.0/rc"),
            Route::Release {
                repo: pr.repo.clone(),
                tag: "v1.0/rc".into()
            }
        );
        assert_eq!(
            page("https://github.com/o/r/releases/latest"),
            Route::Releases(pr.repo.clone())
        );
        assert_eq!(
            page("https://github.com/o/r/tags"),
            Route::Code(code::Tags(pr.repo.clone()))
        );
        assert!(matches!(
            Dest::from_url("https://github.com/o/r/releases/download/v1/x.tgz").target,
            Target::External(_)
        ));
        assert_eq!(
            page("https://github.com/o/r/pull/7/checks"),
            Route::Pulls(pulls::Pr {
                pr: pr.clone(),
                tab: PrTab::Checks
            })
        );
        assert_eq!(
            page("https://github.com/o/r/commit/0123abc"),
            commit("0123abc")
        );
        // A pull request's commit knows it.
        assert_eq!(
            page(&format!("https://github.com/o/r/pull/7/commits/{sha}")),
            Route::Code(code::Commit {
                repo: pr.repo.clone(),
                oid: sha.into(),
                pr: Some(7),
            })
        );
        assert_eq!(
            Dest::from_url(&format!("https://github.com/o/r/commit/{sha}#diff-abc")).target,
            Target::Files(DiffOf::Commit(pr.repo.clone(), Oid::parse(sha).unwrap()))
        );
        assert_eq!(
            page("https://github.com/search?q=tui+lang%3Arust&type=repositories"),
            Route::Search {
                kind: SearchKind::Repos,
                query: "tui lang:rust".into()
            }
        );
        assert_eq!(
            page("https://github.com/o/r/labels/good%20first%20issue"),
            Route::Issues(issues::Issues {
                repo: RepoId::new("o", "r"),
                query: "is:open label:\"good first issue\"".into()
            })
        );
        assert_eq!(
            page("https://github.com/o/r/labels/bug"),
            Route::Issues(issues::Issues {
                repo: RepoId::new("o", "r"),
                query: "is:open label:bug".into()
            })
        );
        assert_eq!(
            page("https://github.com/octocat?tab=repositories&sort=stargazers"),
            Route::User {
                login: "octocat".into(),
                tab: ProfileTab::Repositories(RepoSort::Stars)
            }
        );
        assert_eq!(
            page("https://github.com/octocat?tab=followers"),
            Route::User {
                login: "octocat".into(),
                tab: ProfileTab::Followers
            }
        );
        assert_eq!(
            page("https://github.com/orgs/rust-lang/people"),
            Route::User {
                login: "rust-lang".into(),
                tab: ProfileTab::People
            }
        );
        assert_eq!(
            page("https://github.com/orgs/rust-lang/repositories"),
            Route::User {
                login: "rust-lang".into(),
                tab: ProfileTab::Repositories(RepoSort::Updated)
            }
        );
        assert_eq!(
            page("https://github.com/o/r/actions/runs/9"),
            Route::WorkflowRun {
                repo: pr.repo.clone(),
                run: 9,
                attempt: None
            }
        );
        assert_eq!(
            page("https://github.com/o/r/milestone/3"),
            Route::Issues(issues::Milestone {
                repo: pr.repo.clone(),
                number: 3
            })
        );
        assert_eq!(
            page("https://github.com/o/r/compare/v1...fork:feature/x"),
            Route::Code(code::Compare {
                repo: pr.repo.clone(),
                spec: "v1...fork:feature/x".into()
            })
        );
        let other = "f".repeat(40);
        assert_eq!(
            Dest::from_url(&format!(
                "https://github.com/o/r/compare/{sha}..{other}#files"
            ))
            .target,
            Target::Files(DiffOf::Range(
                pr.repo.clone(),
                Oid::parse(sha).unwrap(),
                Oid::parse(&other).unwrap()
            ))
        );
        // Three dots diff from the merge base, which the comparison finds.
        assert_eq!(
            page(&format!(
                "https://github.com/o/r/compare/{sha}...{other}#files"
            )),
            Route::Code(code::Compare {
                repo: pr.repo.clone(),
                spec: format!("{sha}...{other}")
            })
        );
        // A diff's link leads back to it, and a link to its lines to them.
        let (from, to) = (Oid::parse(sha).unwrap(), Oid::parse(&other).unwrap());
        let line = |side, line| crate::diff::anchor::LinePos { side, line };
        let lines = (
            line(crate::diff::anchor::Side::Left, 3),
            line(crate::diff::anchor::Side::Right, 5),
        );
        let anchor = format!("diff-{}L3-R5", crate::diff_screen::path_hash("a.rs"));
        for of in [
            DiffOf::Pr(pr.clone()),
            DiffOf::Commit(pr.repo.clone(), from.clone()),
            DiffOf::Range(pr.repo.clone(), from, to),
        ] {
            assert_eq!(Dest::from_url(&of.url()).target, Target::Files(of.clone()));
            let dest = Dest::from_url(&of.file_url("a.rs", Some(lines)));
            assert_eq!(dest.target, Target::Files(of.clone()));
            assert_eq!(dest.anchor.as_deref(), Some(anchor.as_str()));
        }
        // A topic search without a topic is an empty search, not `topic:`.
        assert_eq!(
            page("https://github.com/search?type=topics"),
            Route::Search {
                kind: SearchKind::Repos,
                query: String::new()
            }
        );
        // A short number alone is someone's gists; a long one, an old gist.
        assert_eq!(
            page("https://gist.github.com/2000"),
            Route::Gists("2000".into())
        );
        assert_eq!(
            page("https://gist.github.com/1162032"),
            Route::Gist {
                owner: None,
                id: "1162032".into()
            }
        );
        // A repository (or owner) named `blob` keeps its blame.
        let blame = Route::Code(code::Blame {
            repo: RepoId::new("blob", "blob"),
            rev: "main".into(),
            path: "f".into(),
            lines: None,
        });
        assert_eq!(reopened(&blame), Target::Page(blame));
        assert_eq!(
            page("https://github.com/notifications"),
            Route::Notifications {
                query: String::new()
            }
        );
        assert_eq!(
            page("https://github.com/notifications?query=is%3Aunread+repo%3Ao%2Fr"),
            Route::Notifications {
                query: "is:unread repo:o/r".into()
            }
        );
        assert!(matches!(
            Dest::from_url("https://github.com/settings/tokens").target,
            Target::External(_)
        ));
        assert!(matches!(
            Dest::from_url("https://example.com/o/r").target,
            Target::External(_)
        ));
    }

    /// Why a github.com link may open the browser (see the corpus).
    pub(crate) const REASONS: [&str; 6] = [
        "download",
        "session",
        "scope",
        "not-github",
        "no-api",
        "rare",
    ];

    /// The corpus in `tests/github_urls.txt`: every kind of GitHub link,
    /// and what it must open.
    pub(crate) fn corpus() -> Vec<(&'static str, &'static str, Option<&'static str>)> {
        include_str!("../tests/github_urls.txt")
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|line| {
                let mut words = line.split_whitespace();
                let url = words.next().unwrap();
                let expect = words
                    .next()
                    .unwrap_or_else(|| panic!("no expectation: {line}"));
                (url, expect, words.next())
            })
            .collect()
    }

    /// A route's variant, as the corpus names it.
    pub(crate) fn variant(route: &Route) -> String {
        // An unsplit page opens its view's page.
        let route = route.split(None).unwrap_or_else(|| route.clone());
        let named = format!("{route:?}");
        let word = |s: &str| {
            let word = s.split(|c: char| !c.is_alphanumeric()).next();
            word.unwrap_or_default().to_owned()
        };
        // A family's page by its own name.
        let family = word(&named);
        match named.strip_prefix(&format!("{family}(")) {
            Some(page) if ["App", "Code", "Issues", "Pulls", "Site"].contains(&family.as_str()) => {
                word(page)
            }
            _ => family,
        }
    }

    /// Whether `target` is what the corpus says (`todo` pages aren't built
    /// yet, so they still open the browser).
    pub(crate) fn as_expected(target: &Target, expect: &str, reason: Option<&str>) -> bool {
        match (expect, target) {
            ("external", Target::External(_)) => reason.is_some_and(|r| REASONS.contains(&r)),
            ("todo", Target::External(_)) | ("files", Target::Files(_)) => true,
            (variant_name, Target::Page(route)) => variant(route) == variant_name,
            _ => false,
        }
    }

    #[test]
    fn every_github_link_does_what_the_corpus_says() {
        let mut wrong = Vec::new();
        for (url, expect, reason) in corpus() {
            let target = Dest::from_url(url).target;
            if !as_expected(&target, expect, reason) {
                wrong.push(format!(
                    "{url}\n    expected {expect} {}, got {target:?}",
                    reason.unwrap_or("")
                ));
            }
        }
        assert!(
            wrong.is_empty(),
            "{} wrong:\n{}",
            wrong.len(),
            wrong.join("\n")
        );
    }

    /// "Not built yet" isn't a reason: every link opens a page, the diff
    /// viewer, or the browser for a reason the corpus states.
    #[test]
    fn no_link_waits_for_a_page() {
        let todo: Vec<&str> = corpus()
            .iter()
            .filter(|(_, expect, _)| *expect == "todo")
            .map(|(url, _, _)| *url)
            .collect();
        assert!(
            todo.is_empty(),
            "pages still to build:\n{}",
            todo.join("\n")
        );
    }

    #[test]
    fn routes_round_trip_through_urls() {
        let repo = RepoId::new("o", "r");
        for route in [
            Route::App(app::Home),
            Route::Code(code::Repo(repo.clone())),
            Route::Code(code::Tree {
                repo: repo.clone(),
                rev: "main".into(),
                path: "src".into(),
            }),
            Route::Code(code::Blob {
                repo: repo.clone(),
                rev: "main".into(),
                path: "src/lib.rs".into(),
                lines: Some((3, 9)),
            }),
            Route::Issues(issues::Issues {
                repo: repo.clone(),
                query: "is:closed label:bug".into(),
            }),
            Route::Pulls(pulls::Pulls {
                repo: repo.clone(),
                query: OPEN.into(),
            }),
            Route::Code(code::Commit {
                repo: repo.clone(),
                oid: "abc1234".into(),
                pr: None,
            }),
            Route::Issues(issues::Issue { repo, number: 3 }),
            Route::user("octocat"),
            Route::Search {
                kind: SearchKind::Users,
                query: "linus".into(),
            },
            Route::Search {
                kind: SearchKind::Pulls,
                query: "author:linus".into(),
            },
            Route::User {
                login: "octocat".into(),
                tab: ProfileTab::Stars,
            },
        ] {
            assert_eq!(reopened(&route), Target::Page(route));
        }
    }

    /// On an Enterprise Server, its links do what github.com's do (gists
    /// are under `/gist`), its pages link to it, and github.com's links
    /// open in the browser.
    #[test]
    fn an_enterprise_host_owns_its_links() {
        let _host = crate::host::set_for_test("ghe.example.com");
        let mut wrong = Vec::new();
        for (url, expect, reason) in corpus() {
            let moved = url
                .replace("https://gist.github.com/", "https://ghe.example.com/gist/")
                .replace("https://github.com/", "https://ghe.example.com/");
            if moved == url {
                continue;
            }
            let target = Dest::from_url(&moved).target;
            if !as_expected(&target, expect, reason) {
                wrong.push(format!("{moved}\n    expected {expect}, got {target:?}"));
            }
            if !matches!(Dest::from_url(url).target, Target::External(_)) {
                wrong.push(format!("{url}\n    expected the browser"));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));

        let repo = Route::Code(code::Repo(RepoId::new("o", "r")));
        assert_eq!(repo.url(), "https://ghe.example.com/o/r");
        let gist = Route::Gist {
            owner: Some("o".into()),
            id: "1162032".into(),
        };
        assert_eq!(gist.url(), "https://ghe.example.com/gist/o/1162032");
        assert_eq!(reopened(&gist), Target::Page(gist));
        assert_eq!(
            parse_input("ghe.example.com/o/r", None).map(|d| d.target),
            Some(Target::Page(repo))
        );
    }

    /// A host with a port owns its links, port and all.
    #[test]
    fn a_host_with_a_port_owns_its_links() {
        let _host = crate::host::set_for_test("ghe.example.com:8443");
        let repo = Target::Page(Route::Code(code::Repo(RepoId::new("o", "r"))));
        assert_eq!(
            Dest::from_url("https://ghe.example.com:8443/o/r").target,
            repo
        );
        assert_eq!(parse_input("o/r/pull/5/files", None).map(|d| d.target), {
            let pr = crate::api::model::PrRef {
                repo: RepoId::new("o", "r"),
                number: 5,
            };
            Some(Target::Files(DiffOf::Pr(pr)))
        });
        let other = Dest::from_url("https://ghe.example.com/o/r").target;
        assert!(matches!(other, Target::External(_)), "{other:?}");
    }

    /// Projects open from their owner's, their own and their views'
    /// URLs, and an item from a view's side pane; each URL leads back.
    #[test]
    fn projects_open_from_their_urls() {
        let org = ProjectRef {
            owner: ProjectOwner::Org("octo-org".into()),
            number: 5,
        };
        let user = ProjectRef {
            owner: ProjectOwner::User("octocat".into()),
            number: 3,
        };
        let cases = [
            (
                "https://github.com/orgs/octo-org/projects",
                Route::Projects(ProjectsOf::Owner(org.owner.clone())),
            ),
            (
                "https://github.com/users/octocat/projects",
                Route::Projects(ProjectsOf::Owner(user.owner.clone())),
            ),
            (
                "https://github.com/octocat?tab=projects",
                Route::Projects(ProjectsOf::Owner(user.owner.clone())),
            ),
            (
                "https://github.com/o/r/projects",
                Route::Projects(ProjectsOf::Repo(RepoId::new("o", "r"))),
            ),
            (
                "https://github.com/orgs/octo-org/projects/5",
                Route::Project {
                    project: org.clone(),
                    view: None,
                },
            ),
            (
                "https://github.com/users/octocat/projects/3/views/2",
                Route::Project {
                    project: user,
                    view: Some(2),
                },
            ),
            (
                "https://github.com/orgs/octo-org/projects/5/views/1?pane=issue&itemId=42&issue=o%7Cr%7C1",
                Route::ProjectItem {
                    project: org,
                    view: Some(1),
                    item: 42,
                },
            ),
        ];
        for (url, route) in cases {
            assert_eq!(page(url), route, "{url}");
            assert_eq!(reopened(&route), Target::Page(route), "{url}");
        }
        for url in [
            "https://github.com/orgs/octo-org/projects/5/settings",
            "https://github.com/orgs/octo-org/projects/x",
            "https://github.com/o/r/projects/1",
        ] {
            assert!(
                matches!(Dest::from_url(url).target, Target::External(_)),
                "{url}"
            );
        }
    }

    /// A ref with slashes in it (`feature/x`, `dependabot/cargo/serde-1.0`,
    /// a `release/1.0` tag) is one revision: the page's URL leads back to
    /// that revision, not to its first segment with the rest as a path.
    #[test]
    fn refs_with_slashes_round_trip_through_urls() {
        let repo = RepoId::new("o", "r");
        let mut wrong = Vec::new();
        for rev in ["feature/x", "dependabot/cargo/serde-1.0", "release/1.0"] {
            for route in [
                Route::Code(code::Tree {
                    repo: repo.clone(),
                    rev: rev.into(),
                    path: String::new(),
                }),
                Route::Code(code::Tree {
                    repo: repo.clone(),
                    rev: rev.into(),
                    path: "src".into(),
                }),
                Route::blob(repo.clone(), rev.into(), "src/lib.rs".into()),
                Route::Code(code::Blame {
                    repo: repo.clone(),
                    rev: rev.into(),
                    path: "src/lib.rs".into(),
                    lines: None,
                }),
                Route::Code(code::Commits {
                    repo: repo.clone(),
                    rev: rev.into(),
                    path: String::new(),
                }),
                Route::Code(code::Commits {
                    repo: repo.clone(),
                    rev: rev.into(),
                    path: "src".into(),
                }),
            ] {
                let url = route.url();
                let back = Dest::from_url(&url).target;
                if back != Target::Page(route.clone()) {
                    wrong.push(format!(
                        "{url}\n    expected {route:?}\n    got      {back:?}"
                    ));
                }
            }
        }
        assert!(
            wrong.is_empty(),
            "{} URLs lose their ref:\n{}",
            wrong.len(),
            wrong.join("\n")
        );
    }

    /// A commit named by a slashed ref (`commit/feature%2Fx`) and its
    /// checks lead back to that ref.
    #[test]
    fn commits_named_by_slashed_refs_round_trip() {
        let (repo, oid) = (RepoId::new("o", "r"), "feature/x".to_owned());
        for route in [
            Route::Code(code::Commit {
                repo: repo.clone(),
                oid: oid.clone(),
                pr: None,
            }),
            Route::Code(code::CommitChecks { repo, oid }),
        ] {
            let url = route.url();
            assert_eq!(Dest::from_url(&url).target, Target::Page(route), "{url}");
        }
    }

    /// A plain URL (`tree/feature/x/src`, as GitHub and `gh browse` write
    /// them) doesn't say where its ref ends: the page waits for GitHub to
    /// name the longest leading part that is a branch or tag, and is at
    /// the first segment when none longer is. Escaped refs, `HEAD` and
    /// full SHAs need no asking.
    #[test]
    fn plain_urls_wait_for_where_the_ref_ends() {
        let repo = RepoId::new("o", "r");
        let tree = |rev: &str, path: &str| {
            Route::Code(code::Tree {
                repo: repo.clone(),
                rev: rev.into(),
                path: path.into(),
            })
        };
        let split = |url: &str, rev: Option<&str>| {
            let route = page(&format!("https://github.com/o/r/{url}"));
            assert!(
                matches!(route, Route::Code(code::Unsplit { .. })),
                "{url}: {route:?}"
            );
            route.split(rev)
        };
        assert_eq!(
            split("tree/feature/x/src", Some("feature/x")),
            Some(tree("feature/x", "src"))
        );
        assert_eq!(
            split("tree/feature/x/src", None),
            Some(tree("feature", "x/src"))
        );
        assert_eq!(
            split("tree/release/1.0", Some("release/1.0")),
            Some(tree("release/1.0", ""))
        );
        assert_eq!(
            split("blob/deep/a/b/c/src/lib.rs#L3", Some("deep/a/b/c")),
            Some(Route::Code(code::Blob {
                repo: repo.clone(),
                rev: "deep/a/b/c".into(),
                path: "src/lib.rs".into(),
                lines: Some((3, 3)),
            }))
        );
        // A name that isn't where a segment ends is no answer.
        assert_eq!(
            split("tree/feature/xy", Some("feature/x")),
            Some(tree("feature", "xy"))
        );
        // Blame needs a file.
        assert!(matches!(
            split("blame/feature/x", Some("feature/x")),
            Some(Route::Code(code::Blame { rev, path, .. })) if rev == "feature" && path == "x"
        ));
        let sha = "0123456789abcdef0123456789abcdef01234567";
        for (url, rev, path) in [
            ("tree/feature%2Fx/src".to_owned(), "feature/x", "src"),
            ("tree/HEAD/src".to_owned(), "HEAD", "src"),
            (format!("tree/{sha}/src"), sha, "src"),
            ("tree/feature".to_owned(), "feature", ""),
        ] {
            assert_eq!(
                page(&format!("https://github.com/o/r/{url}")),
                tree(rev, path),
                "{url}"
            );
        }
    }

    /// The links the branch and tag lists put on a ref (`url::tree(repo,
    /// name, "")`) open that ref.
    #[test]
    fn branch_links_open_the_branch() {
        let repo = RepoId::new("o", "r");
        let url = crate::ui::pages::url::tree(&repo, "dependabot/cargo/serde-1.0", "");
        assert_eq!(
            Dest::from_url(&url).target,
            Target::Page(Route::Code(code::Tree {
                repo,
                rev: "dependabot/cargo/serde-1.0".into(),
                path: String::new(),
            })),
            "{url}"
        );
    }

    #[test]
    fn palette_input() {
        let repo = RepoId::new("o", "r");
        let target = |input, context| parse_input(input, context).map(|d| d.target);
        assert_eq!(
            target("a/b", None),
            Some(Target::Page(Route::Code(code::Repo(RepoId::new("a", "b")))))
        );
        assert_eq!(
            target("@octocat", None),
            Some(Target::Page(Route::user("octocat")))
        );
        assert_eq!(
            target("#12", Some(&repo)),
            Some(Target::Page(Route::Issues(issues::Issue {
                repo: repo.clone(),
                number: 12
            })))
        );
        assert_eq!(target("12", None), None);
        assert_eq!(
            target("a/b#3", None),
            Some(Target::Page(Route::Issues(issues::Issue {
                repo: RepoId::new("a", "b"),
                number: 3
            })))
        );
        assert_eq!(
            target("a/b/pull/5/files", None),
            Some(Target::Files(DiffOf::Pr(crate::api::model::PrRef {
                repo: RepoId::new("a", "b"),
                number: 5
            })))
        );
        assert_eq!(parse_input("ratatui", None), None);
        assert_eq!(
            target("https://example.com/a", None),
            Some(Target::External("https://example.com/a".into()))
        );
        assert_eq!(parse_input("ftp://example.com", None), None);
    }

    #[test]
    fn list_queries_scope_to_the_repo() {
        let route = Route::Pulls(pulls::Pulls {
            repo: RepoId::new("o", "r"),
            query: "is:open author:me".into(),
        });
        assert_eq!(
            route.search(),
            Some((
                SearchKind::Issues,
                "repo:o/r is:pr is:open author:me sort:created-desc".into()
            ))
        );
    }

    mod props {
        use super::*;
        use crate::route::arb::*;
        use proptest::prelude::*;

        fn discussions_of() -> impl Strategy<Value = DiscussionsOf> {
            prop_oneof![
                repo().prop_map(DiscussionsOf::Repo),
                login().prop_map(DiscussionsOf::Org),
            ]
        }

        fn route() -> impl Strategy<Value = Route> {
            prop_oneof![
                app::arb(),
                prop_oneof![Just(String::new()), query()]
                    .prop_map(|query| Route::Notifications { query }),
                (repo(), query()).prop_map(|(repo, query)| Route::Actions { repo, query }),
                repo().prop_map(Route::Stargazers),
                repo().prop_map(Route::Watchers),
                repo().prop_map(Route::Forks),
                repo().prop_map(Route::Releases),
                repo().prop_map(Route::NewRelease),
                Just(Route::NewGist),
                (login(), "[0-9a-f]{20}")
                    .prop_filter("an owner isn't a gist ID or a gist page", |(o, _)| {
                        !is_gist_id(o) && !GIST_PAGES.contains(&o.as_str())
                    })
                    .prop_map(|(owner, id)| Route::EditGist { owner, id }),
                (repo(), prop::option::of("[A-Za-z0-9][A-Za-z0-9 _-]{0,15}"))
                    .prop_map(|(repo, page)| Route::WikiEdit { repo, page }),
                (repo(), 1..u64::MAX, prop::option::of(1..100u64))
                    .prop_map(|(repo, run, attempt)| Route::WorkflowRun { repo, run, attempt }),
                (
                    repo(),
                    prop::option::of(1..u64::MAX),
                    1..u64::MAX,
                    prop::option::of((1..100u32, 1..10_000u32)),
                    prop_oneof![Just(String::new()), query()],
                )
                    .prop_map(|(repo, run, job, step, query)| Route::Job {
                        repo,
                        run,
                        job,
                        step,
                        query
                    }),
                (repo(), "[A-Za-z0-9_-][A-Za-z0-9_.-]{0,11}\\.ya?ml")
                    .prop_map(|(repo, file)| Route::Workflow { repo, file }),
                (
                    discussions_of(),
                    prop::option::of("[a-z0-9][a-z0-9-]{0,11}")
                )
                    .prop_map(|(of, category)| Route::Discussions { of, category }),
                (discussions_of(), 1..u64::MAX)
                    .prop_map(|(of, number)| Route::Discussion { of, number }),
                (repo(), prop::option::of("[A-Za-z0-9][A-Za-z0-9 _-]{0,15}"))
                    .prop_filter("not a writing page", |(_, p)| {
                        !matches!(p.as_deref(), Some("_new" | "_edit" | "_compare"))
                    })
                    .prop_map(|(repo, page)| Route::Wiki { repo, page }),
                prop::option::of(repo()).prop_map(Route::Advisories),
                crate::site::arb(),
                (
                    repo(),
                    prop::sample::select(AlertKind::ALL.to_vec()),
                    prop_oneof![Just(OPEN.to_owned()), query()]
                )
                    .prop_map(|(repo, kind, query)| Route::Alerts {
                        repo,
                        kind,
                        query
                    }),
                (
                    repo(),
                    prop::sample::select(AlertKind::ALL.to_vec()),
                    1..u64::MAX
                )
                    .prop_map(|(repo, kind, number)| Route::Alert {
                        repo,
                        kind,
                        number
                    }),
                (
                    repo(),
                    prop_oneof![
                        Just(crate::api::settings::SettingsPage::General),
                        Just(crate::api::settings::SettingsPage::Branches),
                        Just(crate::api::settings::SettingsPage::Access),
                        Just(crate::api::settings::SettingsPage::Hooks),
                        Just(crate::api::settings::SettingsPage::Keys),
                        (1..u64::MAX).prop_map(crate::api::settings::SettingsPage::Hook),
                    ]
                )
                    .prop_map(|(repo, page)| Route::Settings { repo, page }),
                (prop::option::of(repo()), "GHSA(-[2-9a-z]{4}){3}")
                    .prop_map(|(repo, ghsa)| Route::Advisory { repo, ghsa }),
                login().prop_map(Route::Teams),
                (login(), segment())
                    .prop_filter("not the new-team form", |(_, slug)| slug != "new")
                    .prop_map(|(org, slug)| Route::Team { org, slug }),
                (prop::option::of(login()), "[0-9a-f]{20}|[0-9]{1,8}")
                    .prop_filter("an owner isn't a gist ID or a gist page", |(o, _)| {
                        o.as_deref()
                            .is_none_or(|o| !is_gist_id(o) && !GIST_PAGES.contains(&o))
                    })
                    .prop_filter("a lone ID is a gist's", |(o, id)| {
                        o.is_some() || is_lone_gist_id(id)
                    })
                    .prop_map(|(owner, id)| Route::Gist { owner, id }),
                login()
                    .prop_filter("not a gist ID or a gist page", |l| {
                        !is_lone_gist_id(l) && !GIST_PAGES.contains(&l.as_str())
                    })
                    .prop_map(Route::Gists),
                (repo(), prop::option::of("[A-Za-z0-9 _-]{1,12}"))
                    .prop_filter("not the log", |(_, e)| e.as_deref() != Some("activity_log"))
                    .prop_map(|(repo, environment)| Route::Deployments { repo, environment }),
                (repo(), "[A-Za-z0-9._-]{1,12}")
                    .prop_filter("not . or ..", |(_, t)| t != "." && t != "..")
                    .prop_map(|(repo, tag)| Route::Release { repo, tag }),
                (repo(), "[A-Za-z0-9._-]{1,12}")
                    .prop_filter("not . or ..", |(_, t)| t != "." && t != "..")
                    .prop_map(|(repo, tag)| Route::EditRelease { repo, tag }),
                login()
                    .prop_filter("not a reserved path", |l| !RESERVED.contains(&l.as_str()))
                    .prop_flat_map(|login| {
                        let tabs = prop::sample::select(vec![
                            ProfileTab::Overview,
                            ProfileTab::Repositories(RepoSort::Updated),
                            ProfileTab::Repositories(RepoSort::Name),
                            ProfileTab::Repositories(RepoSort::Stars),
                            ProfileTab::Stars,
                            ProfileTab::Followers,
                            ProfileTab::Following,
                            ProfileTab::People,
                        ]);
                        tabs.prop_map(move |tab| Route::User {
                            login: login.clone(),
                            tab,
                        })
                    }),
                repo().prop_map(|repo| Route::Projects(ProjectsOf::Repo(repo))),
                (
                    login(),
                    any::<bool>(),
                    1..u64::MAX,
                    prop::option::of(1..u64::MAX)
                )
                    .prop_map(|(login, org, number, view)| {
                        let owner = if org {
                            ProjectOwner::Org(login)
                        } else {
                            ProjectOwner::User(login)
                        };
                        Route::Project {
                            project: ProjectRef { owner, number },
                            view,
                        }
                    }),
                (
                    login(),
                    1..u64::MAX,
                    prop::option::of(1..u64::MAX),
                    1..u64::MAX
                )
                    .prop_map(|(login, number, view, item)| Route::ProjectItem {
                        project: ProjectRef {
                            owner: ProjectOwner::Org(login),
                            number,
                        },
                        view,
                        item,
                    }),
                (any::<String>(), 0..7u8).prop_map(|(query, k)| Route::Search {
                    kind: [
                        SearchKind::Repos,
                        SearchKind::Issues,
                        SearchKind::Pulls,
                        SearchKind::Users,
                        SearchKind::Discussions,
                        SearchKind::Commits,
                        SearchKind::Code,
                    ][usize::from(k)],
                    query,
                }),
                code::arb(),
                issues::arb(),
                pulls::arb(),
            ]
        }

        /// A corpus URL with its owner and repository, numbers and commit
        /// IDs swapped for `owner`, `name`, `number` and `sha`.
        fn vary(url: &str, owner: &str, name: &str, number: u64, sha: &str) -> String {
            let Ok(mut parsed) = url::Url::parse(url) else {
                return url.to_owned();
            };
            let segments: Vec<String> = parsed
                .path_segments()
                .map(|s| s.map(str::to_owned).collect())
                .unwrap_or_default();
            let repo_page = parsed.host_str() == Some("github.com")
                && segments.len() >= 2
                && segments
                    .first()
                    .is_some_and(|s| !RESERVED.contains(&s.as_str()) && s != "orgs");
            let varied: Vec<String> = segments
                .iter()
                .enumerate()
                .map(|(i, s)| match (i, repo_page) {
                    (0, true) => owner.to_owned(),
                    (1, true) => name.to_owned(),
                    _ if s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()) => {
                        sha.to_owned()
                    }
                    // As many digits: a lone gist.github.com number is a gist
                    // only when it's long.
                    _ if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => {
                        let digits = u32::try_from(s.len()).unwrap_or(u32::MAX);
                        let low = 10u64.checked_pow(digits - 1).unwrap_or(1);
                        let high = 10u64.checked_pow(digits).unwrap_or(u64::MAX);
                        (low + number % (high - low)).to_string()
                    }
                    _ => s.clone(),
                })
                .collect();
            parsed.set_path(&varied.join("/"));
            parsed.to_string()
        }

        proptest! {
            /// Any owner, repository, number or commit in a corpus URL opens
            /// the same kind of page (or the browser, if its kind does), and
            /// every page's URL leads back to it.
            #[test]
            fn corpus_shapes_hold_for_any_names(
                owner in segment(),
                name in segment(),
                number in 1..u64::MAX,
                sha in "[0-9a-f]{40}",
            ) {
                for (url, expect, reason) in corpus() {
                    let varied = vary(url, &owner, &name, number, &sha);
                    let target = Dest::from_url(&varied).target;
                    prop_assert!(as_expected(&target, expect, reason), "{varied}: {target:?}, not {expect}");
                    if let Target::Page(route) = &target {
                        let route = route.split(None).unwrap_or_else(|| route.clone());
                        prop_assert_eq!(reopened(&route), Target::Page(route.clone()), "{}", varied);
                    }
                }
            }

            /// Every page's URL leads back to it.
            #[test]
            fn urls_round_trip(route in route()) {
                prop_assert_eq!(reopened(&route), Target::Page(route));
            }

            /// Whatever is typed, parsing doesn't panic.
            #[test]
            fn any_input_parses_or_not(input in any::<String>()) {
                let repo = RepoId::new("o", "r");
                let _ = parse_input(&input, Some(&repo));
                let _ = Dest::from_url(&input).target;
                let _ = decode(&input);
            }
        }
    }
}
