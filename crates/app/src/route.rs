//! Routes: the pages ghtui shows, addressed by their github.com URLs.

use ghtui_api::browse::{DiscussionsOf, RepoSort, SearchKind};
use ghtui_api::model::{PrRef, RepoId};
use ghtui_ui::pages::url as links;
use ghtui_ui::pages::{PrTab, ProfileTab};
use ghtui_ui::text::short_sha;

use crate::diff_screen::DiffOf;

/// The default filter of a repository's issue and pull request lists.
pub const OPEN: &str = "is:open";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Route {
    Home,
    /// The Code tab at the repository root.
    Repo(RepoId),
    Tree {
        repo: RepoId,
        rev: String,
        path: String,
    },
    Blob {
        repo: RepoId,
        rev: String,
        path: String,
        /// Lines a link points at (`#L10-L20`), marked and scrolled to.
        lines: Option<(u32, u32)>,
    },
    /// Which commit last changed each line of a file.
    Blame {
        repo: RepoId,
        rev: String,
        path: String,
        lines: Option<(u32, u32)>,
    },
    Issues {
        repo: RepoId,
        query: String,
    },
    Pulls {
        repo: RepoId,
        query: String,
    },
    /// An issue (or, until GitHub says otherwise, a pull request: those
    /// redirect).
    Issue {
        repo: RepoId,
        number: u64,
    },
    Pr {
        pr: PrRef,
        tab: PrTab,
    },
    /// The checks on the default branch.
    Actions(RepoId),
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
    /// The checks on a commit.
    CommitChecks {
        repo: RepoId,
        oid: String,
    },
    Stargazers(RepoId),
    Watchers(RepoId),
    Forks(RepoId),
    Releases(RepoId),
    Release {
        repo: RepoId,
        tag: String,
    },
    Tags(RepoId),
    Branches(RepoId),
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
    /// Two revisions compared, as the URL names them (`a...b`, `a..b`).
    Compare {
        repo: RepoId,
        spec: String,
    },
    /// Deployments, to one environment if given.
    Deployments {
        repo: RepoId,
        environment: Option<String>,
    },
    /// Open or closed milestones.
    Milestones {
        repo: RepoId,
        closed: bool,
    },
    Milestone {
        repo: RepoId,
        number: u64,
    },
    /// A revision's commits, of `path` if it isn't empty.
    Commits {
        repo: RepoId,
        rev: String,
        path: String,
    },
    /// A commit, by SHA (short or full, as linked).
    Commit {
        repo: RepoId,
        oid: String,
    },
    User {
        login: String,
        tab: ProfileTab,
    },
    Search {
        kind: SearchKind,
        query: String,
    },
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

impl Route {
    /// A pull request's conversation.
    pub fn pr(pr: PrRef) -> Route {
        Route::Pr {
            pr,
            tab: PrTab::Conversation,
        }
    }

    /// A file, from the top.
    pub fn blob(repo: RepoId, rev: String, path: String) -> Route {
        Route::Blob {
            repo,
            rev,
            path,
            lines: None,
        }
    }

    /// A profile's overview.
    pub fn user(login: &str) -> Route {
        Route::User {
            login: login.to_owned(),
            tab: ProfileTab::Overview,
        }
    }

    pub fn url(&self) -> String {
        match self {
            Route::Home => format!("{}/", links::BASE),
            Route::Repo(repo) => links::repo(repo),
            Route::Tree { repo, rev, path } => links::tree(repo, rev, path),
            Route::Blob {
                repo,
                rev,
                path,
                lines,
            } => {
                let url = links::blob(repo, rev, path);
                match lines {
                    None => url,
                    Some((a, b)) if a == b => format!("{url}#L{a}"),
                    Some((a, b)) => format!("{url}#L{a}-L{b}"),
                }
            }
            Route::Blame {
                repo,
                rev,
                path,
                lines,
            } => {
                let url = links::blame(repo, rev, path);
                match lines {
                    None => url,
                    Some((a, b)) if a == b => format!("{url}#L{a}"),
                    Some((a, b)) => format!("{url}#L{a}-L{b}"),
                }
            }
            Route::Issues { repo, query } => list_url(&links::issues(repo), query),
            Route::Pulls { repo, query } => list_url(&links::pulls(repo), query),
            Route::Issue { repo, number } => links::issue(repo, *number),
            Route::Pr { pr, tab } => match tab {
                PrTab::Conversation => links::pull(pr),
                PrTab::Commits => links::pull_tab(pr, "commits"),
                PrTab::Checks => links::pull_tab(pr, "checks"),
            },
            Route::Actions(repo) => format!("{}/actions", links::repo(repo)),
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
            Route::Workflow { repo, file } => {
                format!(
                    "{}/actions/workflows/{}",
                    links::repo(repo),
                    links::encode_path(file)
                )
            }
            Route::CommitChecks { repo, oid } => format!("{}/checks", links::commit(repo, oid)),
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
            Route::Tags(repo) => format!("{}/tags", links::repo(repo)),
            Route::Branches(repo) => format!("{}/branches", links::repo(repo)),
            Route::Wiki { repo, page } => match page {
                Some(page) => format!("{}/wiki/{}", links::repo(repo), links::encode_path(page)),
                None => format!("{}/wiki", links::repo(repo)),
            },
            Route::Advisories(repo) => match repo {
                Some(repo) => format!("{}/security/advisories", links::repo(repo)),
                None => format!("{}/advisories", links::BASE),
            },
            Route::Advisory { repo, ghsa } => match repo {
                Some(repo) => format!("{}/security/advisories/{ghsa}", links::repo(repo)),
                None => format!("{}/advisories/{ghsa}", links::BASE),
            },
            Route::Teams(org) => format!("{}/orgs/{org}/teams", links::BASE),
            Route::Team { org, slug } => format!("{}/orgs/{org}/teams/{slug}", links::BASE),
            Route::Gist { owner, id } => match owner {
                Some(owner) => format!("{GIST}/{owner}/{id}"),
                None => format!("{GIST}/{id}"),
            },
            Route::Gists(login) => format!("{GIST}/{login}"),
            Route::Compare { repo, spec } => {
                format!("{}/compare/{}", links::repo(repo), links::encode_path(spec))
            }
            Route::Deployments { repo, environment } => match environment {
                Some(env) => format!(
                    "{}/deployments/activity_log?environments_filter={}",
                    links::repo(repo),
                    links::encode(env)
                ),
                None => format!("{}/deployments", links::repo(repo)),
            },
            Route::Milestones { repo, closed } => format!(
                "{}/milestones{}",
                links::repo(repo),
                if *closed { "?state=closed" } else { "" }
            ),
            Route::Milestone { repo, number } => {
                format!("{}/milestone/{number}", links::repo(repo))
            }
            Route::Commits { repo, rev, path } => links::commits(repo, rev, path),
            Route::Commit { repo, oid } => links::commit(repo, oid),
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
                ProfileTab::People => format!("{}/orgs/{login}/people", links::BASE),
            },
            Route::Search { kind, query } => links::search(*kind, query),
        }
    }

    /// The short name shown in the top bar.
    pub fn title(&self) -> String {
        match self {
            Route::Home => "Home".to_owned(),
            Route::Repo(repo) => repo.to_string(),
            Route::Tree { repo, path, .. } if path.is_empty() => repo.to_string(),
            Route::Tree { repo, path, .. } | Route::Blob { repo, path, .. } => {
                format!("{}/{path}", repo.name)
            }
            Route::Blame { repo, path, .. } => format!("{}/{path} · Blame", repo.name),
            Route::Issues { repo, .. } => format!("{repo} · Issues"),
            Route::Pulls { repo, .. } => format!("{repo} · Pull requests"),
            Route::Issue { repo, number } => format!("{repo}#{number}"),
            Route::Pr { pr, .. } => pr.to_string(),
            Route::Actions(repo) => format!("{repo} · Actions"),
            Route::WorkflowRun { repo, run, .. } => format!("{repo} · Run {run}"),
            Route::Job { repo, job, .. } => format!("{repo} · Job {job}"),
            Route::Workflow { repo, file } => format!("{repo} · {file}"),
            Route::CommitChecks { repo, oid } => format!("{repo}@{} · Checks", short_sha(oid)),
            Route::Stargazers(repo) => format!("{repo} · Stargazers"),
            Route::Watchers(repo) => format!("{repo} · Watchers"),
            Route::Forks(repo) => format!("{repo} · Forks"),
            Route::Releases(repo) => format!("{repo} · Releases"),
            Route::Release { repo, tag } => format!("{repo} {tag}"),
            Route::Discussions { of, .. } => format!("{} · Discussions", of_title(of)),
            Route::Discussion { of, number } => format!("{} · Discussion {number}", of_title(of)),
            Route::Tags(repo) => format!("{repo} · Tags"),
            Route::Branches(repo) => format!("{repo} · Branches"),
            Route::Wiki { repo, page } => match page {
                Some(page) => format!("{repo} · {}", page.replace('-', " ")),
                None => format!("{repo} · Wiki"),
            },
            Route::Advisories(Some(repo)) => format!("{repo} · Security"),
            Route::Advisories(None) => "Advisories".to_owned(),
            Route::Advisory { ghsa, .. } => ghsa.clone(),
            Route::Teams(org) => format!("@{org} · Teams"),
            Route::Team { org, slug } => format!("@{org}/{slug}"),
            Route::Gist { id, .. } => format!("Gist {}", id.get(..7).unwrap_or(id)),
            Route::Gists(login) => format!("@{login} · Gists"),
            Route::Compare { repo, spec } => format!("{repo} · {spec}"),
            Route::Deployments { repo, .. } => format!("{repo} · Deployments"),
            Route::Milestones { repo, .. } => format!("{repo} · Milestones"),
            Route::Milestone { repo, number } => format!("{repo} · Milestone {number}"),
            Route::Commits { repo, path, .. } if path.is_empty() => format!("{repo} · Commits"),
            Route::Commits { repo, path, .. } => format!("{}/{path} · Commits", repo.name),
            Route::Commit { repo, oid } => format!("{repo}@{}", short_sha(oid)),
            Route::User { login, .. } => format!("@{login}"),
            Route::Search { query, .. } => format!("Search “{query}”"),
        }
    }

    /// The repository the page belongs to.
    pub fn repo(&self) -> Option<&RepoId> {
        match self {
            Route::Repo(repo)
            | Route::Tree { repo, .. }
            | Route::Blob { repo, .. }
            | Route::Issues { repo, .. }
            | Route::Pulls { repo, .. }
            | Route::Issue { repo, .. }
            | Route::Commits { repo, .. }
            | Route::Actions(repo)
            | Route::WorkflowRun { repo, .. }
            | Route::Job { repo, .. }
            | Route::Workflow { repo, .. }
            | Route::CommitChecks { repo, .. }
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
            | Route::Tags(repo)
            | Route::Branches(repo)
            | Route::Wiki { repo, .. }
            | Route::Advisories(Some(repo))
            | Route::Advisory {
                repo: Some(repo), ..
            }
            | Route::Blame { repo, .. }
            | Route::Compare { repo, .. }
            | Route::Deployments { repo, .. }
            | Route::Milestones { repo, .. }
            | Route::Milestone { repo, .. }
            | Route::Commit { repo, .. } => Some(repo),
            Route::Pr { pr, .. } => Some(&pr.repo),
            Route::Home
            | Route::User { .. }
            | Route::Search { .. }
            | Route::Gist { .. }
            | Route::Gists(_)
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

    /// A list's filter: issues, pull requests, search results.
    pub fn list_query(&self) -> Option<&str> {
        match self {
            Route::Issues { query, .. }
            | Route::Pulls { query, .. }
            | Route::Search { query, .. }
            | Route::Job { query, .. } => Some(query),
            _ => None,
        }
    }

    /// The same list with another filter.
    pub fn with_query(&self, query: String) -> Option<Route> {
        Some(match self {
            Route::Issues { repo, .. } => Route::Issues {
                repo: repo.clone(),
                query,
            },
            Route::Pulls { repo, .. } => Route::Pulls {
                repo: repo.clone(),
                query,
            },
            Route::Search { kind, .. } => Route::Search { kind: *kind, query },
            Route::Job { repo, run, job, .. } => Route::Job {
                repo: repo.clone(),
                run: *run,
                job: *job,
                step: None,
                query,
            },
            _ => return None,
        })
    }

    /// The search behind a list page.
    pub fn search(&self) -> Option<(SearchKind, String)> {
        match self {
            Route::Issues { repo, query } => {
                Some((SearchKind::Issues, list_query(repo, "issue", query)))
            }
            Route::Pulls { repo, query } => {
                Some((SearchKind::Issues, list_query(repo, "pr", query)))
            }
            Route::Search { kind, query } => Some((*kind, query.clone())),
            _ => None,
        }
    }
}

/// The search for a repository's issue or pull request list. Newest first,
/// as on GitHub, unless the filter sorts.
fn list_query(repo: &RepoId, is: &str, query: &str) -> String {
    let mut q = format!("repo:{repo} is:{is} {}", query.trim());
    if !query.contains("sort:") {
        q.push_str(" sort:created-desc");
    }
    q
}

fn list_url(base: &str, query: &str) -> String {
    if query == OPEN {
        base.to_owned()
    } else {
        format!("{base}?q={}", links::encode(query))
    }
}

impl Target {
    /// Parses a github.com URL (or a ghtui link). Other URLs are external.
    pub fn from_url(s: &str) -> Target {
        let external = || Target::External(s.to_owned());
        let with_scheme = if s.starts_with("github.com/") {
            format!("https://{s}")
        } else {
            s.to_owned()
        };
        let Ok(parsed) = url::Url::parse(&with_scheme) else {
            return external();
        };
        if !matches!(parsed.scheme(), "https" | "http") {
            return external();
        }
        if parsed.host_str() == Some("gist.github.com") {
            return gist(&parsed).map_or_else(external, Target::Page);
        }
        if !matches!(parsed.host_str(), Some("github.com" | "www.github.com")) {
            return external();
        }
        let param = |name: &str| {
            parsed
                .query_pairs()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.into_owned())
        };
        let (query, kind, tab) = (param("q"), param("type"), param("tab"));
        let repos = || {
            ProfileTab::Repositories(match param("sort").as_deref() {
                Some("name") => RepoSort::Name,
                Some("stargazers") => RepoSort::Stars,
                _ => RepoSort::Updated,
            })
        };
        let segments: Vec<String> = parsed
            .path_segments()
            .map(|s| s.filter(|p| !p.is_empty()).map(decode).collect())
            .unwrap_or_default();
        let segments: Vec<&str> = segments.iter().map(String::as_str).collect();
        let repo = |o: &str, r: &str| RepoId::parse(&format!("{o}/{r}"));
        let route = match segments.as_slice() {
            [] | ["dashboard"] => Route::Home,
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
                        return Target::Page(Route::Search {
                            kind: SearchKind::Repos,
                            query: format!("topic:{}", query.unwrap_or_default().trim()),
                        });
                    }
                    // Wikis, packages, the marketplace…
                    Some(_) => return external(),
                },
                query: query.unwrap_or_default(),
            },
            ["topics", topic] => Route::Search {
                kind: SearchKind::Repos,
                query: format!("topic:{topic}"),
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
                    _ => return external(),
                };
                Route::Search {
                    kind,
                    query: query
                        .unwrap_or_else(|| format!("is:open is:{is} {whose} archived:false")),
                }
            }
            ["stars", login] => Route::User {
                login: (*login).to_owned(),
                tab: ProfileTab::Stars,
            },
            ["orgs", login] => Route::user(login),
            ["advisories"] => Route::Advisories(None),
            ["advisories", ghsa] => Route::Advisory {
                repo: None,
                ghsa: (*ghsa).to_owned(),
            },
            ["orgs", org, "teams"] => Route::Teams((*org).to_owned()),
            // Its members, repositories and child teams are on its page.
            ["orgs", org, "teams", slug, ..] => Route::Team {
                org: (*org).to_owned(),
                slug: (*slug).to_owned(),
            },
            ["orgs", org, "discussions", rest @ ..] => {
                match discussions(DiscussionsOf::Org((*org).to_owned()), rest) {
                    Some(route) => route,
                    None => return external(),
                }
            }
            ["orgs", login, page @ ("repositories" | "people")] => Route::User {
                login: (*login).to_owned(),
                tab: if *page == "people" {
                    ProfileTab::People
                } else {
                    repos()
                },
            },
            [login] if !RESERVED.contains(login) => Route::User {
                login: (*login).to_owned(),
                tab: match tab.as_deref() {
                    None | Some("overview") => ProfileTab::Overview,
                    Some("repositories") => repos(),
                    Some("stars") => ProfileTab::Stars,
                    Some("followers") => ProfileTab::Followers,
                    Some("following") => ProfileTab::Following,
                    // Packages and projects need scopes gh doesn't grant.
                    Some(_) => return external(),
                },
            },
            // GitHub's own pages: settings, apps, the marketplace…
            [first, ..] if RESERVED.contains(first) => return external(),
            [o, r] => match repo(o, r) {
                Some(repo) => Route::Repo(repo),
                None => return external(),
            },
            [o, r, kind @ ("tree" | "blob"), rev, path @ ..] => {
                let Some(repo) = repo(o, r) else {
                    return external();
                };
                let (rev, path) = ((*rev).to_owned(), path.join("/"));
                if *kind == "tree" {
                    Route::Tree { repo, rev, path }
                } else {
                    let lines = parsed.fragment().and_then(line_range);
                    Route::Blob {
                        repo,
                        rev,
                        path,
                        lines,
                    }
                }
            }
            [o, r, list @ ("issues" | "pulls")] => {
                let Some(repo) = repo(o, r) else {
                    return external();
                };
                let query = query.map_or_else(|| OPEN.to_owned(), |q| strip_is(&q));
                if *list == "issues" {
                    Route::Issues { repo, query }
                } else {
                    Route::Pulls { repo, query }
                }
            }
            // A label's issues. (Milestones are by number in URLs, but
            // search only filters by title, so they stay on GitHub.)
            [o, r, "labels", label] => match repo(o, r) {
                Some(repo) => {
                    let label = if label.contains(char::is_whitespace) {
                        format!("\"{label}\"")
                    } else {
                        (*label).to_owned()
                    };
                    Route::Issues {
                        repo,
                        query: format!("{OPEN} label:{label}"),
                    }
                }
                None => return external(),
            },
            [
                o,
                r,
                page @ ("actions" | "stargazers" | "watchers" | "forks" | "network"),
            ] => {
                let Some(repo) = repo(o, r) else {
                    return external();
                };
                match *page {
                    "actions" => Route::Actions(repo),
                    "stargazers" => Route::Stargazers(repo),
                    "watchers" => Route::Watchers(repo),
                    "forks" => Route::Forks(repo),
                    // The network graph.
                    _ => return external(),
                }
            }
            [o, r, "network", "members"] => match repo(o, r) {
                Some(repo) => Route::Forks(repo),
                None => return external(),
            },
            [o, r, "releases", "tag", tag @ ..] if !tag.is_empty() => match repo(o, r) {
                Some(repo) => Route::Release {
                    repo,
                    tag: tag.join("/"),
                },
                None => return external(),
            },
            // Assets are downloads, and writing wiki pages is on GitHub.
            [_, _, "releases", "download", ..]
            | [_, _, "wiki", .., "_new" | "_edit" | "_compare"] => return external(),
            [o, r, "releases", ..] => match repo(o, r) {
                Some(repo) => Route::Releases(repo),
                None => return external(),
            },
            [o, r, "tags"] => match repo(o, r) {
                Some(repo) => Route::Tags(repo),
                None => return external(),
            },
            // A milestone by its title is in the list.
            [o, r, "milestones", ..] => match repo(o, r) {
                Some(repo) => Route::Milestones {
                    repo,
                    closed: param("state").as_deref() == Some("closed"),
                },
                None => return external(),
            },
            [o, r, "milestone", n] => match (repo(o, r), n.parse()) {
                (Some(repo), Ok(number)) => Route::Milestone { repo, number },
                _ => return external(),
            },
            // A comparison's patch and diff are downloads.
            [_, _, "compare", .., last] if last.ends_with(".patch") || last.ends_with(".diff") => {
                return external();
            }
            [o, r, "compare", rest @ ..] => {
                let Some(repo) = repo(o, r) else {
                    return external();
                };
                let spec = rest.join("/");
                // Without revisions it's the form for opening a pull
                // request.
                if spec.is_empty() {
                    return external();
                }
                // Its files, from one commit to the other (`a..b`; with
                // three dots it's from their merge base, which the
                // comparison page finds).
                if parsed.fragment() == Some("files")
                    && !spec.contains("...")
                    && let Some((from, to)) = spec.split_once("..")
                    && is_full_sha(from)
                    && is_full_sha(to)
                {
                    return Target::Files(DiffOf::Range(repo, from.to_owned(), to.to_owned()));
                }
                Route::Compare { repo, spec }
            }
            [o, r, "blame", rev, path @ ..] if !path.is_empty() => match repo(o, r) {
                Some(repo) => Route::Blame {
                    repo,
                    rev: (*rev).to_owned(),
                    path: path.join("/"),
                    lines: parsed.fragment().and_then(line_range),
                },
                None => return external(),
            },
            // The security overview's part ghtui can show: advisories.
            [o, r, "security"] | [o, r, "security", "advisories"] => match repo(o, r) {
                Some(repo) => Route::Advisories(Some(repo)),
                None => return external(),
            },
            [o, r, "security", "advisories", ghsa] => match repo(o, r) {
                Some(repo) => Route::Advisory {
                    repo: Some(repo),
                    ghsa: (*ghsa).to_owned(),
                },
                None => return external(),
            },
            // A page's revisions and history: the page.
            [o, r, "wiki", rest @ ..] => match repo(o, r) {
                Some(repo) => Route::Wiki {
                    repo,
                    page: rest.first().map(|p| (*p).to_owned()),
                },
                None => return external(),
            },
            [o, r, "deployments", rest @ ..] => {
                let Some(repo) = repo(o, r) else {
                    return external();
                };
                let environment = match rest {
                    [] => None,
                    ["activity_log"] => param("environments_filter"),
                    [env] => Some((*env).to_owned()),
                    _ => return external(),
                };
                Route::Deployments { repo, environment }
            }
            // All, active, stale, yours: one list.
            [o, r, "branches", ..] => match repo(o, r) {
                Some(repo) => Route::Branches(repo),
                None => return external(),
            },
            [o, r, "commits", rest @ ..] => {
                let Some(repo) = repo(o, r) else {
                    return external();
                };
                let (rev, path) = match rest {
                    [] => ("HEAD".to_owned(), String::new()),
                    [rev, path @ ..] => ((*rev).to_owned(), path.join("/")),
                };
                Route::Commits { repo, rev, path }
            }
            [o, r, "discussions", rest @ ..] => {
                match repo(o, r).and_then(|repo| discussions(DiscussionsOf::Repo(repo), rest)) {
                    Some(route) => route,
                    None => return external(),
                }
            }
            [o, r, "actions", "runs", run, rest @ ..] => {
                let (Some(repo), Ok(run)) = (repo(o, r), run.parse()) else {
                    return external();
                };
                match rest {
                    // The run's workflow file shows on the run's page.
                    [] | ["workflow"] => Route::WorkflowRun {
                        repo,
                        run,
                        attempt: None,
                    },
                    ["attempts", n] => match n.parse() {
                        Ok(n) => Route::WorkflowRun {
                            repo,
                            run,
                            attempt: Some(n),
                        },
                        Err(_) => return external(),
                    },
                    ["job", job] => match job.parse() {
                        Ok(job) => Route::Job {
                            repo,
                            run: Some(run),
                            job,
                            step: parsed.fragment().and_then(step_line),
                            query: query.unwrap_or_default(),
                        },
                        Err(_) => return external(),
                    },
                    _ => return external(),
                }
            }
            // A check run's URL, which is its job's for Actions.
            [o, r, "runs", job] | [o, r, "runs", _, "jobs", job] => {
                match (repo(o, r), job.parse()) {
                    (Some(repo), Ok(job)) => Route::Job {
                        repo,
                        run: None,
                        job,
                        step: parsed.fragment().and_then(step_line),
                        query: query.unwrap_or_default(),
                    },
                    _ => return external(),
                }
            }
            [o, r, "actions", "workflows", file] => match repo(o, r) {
                Some(repo) => Route::Workflow {
                    repo,
                    file: (*file).to_owned(),
                },
                None => return external(),
            },
            // A check run's logs, from a pull request's checks.
            [o, r, "commit", _, "checks", job, ..] => match (repo(o, r), job.parse()) {
                (Some(repo), Ok(job)) => Route::Job {
                    repo,
                    run: None,
                    job,
                    step: None,
                    query: String::new(),
                },
                _ => return external(),
            },
            [o, r, "commit", sha, "checks"] => match repo(o, r) {
                Some(repo) => Route::CommitChecks {
                    repo,
                    oid: (*sha).to_owned(),
                },
                None => return external(),
            },
            [o, r, "git", "commit", sha] => match repo(o, r) {
                Some(repo) => Route::Commit {
                    repo,
                    oid: (*sha).to_owned(),
                },
                None => return external(),
            },
            // A commit's patch is a download; its diff is the diff viewer.
            [_, _, "commit", sha] if sha.ends_with(".patch") => return external(),
            [o, r, "commit", sha] if sha.ends_with(".diff") => match repo(o, r) {
                Some(repo) => {
                    let sha = sha.trim_end_matches(".diff").to_owned();
                    return Target::Files(DiffOf::Commit(repo, sha));
                }
                None => return external(),
            },
            [o, r, "pull", n] if n.ends_with(".diff") => {
                match (repo(o, r), n.trim_end_matches(".diff").parse()) {
                    (Some(repo), Ok(number)) => {
                        return Target::Files(DiffOf::Pr(PrRef { repo, number }));
                    }
                    _ => return external(),
                }
            }
            [o, r, "commit", sha] => {
                let Some(repo) = repo(o, r) else {
                    return external();
                };
                // GitHub anchors a commit's files as `#diff-…`.
                let to_files = parsed
                    .fragment()
                    .is_some_and(|f| f == "files" || f.starts_with("diff-"));
                if to_files && is_full_sha(sha) {
                    return Target::Files(DiffOf::Commit(repo, (*sha).to_owned()));
                }
                Route::Commit {
                    repo,
                    oid: (*sha).to_owned(),
                }
            }
            [o, r, "issues", n] => match (repo(o, r), n.parse()) {
                (Some(repo), Ok(number)) => Route::Issue { repo, number },
                _ => return external(),
            },
            [o, r, "pull", n, rest @ ..] => {
                let (Some(repo), Ok(number)) = (repo(o, r), n.parse()) else {
                    return external();
                };
                let pr = PrRef { repo, number };
                // Review comments are threads in the diff.
                let review_comment = parsed.fragment().is_some_and(|f| {
                    f.strip_prefix("discussion_r")
                        .or_else(|| f.strip_prefix('r'))
                        .is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
                });
                match rest {
                    [] if review_comment => return Target::Files(DiffOf::Pr(pr)),
                    [] => Route::pr(pr),
                    ["commits" | "changes", sha] => Route::Commit {
                        repo: pr.repo,
                        oid: (*sha).to_owned(),
                    },
                    ["files" | "changes", ..] => return Target::Files(DiffOf::Pr(pr)),
                    ["commits", ..] => Route::Pr {
                        pr,
                        tab: PrTab::Commits,
                    },
                    ["checks", ..] => Route::Pr {
                        pr,
                        tab: PrTab::Checks,
                    },
                    _ => return external(),
                }
            }
            _ => return external(),
        };
        Target::Page(route)
    }
}

