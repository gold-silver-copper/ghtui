use super::*;

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
    let hash = crate::diff_screen::path_hash(path);
    let line = line.map(|(side, n)| format!("{side}{n}"));
    format!(
        "{}#diff-{hash}{}",
        pull_tab(pr, "files"),
        line.unwrap_or_default()
    )
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
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || safe.contains(&b) {
            out.push(char::from(b));
        } else if b == b' ' && plus {
            out.push('+');
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
