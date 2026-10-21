use super::*;
use crate::api::explore::Since;
use crate::api::packages::{PackageRef, PackagesOf};
use crate::api::project::ProjectRef;
use std::fmt::Write as _;

/// The website of the GitHub ghtui works with.
pub fn base() -> String {
    crate::host::current().web()
}
/// Gists' website.
pub fn gists() -> String {
    crate::host::current().gists()
}
/// A file's raw content, to download.
pub fn raw(repo: &RepoId, rev: &str, path: &str) -> String {
    format!("{}/{repo}/{rev}/{path}", crate::host::current().raw())
}

pub fn user(login: &str) -> String {
    format!("{}/{login}", base())
}
/// A team, `org/slug`.
pub fn team(org: &str, slug: &str) -> String {
    format!("{}/orgs/{org}/teams/{slug}", base())
}
pub fn repo(repo: &RepoId) -> String {
    format!("{}/{repo}", base())
}
pub fn tree(repo: &RepoId, rev: &str, path: &str) -> String {
    at_rev(repo, "tree", rev, &encode_path(path))
}
pub fn blob(repo: &RepoId, rev: &str, path: &str) -> String {
    at_rev(repo, "blob", rev, &encode_path(path))
}
pub fn blame(repo: &RepoId, rev: &str, path: &str) -> String {
    at_rev(repo, "blame", rev, &encode_path(path))
}
pub fn issues(repo: &RepoId) -> String {
    format!("{}/{repo}/issues", base())
}
pub fn pulls(repo: &RepoId) -> String {
    format!("{}/{repo}/pulls", base())
}
pub fn issue(repo: &RepoId, number: u64) -> String {
    format!("{}/{repo}/issues/{number}", base())
}
pub fn pull(pr: &PrRef) -> String {
    format!("{}/{}/pull/{}", base(), pr.repo, pr.number)
}
pub fn pull_tab(pr: &PrRef, tab: &str) -> String {
    format!("{}/{tab}", pull(pr))
}
/// A file in a pull request's Files changed, at a line (`R12` new,
/// `L12` old) if given.
pub fn diff_line(pr: &PrRef, path: &str, line: Option<(char, u32)>) -> String {
    let line = line.map(|(side, n)| format!("{side}{n}"));
    diff_file(&pull_tab(pr, "files"), path, &line.unwrap_or_default())
}
/// A file in the diff on `page` (a pull request's files, a commit, a
/// comparison), at `lines` (`R10-R12`), as GitHub anchors them.
pub fn diff_file(page: &str, path: &str, lines: &str) -> String {
    format!("{page}#diff-{}{lines}", path_hash(path))
}
/// How GitHub names a file in its diffs' anchors: the SHA-256 of its path,
/// in hex.
pub fn path_hash(path: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hex = String::with_capacity(64);
    for b in Sha256::digest(path.as_bytes()) {
        let _ = write!(hex, "{b:02x}");
    }
    hex
}
/// A commit's or a comparison's files, from its page.
pub fn files(page: &str) -> String {
    format!("{page}#files")
}
pub fn search(kind: SearchKind, query: &str) -> String {
    let kind = match kind {
        SearchKind::Repos => "repositories",
        SearchKind::Issues => "issues",
        SearchKind::Pulls => "pullrequests",
        SearchKind::Users => "users",
        SearchKind::Discussions => "discussions",
        SearchKind::Commits => "commits",
        SearchKind::Code => "code",
    };
    format!("{}/search?q={}&type={kind}", base(), encode(query))
}
pub fn commit(repo: &RepoId, oid: &str) -> String {
    at_rev(repo, "commit", oid, "")
}
pub fn compare(repo: &RepoId, from: &str, to: &str) -> String {
    format!("{}/compare/{from}..{to}", self::repo(repo))
}
/// A label's link: its open issues.
pub fn label(repo: &RepoId, name: &str) -> String {
    format!("{}/labels/{}", self::repo(repo), encode_path(name))
}
pub fn release(repo: &RepoId, tag: &str) -> String {
    format!("{}/{repo}/releases/tag/{}", base(), encode_path(tag))
}
/// A revision's history, of `path` if it isn't empty.
pub fn commits(repo: &RepoId, rev: &str, path: &str) -> String {
    at_rev(repo, "commits", rev, &encode_path(path))
}
/// A revision's page, `/<repo>/<view>/<rev>[/<path>]`. The rev is one
/// segment, its '/' escaped (%2F, which GitHub resolves), so `feature/x`
/// can't be read as `feature` and a path. `path` is already URL-encoded.
pub(crate) fn at_rev(repo: impl std::fmt::Display, view: &str, rev: &str, path: &str) -> String {
    let rev = escape(rev, b"-_.~", false);
    if path.is_empty() {
        format!("{}/{repo}/{view}/{rev}", base())
    } else {
        format!("{}/{repo}/{view}/{rev}/{path}", base())
    }
}

/// A file path in a URL: `/` separates segments, everything else that
/// isn't plainly safe (`#`, `?`, `%`, `\\`, spaces, non-ASCII) is
/// escaped.
pub fn encode_path(s: &str) -> String {
    escape(s, b"-_.~/", false)
}

/// Minimal query-string encoding.
pub fn encode(s: &str) -> String {
    escape(s, b"-_.~:/@", true)
}

/// `s` with every byte but ASCII alphanumerics and `safe` escaped, and
/// spaces as `+` when `plus`.
fn escape(s: &str, safe: &[u8], plus: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || safe.contains(&b) {
            out.push(char::from(b));
        } else if b == b' ' && plus {
            out.push('+');
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// A repository's Insights, at a tab.
pub fn insights(repo: &RepoId, tab: InsightsTab) -> String {
    format!("{}/{}", self::repo(repo), tab.path())
}
/// Trending: new repositories, in a language if given.
pub fn explore(language: Option<&str>, since: Since) -> String {
    let mut url = format!("{}/trending", base());
    if let Some(language) = language {
        // As GitHub's Trending URLs write it (`c++`, `visual-basic-.net`).
        url.push('/');
        url.push_str(&encode_path(
            &language.trim().to_lowercase().replace(' ', "-"),
        ));
    }
    if since != Since::Daily {
        let _ = write!(url, "?since={}", since.slug());
    }
    url
}
pub fn topic(name: &str) -> String {
    format!("{}/topics/{}", base(), encode_path(name))
}
pub fn star_list(login: &str, slug: &str) -> String {
    format!("{}/stars/{login}/lists/{}", base(), encode_path(slug))
}
pub fn packages(of: &PackagesOf) -> String {
    match of {
        PackagesOf::Owner(login) => format!("{}?tab=packages", user(login)),
        PackagesOf::Repo(repo) => format!("{}/packages", self::repo(repo)),
    }
}
/// A package, at one of its versions (else its newest).
pub fn package(at: &PackageRef, version: Option<u64>) -> String {
    let (owner, kind, name) = (&at.owner, at.kind.slug(), encode_path(&at.name));
    match version {
        None => format!("{}/users/{owner}/packages/{kind}/package/{name}", base()),
        Some(id) => format!("{}/users/{owner}/packages/{kind}/{name}/{id}", base()),
    }
}
/// A project at one of its views (the project, without one).
pub fn project_view(project: &ProjectRef, view: Option<u64>) -> String {
    match view {
        Some(n) => format!("{}/views/{n}", project.url()),
        None => project.url(),
    }
}
/// An item in a project's view's side pane.
pub fn project_item(project: &ProjectRef, view: Option<u64>, item: u64) -> String {
    format!("{}?pane=issue&itemId={item}", project_view(project, view))
}