fn is_full_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

const GIST: &str = "https://gist.github.com";

/// gist.github.com's own pages, not people's gists.
const GIST_PAGES: &[&str] = &[
    "discover", "starred", "search", "auth", "login", "join", "mine",
];

/// Whether a gist.github.com path segment is a gist's ID (numbers for old
/// gists, 20 or 32 hex digits for newer) rather than someone's login.
fn is_gist_id(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_digit())
        || (matches!(s.len(), 20 | 32) && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// A gist.github.com page: a gist (`/<id>`, `/<owner>/<id>`) or someone's
/// gists (`/<owner>`). Raw files and embeds are downloads.
fn gist(parsed: &url::Url) -> Option<Route> {
    let segments: Vec<&str> = parsed.path_segments()?.filter(|s| !s.is_empty()).collect();
    Some(match segments.as_slice() {
        [first, ..] if GIST_PAGES.contains(first) => return None,
        [id] if is_gist_id(id) => Route::Gist {
            owner: None,
            id: (*id).to_owned(),
        },
        [login] => Route::Gists((*login).to_owned()),
        [owner, id] if is_gist_id(id) => Route::Gist {
            owner: Some((*owner).to_owned()),
            id: (*id).to_owned(),
        },
        // Revisions, forks, stars: the gist.
        [owner, id, "revisions" | "forks" | "stargazers"] if is_gist_id(id) => Route::Gist {
            owner: Some((*owner).to_owned()),
            id: (*id).to_owned(),
        },
        _ => return None,
    })
}
/// The comparison a range of files is from.
pub fn compare_url(of: &DiffOf) -> String {
    match of {
        DiffOf::Range(repo, from, to) => format!("{}/compare/{from}..{to}", links::repo(repo)),
        DiffOf::Pr(pr) => format!("{}/files", pr.url()),
        DiffOf::Commit(repo, oid) => links::commit(repo, oid),
    }
}
/// A discussions page from what follows `/discussions` in its URL.
fn discussions(of: DiscussionsOf, rest: &[&str]) -> Option<Route> {
    Some(match rest {
        [] => Route::Discussions { of, category: None },
        ["categories", slug] => Route::Discussions {
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

fn discussions_url(of: &DiscussionsOf) -> String {
    match of {
        DiscussionsOf::Repo(repo) => format!("{}/discussions", links::repo(repo)),
        DiscussionsOf::Org(org) => format!("{}/orgs/{org}/discussions", links::BASE),
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

/// `L10` or `L10-L20` (as GitHub writes line links), in order.
fn line_range(fragment: &str) -> Option<(u32, u32)> {
    let line = |s: &str| s.strip_prefix('L')?.parse::<u32>().ok().filter(|&n| n > 0);
    let (a, b) = match fragment.split_once('-') {
        Some((a, b)) => (line(a)?, line(b)?),
        None => (line(fragment)?, line(fragment)?),
    };
    Some((a.min(b), a.max(b)))
}

/// Top-level github.com paths that aren't users.
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

/// GitHub's list URLs repeat `is:issue`/`is:pr`; the route adds them back.
fn strip_is(query: &str) -> String {
    let words: Vec<&str> = query
        .split_whitespace()
        .filter(|w| !matches!(*w, "is:issue" | "is:pr"))
        .collect();
    if words.is_empty() {
        OPEN.to_owned()
    } else {
        words.join(" ")
    }
}

/// What the command palette makes of its input.
pub fn parse_input(input: &str, context: Option<&RepoId>) -> Option<Target> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    if input.contains("://") || input.starts_with("github.com/") {
        return match Target::from_url(input) {
            Target::External(_) => None,
            target => Some(target),
        };
    }
    if let Some(login) = input.strip_prefix('@') {
        return valid_login(login).then(|| Target::Page(Route::user(login)));
    }
    let issue = |repo: RepoId, n: &str| {
        let number = n.trim_start_matches('#').parse().ok()?;
        Some(Target::Page(Route::Issue { repo, number }))
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
        return RepoId::parse(input).map(|r| Target::Page(Route::Repo(r)));
    }
    None
}

fn valid_login(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn page(url: &str) -> Route {
        match Target::from_url(url) {
            Target::Page(route) => route,
            other => panic!("{url}: {other:?}"),
        }
    }

    #[test]
    fn parses_github_urls() {
        let repo = RepoId::new("o", "r");
        assert_eq!(page("https://github.com/"), Route::Home);
        assert_eq!(page("https://github.com/octocat"), Route::user("octocat"));
        assert_eq!(page("https://github.com/o/r"), Route::Repo(repo.clone()));
        assert_eq!(
            page("https://github.com/o/r/tree/main/src/ui"),
            Route::Tree {
                repo: repo.clone(),
                rev: "main".into(),
                path: "src/ui".into()
            }
        );
        assert_eq!(
            page("https://github.com/o/r/blob/v1.0/a%20b.md#readme"),
            Route::blob(repo.clone(), "v1.0".into(), "a b.md".into())
        );
        for (fragment, lines) in [("L7", (7, 7)), ("L20-L10", (10, 20)), ("L0", (0, 0))] {
            let url = format!("https://github.com/o/r/blob/main/a.rs#{fragment}");
            let expected = (lines.0 > 0).then_some(lines);
            assert!(
                matches!(page(&url), Route::Blob { lines, .. } if lines == expected),
                "{fragment}"
            );
        }
        assert_eq!(
            page("https://github.com/o/r/issues?q=is%3Aissue+is%3Aclosed"),
            Route::Issues {
                repo: repo.clone(),
                query: "is:closed".into()
            }
        );
        assert_eq!(
            page("https://github.com/o/r/pulls"),
            Route::Pulls {
                repo: repo.clone(),
                query: OPEN.into()
            }
        );
        assert_eq!(
            page("https://github.com/o/r/issues/5"),
            Route::Issue {
                repo: repo.clone(),
                number: 5
            }
        );
        let pr = PrRef { repo, number: 7 };
        assert_eq!(
            page("https://github.com/o/r/pull/7/commits"),
            Route::Pr {
                pr: pr.clone(),
                tab: PrTab::Commits
            }
        );
        assert_eq!(
            Target::from_url("https://github.com/o/r/pull/7/files"),
            Target::Files(DiffOf::Pr(pr.clone()))
        );
        let sha = "0123456789abcdef0123456789abcdef01234567";
        let commit = |oid: &str| Route::Commit {
            repo: pr.repo.clone(),
            oid: oid.into(),
        };
        assert_eq!(
            page(&format!("https://github.com/o/r/commit/{sha}")),
            commit(sha)
        );
        let commits = |rev: &str, path: &str| Route::Commits {
            repo: pr.repo.clone(),
            rev: rev.into(),
            path: path.into(),
        };
        assert_eq!(
            page("https://github.com/o/r/commits/main/src/ui"),
            commits("main", "src/ui")
        );
        assert_eq!(page("https://github.com/o/r/commits"), commits("HEAD", ""));
        assert_eq!(
            page("https://github.com/o/r/actions"),
            Route::Actions(pr.repo.clone())
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
            Route::Tags(pr.repo.clone())
        );
        assert!(matches!(
            Target::from_url("https://github.com/o/r/releases/download/v1/x.tgz"),
            Target::External(_)
        ));
        assert_eq!(
            page("https://github.com/o/r/pull/7/checks"),
            Route::Pr {
                pr: pr.clone(),
                tab: PrTab::Checks
            }
        );
        assert_eq!(
            page("https://github.com/o/r/commit/0123abc"),
            commit("0123abc")
        );
        assert_eq!(
            page(&format!("https://github.com/o/r/pull/7/commits/{sha}")),
            commit(sha)
        );
        assert_eq!(
            Target::from_url(&format!("https://github.com/o/r/commit/{sha}#diff-abc")),
            Target::Files(DiffOf::Commit(pr.repo.clone(), sha.into()))
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
            Route::Issues {
                repo: RepoId::new("o", "r"),
                query: "is:open label:\"good first issue\"".into()
            }
        );
        assert_eq!(
            page("https://github.com/o/r/labels/bug"),
            Route::Issues {
                repo: RepoId::new("o", "r"),
                query: "is:open label:bug".into()
            }
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
            Route::Milestone {
                repo: pr.repo.clone(),
                number: 3
            }
        );
        assert_eq!(
            page("https://github.com/o/r/compare/v1...fork:feature/x"),
            Route::Compare {
                repo: pr.repo.clone(),
                spec: "v1...fork:feature/x".into()
            }
        );
        let other = "f".repeat(40);
        assert_eq!(
            Target::from_url(&format!(
                "https://github.com/o/r/compare/{sha}..{other}#files"
            )),
            Target::Files(DiffOf::Range(pr.repo.clone(), sha.into(), other.clone()))
        );
        // Three dots diff from the merge base, which the comparison finds.
        assert_eq!(
            page(&format!(
                "https://github.com/o/r/compare/{sha}...{other}#files"
            )),
            Route::Compare {
                repo: pr.repo.clone(),
                spec: format!("{sha}...{other}")
            }
        );
        assert_eq!(
            compare_url(&DiffOf::Range(pr.repo.clone(), sha.into(), other.clone())),
            format!("https://github.com/o/r/compare/{sha}..{other}")
        );
        // A repository (or owner) named `blob` keeps its blame.
        let blame = Route::Blame {
            repo: RepoId::new("blob", "blob"),
            rev: "main".into(),
            path: "f".into(),
            lines: None,
        };
        assert_eq!(page(&blame.url()), blame);
        assert!(matches!(
            Target::from_url("https://github.com/settings/tokens"),
            Target::External(_)
        ));
        assert!(matches!(
            Target::from_url("https://example.com/o/r"),
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
        format!("{route:?}")
            .split(|c: char| !c.is_alphanumeric())
            .next()
            .unwrap_or_default()
            .to_owned()
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
            let target = Target::from_url(url);
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
            Route::Home,
            Route::Repo(repo.clone()),
            Route::Tree {
                repo: repo.clone(),
                rev: "main".into(),
                path: "src".into(),
            },
            Route::Blob {
                repo: repo.clone(),
                rev: "main".into(),
                path: "src/lib.rs".into(),
                lines: Some((3, 9)),
            },
            Route::Issues {
                repo: repo.clone(),
                query: "is:closed label:bug".into(),
            },
            Route::Pulls {
                repo: repo.clone(),
                query: OPEN.into(),
            },
            Route::Commit {
                repo: repo.clone(),
                oid: "abc1234".into(),
            },
            Route::Issue { repo, number: 3 },
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
            assert_eq!(page(&route.url()), route);
        }
    }

    #[test]
    fn palette_input() {
        let repo = RepoId::new("o", "r");
        assert_eq!(
            parse_input("a/b", None),
            Some(Target::Page(Route::Repo(RepoId::new("a", "b"))))
        );
        assert_eq!(
            parse_input("@octocat", None),
            Some(Target::Page(Route::user("octocat")))
        );
        assert_eq!(
            parse_input("#12", Some(&repo)),
            Some(Target::Page(Route::Issue {
                repo: repo.clone(),
                number: 12
            }))
        );
        assert_eq!(parse_input("12", None), None);
        assert_eq!(
            parse_input("a/b#3", None),
            Some(Target::Page(Route::Issue {
                repo: RepoId::new("a", "b"),
                number: 3
            }))
        );
        assert_eq!(parse_input("ratatui", None), None);
    }

    #[test]
    fn list_queries_scope_to_the_repo() {
        let route = Route::Pulls {
            repo: RepoId::new("o", "r"),
            query: "is:open author:me".into(),
        };
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
        use proptest::prelude::*;

        fn segment() -> impl Strategy<Value = String> {
            "[A-Za-z0-9][A-Za-z0-9_.-]{0,15}".prop_filter("not . or .. or .git", |s| {
                s != "." && s != ".." && !s.ends_with(".git")
            })
        }

        fn repo() -> impl Strategy<Value = RepoId> {
            (segment(), segment()).prop_map(|(o, r)| RepoId::new(o, r))
        }

        /// Any text a path segment can hold, spaces and non-ASCII included
        /// (git has no `.` or `..` segments).
        fn path() -> impl Strategy<Value = String> {
            let part =
                "[^/\\x00-\\x1f]{1,12}".prop_filter("not . or ..", |p| p != "." && p != "..");
            prop::collection::vec(part, 0..4).prop_map(|parts| parts.join("/"))
        }

        /// Filter words (an `is:` word would be dropped by design).
        fn query() -> impl Strategy<Value = String> {
            prop::collection::vec("[a-z0-9:#&+%\"]{1,8}", 1..4)
                .prop_map(|words| words.join(" "))
                .prop_filter("no is: words", |q| {
                    !q.split(' ').any(|w| w.starts_with("is:"))
                })
        }

        fn discussions_of() -> impl Strategy<Value = DiscussionsOf> {
            prop_oneof![
                repo().prop_map(DiscussionsOf::Repo),
                segment().prop_map(DiscussionsOf::Org),
            ]
        }

        fn route() -> impl Strategy<Value = Route> {
            // A valid one-segment git ref name.
            let rev = "[A-Za-z0-9_-][A-Za-z0-9._-]{0,11}"
                .prop_filter("valid ref name", |r| !r.contains("..") && !r.ends_with('.'));
            prop_oneof![
                Just(Route::Home),
                repo().prop_map(Route::Actions),
                repo().prop_map(Route::Stargazers),
                repo().prop_map(Route::Watchers),
                repo().prop_map(Route::Forks),
                repo().prop_map(Route::Releases),
                repo().prop_map(Route::Tags),
                repo().prop_map(Route::Branches),
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
                (repo(), "[0-9a-f]{40}").prop_map(|(repo, oid)| Route::CommitChecks { repo, oid }),
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
                (prop::option::of(repo()), "GHSA(-[2-9a-z]{4}){3}")
                    .prop_map(|(repo, ghsa)| Route::Advisory { repo, ghsa }),
                segment().prop_map(Route::Teams),
                (segment(), segment()).prop_map(|(org, slug)| Route::Team { org, slug }),
                (prop::option::of(segment()), "[0-9a-f]{20}|[0-9]{1,8}")
                    .prop_filter("an owner isn't a gist ID or a gist page", |(o, _)| {
                        o.as_deref()
                            .is_none_or(|o| !is_gist_id(o) && !GIST_PAGES.contains(&o))
                    })
                    .prop_map(|(owner, id)| Route::Gist { owner, id }),
                segment()
                    .prop_filter("not a gist ID or a gist page", |l| {
                        !is_gist_id(l) && !GIST_PAGES.contains(&l.as_str())
                    })
                    .prop_map(Route::Gists),
                (
                    repo(),
                    rev.clone(),
                    path(),
                    prop::option::of((1..1000u32, 1..1000u32))
                )
                    .prop_filter("a file has a path", |(_, _, p, _)| !p.is_empty())
                    .prop_map(|(repo, rev, path, lines)| Route::Blame {
                        repo,
                        rev,
                        path,
                        lines: lines.map(|(a, b)| (a.min(b), a.max(b))),
                    }),
                (repo(), rev.clone(), rev.clone(), any::<bool>()).prop_map(|(repo, a, b, two)| {
                    Route::Compare {
                        repo,
                        spec: format!("{a}{}{b}", if two { ".." } else { "..." }),
                    }
                }),
                (repo(), prop::option::of("[A-Za-z0-9 _-]{1,12}"))
                    .prop_filter("not the log", |(_, e)| e.as_deref() != Some("activity_log"))
                    .prop_map(|(repo, environment)| Route::Deployments { repo, environment }),
                (repo(), any::<bool>())
                    .prop_map(|(repo, closed)| Route::Milestones { repo, closed }),
                (repo(), 1..u64::MAX).prop_map(|(repo, number)| Route::Milestone { repo, number }),
                (repo(), "[A-Za-z0-9._-]{1,12}")
                    .prop_filter("not . or ..", |(_, t)| t != "." && t != "..")
                    .prop_map(|(repo, tag)| Route::Release { repo, tag }),
                repo().prop_map(Route::Repo),
                (repo(), rev.clone(), path()).prop_map(|(repo, rev, path)| Route::Tree {
                    repo,
                    rev,
                    path
                }),
                (
                    repo(),
                    rev.clone(),
                    path(),
                    prop::option::of((1..1000u32, 1..1000u32))
                )
                    .prop_filter("a file has a path", |(_, _, p, _)| !p.is_empty())
                    .prop_map(|(repo, rev, path, lines)| Route::Blob {
                        repo,
                        rev,
                        path,
                        lines: lines.map(|(a, b)| (a.min(b), a.max(b))),
                    }),
                (repo(), query()).prop_map(|(repo, query)| Route::Issues { repo, query }),
                (repo(), query()).prop_map(|(repo, query)| Route::Pulls { repo, query }),
                (repo(), 1..u64::MAX).prop_map(|(repo, number)| Route::Issue { repo, number }),
                (repo(), "[0-9a-f]{7,40}").prop_map(|(repo, oid)| Route::Commit { repo, oid }),
                (repo(), rev, path()).prop_map(|(repo, rev, path)| Route::Commits {
                    repo,
                    rev,
                    path
                }),
                (repo(), 1..u64::MAX).prop_map(|(repo, number)| Route::Pr {
                    pr: PrRef { repo, number },
                    tab: PrTab::Commits,
                }),
                segment()
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
                    _ if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => {
                        number.to_string()
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
                    let target = Target::from_url(&varied);
                    prop_assert!(as_expected(&target, expect, reason), "{varied}: {target:?}, not {expect}");
                    if let Target::Page(route) = &target {
                        prop_assert_eq!(Target::from_url(&route.url()), Target::Page(route.clone()), "{}", varied);
                    }
                }
            }

            /// Every page's URL leads back to it.
            #[test]
            fn urls_round_trip(route in route()) {
                prop_assert_eq!(Target::from_url(&route.url()), Target::Page(route));
            }

            /// Whatever is typed, parsing doesn't panic.
            #[test]
            fn any_input_parses_or_not(input in any::<String>()) {
                let repo = RepoId::new("o", "r");
                let _ = parse_input(&input, Some(&repo));
                let _ = Target::from_url(&input);
                let _ = decode(&input);
            }
        }
    }
}
