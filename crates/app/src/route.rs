//! Routes: the pages ghtui shows, addressed by their github.com URLs.

use ghtui_api::browse::SearchKind;
use ghtui_api::model::{PrRef, RepoId};
use ghtui_ui::pages::url as links;
use ghtui_ui::pages::{PrTab, ProfileTab};

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
    /// A pull request's Files changed tab: the diff screen.
    Files(PrRef),
    /// Anything ghtui doesn't show itself.
    External(String),
}

impl Route {
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
            Route::Blob { repo, rev, path } => links::blob(repo, rev, path),
            Route::Issues { repo, query } => list_url(&links::issues(repo), query),
            Route::Pulls { repo, query } => list_url(&links::pulls(repo), query),
            Route::Issue { repo, number } => links::issue(repo, *number),
            Route::Pr { pr, tab } => match tab {
                PrTab::Conversation => links::pull(pr),
                PrTab::Commits => links::pull_tab(pr, "commits"),
            },
            Route::User { login, tab } => match tab {
                ProfileTab::Overview => links::user(login),
                ProfileTab::Repositories => format!("{}?tab=repositories", links::user(login)),
                ProfileTab::Stars => format!("{}?tab=stars", links::user(login)),
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
            | Route::Issue { repo, .. } => Some(repo),
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
                    Some("issues") => SearchKind::Issues,
                    Some("pullrequests") => SearchKind::Pulls,
                    Some("users") => SearchKind::Users,
                    _ => SearchKind::Repos,
                },
                query: query.unwrap_or_default(),
            },
            ["orgs", login] => Route::user(login),
            [login] if !RESERVED.contains(login) => Route::User {
                login: (*login).to_owned(),
                tab: match tab.as_deref() {
                    Some("repositories") => ProfileTab::Repositories,
                    Some("stars") => ProfileTab::Stars,
                    _ => ProfileTab::Overview,
                },
            },
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
                    Route::Blob { repo, rev, path }
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
            [o, r, "issues", n] => match (repo(o, r), n.parse()) {
                (Some(repo), Ok(number)) => Route::Issue { repo, number },
                _ => return external(),
            },
            [o, r, "pull", n, rest @ ..] => {
                let (Some(repo), Ok(number)) = (repo(o, r), n.parse()) else {
                    return external();
                };
                let pr = PrRef { repo, number };
                match rest.first().copied() {
                    Some("files" | "changes") => return Target::Files(pr),
                    Some("commits") => Route::Pr {
                        pr,
                        tab: PrTab::Commits,
                    },
                    None => Route::Pr {
                        pr,
                        tab: PrTab::Conversation,
                    },
                    Some(_) => return external(),
                }
            }
            _ => return external(),
        };
        Target::Page(route)
    }
}

/// Top-level github.com paths that aren't users.
const RESERVED: &[&str] = &[
    "about",
    "codespaces",
    "explore",
    "features",
    "issues",
    "login",
    "marketplace",
    "new",
    "notifications",
    "pricing",
    "pulls",
    "settings",
    "sponsors",
    "topics",
    "trending",
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
mod tests {
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
            Route::Blob {
                repo: repo.clone(),
                rev: "v1.0".into(),
                path: "a b.md".into()
            }
        );
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
            Target::Files(pr)
        );
        assert_eq!(
            page("https://github.com/search?q=tui+lang%3Arust&type=repositories"),
            Route::Search {
                kind: SearchKind::Repos,
                query: "tui lang:rust".into()
            }
        );
        assert!(matches!(
            Target::from_url("https://github.com/o/r/actions"),
            Target::External(_)
        ));
        assert!(matches!(
            Target::from_url("https://example.com/o/r"),
            Target::External(_)
        ));
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
            },
            Route::Issues {
                repo: repo.clone(),
                query: "is:closed label:bug".into(),
            },
            Route::Pulls {
                repo: repo.clone(),
                query: OPEN.into(),
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

        /// Any text a path segment can hold, spaces and non-ASCII included.
        fn path() -> impl Strategy<Value = String> {
            prop::collection::vec("[^/\\x00-\\x1f]{1,12}", 0..4).prop_map(|parts| parts.join("/"))
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
            let rev = "[A-Za-z0-9._-]{1,12}";
            prop_oneof![
                Just(Route::Home),
                repo().prop_map(Route::Repo),
                (repo(), rev, path()).prop_map(|(repo, rev, path)| Route::Tree { repo, rev, path }),
                (repo(), rev, path())
                    .prop_filter("a file has a path", |(_, _, p)| !p.is_empty())
                    .prop_map(|(repo, rev, path)| Route::Blob { repo, rev, path }),
                (repo(), query()).prop_map(|(repo, query)| Route::Issues { repo, query }),
                (repo(), query()).prop_map(|(repo, query)| Route::Pulls { repo, query }),
                (repo(), 1..u64::MAX).prop_map(|(repo, number)| Route::Issue { repo, number }),
                (repo(), 1..u64::MAX).prop_map(|(repo, number)| Route::Pr {
                    pr: PrRef { repo, number },
                    tab: PrTab::Commits,
                }),
                segment()
                    .prop_filter("not a reserved path", |l| !RESERVED.contains(&l.as_str()))
                    .prop_map(|login| Route::User {
                        login,
                        tab: ProfileTab::Stars,
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

        proptest! {
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
