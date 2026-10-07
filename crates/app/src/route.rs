//! Routes: the pages ghtui shows, addressed by their github.com URLs.

use ghtui_api::browse::{RepoSort, SearchKind};
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
    Stargazers(RepoId),
    Watchers(RepoId),
    Forks(RepoId),
    Releases(RepoId),
    Release {
        repo: RepoId,
        tag: String,
    },
    Tags(RepoId),
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
            Route::Issues { repo, query } => list_url(&links::issues(repo), query),
            Route::Pulls { repo, query } => list_url(&links::pulls(repo), query),
            Route::Issue { repo, number } => links::issue(repo, *number),
            Route::Pr { pr, tab } => match tab {
                PrTab::Conversation => links::pull(pr),
                PrTab::Commits => links::pull_tab(pr, "commits"),
                PrTab::Checks => links::pull_tab(pr, "checks"),
            },
            Route::Actions(repo) => format!("{}/actions", links::repo(repo)),
            Route::Stargazers(repo) => format!("{}/stargazers", links::repo(repo)),
            Route::Watchers(repo) => format!("{}/watchers", links::repo(repo)),
            Route::Forks(repo) => format!("{}/forks", links::repo(repo)),
            Route::Releases(repo) => format!("{}/releases", links::repo(repo)),
            Route::Release { repo, tag } => links::release(repo, tag),
            Route::Tags(repo) => format!("{}/tags", links::repo(repo)),
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
            Route::Issues { repo, .. } => format!("{repo} · Issues"),
            Route::Pulls { repo, .. } => format!("{repo} · Pull requests"),
            Route::Issue { repo, number } => format!("{repo}#{number}"),
            Route::Pr { pr, .. } => pr.to_string(),
            Route::Actions(repo) => format!("{repo} · Actions"),
            Route::Stargazers(repo) => format!("{repo} · Stargazers"),
            Route::Watchers(repo) => format!("{repo} · Watchers"),
            Route::Forks(repo) => format!("{repo} · Forks"),
            Route::Releases(repo) => format!("{repo} · Releases"),
            Route::Release { repo, tag } => format!("{repo} {tag}"),
            Route::Tags(repo) => format!("{repo} · Tags"),
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
            | Route::Stargazers(repo)
            | Route::Watchers(repo)
            | Route::Forks(repo)
            | Route::Releases(repo)
            | Route::Release { repo, .. }
            | Route::Tags(repo)
            | Route::Commit { repo, .. } => Some(repo),
            Route::Pr { pr, .. } => Some(&pr.repo),
            Route::Home | Route::User { .. } | Route::Search { .. } => None,
        }
    }

    /// A list's filter: issues, pull requests, search results.
    pub fn list_query(&self) -> Option<&str> {
        match self {
            Route::Issues { query, .. }
            | Route::Pulls { query, .. }
            | Route::Search { query, .. } => Some(query),
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
        if !matches!(parsed.scheme(), "https" | "http")
            || !matches!(parsed.host_str(), Some("github.com" | "www.github.com"))
        {
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
                    // Wikis, packages, the marketplace…
                    Some(_) => return external(),
                },
                query: query.unwrap_or_default(),
            },
            ["stars", login] => Route::User {
                login: (*login).to_owned(),
                tab: ProfileTab::Stars,
            },
            ["orgs", login] => Route::user(login),
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
            // Assets are downloads.
            [_, _, "releases", "download", ..] => return external(),
            [o, r, "releases", ..] => match repo(o, r) {
                Some(repo) => Route::Releases(repo),
                None => return external(),
            },
            [o, r, "tags"] => match repo(o, r) {
                Some(repo) => Route::Tags(repo),
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
                match rest {
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
        assert!(matches!(
            Target::from_url("https://github.com/o/r/actions/runs/9"),
            Target::External(_)
        ));
        assert!(matches!(
            Target::from_url("https://github.com/o/r/milestone/3"),
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
                (any::<String>(), 0..4u8).prop_map(|(query, k)| Route::Search {
                    kind: [
                        SearchKind::Repos,
                        SearchKind::Issues,
                        SearchKind::Pulls,
                        SearchKind::Users
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
