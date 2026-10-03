//! Domain types the app works with, decoupled from the GraphQL wire shapes.
//! These are what get cached, so changing them requires bumping
//! `ghtui_store::SCHEMA_VERSION`.

use serde::{Deserialize, Serialize};

use crate::queries as q;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RepoId {
    pub owner: String,
    pub name: String,
}

impl RepoId {
    pub fn new(owner: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            owner: owner.into(),
            name: name.into(),
        }
    }

    /// Parses `owner/name`.
    pub fn parse(s: &str) -> Option<Self> {
        let (owner, name) = s.split_once('/')?;
        let name = name.strip_suffix(".git").unwrap_or(name);
        (valid_segment(owner) && valid_segment(name)).then(|| Self::new(owner, name))
    }
}

fn valid_segment(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

impl std::fmt::Display for RepoId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

/// A pull request address: `owner/repo#N`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PrRef {
    pub repo: RepoId,
    pub number: u64,
}

impl PrRef {
    /// Parses `owner/repo#N` or a github.com pull request URL (any sub-page,
    /// e.g. `/files`, is accepted).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if !s.contains("://")
            && let Some((repo, number)) = s.split_once('#')
        {
            return Some(Self {
                repo: RepoId::parse(repo)?,
                number: parse_number(number)?,
            });
        }
        let url = url::Url::parse(s).ok()?;
        if !matches!(url.scheme(), "https" | "http")
            || !matches!(url.host_str(), Some("github.com" | "www.github.com"))
        {
            return None;
        }
        let mut segments = url.path_segments()?;
        let owner = segments.next()?;
        let name = segments.next()?;
        if segments.next()? != "pull" {
            return None;
        }
        Some(Self {
            repo: RepoId::parse(&format!("{owner}/{name}"))?,
            number: parse_number(segments.next()?)?,
        })
    }

    pub fn url(&self) -> String {
        format!("https://github.com/{}/pull/{}", self.repo, self.number)
    }
}

fn parse_number(s: &str) -> Option<u64> {
    s.parse().ok().filter(|n| *n > 0)
}

