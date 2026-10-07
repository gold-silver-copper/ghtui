//! Domain types the app works with, decoupled from the GraphQL wire shapes.
//! These are what get cached, so changing them requires bumping
//! `ghtui_store::SCHEMA_VERSION`.

use ghtui_schema::schema;
use serde::{Deserialize, Serialize};

use crate::queries::{self as q, nodes};

/// Some of a list: the items fetched, and how many GitHub has. A list
/// fetched with `first:` or `last:` is kept as one of these, so what shows
/// it can say what it left out ("+3", "20 of 45").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capped<T> {
    pub items: Vec<T>,
    /// Never fewer than `items`.
    pub total: u64,
}

impl<T> Capped<T> {
    /// `items` of `total` (raised to the items' count if GitHub said fewer).
    pub fn new(items: Vec<T>, total: u64) -> Self {
        let total = total.max(items.len() as u64);
        Self { items, total }
    }

    /// From a connection's total and nodes; its null nodes (ones your
    /// token can't see) count as left out.
    pub(crate) fn from_nodes<N>(
        total: i32,
        list: Option<Vec<Option<N>>>,
        f: impl FnMut(N) -> T,
    ) -> Self {
        Self::new(
            nodes(list).map(f).collect(),
            u64::try_from(total).unwrap_or_default(),
        )
    }

    /// How many GitHub has that aren't here.
    pub fn left_out(&self) -> u64 {
        self.total.saturating_sub(self.items.len() as u64)
    }
}

impl<T> Default for Capped<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            total: 0,
        }
    }
}

/// All of a list.
impl<T> From<Vec<T>> for Capped<T> {
    fn from(items: Vec<T>) -> Self {
        Self::new(items, 0)
    }
}

impl<T> std::ops::Deref for Capped<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &self.items
    }
}

impl<T> std::ops::DerefMut for Capped<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.items
    }
}

impl<'a, T> IntoIterator for &'a Capped<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

/// A GitHub GraphQL node ID (`PR_kwDO…`, `PRRT_…`): what mutations name
/// things by. Its own type so it can't be mixed up with a path, a SHA or
/// another string.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(String);

impl NodeId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn gql(&self) -> cynic::Id {
        cynic::Id::new(&self.0)
    }
}

impl From<cynic::Id> for NodeId {
    fn from(id: cynic::Id) -> Self {
        Self(id.into_inner())
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

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
    pub labels: Capped<Label>,
    #[serde(default)]
    pub milestone: Option<MilestoneRef>,
}

/// The milestone an issue or pull request is in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MilestoneRef {
    pub number: u64,
    pub title: String,
}

impl MilestoneRef {
    pub(crate) fn from_wire(m: q::MilestoneName) -> Self {
        Self {
            number: count(m.number),
            title: m.title,
        }
    }
}

#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq)]
#[cynic(graphql_type = "FileViewedState", schema_module = "schema")]
pub enum ViewedState {
    Unviewed,
    Viewed,
    /// Viewed, but the file changed since.
    Dismissed,
}

/// Per-file viewed state for one PR, and the PR's node ID (needed to change
/// it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewedFiles {
    pub pull_request_id: NodeId,
    pub states: std::collections::HashMap<String, ViewedState>,
}

