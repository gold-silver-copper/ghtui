use super::*;

pub const BASE: &str = "https://github.com";

pub fn user(login: &str) -> String {
    format!("{BASE}/{login}")
}
pub fn repo(repo: &RepoId) -> String {
    format!("{BASE}/{repo}")
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
    format!("{BASE}/{repo}/issues")
}
pub fn pulls(repo: &RepoId) -> String {
    format!("{BASE}/{repo}/pulls")
}
pub fn issue(repo: &RepoId, number: u64) -> String {
    format!("{BASE}/{repo}/issues/{number}")
}
pub fn pull(pr: &PrRef) -> String {
    format!("{BASE}/{}/pull/{}", pr.repo, pr.number)
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
    format!("{BASE}/search?q={}&type={kind}", encode(query))
}
pub fn commit(repo: &RepoId, oid: &str) -> String {
    at_rev(repo, "commit", oid, "")
}
pub fn compare(repo: &RepoId, from: &str, to: &str) -> String {
    format!("{}/compare/{from}..{to}", self::repo(repo))
}
pub fn release(repo: &RepoId, tag: &str) -> String {
    format!("{BASE}/{repo}/releases/tag/{}", encode_path(tag))
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
        format!("{BASE}/{repo}/{view}/{rev}")
    } else {
        format!("{BASE}/{repo}/{view}/{rev}/{path}")
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
