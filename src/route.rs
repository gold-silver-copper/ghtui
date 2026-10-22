//! Routes: the pages ghtui shows, addressed by their GitHub URLs.
//!
//! Pages come in families, each one enum in its own file that says, in
//! one place, what each of its pages is: its URL and title, the page above
//! it, what it needs fetched, and how it's built ([`Family`]). [`Route`]
//! is one of them, and asks the family.

pub mod app;
pub mod code;
pub mod discussions;
pub mod gists;
pub mod issues;
pub mod people;
pub mod projects;
pub mod pulls;
pub mod releases;
pub mod runs;
pub mod search;
pub mod security;
pub mod settings;
pub mod wiki;

use std::borrow::Cow;

use crate::api::browse::SearchKind;
use crate::api::model::{PrRef, RepoId};
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
    /// Searches, and your notifications.
    Search(search::Page),
    /// A repository's Settings tab.
    Settings(settings::Page),
    /// Projects.
    Projects(projects::Page),
    /// Security advisories and alerts.
    Security(security::Page),
    /// A repository's wiki.
    Wiki(wiki::Page),
    /// Discussions, a repository's or an organization's.
    Discussions(discussions::Page),
    /// Gists.
    Gists(gists::Page),
    /// Profiles, teams, and a repository's stargazers, watchers and forks.
    People(people::Page),
    /// Its releases.
    Releases(releases::Page),
    /// Its Actions: runs, jobs, workflows and deployments.
    Runs(runs::Page),
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
    /// The form the page is, if it's one.
    fn form(&self) -> Option<&'static crate::forms::Kind> {
        None
    }
}

impl Route {
    /// The family the page is one of, which says what the page is.
    pub fn family(&self) -> &dyn Family {
        match self {
            Route::App(p) => p,
            Route::Code(p) => p,
            Route::Issues(p) => p,
            Route::Pulls(p) => p,
            Route::Search(p) => p,
            Route::Settings(p) => p,
            Route::Projects(p) => p,
            Route::Security(p) => p,
            Route::Wiki(p) => p,
            Route::Discussions(p) => p,
            Route::Gists(p) => p,
            Route::People(p) => p,
            Route::Releases(p) => p,
            Route::Runs(p) => p,
            Route::Site(p) => p,
            Route::Housekeeping(p) => p,
        }
    }

    pub fn url(&self) -> String {
        self.family().url()
    }

    /// The short name shown in the top bar.
    pub fn title(&self) -> String {
        self.family().title()
    }

    /// The repository the page belongs to.
    pub fn repo(&self) -> Option<&RepoId> {
        self.family().repo()
    }

    /// A pull request's conversation.
    pub fn pr(pr: PrRef) -> Route {
        Route::Pulls(pulls::Pr {
            pr,
            tab: PrTab::Conversation,
        })
    }