#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cynic(graphql_type = "DiffSide", schema_module = "schema")]
pub enum Side {
    /// Base (old) lines.
    Left,
    /// Head (new) lines.
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewComment {
    pub id: NodeId,
    pub author: String,
    pub body: String,
    /// ISO 8601.
    pub created_at: String,
    pub url: String,
    /// The commit the comment was originally made on.
    pub original_commit: Option<String>,
    /// Part of the viewer's unsubmitted review on GitHub.
    pub pending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewThread {
    pub id: NodeId,
    pub path: String,
    pub side: Side,
    pub start_side: Option<Side>,
    /// Current line; `None` when the thread is outdated.
    pub line: Option<u32>,
    pub start_line: Option<u32>,
    /// Line on the comment's original commit.
    pub original_line: Option<u32>,
    pub original_start_line: Option<u32>,
    pub outdated: bool,
    pub resolved: bool,
    /// A comment on the whole file rather than a line.
    pub file_level: bool,
    pub can_reply: bool,
    pub can_resolve: bool,
    pub can_unresolve: bool,
    /// The first comment and the newest replies.
    pub comments: Capped<ReviewComment>,
}

/// GitHub's patch for one file (from REST `pulls/{n}/files`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PatchFile {
    pub filename: String,
    #[serde(default)]
    pub previous_filename: Option<String>,
    /// Missing for binary and very large diffs.
    #[serde(default)]
    pub patch: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewEvent {
    Comment,
    Approve,
    RequestChanges,
}

/// A new thread in a pending review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewThread {
    pub path: String,
    pub body: String,
    /// `None` for a file-level comment.
    pub line: Option<u32>,
    pub side: Side,
    pub start_line: Option<u32>,
    pub start_side: Option<Side>,
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

pub(crate) fn review(decision: Option<q::PullRequestReviewDecision>) -> Option<ReviewDecision> {
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

pub(crate) fn author(actor: Option<q::Actor>) -> String {
    actor.map_or_else(|| "ghost".to_owned(), |a| a.login)
}

pub(crate) fn labels(labels: Option<q::LabelConnection>) -> Capped<Label> {
    labels.map_or_else(Capped::default, |l| {
        Capped::from_nodes(l.total_count, l.nodes, |l| Label {
            name: l.name,
            color: l.color,
        })
    })
}

fn pr_ref(name_with_owner: &str, number: i32) -> Option<PrRef> {
    Some(PrRef {
        repo: RepoId::parse(name_with_owner)?,
        number: u64::try_from(number).ok()?,
    })
}

pub(crate) fn count(n: i32) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

impl PrSummary {
    pub(crate) fn from_wire(pr: q::PrSummary) -> Option<Self> {
        Some(Self {
            pr: pr_ref(&pr.repository.name_with_owner, pr.number)?,
            title: pr.title,
            author: author(pr.author),
            state: state(pr.state, pr.is_draft),
            updated_at: pr.updated_at.0,
            additions: count(pr.additions),
            deletions: count(pr.deletions),
            comments: count(pr.comments.total_count),
            review: review(pr.review_decision),
            checks: checks(&pr.commits),
        })
    }
}

/// PRs from one search, and GitHub's total match count.
pub(crate) fn search_results(conn: q::SearchConnection) -> (Vec<PrSummary>, u64) {
    let prs = nodes(conn.nodes)
        .filter_map(|item| match item {
            q::SearchItem::PullRequest(pr) => PrSummary::from_wire(pr),
            q::SearchItem::Other => None,
        })
        .collect();
    (prs, count(conn.issue_count))
}

impl PrDetail {
    pub(crate) fn from_wire(pr: q::PrDetail) -> Option<Self> {
        Some(Self {
            summary: PrSummary::from_wire(pr.summary)?,
            body: pr.body,
            created_at: pr.created_at.0,
            base_ref: pr.base_ref_name,
            head_ref: pr.head_ref_name,
            base_oid: pr.base_ref_oid.0,
            head_oid: pr.head_ref_oid.0,
            head_repo: pr.head_repository.map(|r| r.name_with_owner),
            changed_files: count(pr.changed_files),
            mergeable: match pr.mergeable {
                q::MergeableState::Mergeable => Mergeable::Yes,
                q::MergeableState::Conflicting => Mergeable::Conflicting,
                q::MergeableState::Unknown => Mergeable::Unknown,
            },
            labels: labels(pr.labels),
            milestone: pr.milestone.map(MilestoneRef::from_wire),
        })
    }
}

impl ReviewThread {
    pub(crate) fn from_wire(t: q::ReviewThread) -> Self {
        let line = |n: Option<i32>| n.and_then(|n| u32::try_from(n).ok());
        Self {
            id: t.id.into(),
            path: t.path,
            side: t.diff_side,
            start_side: t.start_diff_side,
            line: line(t.line),
            start_line: line(t.start_line),
            original_line: line(t.original_line),
            original_start_line: line(t.original_start_line),
            outdated: t.is_outdated,
            resolved: t.is_resolved,
            file_level: t.subject_type == q::ThreadSubjectType::File,
            can_reply: t.viewer_can_reply,
            can_resolve: t.viewer_can_resolve,
            can_unresolve: t.viewer_can_unresolve,
            comments: {
                let comment = |c: q::ReviewComment| ReviewComment {
                    id: c.id.into(),
                    author: author(c.author),
                    body: c.body,
                    created_at: c.created_at.0,
                    url: c.url.0,
                    original_commit: c.original_commit.map(|o| o.oid.0),
                    pending: c.state == q::ReviewCommentState::Pending,
                };
                let mut items: Vec<ReviewComment> =
                    nodes(t.first_comment.nodes).map(comment).collect();
                let first = items.first().map(|c| c.id.clone());
                // In a short thread the first is among the newest too.
                items.extend(
                    nodes(t.comments.nodes)
                        .map(comment)
                        .filter(|c| first.as_ref() != Some(&c.id)),
                );
                Capped::new(
                    items,
                    u64::try_from(t.comments.total_count).unwrap_or_default(),
                )
            },
        }
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