impl std::fmt::Display for PrRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}#{}", self.repo, self.number)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrState {
    Open,
    Draft,
    Closed,
    Merged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewDecision {
    Approved,
    ChangesRequested,
    ReviewRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChecksState {
    Passing,
    Failing,
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mergeable {
    Yes,
    Conflicting,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    pub name: String,
    /// `rrggbb`, as GitHub returns it.
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrSummary {
    pub pr: PrRef,
    pub title: String,
    pub author: String,
    pub state: PrState,
    /// ISO 8601.
    pub updated_at: String,
    pub additions: u64,
    pub deletions: u64,
    pub comments: u64,
    pub review: Option<ReviewDecision>,
    pub checks: Option<ChecksState>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inbox {
    pub authored: Vec<PrSummary>,
    pub review_requested: Vec<PrSummary>,
    /// Total matches on GitHub; the lists hold the most recently updated.
    pub authored_total: u64,
    pub review_requested_total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrDetail {
    pub summary: PrSummary,
    pub body: String,
    pub created_at: String,
    pub base_ref: String,
    pub head_ref: String,
    pub base_oid: String,
    pub head_oid: String,
    /// `owner/name` of the head repository; `None` if it was deleted.
    pub head_repo: Option<String>,
    pub changed_files: u64,
    pub mergeable: Mergeable,
    pub labels: Vec<Label>,
}

// ---- conversions from the wire types ---------------------------------------

fn state(state: q::PullRequestState, draft: bool) -> PrState {
    match state {
        q::PullRequestState::Open if draft => PrState::Draft,
        q::PullRequestState::Open => PrState::Open,
        q::PullRequestState::Closed => PrState::Closed,
        q::PullRequestState::Merged => PrState::Merged,
    }
}

fn review(decision: Option<q::PullRequestReviewDecision>) -> Option<ReviewDecision> {
    decision.map(|d| match d {
        q::PullRequestReviewDecision::Approved => ReviewDecision::Approved,
        q::PullRequestReviewDecision::ChangesRequested => ReviewDecision::ChangesRequested,
        q::PullRequestReviewDecision::ReviewRequired => ReviewDecision::ReviewRequired,
    })
}

fn checks(commits: &q::CommitRollupConnection) -> Option<ChecksState> {
    let rollup = commits
        .nodes
        .as_ref()?
        .iter()
        .flatten()
        .last()?
        .commit
        .status_check_rollup
        .as_ref()?;
    Some(match rollup.state {
        q::StatusState::Success => ChecksState::Passing,
        q::StatusState::Failure | q::StatusState::Error => ChecksState::Failing,
        q::StatusState::Pending | q::StatusState::Expected => ChecksState::Pending,
    })
}

fn author(actor: &Option<q::Actor>) -> String {
    actor
        .as_ref()
        .map_or_else(|| "ghost".to_owned(), |a| a.login.clone())
}

fn pr_ref(name_with_owner: &str, number: i32) -> Option<PrRef> {
    Some(PrRef {
        repo: RepoId::parse(name_with_owner)?,
        number: u64::try_from(number).ok()?,
    })
}

fn count(n: i32) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

impl PrSummary {
    pub(crate) fn from_wire(pr: &q::PrSummary) -> Option<Self> {
        Some(Self {
            pr: pr_ref(&pr.repository.name_with_owner, pr.number)?,
            title: pr.title.clone(),
            author: author(&pr.author),
            state: state(pr.state, pr.is_draft),
            updated_at: pr.updated_at.0.clone(),
            additions: count(pr.additions),
            deletions: count(pr.deletions),
            comments: count(pr.comments.total_count),
            review: review(pr.review_decision),
            checks: checks(&pr.commits),
        })
    }
}

/// PRs from one search, and GitHub's total match count.
pub(crate) fn search_results(conn: &q::SearchConnection) -> (Vec<PrSummary>, u64) {
    let prs = conn
        .nodes
        .iter()
        .flatten()
        .flatten()
        .filter_map(|item| match item {
            q::SearchItem::PullRequest(pr) => PrSummary::from_wire(pr),
            q::SearchItem::Other => None,
        })
        .collect();
    (prs, count(conn.issue_count))
}

impl PrDetail {
    pub(crate) fn from_wire(pr: &q::PrDetail) -> Option<Self> {
        let summary = PrSummary {
            pr: pr_ref(&pr.repository.name_with_owner, pr.number)?,
            title: pr.title.clone(),
            author: author(&pr.author),
            state: state(pr.state, pr.is_draft),
            updated_at: pr.updated_at.0.clone(),
            additions: count(pr.additions),
            deletions: count(pr.deletions),
            comments: count(pr.comments.total_count),
            review: review(pr.review_decision),
            checks: checks(&pr.commits),
        };
        Some(Self {
            summary,
            body: pr.body.clone(),
            created_at: pr.created_at.0.clone(),
            base_ref: pr.base_ref_name.clone(),
            head_ref: pr.head_ref_name.clone(),
            base_oid: pr.base_ref_oid.0.clone(),
            head_oid: pr.head_ref_oid.0.clone(),
            head_repo: pr
                .head_repository
                .as_ref()
                .map(|r| r.name_with_owner.clone()),
            changed_files: count(pr.changed_files),
            mergeable: match pr.mergeable {
                q::MergeableState::Mergeable => Mergeable::Yes,
                q::MergeableState::Conflicting => Mergeable::Conflicting,
                q::MergeableState::Unknown => Mergeable::Unknown,
            },
            labels: pr
                .labels
                .iter()
                .flat_map(|c| c.nodes.iter().flatten().flatten())
                .map(|l| Label {
                    name: l.name.clone(),
                    color: l.color.clone(),
                })
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_short_refs() {
        let pr = PrRef::parse("rust-lang/rust#12345").unwrap();
        assert_eq!(pr.repo, RepoId::new("rust-lang", "rust"));
        assert_eq!(pr.number, 12345);
        assert_eq!(pr.to_string(), "rust-lang/rust#12345");
    }

    #[test]
    fn parses_urls() {
        for url in [
            "https://github.com/ratatui/ratatui/pull/42",
            "https://github.com/ratatui/ratatui/pull/42/files",
            "https://github.com/ratatui/ratatui/pull/42/files#diff-abc",
            "https://www.github.com/ratatui/ratatui/pull/42?w=1",
        ] {
            let pr = PrRef::parse(url).unwrap_or_else(|| panic!("{url}"));
            assert_eq!(pr.to_string(), "ratatui/ratatui#42");
        }
    }

    #[test]
    fn rejects_malformed_refs() {
        for bad in [
            "",
            "rust",
            "rust#1",
            "a/b#",
            "a/b#0",
            "a/b#x",
            "a/b/c#1",
            "../b#1",
            "https://gitlab.com/a/b/pull/1",
            "https://github.com/a/b/issues/1",
            "https://github.com/a/b",
            "ftp://github.com/a/b/pull/1",
        ] {
            assert_eq!(PrRef::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn repo_parse_strips_git_suffix() {
        assert_eq!(RepoId::parse("a/b.git"), Some(RepoId::new("a", "b")));
    }
}