    /// An issue.
    pub fn issue(repo: RepoId, number: u64) -> Route {
        Route::Issues(issues::Issue { repo, number })
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

    /// A profile's overview.
    pub fn user(login: &str) -> Route {
        Route::People(people::User {
            login: login.to_owned(),
            tab: ProfileTab::Overview,
        })
    }

    /// A repository's Code tab: its overview.
    pub fn code(repo: RepoId) -> Route {
        Route::Code(code::Repo(repo))
    }

    /// A repository's Actions tab: all its runs.
    pub fn actions(repo: RepoId) -> Route {
        Route::Runs(runs::Actions {
            repo,
            query: String::new(),
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

    /// The search behind a list page.
    pub fn search(&self) -> Option<(SearchKind, String)> {
        let key = match self {
            Route::Issues(p @ issues::Issues { .. }) => p.paged(),
            Route::Pulls(p @ pulls::Pulls { .. }) => p.paged(),
            Route::Search(p @ search::Search { .. }) => p.paged(),
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
        let parsed = if has_host_prefix(s) {
            url::Url::parse(&format!("https://{s}"))
        } else {
            url::Url::parse(s)
        };
        let parsed = parsed.ok();
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
const TOP: &[fn(&Url<'_>) -> Option<Target>] = &[
    app::parse,
    search::parse,
    projects::parse,
    security::parse,
    discussions::parse,
    crate::housekeeping::parse,
    // A profile is any name GitHub hasn't taken.
    people::parse,
];

/// What they make of the path after `/<owner>/<repo>`, asked in this
/// order: housekeeping's forms (`milestones/new`) before the milestones.
const REPO: &[RepoParse] = &[
    crate::housekeeping::parse_repo,
    code::parse_repo,
    issues::parse_repo,
    pulls::parse_repo,
    runs::parse_repo,
    releases::parse_repo,
    discussions::parse_repo,
    wiki::parse_repo,
    people::parse_repo,
    projects::parse_repo,
    security::parse_repo,
    settings::parse_repo,
];

/// What a URL opens in ghtui, or `None` for the browser.
fn parse(parsed: &url::Url) -> Option<Target> {
    if !matches!(parsed.scheme(), "https" | "http") {
        return None;
    }
    let host = crate::host::current();
    let name = crate::host::url_host(parsed)?;
    if host.serves_gists(&name) {
        return gists::parse(parsed.path_segments()?);
    }
    if !host.serves(&name) {
        return None;
    }
    // An Enterprise Server's gists are under `/gist`.
    if host.is_enterprise()
        && let Some(mut segments) = parsed.path_segments()
        && segments.next() == Some("gist")
    {
        return gists::parse(segments);
    }
    let segments: Vec<Cow<'_, str>> = parsed
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).map(decode).collect())
        .unwrap_or_default();
    let segments: Vec<&str> = segments.iter().map(|s| &**s).collect();
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
    let [owner, name, rest @ ..] = segments.as_slice() else { return None };
    if is_reserved(owner) {
        return None;
    }
    let repo = RepoId::parse(&format!("{owner}/{name}"))?;
    REPO.iter().find_map(|parse| parse(&repo, rest, &url))
}

/// A query parameter's value.
fn param(url: &url::Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
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
    "search",
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

fn decode(segment: &str) -> Cow<'_, str> {
    if !segment.contains('%') {
        return Cow::Borrowed(segment);
    }
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
    Cow::Owned(String::from_utf8_lossy(&out).into_owned())
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
        Some(Target::Page(Route::issue(repo, number)).into())
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
        return RepoId::parse(input).map(|r| Target::Page(Route::code(r)).into());
    }
    None
}

/// Whether `s` is one of GitHub's own first path segments (settings,
/// the marketplace…), never an owner.
pub(crate) fn is_reserved(s: &str) -> bool {
    RESERVED.contains(&s)
}

/// Whether `s` can be a GitHub login: up to 39 letters, digits and
/// dashes, not starting with a dash.
pub(crate) fn valid_login(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 39
        && !s.starts_with('-')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Why `name` can't name a branch or tag, as `git check-ref-format
/// --branch` has it: no spaces or control characters, none of `~^:?*[\`,
/// no `..` or `@{`, not `@` or starting with `-`, and no `/`-separated
/// part that is empty, starts with `.` or ends with `.lock`.
pub(crate) fn ref_problem(name: &str) -> Option<&'static str> {
    let bad = |c: char| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c);
    let part = |p: &str| p.is_empty() || p.starts_with('.') || p.ends_with(".lock");
    if name.is_empty() {
        Some("Type its name")
    } else if name.chars().any(bad) || name.contains("..") || name.contains("@{") || name == "@" {
        Some("A name can't have spaces, .., @{, or any of ~^:?*[\\")
    } else if name.starts_with('-') || name.ends_with('.') || name.split('/').any(part) {
        Some("A name can't start with / . or -, or end with / . or .lock")
    } else {
        None
    }
}

/// A revision as git allows ref names and SHAs to be written.
fn valid_rev(s: &str) -> bool {
    ref_problem(s).is_none()
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
                    && !r.starts_with('-')
                    && r != "@"
                    && !r.split('/').any(|c| c.ends_with(".lock"))
            })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::api::browse::RepoSort;
    use crate::api::project::{ProjectOwner, ProjectRef, ProjectsOf};
    use crate::git::Oid;
    use crate::route::security::valid_ghsa;

    #[test]
    fn ref_names_are_gits() {
        assert_eq!(ref_problem("feature/x"), None);
        assert_eq!(ref_problem("v1.2.0"), None);
        for bad in [
            "has space",
            "a..b",
            "-x",
            "x.lock",
            "a.lock/b",
            "a/.b",
            "a//b",
            "@",
        ] {
            assert!(ref_problem(bad).is_some(), "{bad}");
        }
    }

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
        let pr = PrRef {
            repo: repo.clone(),
            number: 7,
        };
        let sha = "0123456789abcdef0123456789abcdef01234567";
        let other = "f".repeat(40);
        let commit = |oid: &str, pr| {
            Route::Code(code::Commit {
                repo: repo.clone(),
                oid: oid.into(),
                pr,
            })
        };
        let commits = |rev: &str| {
            Route::Code(code::Commits {
                repo: repo.clone(),
                rev: rev.into(),
                path: String::new(),
            })
        };
        let issues = |query: &str| {
            Route::Issues(issues::Issues {
                repo: repo.clone(),
                query: query.into(),
            })
        };
        let user = |login: &str, tab| {
            Route::People(people::User {
                login: login.into(),
                tab,
            })
        };
        let notifications = |query: &str| {
            Route::Search(search::Notifications {
                query: query.into(),
            })
        };
        let pr_tab = |tab| {
            Route::Pulls(pulls::Pr {
                pr: pr.clone(),
                tab,
            })
        };
        let compare = |spec: String| {
            Route::Code(code::Compare {
                repo: repo.clone(),
                spec,
            })
        };
        let search = |kind, query: &str| {
            Route::Search(search::Search {
                kind,
                query: query.into(),
            })
        };
        for (url, want) in [
            ("https://github.com/".to_owned(), Route::App(app::Home)),
            ("https://github.com/octocat".into(), Route::user("octocat")),
            ("https://github.com/o/r".into(), Route::code(repo.clone())),
            (
                "https://github.com/o/r/issues?q=is%3Aissue+is%3Aclosed".into(),
                issues("is:closed"),
            ),
            (
                "https://github.com/o/r/pulls".into(),
                Route::Pulls(pulls::Pulls {
                    repo: repo.clone(),
                    query: OPEN.into(),
                }),
            ),
            (
                "https://github.com/o/r/issues/5".into(),
                Route::issue(repo.clone(), 5),
            ),
            (
                "https://github.com/o/r/pull/7/commits".into(),
                pr_tab(PrTab::Commits),
            ),
            (
                "https://github.com/o/r/pull/7/checks".into(),
                pr_tab(PrTab::Checks),
            ),
            (
                format!("https://github.com/o/r/commit/{sha}"),
                commit(sha, None),
            ),
            (
                "https://github.com/o/r/commit/0123abc".into(),
                commit("0123abc", None),
            ),
            // A pull request's commit knows it.
            (
                format!("https://github.com/o/r/pull/7/commits/{sha}"),
                commit(sha, Some(7)),
            ),
            ("https://github.com/o/r/commits".into(), commits("HEAD")),
            (
                "https://github.com/o/r/actions".into(),
                Route::actions(repo.clone()),
            ),
            (
                "https://github.com/o/r/actions?query=branch%3Amain+is%3Afailure".into(),
                Route::Runs(runs::Actions {
                    repo: repo.clone(),
                    query: "branch:main is:failure".into(),
                }),
            ),
            (
                "https://github.com/o/r/actions/runs/9".into(),
                Route::Runs(runs::WorkflowRun {
                    repo: repo.clone(),
                    run: 9,
                    attempt: None,
                }),
            ),
            (
                "https://github.com/o/r/stargazers".into(),
                Route::People(people::Stargazers(repo.clone())),
            ),
            (
                "https://github.com/o/r/network/members".into(),
                Route::People(people::Forks(repo.clone())),
            ),
            (
                "https://github.com/o/r/releases/tag/v1.0/rc".into(),
                Route::Releases(releases::Release {
                    repo: repo.clone(),
                    tag: "v1.0/rc".into(),
                }),
            ),
            (
                "https://github.com/o/r/releases/latest".into(),
                Route::Releases(releases::Releases(repo.clone())),
            ),
            (
                "https://github.com/o/r/tags".into(),
                Route::Code(code::Tags(repo.clone())),
            ),
            (
                "https://github.com/search?q=tui+lang%3Arust&type=repositories".into(),
                search(SearchKind::Repos, "tui lang:rust"),
            ),
            // A topic search without a topic is an empty search, not `topic:`.
            (
                "https://github.com/search?type=topics".into(),
                search(SearchKind::Repos, ""),
            ),
            (
                "https://github.com/o/r/labels/good%20first%20issue".into(),
                issues("is:open label:\"good first issue\""),
            ),
            (
                "https://github.com/o/r/labels/bug".into(),
                issues("is:open label:bug"),
            ),
            (
                "https://github.com/octocat?tab=repositories&sort=stargazers".into(),
                user("octocat", ProfileTab::Repositories(RepoSort::Stars)),
            ),
            (
                "https://github.com/octocat?tab=followers".into(),
                user("octocat", ProfileTab::Followers),
            ),
            (
                "https://github.com/orgs/rust-lang/people".into(),
                user("rust-lang", ProfileTab::People),
            ),
            (
                "https://github.com/orgs/rust-lang/repositories".into(),
                user("rust-lang", ProfileTab::Repositories(RepoSort::Updated)),
            ),
            (
                "https://github.com/o/r/milestone/3".into(),
                Route::Issues(issues::Milestone {
                    repo: repo.clone(),
                    number: 3,
                }),
            ),
            (
                "https://github.com/o/r/compare/v1...fork:feature/x".into(),
                compare("v1...fork:feature/x".into()),
            ),
            // Three dots diff from the merge base, which the comparison finds.
            (
                format!("https://github.com/o/r/compare/{sha}...{other}#files"),
                compare(format!("{sha}...{other}")),
            ),
            // A short number alone is someone's gists; a long one, an old gist.
            (
                "https://gist.github.com/2000".into(),
                Route::Gists(gists::Gists("2000".into())),
            ),
            (
                "https://gist.github.com/1162032".into(),
                Route::Gists(gists::Gist {
                    owner: None,
                    id: "1162032".into(),
                }),
            ),
            ("https://github.com/notifications".into(), notifications("")),
            (
                "https://github.com/notifications?query=is%3Aunread+repo%3Ao%2Fr".into(),
                notifications("is:unread repo:o/r"),
            ),
        ] {
            assert_eq!(page(&url), want, "{url}");
        }
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
            page("https://github.com/o/r/commits/main/src/ui").split(None),
            Some(Route::Code(code::Commits {
                repo: repo.clone(),
                rev: "main".into(),
                path: "src/ui".into(),
            }))
        );
        let (from, to) = (Oid::parse(sha).unwrap(), Oid::parse(&other).unwrap());
        for (url, want) in [
            (
                "https://github.com/o/r/pull/7/files".to_owned(),
                DiffOf::Pr(pr.clone()),
            ),
            (
                format!("https://github.com/o/r/commit/{sha}#diff-abc"),
                DiffOf::Commit(repo.clone(), from.clone()),
            ),
            (
                format!("https://github.com/o/r/compare/{sha}..{other}#files"),
                DiffOf::Range(repo.clone(), from.clone(), to.clone()),
            ),
        ] {
            assert_eq!(Dest::from_url(&url).target, Target::Files(want), "{url}");
        }
        for url in [
            "https://github.com/o/r/releases/download/v1/x.tgz",
            "https://github.com/settings/tokens",
            "https://example.com/o/r",
        ] {
            assert!(
                matches!(Dest::from_url(url).target, Target::External(_)),
                "{url}"
            );
        }
        // A diff's link leads back to it, and a link to its lines to them.
        let line = |side, line| crate::diff::anchor::LinePos { side, line };
        let lines = (
            line(crate::diff::anchor::Side::Left, 3),
            line(crate::diff::anchor::Side::Right, 5),
        );
        let anchor = format!("diff-{}L3-R5", crate::ui::pages::url::path_hash("a.rs"));
        for of in [
            DiffOf::Pr(pr.clone()),
            DiffOf::Commit(repo.clone(), from.clone()),
            DiffOf::Range(repo.clone(), from, to),
        ] {
            assert_eq!(Dest::from_url(&of.url()).target, Target::Files(of.clone()));
            let dest = Dest::from_url(&of.file_url("a.rs", Some(lines)));
            assert_eq!(dest.target, Target::Files(of.clone()));
            assert_eq!(dest.anchor.as_deref(), Some(anchor.as_str()));
        }
        // A repository (or owner) named `blob` keeps its blame.
        let blame = Route::Code(code::Blame {
            repo: RepoId::new("blob", "blob"),
            rev: "main".into(),
            path: "f".into(),
            lines: None,
        });
        assert_eq!(reopened(&blame), Target::Page(blame));
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
        // A family's page by its own name; housekeeping's forms, and a
        // settings page, by the family's.
        let family = word(&named);
        match named.strip_prefix(&format!("{family}(")).map(word) {
            Some(page) if family != "Housekeeping" && page != "Page" => page,
            _ => family,
        }
    }

    /// Whether `target` is what the corpus says: "not built yet" isn't a
    /// reason to open the browser.
    pub(crate) fn as_expected(target: &Target, expect: &str, reason: Option<&str>) -> bool {
        match (expect, target) {
            ("external", Target::External(_)) => reason.is_some_and(|r| REASONS.contains(&r)),
            ("files", Target::Files(_)) => true,
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

        let repo = Route::code(RepoId::new("o", "r"));
        assert_eq!(repo.url(), "https://ghe.example.com/o/r");
        let gist = Route::Gists(gists::Gist {
            owner: Some("o".into()),
            id: "1162032".into(),
        });
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
        let repo = Target::Page(Route::code(RepoId::new("o", "r")));
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
                Route::Projects(projects::Projects(ProjectsOf::Owner(org.owner.clone()))),
            ),
            (
                "https://github.com/users/octocat/projects",
                Route::Projects(projects::Projects(ProjectsOf::Owner(user.owner.clone()))),
            ),
            (
                "https://github.com/octocat?tab=projects",
                Route::Projects(projects::Projects(ProjectsOf::Owner(user.owner.clone()))),
            ),
            (
                "https://github.com/o/r/projects",
                Route::Projects(projects::Projects(ProjectsOf::Repo(RepoId::new("o", "r")))),
            ),
            (
                "https://github.com/orgs/octo-org/projects/5",
                Route::Projects(projects::Project {
                    project: org.clone(),
                    view: None,
                }),
            ),
            (
                "https://github.com/users/octocat/projects/3/views/2",
                Route::Projects(projects::Project {
                    project: user,
                    view: Some(2),
                }),
            ),
            (
                "https://github.com/orgs/octo-org/projects/5/views/1?pane=issue&itemId=42&issue=o%7Cr%7C1",
                Route::Projects(projects::ProjectItem {
                    project: org,
                    view: Some(1),
                    item: 42,
                }),
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
            Some(Target::Page(Route::code(RepoId::new("a", "b"))))
        );
        assert_eq!(
            target("@octocat", None),
            Some(Target::Page(Route::user("octocat")))
        );
        assert_eq!(
            target("#12", Some(&repo)),
            Some(Target::Page(Route::issue(repo.clone(), 12)))
        );
        assert_eq!(target("12", None), None);
        assert_eq!(
            target("a/b#3", None),
            Some(Target::Page(Route::issue(RepoId::new("a", "b"), 3)))
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

        /// Any page, each family as often as it has kinds of page.
        fn route() -> impl Strategy<Value = Route> {
            prop_oneof![
                1 => app::arb(),
                7 => crate::site::arb(),
                10 => code::arb(),
                5 => issues::arb(),
                3 => pulls::arb(),
                5 => runs::arb(),
                4 => releases::arb(),
                6 => people::arb(),
                4 => gists::arb(),
                2 => discussions::arb(),
                2 => wiki::arb(),
                4 => security::arb(),
                3 => projects::arb(),
                1 => settings::arb(),
                2 => search::arb(),
            ]
        }

        /// A corpus URL with its owner and repository, numbers and commit
        /// IDs swapped for `owner`, `name`, `number` and `sha`.
        fn vary(url: &str, owner: &str, name: &str, number: u64, sha: &str) -> String {
            let Ok(mut parsed) = url::Url::parse(url) else { return url.to_owned() };
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
