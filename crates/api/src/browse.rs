//! Browsing GitHub: repositories (overview, directories, files, README),
//! issues, profiles, search and pull request conversations.
//!
//! The wire types are typed against GitHub's schema; the models after them
//! are what the app renders and caches.

use serde::{Deserialize, Serialize};

use crate::model::{Capped, Label, MilestoneRef, NodeId, RepoId, author, count, labels};
use crate::queries::{
    Actor, AddCommentPayload, CommentCount, CommitCount, DateTime, FollowCount, FollowingCount,
    GitObjectId, IssueCount, LabelConnection, NumberVariablesFields, PageInfo, PrCount,
    PullRequestReviewDecision, PullRequestState, RefCount, RepositoryName, ReviewState,
    StarPayload, StatusState, UnstarPayload, Uri, UserCount, fragments, nodes,
};
use ghtui_schema::schema;

pub use crate::person::{Login, Person};

// ---- models ------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoSummary {
    pub repo: RepoId,
    pub description: Option<String>,
    pub stars: u64,
    pub forks: u64,
    pub language: Option<String>,
    /// `#rrggbb`.
    pub language_color: Option<String>,
    /// ISO 8601.
    pub pushed_at: Option<String>,
    pub private: bool,
    pub fork: bool,
    pub archived: bool,
}

/// How a check came out. Ordered worst first, for sorting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CheckOutcome {
    Failure,
    Pending,
    Cancelled,
    Neutral,
    Skipped,
    Success,
}

/// One check run or commit status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckItem {
    pub name: String,
    /// The workflow or app it belongs to.
    pub group: String,
    pub outcome: CheckOutcome,
    /// ISO 8601.
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    /// Its title or description.
    pub summary: Option<String>,
    /// Its details page (logs, re-runs).
    pub url: Option<String>,
}

/// The checks on a commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checks {
    pub oid: String,
    pub items: Vec<CheckItem>,
    /// All of them, when there are more than were fetched.
    pub total: u64,
}

/// A commit's page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitDetail {
    pub oid: String,
    pub headline: String,
    /// The message after the headline.
    pub body: String,
    pub author: Person,
    /// ISO 8601.
    pub authored_at: String,
    /// Who committed it, when that isn't the author (rebased, applied).
    pub committer: Option<Person>,
    pub committed_at: String,
    pub parents: Capped<String>,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: Option<u64>,
    /// Whether GitHub verified its signature; `None` when unsigned.
    pub verified: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitInfo {
    pub oid: String,
    pub headline: String,
    pub author: Person,
    /// ISO 8601.
    pub date: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryKind {
    Dir,
    File,
    Symlink,
    Submodule,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeEntry {
    pub name: String,
    pub path: String,
    pub kind: EntryKind,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoOverview {
    pub summary: RepoSummary,
    pub homepage: Option<String>,
    pub watchers: u64,
    pub open_issues: u64,
    pub open_prs: u64,
    #[serde(default)]
    pub closed_issues: u64,
    #[serde(default)]
    pub closed_prs: u64,
    pub license: Option<String>,
    pub topics: Capped<String>,
    pub default_branch: Option<String>,
    pub last_commit: Option<CommitInfo>,
    pub commits: u64,
    /// The repository this one was forked from.
    pub parent: Option<String>,
    pub starred: bool,
    /// Node ID, for starring.
    pub id: NodeId,
    pub has_issues: bool,
    #[serde(default)]
    pub has_discussions: bool,
    #[serde(default)]
    pub has_wiki: bool,
    #[serde(default)]
    pub branches: u64,
    #[serde(default)]
    pub tags: u64,
    /// Root directory at the default branch (empty for empty repos).
    pub entries: Vec<TreeEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Readme {
    pub path: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blob {
    pub path: String,
    /// `None` for binary files.
    pub text: Option<String>,
    pub size: u64,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IssueState {
    Open,
    Closed,
    /// Closed as not planned (or duplicate).
    NotPlanned,
    Merged,
    Draft,
}

/// An issue or pull request in a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueSummary {
    pub repo: RepoId,
    pub number: u64,
    pub title: String,
    pub is_pr: bool,
    pub state: IssueState,
    pub author: String,
    pub updated_at: String,
    pub comments: u64,
    pub labels: Capped<Label>,
    #[serde(default)]
    pub created_at: String,
    /// Pull requests only.
    #[serde(default)]
    pub review: Option<crate::model::ReviewDecision>,
    /// Pull requests only, where known.
    #[serde(default)]
    pub checks: Option<crate::model::ChecksState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSummary {
    pub login: String,
    pub name: Option<String>,
    pub bio: Option<String>,
    pub is_org: bool,
}

/// A release, as listed (no notes or assets) or on its page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub name: String,
    pub tag: String,
    /// ISO 8601; drafts aren't published.
    pub published_at: Option<String>,
    pub prerelease: bool,
    pub draft: bool,
    pub latest: bool,
    pub author: Option<String>,
    /// Its notes, as markdown (on its page).
    pub notes: Option<String>,
    pub assets: Capped<Asset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub downloads: u64,
    pub url: String,
}

/// A tag and the commit it points at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagInfo {
    pub name: String,
    pub oid: Option<String>,
    /// ISO 8601: when its commit was committed.
    pub date: Option<String>,
}

/// How a profile's repositories are ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum RepoSort {
    #[default]
    Updated,
    Name,
    Stars,
}

impl RepoSort {
    pub fn label(self) -> &'static str {
        match self {
            RepoSort::Updated => "Last updated",
            RepoSort::Name => "Name",
            RepoSort::Stars => "Stars",
        }
    }

    /// The next order, round and round.
    pub fn next(self) -> RepoSort {
        match self {
            RepoSort::Updated => RepoSort::Name,
            RepoSort::Name => RepoSort::Stars,
            RepoSort::Stars => RepoSort::Updated,
        }
    }
}

/// A list of people: a repository's stargazers or watchers, someone's
/// followers or who they follow, an organization's public members.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UserList {
    Stargazers(RepoId),
    Watchers(RepoId),
    Followers(String),
    Following(String),
    People(String),
}

/// One page of search results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Results<T> {
    pub total: u64,
    pub items: Vec<T>,
    /// Cursor for the next page, if there is one.
    pub next: Option<String>,
}

/// A list's page and its total, for checking they add up.
pub trait Page {
    /// Items on this page, and how many GitHub counts in all.
    fn counts(&self) -> (usize, u64);
}

impl<T> Page for Results<T> {
    fn counts(&self) -> (usize, u64) {
        (self.items.len(), self.total)
    }
}

macro_rules! page_in {
    ($($list:ty => $field:ident),* $(,)?) => {$(
        impl Page for $list {
            fn counts(&self) -> (usize, u64) {
                self.$field.counts()
            }
        }
    )*};
}
page_in! {
    MilestoneList => results,
    MilestoneDetail => items,
    DeploymentList => results,
    DiscussionList => results,
}

/// No results (without asking `T` for a default).
impl<T> Default for Results<T> {
    fn default() -> Self {
        Self {
            total: 0,
            items: Vec::new(),
            next: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SearchKind {
    Repos,
    /// Issues and pull requests (unless the query says `is:issue` or `is:pr`).
    Issues,
    /// Pull requests only.
    Pulls,
    Users,
    Discussions,
    Commits,
    Code,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchResults {
    Repos(Results<RepoSummary>),
    Issues(Results<IssueSummary>),
    Users(Results<UserSummary>),
    Discussions(Results<DiscussionHit>),
    Commits(Results<CommitHit>),
    Code(Results<CodeHit>),
}

impl SearchResults {
    /// How many there are in all, and how many are here.
    pub fn counts(&self) -> (u64, usize) {
        match self {
            SearchResults::Repos(r) => (r.total, r.items.len()),
            SearchResults::Issues(r) => (r.total, r.items.len()),
            SearchResults::Users(r) => (r.total, r.items.len()),
            SearchResults::Discussions(r) => (r.total, r.items.len()),
            SearchResults::Commits(r) => (r.total, r.items.len()),
            SearchResults::Code(r) => (r.total, r.items.len()),
        }
    }

    pub fn next(&self) -> Option<&str> {
        match self {
            SearchResults::Repos(r) => r.next.as_deref(),
            SearchResults::Issues(r) => r.next.as_deref(),
            SearchResults::Users(r) => r.next.as_deref(),
            SearchResults::Discussions(r) => r.next.as_deref(),
            SearchResults::Commits(r) => r.next.as_deref(),
            SearchResults::Code(r) => r.next.as_deref(),
        }
    }
}

/// A discussion a search found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscussionHit {
    pub repo: RepoId,
    pub summary: DiscussionSummary,
}

/// A commit a search found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitHit {
    pub repo: RepoId,
    pub commit: CommitInfo,
}

/// A file a code search found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeHit {
    pub repo: RepoId,
    pub path: String,
    /// The blob's ID: the file as it was indexed.
    pub sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    /// GitHub's number for it, which its links' anchors use
    /// (`#issuecomment-…`, `#discussioncomment-…`).
    #[serde(default)]
    pub id: Option<u64>,
    pub author: String,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueDetail {
    pub repo: RepoId,
    pub number: u64,
    pub title: String,
    pub body: String,
    pub state: IssueState,
    pub author: String,
    pub created_at: String,
    pub labels: Capped<Label>,
    pub assignees: Capped<String>,
    #[serde(default)]
    pub milestone: Option<MilestoneRef>,
    /// The newest comments, oldest first, and how many there are in all.
    pub comments: Vec<Comment>,
    #[serde(default)]
    pub total_comments: u64,
    /// Node ID, for commenting.
    pub id: NodeId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewSummary {
    pub author: String,
    /// "approved", "changes requested", "commented", ...
    pub state: String,
    pub body: String,
    pub submitted_at: String,
}

/// Everything on a PR's Conversation and Commits tabs that `PrDetail`
/// doesn't carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrActivity {
    /// Node ID, for commenting.
    pub id: NodeId,
    /// The newest comments, reviews and commits, oldest first, and how
    /// many there are in all.
    pub comments: Vec<Comment>,
    pub reviews: Vec<ReviewSummary>,
    pub commits: Vec<CommitInfo>,
    #[serde(default)]
    pub total_comments: u64,
    #[serde(default)]
    pub total_reviews: u64,
    #[serde(default)]
    pub total_commits: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub login: String,
    pub name: Option<String>,
    pub bio: Option<String>,
    pub company: Option<String>,
    pub location: Option<String>,
    pub website: Option<String>,
    pub followers: Option<u64>,
    pub following: Option<u64>,
    pub is_org: bool,
    pub pinned: Vec<RepoSummary>,
    pub repos: Vec<RepoSummary>,
    pub repo_count: u64,
    #[serde(default)]
    pub star_count: u64,
    /// The profile README, as markdown.
    #[serde(default)]
    pub readme: Option<String>,
    /// "🌴 On vacation".
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub pronouns: Option<String>,
    /// Social accounts: (how it's shown, link).
    #[serde(default)]
    pub socials: Vec<(String, String)>,
    /// Organizations a user belongs to (the public ones).
    #[serde(default)]
    pub orgs: Capped<String>,
    /// An organization's verified domain badge.
    #[serde(default)]
    pub verified: bool,
    /// An organization's first public members, and how many there are.
    #[serde(default)]
    pub people: Vec<String>,
    #[serde(default)]
    pub people_count: u64,
    /// A user's contributions in the last year.
    #[serde(default)]
    pub contributions: Option<Contributions>,
}

/// The contribution graph and recent activity on a user's profile.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contributions {
    pub total: u64,
    /// Oldest first, each starting on a Sunday (the first may start later).
    pub weeks: Vec<Week>,
    /// Newest month first.
    pub activity: Vec<MonthActivity>,
    /// Where its counts may be short.
    #[serde(default)]
    pub short: Short,
}

/// For each kind of activity, the month (`YYYY-MM`) in and before which
/// GitHub has more than was fetched: only the newest are.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Short {
    pub commits: Option<String>,
    pub pulls: Option<String>,
    pub issues: Option<String>,
    pub reviews: Option<String>,
}

impl Short {
    /// Counts may be short in every month.
    pub const EVERY_MONTH: &str = "9999-12";

    /// Whether a count in `month` may be short, given the month a kind is
    /// short from.
    pub fn in_month(from: Option<&String>, month: &str) -> bool {
        from.is_some_and(|from| month <= from.as_str())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Week {
    /// `YYYY-MM-DD` of its first day.
    pub start: String,
    /// Each day's level, 0 (none) to 4 (most).
    pub days: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonthActivity {
    /// `YYYY-MM`.
    pub month: String,
    /// Commits per repository, most first.
    pub commits: Vec<(String, u64)>,
    pub pulls: Vec<Contributed>,
    pub issues: Vec<Contributed>,
    pub reviews: Vec<Contributed>,
}

/// An issue or pull request someone opened or reviewed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contributed {
    /// `owner/name`.
    pub repo: String,
    pub number: u64,
    pub title: String,
}

// ---- wire types: shared fragments ---------------------------------------------

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Repository", schema_module = "schema")]
pub struct RepoCard {
    pub name_with_owner: String,
    pub description: Option<String>,
    pub stargazer_count: i32,
    pub fork_count: i32,
    pub primary_language: Option<Language>,
    pub pushed_at: Option<DateTime>,
    pub is_private: bool,
    pub is_fork: bool,
    pub is_archived: bool,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Language", schema_module = "schema")]
pub struct Language {
    pub name: String,
    pub color: Option<String>,
}

// A conversation's lists: the newest page (`last:`, oldest first within
// it) and how many there are, so what's left out can be said.
fragments! {
    counted:
    IssueComments = "IssueCommentConnection" => WireComment,
    Reviews = "PullRequestReviewConnection" => WireReview,
    PrCommits = "PullRequestCommitConnection" => PrCommitNode,
}

// Lists of nodes, and their nodes' types.
fragments! {
    counted:
    Topics = "RepositoryTopicConnection" => RepoTopic,
    Assignees = "UserConnection" => UserLogin,
}

fragments! {
    nodes:
    Pinned = "PinnableItemConnection" => PinnedItem,
}

// ---- repository ---------------------------------------------------------------------

#[derive(cynic::QueryVariables, Debug)]
pub struct RepoVariables {
    pub owner: String,
    pub name: String,
    /// `HEAD:` for the root of the default branch.
    pub expression: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "RepoVariables"
)]
pub struct RepoQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<RepoFull>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Repository",
    schema_module = "schema",
    variables = "RepoVariables"
)]
pub struct RepoFull {
    pub id: cynic::Id,
    #[cynic(spread)]
    pub card: RepoCard,
    pub homepage_url: Option<Uri>,
    pub watchers: UserCount,
    #[arguments(states: [OPEN])]
    pub issues: IssueCount,
    #[arguments(states: [OPEN])]
    pub pull_requests: PrCount,
    #[cynic(rename = "issues", alias)]
    #[arguments(states: [CLOSED])]
    pub closed_issues: IssueCount,
    #[cynic(rename = "pullRequests", alias)]
    #[arguments(states: [CLOSED, MERGED])]
    pub closed_pull_requests: PrCount,
    pub license_info: Option<License>,
    #[arguments(first: 20)]
    pub repository_topics: Topics,
    pub default_branch_ref: Option<BranchRef>,
    pub parent: Option<RepositoryName>,
    pub viewer_has_starred: bool,
    pub has_issues_enabled: bool,
    pub has_discussions_enabled: bool,
    pub has_wiki_enabled: bool,
    #[cynic(rename = "refs", alias)]
    #[arguments(refPrefix: "refs/heads/")]
    pub branch_count: Option<RefCount>,
    #[cynic(rename = "refs", alias)]
    #[arguments(refPrefix: "refs/tags/")]
    pub tag_count: Option<RefCount>,
    #[arguments(expression: $expression)]
    pub object: Option<GitObject>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "License", schema_module = "schema")]
pub struct License {
    pub name: String,
    pub spdx_id: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RepositoryTopic", schema_module = "schema")]
pub struct RepoTopic {
    pub topic: Topic,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Topic", schema_module = "schema")]
pub struct Topic {
    pub name: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Ref", schema_module = "schema")]
pub struct BranchRef {
    pub name: String,
    pub target: Option<RefTarget>,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "GitObject", schema_module = "schema")]
pub enum RefTarget {
    Commit(HeadCommit),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Commit", schema_module = "schema")]
pub struct HeadCommit {
    #[cynic(spread)]
    pub commit: CommitCard,
    #[arguments(first: 0)]
    pub history: CommitCount,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "GitActor", schema_module = "schema")]
pub struct GitActor {
    pub name: Option<String>,
    pub user: Option<UserLogin>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "User", schema_module = "schema")]
pub struct UserLogin {
    pub login: String,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "GitObject", schema_module = "schema")]
pub enum GitObject {
    Tree(Tree),
    Blob(BlobObject),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Tree", schema_module = "schema")]
pub struct Tree {
    pub entries: Option<Vec<WireEntry>>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "TreeEntry", schema_module = "schema")]
pub struct WireEntry {
    pub name: String,
    pub path: Option<String>,
    #[cynic(rename = "type")]
    pub kind: String,
    pub mode: i32,
    pub size: i32,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Blob", schema_module = "schema")]
pub struct BlobObject {
    pub text: Option<String>,
    pub is_binary: Option<bool>,
    pub byte_size: i32,
    pub is_truncated: bool,
}

/// A directory listing or file by `<ref>:<path>`.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "RepoVariables"
)]
pub struct ObjectQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<RepoObject>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Repository",
    schema_module = "schema",
    variables = "RepoVariables"
)]
pub struct RepoObject {
    #[arguments(expression: $expression)]
    pub object: Option<GitObject>,
}

// ---- search ----------------------------------------------------------------------

#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq)]
#[cynic(graphql_type = "SearchType", schema_module = "schema")]
pub enum SearchType {
    Discussion,
    Issue,
    IssueAdvanced,
    IssueHybrid,
    IssueSemantic,
    Repository,
    User,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct BrowseSearchVariables {
    pub query: String,
    pub kind: SearchType,
    pub first: i32,
    pub after: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "BrowseSearchVariables"
)]
pub struct BrowseSearch {
    #[arguments(query: $query, type: $kind, first: $first, after: $after)]
    pub search: BrowseSearchConnection,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "SearchResultItemConnection", schema_module = "schema")]
pub struct BrowseSearchConnection {
    pub issue_count: i32,
    pub repository_count: i32,
    pub user_count: i32,
    pub page_info: PageInfo,
    pub nodes: Option<Vec<Option<BrowseItem>>>,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "SearchResultItem", schema_module = "schema")]
pub enum BrowseItem {
    Repository(RepoCard),
    Issue(IssueCard),
    PullRequest(PrCard),
    User(UserCard),
    Organization(OrgCard),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Issue", schema_module = "schema")]
pub struct IssueCard {
    pub number: i32,
    pub title: String,
    pub state: WireIssueState,
    pub state_reason: Option<StateReason>,
    pub author: Option<Actor>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
    pub comments: CommentCount,
    #[arguments(first: 6)]
    pub labels: Option<LabelConnection>,
    pub repository: RepositoryName,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequest", schema_module = "schema")]
pub struct PrCard {
    pub number: i32,
    pub title: String,
    pub state: PullRequestState,
    pub is_draft: bool,
    pub author: Option<Actor>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
    pub review_decision: Option<PullRequestReviewDecision>,
    pub comments: CommentCount,
    #[arguments(first: 6)]
    pub labels: Option<LabelConnection>,
    pub repository: RepositoryName,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "User", schema_module = "schema")]
pub struct UserCard {
    pub login: String,
    pub name: Option<String>,
    pub bio: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Organization", schema_module = "schema")]
pub struct OrgCard {
    pub login: String,
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq)]
#[cynic(graphql_type = "IssueState", schema_module = "schema")]
pub enum WireIssueState {
    Closed,
    Open,
}

#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq)]
#[cynic(graphql_type = "IssueStateReason", schema_module = "schema")]
pub enum StateReason {
    Completed,
    Duplicate,
    NotPlanned,
    Reopened,
}

// ---- issue ------------------------------------------------------------------------

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "NumberVariables"
)]
pub struct IssueQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<RepoIssue>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Repository",
    schema_module = "schema",
    variables = "NumberVariables"
)]
pub struct RepoIssue {
    #[arguments(number: $number)]
    pub issue_or_pull_request: Option<IssueOrPr>,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "IssueOrPullRequest", schema_module = "schema")]
pub enum IssueOrPr {
    Issue(Box<IssueFull>),
    PullRequest(PrNumber),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequest", schema_module = "schema")]
pub struct PrNumber {
    pub number: i32,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Issue", schema_module = "schema")]
pub struct IssueFull {
    pub id: cynic::Id,
    pub number: i32,
    pub title: String,
    pub body: String,
    pub state: WireIssueState,
    pub state_reason: Option<StateReason>,
    pub author: Option<Actor>,
    pub created_at: DateTime,
    #[arguments(first: 20)]
    pub labels: Option<LabelConnection>,
    #[arguments(first: 10)]
    pub assignees: Assignees,
    pub milestone: Option<crate::queries::MilestoneName>,
    #[arguments(last: 100)]
    pub comments: IssueComments,
    pub repository: RepositoryName,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "IssueComment", schema_module = "schema")]
pub struct WireComment {
    /// `databaseId` overflows GraphQL's `Int` on newer comments.
    pub full_database_id: Option<crate::queries::BigInt>,
    pub author: Option<Actor>,
    pub body: String,
    pub created_at: DateTime,
}

// ---- PR activity -----------------------------------------------------------------

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "NumberVariables"
)]
pub struct PrActivityQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<RepoPrActivity>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Repository",
    schema_module = "schema",
    variables = "NumberVariables"
)]
pub struct RepoPrActivity {
    #[arguments(number: $number)]
    pub pull_request: Option<WirePrActivity>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequest", schema_module = "schema")]
pub struct WirePrActivity {
    pub id: cynic::Id,
    #[arguments(last: 100)]
    pub comments: IssueComments,
    #[arguments(last: 50)]
    pub reviews: Option<Reviews>,
    #[arguments(last: 100)]
    pub commits: PrCommits,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestReview", schema_module = "schema")]
pub struct WireReview {
    pub author: Option<Actor>,
    pub state: ReviewState,
    pub body: String,
    pub submitted_at: Option<DateTime>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestCommit", schema_module = "schema")]
pub struct PrCommitNode {
    pub commit: CommitCard,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Commit", schema_module = "schema")]
pub struct CommitCard {
    pub oid: GitObjectId,
    /// The whole message: GitHub cuts `messageHeadline` at 72 columns.
    pub message: String,
    pub committed_date: DateTime,
    pub author: Option<GitActor>,
}

// ---- commits -------------------------------------------------------------------------

/// A commit by revision (`<sha>`, short or full).
#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "RepoVariables"
)]
pub struct CommitQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<RepoCommit>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Repository",
    schema_module = "schema",
    variables = "RepoVariables"
)]
pub struct RepoCommit {
    #[arguments(expression: $expression)]
    pub object: Option<CommitObject>,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "GitObject", schema_module = "schema")]
pub enum CommitObject {
    Commit(Box<WireCommit>),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Commit", schema_module = "schema")]
pub struct WireCommit {
    pub oid: GitObjectId,
    pub message: String,
    pub authored_date: DateTime,
    pub committed_date: DateTime,
    pub author: Option<GitActor>,
    pub committer: Option<GitActor>,
    #[arguments(first: 5)]
    pub parents: CommitParents,
    pub additions: i32,
    pub deletions: i32,
    pub changed_files_if_available: Option<i32>,
    pub signature: Option<WireSignature>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "CommitConnection", schema_module = "schema")]
pub struct CommitParents {
    pub total_count: i32,
    pub nodes: Option<Vec<Option<ParentCommit>>>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Commit", schema_module = "schema")]
pub struct ParentCommit {
    pub oid: GitObjectId,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "GitSignature", schema_module = "schema")]
pub struct WireSignature {
    pub is_valid: bool,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct HistoryVariables {
    pub owner: String,
    pub name: String,
    pub expression: String,
    pub path: Option<String>,
    pub after: Option<String>,
}

/// A revision's commits, newest first (touching `path`, if given).
#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "HistoryVariables"
)]
pub struct HistoryQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<RepoHistory>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Repository",
    schema_module = "schema",
    variables = "HistoryVariables"
)]
pub struct RepoHistory {
    #[arguments(expression: $expression)]
    pub object: Option<HistoryObject>,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(
    graphql_type = "GitObject",
    schema_module = "schema",
    variables = "HistoryVariables"
)]
pub enum HistoryObject {
    Commit(HistoryCommit),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Commit",
    schema_module = "schema",
    variables = "HistoryVariables"
)]
pub struct HistoryCommit {
    #[arguments(first: 30, after: $after, path: $path)]
    pub history: CommitHistory,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "CommitHistoryConnection", schema_module = "schema")]
pub struct CommitHistory {
    pub total_count: i32,
    pub page_info: PageInfo,
    pub nodes: Option<Vec<Option<CommitCard>>>,
}

impl CommitHistory {
    pub(crate) fn into_results(self) -> Results<CommitInfo> {
        Results {
            total: count(self.total_count),
            items: nodes(self.nodes).map(CommitCard::into_info).collect(),
            next: self.page_info.next(),
        }
    }
}

// ---- checks --------------------------------------------------------------------------

/// The checks on a pull request's head.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "NumberVariables"
)]
pub struct PrChecksQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<RepoPrChecks>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Repository",
    schema_module = "schema",
    variables = "NumberVariables"
)]
pub struct RepoPrChecks {
    #[arguments(number: $number)]
    pub pull_request: Option<PrChecks>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequest", schema_module = "schema")]
pub struct PrChecks {
    /// What the newest commit should be.
    pub head_ref_oid: GitObjectId,
    #[arguments(last: 1)]
    pub commits: PrHeadCommits,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestCommitConnection", schema_module = "schema")]
pub struct PrHeadCommits {
    pub nodes: Option<Vec<Option<PrHeadCommit>>>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestCommit", schema_module = "schema")]
pub struct PrHeadCommit {
    pub commit: ChecksCommit,
}

/// The checks on a repository's default branch.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "BranchesVariables"
)]
pub struct BranchChecksQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<RepoBranchChecks>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Repository", schema_module = "schema")]
pub struct RepoBranchChecks {
    pub default_branch_ref: Option<ChecksRef>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Ref", schema_module = "schema")]
pub struct ChecksRef {
    pub target: Option<ChecksTarget>,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "GitObject", schema_module = "schema")]
pub enum ChecksTarget {
    Commit(ChecksCommit),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Commit", schema_module = "schema")]
pub struct ChecksCommit {
    pub oid: GitObjectId,
    pub status_check_rollup: Option<WireRollup>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "StatusCheckRollup", schema_module = "schema")]
pub struct WireRollup {
    #[arguments(first: 100)]
    pub contexts: RollupContexts,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "StatusCheckRollupContextConnection",
    schema_module = "schema"
)]
pub struct RollupContexts {
    pub total_count: i32,
    pub page_info: crate::queries::PageInfo,
    pub nodes: Option<Vec<Option<RollupContext>>>,
}

/// A later page of a commit's checks (by its oid in `expression`): a
/// re-run on a later page is a check's newest run.
#[derive(cynic::QueryVariables, Debug)]
pub struct ContextsVariables {
    pub owner: String,
    pub name: String,
    pub expression: String,
    pub after: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "ContextsVariables"
)]
pub struct ContextsQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<ContextsRepo>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Repository",
    schema_module = "schema",
    variables = "ContextsVariables"
)]
pub struct ContextsRepo {
    #[arguments(expression: $expression)]
    pub object: Option<ContextsTarget>,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(
    graphql_type = "GitObject",
    schema_module = "schema",
    variables = "ContextsVariables"
)]
pub enum ContextsTarget {
    Commit(ContextsCommit),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Commit",
    schema_module = "schema",
    variables = "ContextsVariables"
)]
pub struct ContextsCommit {
    pub status_check_rollup: Option<ContextsRollup>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "StatusCheckRollup",
    schema_module = "schema",
    variables = "ContextsVariables"
)]
pub struct ContextsRollup {
    #[arguments(first: 100, after: $after)]
    pub contexts: RollupContexts,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "StatusCheckRollupContext", schema_module = "schema")]
pub enum RollupContext {
    CheckRun(Box<WireCheckRun>),
    StatusContext(WireStatusContext),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "CheckRun", schema_module = "schema")]
pub struct WireCheckRun {
    pub name: String,
    pub status: CheckStatusState,
    pub conclusion: Option<CheckConclusionState>,
    pub started_at: Option<DateTime>,
    pub completed_at: Option<DateTime>,
    pub title: Option<String>,
    pub details_url: Option<Uri>,
    pub check_suite: Option<WireSuite>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "CheckSuite", schema_module = "schema")]
pub struct WireSuite {
    pub app: Option<AppName>,
    pub workflow_run: Option<WireWorkflowRun>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "App", schema_module = "schema")]
pub struct AppName {
    pub name: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "WorkflowRun", schema_module = "schema")]
pub struct WireWorkflowRun {
    pub workflow: WorkflowName,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Workflow", schema_module = "schema")]
pub struct WorkflowName {
    pub name: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "StatusContext", schema_module = "schema")]
pub struct WireStatusContext {
    pub context: String,
    pub state: StatusState,
    pub description: Option<String>,
    pub target_url: Option<Uri>,
    pub created_at: DateTime,
}

#[derive(cynic::Enum, Debug, Clone, Copy)]
#[cynic(graphql_type = "CheckStatusState", schema_module = "schema")]
pub enum CheckStatusState {
    Completed,
    InProgress,
    Pending,
    Queued,
    Requested,
    Waiting,
}

#[derive(cynic::Enum, Debug, Clone, Copy)]
#[cynic(graphql_type = "CheckConclusionState", schema_module = "schema")]
pub enum CheckConclusionState {
    ActionRequired,
    Cancelled,
    Failure,
    Neutral,
    Skipped,
    Stale,
    StartupFailure,
    Success,
    TimedOut,
}

impl ChecksCommit {
    pub(crate) fn into_checks(self) -> Checks {
        let contexts = self.status_check_rollup.map(|r| r.contexts);
        let total = contexts.as_ref().map_or(0, |c| count(c.total_count));
        let mut fetched = 0;
        let items = nodes(contexts.and_then(|c| c.nodes))
            .inspect(|_| fetched += 1)
            .filter_map(|c| match c {
                RollupContext::CheckRun(run) => Some(run.into_item()),
                RollupContext::StatusContext(s) => Some(s.into_item()),
                RollupContext::Other => None,
            })
            .collect();
        let items = latest_runs(items);
        // Those past the first page may be earlier runs too; count them as
        // checks still to show.
        let total = items.len() as u64 + total.saturating_sub(fetched);
        Checks {
            oid: self.oid.0,
            items,
            total,
        }
    }
}

/// The latest run of each check. A workflow that runs again on the same
/// commit (a re-run, or one a label or title edit triggers again) leaves
/// every run in the rollup; GitHub shows only the newest. One not started
/// yet is the newest.
fn latest_runs(items: Vec<CheckItem>) -> Vec<CheckItem> {
    let newness = |i: &CheckItem| (i.started_at.is_none(), i.started_at.clone());
    let mut out: Vec<CheckItem> = Vec::new();
    for item in items {
        match out
            .iter_mut()
            .find(|o| o.group == item.group && o.name == item.name)
        {
            Some(kept) if newness(&item) > newness(kept) => *kept = item,
            Some(_) => {}
            None => out.push(item),
        }
    }
    out
}

#[cfg(test)]
mod tests {

    /// A tag's commit, lightweight or annotated; a tree's entries by kind,
    /// directories first, then by name whatever its case.
    #[test]
    fn tags_and_tree_entries() {
        let tags: TagRefs = serde_json::from_value(serde_json::json!({
            "totalCount": 3,
            "pageInfo": {"hasNextPage": true, "endCursor": "c"},
            "nodes": [
                {"name": "light", "target": {"__typename": "Commit", "oid": "a1", "committedDate": "2026-01-01T00:00:00Z"}},
                {"name": "annotated", "target": {"__typename": "Tag", "target": {"__typename": "Commit", "oid": "b2", "committedDate": "2026-02-01T00:00:00Z"}}},
                {"name": "tree", "target": {"__typename": "Tree"}}
            ]
        }))
        .unwrap();
        let tags = tags.into_results();
        let got: Vec<(&str, Option<&str>)> = tags
            .items
            .iter()
            .map(|t| (t.name.as_str(), t.oid.as_deref()))
            .collect();
        assert_eq!(
            got,
            [
                ("light", Some("a1")),
                ("annotated", Some("b2")),
                ("tree", None)
            ]
        );
        assert_eq!((tags.total, tags.next.as_deref()), (3, Some("c")));

        let tree: Tree = serde_json::from_value(serde_json::json!({"entries": [
            {"name": "b.txt", "path": "b.txt", "type": "blob", "mode": 0o100_644, "size": 5},
            {"name": "link", "path": "link", "type": "blob", "mode": 0o120_000, "size": 4},
            {"name": "Zdir", "path": "Zdir", "type": "tree", "mode": 0o040_000, "size": 0},
            {"name": "sub", "path": "sub", "type": "commit", "mode": 0o160_000, "size": 0},
            {"name": "a.txt", "path": null, "type": "blob", "mode": 0o100_644, "size": 1}
        ]}))
        .unwrap();
        let got: Vec<(String, EntryKind, Option<u64>)> = entries(tree)
            .into_iter()
            .map(|e| (e.path, e.kind, e.size))
            .collect();
        assert_eq!(
            got,
            [
                ("Zdir".into(), EntryKind::Dir, None),
                ("a.txt".into(), EntryKind::File, Some(1)),
                ("b.txt".into(), EntryKind::File, Some(5)),
                ("link".into(), EntryKind::Symlink, None),
                ("sub".into(), EntryKind::Submodule, None),
            ]
        );
    }

    /// A month's activity: commits summed per repository, most first; a PR
    /// reviewed twice once; months newest first, three of them; and with
    /// more repositories than fetched, every month's commits short.
    #[test]
    fn activity_is_grouped_by_month() {
        let item = |at: &str, number: u32| serde_json::json!({"occurredAt": at, "pullRequest": {"number": number, "title": "t", "repository": {"nameWithOwner": "o/r"}}});
        let commits = |repo: &str, days: &[(&str, u32)]| {
            serde_json::json!({"repository": {"nameWithOwner": repo}, "contributions": {
                "totalCount": days.len(),
                "nodes": days.iter().map(|(at, n)| serde_json::json!({"occurredAt": at, "commitCount": n})).collect::<Vec<_>>()
            }})
        };
        let wire: WireContributions = serde_json::from_value(serde_json::json!({
            "contributionCalendar": {"totalContributions": 9, "weeks": []},
            "commitContributionsByRepository": [
                commits("o/a", &[("2026-10-02T00:00:00Z", 2), ("2026-10-01T00:00:00Z", 3)]),
                commits("o/b", &[("2026-10-03T00:00:00Z", 7)]),
            ],
            "totalRepositoriesWithContributedCommits": 3,
            "pullRequestContributions": {"totalCount": 1, "nodes": [item("2026-07-01T00:00:00Z", 1)]},
            "issueContributions": {"totalCount": 0, "nodes": []},
            "pullRequestReviewContributions": {"totalCount": 3, "nodes": [
                item("2026-09-05T00:00:00Z", 5), item("2026-09-04T00:00:00Z", 5), item("2026-08-01T00:00:00Z", 6)
            ]},
        }))
        .unwrap();
        let c = wire.into_contributions();
        let months: Vec<&str> = c.activity.iter().map(|m| m.month.as_str()).collect();
        assert_eq!(months, ["2026-10", "2026-09", "2026-08"]);
        assert_eq!(
            c.activity[0].commits,
            [("o/b".to_owned(), 7), ("o/a".to_owned(), 5)]
        );
        assert_eq!(c.activity[1].reviews.len(), 1);
        assert_eq!(c.short.commits.as_deref(), Some(Short::EVERY_MONTH));
        assert_eq!(
            (c.short.pulls.as_ref(), c.short.reviews.as_ref()),
            (None, None)
        );
    }

    /// How REST names a run's, job's or step's outcome (found by mutation
    /// testing: no test pinned it).
    #[test]
    fn rest_outcomes() {
        for (status, conclusion, outcome) in [
            ("completed", Some("success"), CheckOutcome::Success),
            ("completed", Some("skipped"), CheckOutcome::Skipped),
            ("completed", Some("cancelled"), CheckOutcome::Cancelled),
            ("completed", Some("neutral"), CheckOutcome::Neutral),
            ("completed", Some("stale"), CheckOutcome::Neutral),
            ("completed", Some("failure"), CheckOutcome::Failure),
            ("completed", Some("timed_out"), CheckOutcome::Failure),
            ("in_progress", None, CheckOutcome::Pending),
            ("queued", None, CheckOutcome::Pending),
        ] {
            assert_eq!(
                rest_outcome(status, conclusion),
                outcome,
                "{status} {conclusion:?}"
            );
        }
    }

    /// A search's next page and counts, whatever it searched.
    #[test]
    fn search_results_page_and_count() {
        let user = UserSummary {
            login: "a".into(),
            name: None,
            bio: None,
            is_org: false,
        };
        let users = Results {
            total: 40,
            items: vec![user; 2],
            next: Some("c2".into()),
        };
        let r = SearchResults::Users(users);
        assert_eq!((r.next(), r.counts()), (Some("c2"), (40, 2)));
        let empty = SearchResults::Repos(Results::default());
        assert_eq!((empty.next(), empty.counts()), (None, (0, 0)));
    }

    /// A commit's checks past the first page are counted, not dropped;
    /// blank summaries are none.
    #[test]
    fn checks_count_what_wasnt_fetched() {
        let run = |name: &str, title: &str| {
            serde_json::json!({"__typename": "CheckRun", "name": name, "status": "COMPLETED",
                "conclusion": "SUCCESS", "startedAt": null, "completedAt": null, "title": title,
                "detailsUrl": null, "checkSuite": null})
        };
        let commit: ChecksCommit = serde_json::from_value(serde_json::json!({
            "oid": "abc",
            "statusCheckRollup": {"contexts": {"totalCount": 5,
                "pageInfo": {"hasNextPage": true, "endCursor": "c"},
                "nodes": [run("a", "  "), run("b", "Built")]}}
        }))
        .unwrap();
        let checks = commit.into_checks();
        assert_eq!((checks.items.len(), checks.total), (2, 5));
        assert_eq!(checks.items[0].summary, None);
        assert_eq!(checks.items[1].summary.as_deref(), Some("Built"));
    }

    /// Only a line that starts with GitHub's timestamp has one.
    #[test]
    fn log_lines_split_off_their_timestamps() {
        use super::log::split;
        assert_eq!(
            split("2026-10-05T17:42:01.1234567Z hello there"),
            (Some("2026-10-05T17:42:01.1234567Z"), "hello there")
        );
        assert_eq!(
            split("\u{feff}2026-10-05T17:42:01.1Z x"),
            (Some("2026-10-05T17:42:01.1Z"), "x")
        );
        for line in [
            "hello there",
            "2026-10-05 short",
            "2026/10/05T17:42:01.1234Z x",
            "20261005T17:42:01.12345Z x",
            "2026-10-05T17:42:01.12345+ x",
        ] {
            assert_eq!(split(line).0, None, "{line}");
        }
    }

    /// A profile's activity lists are its newest, and where GitHub has
    /// more than came, the counts say they may be short.
    #[test]
    fn short_activity_says_so() {
        let pr = |at: &str| serde_json::json!({"occurredAt": at, "pullRequest": {"number": 1, "title": "t", "repository": {"nameWithOwner": "o/r"}}});
        let wire: WireContributions = serde_json::from_value(serde_json::json!({
            "contributionCalendar": {"totalContributions": 3, "weeks": []},
            "commitContributionsByRepository": [],
            "totalRepositoriesWithContributedCommits": 0,
            "pullRequestContributions": {"totalCount": 50, "nodes": [pr("2026-10-02T00:00:00Z"), pr("2026-09-20T00:00:00Z")]},
            "issueContributions": {"totalCount": 0, "nodes": []},
            "pullRequestReviewContributions": {"totalCount": 1, "nodes": [pr("2026-08-01T00:00:00Z")]},
        }))
        .unwrap();
        let c = wire.into_contributions();
        assert_eq!(c.short.pulls.as_deref(), Some("2026-09"));
        assert_eq!(
            (c.short.reviews.as_ref(), c.short.commits.as_ref()),
            (None, None)
        );
        assert!(Short::in_month(c.short.pulls.as_ref(), "2026-09"));
        assert!(!Short::in_month(c.short.pulls.as_ref(), "2026-10"));
    }

    /// A long log is cut at a line's start; the lines cut keep their
    /// timestamps, and the ones that start steps their text.
    #[test]
    fn a_long_log_is_cut_at_a_line() {
        use super::log;
        assert_eq!(log::cut("a\nb\n"), (String::new(), "a\nb\n"));
        let filler = "x".repeat(100);
        let mut full = String::from("2026-10-05T17:42:00.1Z ##[group]Run ./build\nno time\n");
        while full.len() <= log::KEEP + 1000 {
            full.push_str(&format!("2026-10-05T17:42:01.1Z {filler}\n"));
        }
        let (cut, kept) = log::cut(&full);
        assert!(kept.len() <= log::KEEP && kept.starts_with("2026-"));
        assert_eq!(
            cut.lines().count() + kept.lines().count(),
            full.lines().count()
        );
        let mut lines = cut.lines();
        assert_eq!(
            lines.next(),
            Some("2026-10-05T17:42:00.1Z ##[group]Run ./build")
        );
        assert_eq!(lines.next(), Some(""));
        assert_eq!(lines.next(), Some("2026-10-05T17:42:01.1Z"));
    }

    use super::*;

    /// A branch's pull request is one in its own repository from that
    /// branch: GitHub also lists forks' PRs whose head it is.
    #[test]
    fn a_branchs_pr_is_its_own() {
        let branch: wire_branches::Branch = serde_json::from_value(serde_json::json!({
            "name": "trunk",
            "target": null,
            "associatedPullRequests": { "nodes": [
                { "number": 1, "state": "MERGED", "isDraft": false, "headRefName": "trunk",
                  "repository": { "nameWithOwner": "someone/cli-fork" } },
                { "number": 9, "state": "OPEN", "isDraft": false, "headRefName": "other",
                  "repository": { "nameWithOwner": "cli/cli" } },
                { "number": 7, "state": "CLOSED", "isDraft": false, "headRefName": "trunk",
                  "repository": { "nameWithOwner": "CLI/cli" } },
            ] },
        }))
        .unwrap();
        let info = branch.into_info(&RepoId::new("cli", "cli"), Some("trunk"));
        assert_eq!(info.pr, Some((7, IssueState::Closed)));
        assert!(info.default);
    }

    /// A branch whose pull request is a draft shows it as a draft, as a
    /// PR's own page and the PR lists do, and the branches query asks.
    #[test]
    fn a_branchs_draft_pr_is_a_draft() {
        let branch: wire_branches::Branch = serde_json::from_value(serde_json::json!({
            "name": "wip",
            "target": null,
            "associatedPullRequests": { "nodes": [
                { "number": 5, "state": "OPEN", "isDraft": true, "headRefName": "wip",
                  "repository": { "nameWithOwner": "cli/cli" } },
            ] },
        }))
        .unwrap();
        let info = branch.into_info(&RepoId::new("cli", "cli"), None);
        assert_eq!(info.pr, Some((5, IssueState::Draft)));
        assert!(
            crate::raw::BRANCHES.contains("isDraft"),
            "the branches query doesn't ask whether a PR is a draft"
        );
    }

    /// A branch whose tip has no author shows only its headline, not a
    /// made-up "unknown"; a git name is still shown.
    #[test]
    fn a_branch_tip_without_an_author_names_nobody() {
        let author = |author: serde_json::Value| {
            let branch: wire_branches::Branch = serde_json::from_value(serde_json::json!({
                "name": "trunk",
                "target": { "oid": "abc", "messageHeadline": "Fix",
                    "committedDate": "2026-01-01T00:00:00Z", "author": author },
                "associatedPullRequests": { "nodes": [] },
            }))
            .unwrap();
            branch.into_info(&RepoId::new("cli", "cli"), None).author
        };
        assert_eq!(author(serde_json::Value::Null), None);
        assert_eq!(
            author(serde_json::json!({ "name": "", "user": null })),
            None
        );
        assert_eq!(
            author(serde_json::json!({ "name": "Jane", "user": null })),
            Some(Person::Git("Jane".into()))
        );
    }

    /// A gist file without a name (GitHub's schema allows it) doesn't
    /// spoil the list.
    #[test]
    fn gist_files_may_lack_names() {
        let gists: wire::Connection<wire_gists::Gist> = serde_json::from_value(serde_json::json!({
            "totalCount": 1,
            "pageInfo": { "hasNextPage": false, "endCursor": null },
            "nodes": [{ "name": "abc", "description": null, "updatedAt": "2026-10-01T00:00:00Z",
                "stargazerCount": 0, "files": [{ "name": null }, { "name": "a.rs" }],
                "comments": { "totalCount": 0 } }],
        }))
        .unwrap();
        let gists = gists.into_results(wire_gists::Gist::into_summary);
        assert_eq!(gists.items[0].files, ["a.rs"]);
    }

    /// A comparison's head is the newest commit GitHub lists, or, with
    /// none listed (the head is behind or at the base), the merge base.
    #[test]
    fn a_comparisons_head() {
        let compare = |commits: &[&str]| -> rest_compare::Compare {
            let commits: Vec<serde_json::Value> = commits
                .iter()
                .map(|sha| serde_json::json!({ "sha": sha, "commit": { "message": "m" } }))
                .collect();
            serde_json::from_value(serde_json::json!({
                "status": "diverged", "ahead_by": 2, "behind_by": 1, "total_commits": 2,
                "base_commit": { "sha": "base" }, "merge_base_commit": { "sha": "mb" },
                "commits": commits,
            }))
            .unwrap()
        };
        let c = compare(&["old", "head"]).into_comparison(false);
        assert_eq!((c.from.as_str(), c.to.as_str()), ("mb", "head"));
        let c = compare(&[]).into_comparison(true);
        assert_eq!((c.from.as_str(), c.to.as_str()), ("base", "mb"));
    }

    fn run(group: &str, name: &str, started: Option<&str>, outcome: CheckOutcome) -> CheckItem {
        CheckItem {
            name: name.into(),
            group: group.into(),
            outcome,
            started_at: started.map(|s| format!("2026-09-28T{s}Z")),
            completed_at: None,
            summary: None,
            url: None,
        }
    }

    /// Re-runs, and runs a label edit triggered again, show once: the
    /// newest, which a run not yet started is.
    #[test]
    fn only_the_latest_run_of_a_check_shows() {
        use CheckOutcome::{Failure, Pending, Success};
        let items = vec![
            run("CI", "test", Some("15:46:00"), Failure),
            run("CI", "test", Some("15:52:00"), Success),
            run("PR", "check-title", Some("15:58:00"), Success),
            run("PR", "check-title", Some("16:01:00"), Failure),
            run("PR", "check-title", Some("15:59:00"), Success),
            run("Release", "test", Some("10:00:00"), Success),
            run("Lint", "fmt", Some("10:00:00"), Success),
            run("Lint", "fmt", None, Pending),
        ];
        let latest: Vec<String> = latest_runs(items)
            .iter()
            .map(|i| format!("{}/{} {:?}", i.group, i.name, i.outcome))
            .collect();
        assert_eq!(
            latest,
            [
                "CI/test Success",
                "PR/check-title Failure",
                "Release/test Success",
                "Lint/fmt Pending",
            ]
        );
    }

    /// A commit found by search whose author has no account is by its
    /// author's git name, not by whoever committed it (cherry-picked, applied).
    #[test]
    fn a_found_commit_by_an_unlinked_author_isnt_by_its_committer() {
        let found: super::rest_search::Commit = serde_json::from_value(serde_json::json!({
            "sha": "abc123",
            "repository": {"full_name": "o/r"},
            "author": null,
            "commit": {
                "message": "Fix the thing",
                "author": {"name": "Jane Doe", "date": "2026-01-01T00:00:00Z"},
                "committer": {"name": "Bob Maintainer", "date": "2026-02-01T00:00:00Z"}
            }
        }))
        .unwrap();
        let hit = found.into_hit().unwrap();
        assert_eq!(hit.commit.author, super::Person::Git("Jane Doe".into()));
        assert_eq!(hit.commit.date, "2026-02-01T00:00:00Z");
    }

    /// A commit's committer is shown unless they read as its author, or are
    /// nobody or GitHub's own web-flow account; a git name that happens to
    /// read "web-flow" is somebody.
    #[test]
    fn a_commits_committer_is_hidden_only_for_web_flows_account() {
        let committer = |c: serde_json::Value| {
            let wire: super::WireCommit = serde_json::from_value(serde_json::json!({
                "oid": "abc123",
                "message": "Fix",
                "authoredDate": "2026-01-01T00:00:00Z",
                "committedDate": "2026-01-02T00:00:00Z",
                "author": {"name": "Jane", "user": {"login": "jane"}},
                "committer": c,
                "parents": {"totalCount": 0, "nodes": []},
                "additions": 0,
                "deletions": 0,
                "changedFilesIfAvailable": null,
                "signature": null
            }))
            .unwrap();
            wire.into_detail().committer
        };
        let web_flow = serde_json::json!({"name": "GitHub", "user": {"login": "web-flow"}});
        assert_eq!(committer(web_flow), None);
        assert_eq!(
            committer(serde_json::json!({"name": "web-flow", "user": null})),
            Some(super::Person::Git("web-flow".into()))
        );
        let jane = serde_json::json!({"name": "J", "user": {"login": "jane"}});
        assert_eq!(committer(jane), None);
        // Shown as the author is: "jane", whether by login or by git name.
        assert_eq!(
            committer(serde_json::json!({"name": "jane", "user": null})),
            None
        );
        assert_eq!(committer(serde_json::json!(null)), None);
    }
}

impl WireCheckRun {
    fn into_item(self) -> CheckItem {
        use CheckConclusionState as C;
        let outcome = match (self.status, self.conclusion) {
            (CheckStatusState::Completed, Some(conclusion)) => match conclusion {
                C::Success => CheckOutcome::Success,
                C::Skipped => CheckOutcome::Skipped,
                C::Neutral | C::Stale => CheckOutcome::Neutral,
                C::Cancelled => CheckOutcome::Cancelled,
                C::Failure | C::TimedOut | C::StartupFailure | C::ActionRequired => {
                    CheckOutcome::Failure
                }
            },
            _ => CheckOutcome::Pending,
        };
        let suite = self.check_suite;
        let group = suite
            .as_ref()
            .and_then(|s| s.workflow_run.as_ref())
            .map(|w| w.workflow.name.clone())
            .or_else(|| suite.and_then(|s| s.app).map(|a| a.name))
            .unwrap_or_else(|| "Checks".into());
        CheckItem {
            name: self.name,
            group,
            outcome,
            started_at: self.started_at.map(|d| d.0),
            completed_at: self.completed_at.map(|d| d.0),
            summary: self.title.filter(|t| !t.trim().is_empty()),
            url: self.details_url.map(|u| u.0),
        }
    }
}

impl WireStatusContext {
    fn into_item(self) -> CheckItem {
        let outcome = match self.state {
            StatusState::Success => CheckOutcome::Success,
            StatusState::Failure | StatusState::Error => CheckOutcome::Failure,
            StatusState::Pending | StatusState::Expected => CheckOutcome::Pending,
        };
        CheckItem {
            name: self.context,
            group: "Statuses".into(),
            outcome,
            started_at: Some(self.created_at.0),
            completed_at: None,
            summary: self.description.filter(|d| !d.trim().is_empty()),
            url: self.target_url.map(|u| u.0),
        }
    }
}

// ---- actions --------------------------------------------------------------------------------

/// How a REST check run, job or step came out.
fn rest_outcome(status: &str, conclusion: Option<&str>) -> CheckOutcome {
    match (status, conclusion) {
        ("completed", Some("success")) => CheckOutcome::Success,
        ("completed", Some("skipped")) => CheckOutcome::Skipped,
        ("completed", Some("cancelled")) => CheckOutcome::Cancelled,
        ("completed", Some("neutral" | "stale")) => CheckOutcome::Neutral,
        ("completed", _) => CheckOutcome::Failure,
        _ => CheckOutcome::Pending,
    }
}

/// A workflow run and its jobs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub id: u64,
    /// The workflow's name.
    pub name: String,
    /// What it ran for: the commit's headline or the pull request's title.
    pub title: String,
    pub number: u64,
    pub attempt: u64,
    /// `push`, `pull_request`, `schedule`…
    pub event: String,
    pub branch: Option<String>,
    pub sha: String,
    pub outcome: CheckOutcome,
    pub actor: Option<String>,
    /// ISO 8601.
    pub started_at: Option<String>,
    pub updated_at: Option<String>,
    /// The workflow file, `.github/workflows/ci.yml`.
    pub path: String,
    pub jobs: Vec<JobSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobSummary {
    pub id: u64,
    pub name: String,
    pub outcome: CheckOutcome,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

/// How many of a directory's entries get their latest commit, as on
/// GitHub (whose file list stops at 1,000).
pub const LAST_COMMIT_ENTRIES: usize = 1000;

/// A job's log. A long one is cut to its last 2 MB (where failures are);
/// the lines cut are kept as their timestamps, and the ones that start a
/// step whole, so the rest still have their steps and line numbers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobLog {
    /// The lines cut, reduced as above.
    pub cut: String,
    pub text: String,
    /// The job hadn't finished, so there was no log to fetch yet.
    pub running: bool,
}

/// A job's log lines, as GitHub writes them.
pub mod log {
    /// Bytes of log kept.
    pub const KEEP: usize = 2 << 20;

    /// A log line without its timestamp, and when it was written (ISO 8601).
    pub fn split(line: &str) -> (Option<&str>, &str) {
        let line = line.trim_start_matches('\u{feff}');
        match line.split_once(' ') {
            Some((at, rest))
                if at.len() >= 20 && at.ends_with('Z') && at.as_bytes().get(4) == Some(&b'-') =>
            {
                (Some(at), rest)
            }
            _ => (None, line),
        }
    }

    /// Whether a line (without its timestamp) is one that starts a step:
    /// `##[group]Run …`, `Post job cleanup.`, `Cleaning up orphan processes`.
    pub fn starts_step(text: &str) -> bool {
        text.starts_with("##[group]Run ")
            || text == "Post job cleanup."
            || text == "Cleaning up orphan processes"
    }

    /// `log` cut to its last [`KEEP`] bytes, at a line's start: the lines
    /// cut, reduced to what places the rest, and the lines kept.
    pub fn cut(log: &str) -> (String, &str) {
        if log.len() <= KEEP {
            return (String::new(), log);
        }
        let from = log.len() - KEEP;
        let start = log
            .get(from..)
            .and_then(|tail| tail.find('\n'))
            .map_or(log.len(), |i| from + i + 1);
        let (head, kept) = log.split_at(start);
        let mut cut = String::new();
        for line in head.lines() {
            match split(line) {
                (Some(at), text) if starts_step(text) => {
                    cut.push_str(at);
                    cut.push(' ');
                    cut.push_str(text);
                }
                (Some(at), _) => cut.push_str(at),
                (None, _) => {}
            }
            cut.push('\n');
        }
        (cut, kept)
    }
}

/// A job: its steps (its log comes separately).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub id: u64,
    pub run_id: u64,
    pub name: String,
    pub outcome: CheckOutcome,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub number: u32,
    pub name: String,
    pub outcome: CheckOutcome,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

/// A workflow (its runs come separately, a page at a time).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workflow {
    pub name: String,
    pub path: String,
    /// `active`, `disabled_manually`…
    pub state: String,
}

/// A run in a workflow's list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSummary {
    pub id: u64,
    pub title: String,
    pub number: u64,
    pub event: String,
    pub branch: Option<String>,
    pub outcome: CheckOutcome,
    pub actor: Option<String>,
    pub created_at: Option<String>,
}

/// The shapes GraphQL responses read as raw JSON share.
pub(crate) mod wire {
    use serde::Deserialize;

    use super::Results;

    #[derive(Deserialize)]
    pub struct Name {
        pub name: String,
    }

    /// A REST commit's git author or committer.
    #[derive(Deserialize)]
    pub struct Signature {
        pub name: Option<String>,
        pub date: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct Message {
        pub message: String,
        pub author: Option<Signature>,
        pub committer: Option<Signature>,
    }

    /// A commit as REST lists them (compare, search).
    #[derive(Deserialize)]
    pub struct RestCommit {
        pub sha: String,
        pub commit: Message,
        pub author: Option<super::UserLogin>,
    }

    /// Which signature dates a REST commit.
    #[derive(Clone, Copy)]
    pub enum Dated {
        Authored,
        /// When committed, else when authored.
        Committed,
    }

    impl RestCommit {
        /// By GitHub's account for it, else its author's git name (never the
        /// committer's).
        pub fn into_info(self, dated: Dated) -> super::CommitInfo {
            let Message {
                message,
                author,
                committer,
            } = self.commit;
            let committed = match dated {
                Dated::Authored => None,
                Dated::Committed => committer.and_then(|c| c.date),
            };
            let (name, authored) = author.map_or((None, None), |a| (a.name, a.date));
            let author = super::GitActor {
                name,
                user: self.author,
            };
            super::CommitInfo {
                headline: message.lines().next().unwrap_or_default().to_owned(),
                author: Some(author).into(),
                date: committed.or(authored).unwrap_or_default(),
                oid: self.sha,
            }
        }
    }

    /// A commit as raw GraphQL lists them (blame, a path's last commit).
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Commit {
        pub oid: String,
        pub message_headline: String,
        pub committed_date: String,
        pub author: Option<super::GitActor>,
    }

    impl Commit {
        pub fn into_info(self) -> super::CommitInfo {
            super::CommitInfo {
                author: self.author.into(),
                oid: self.oid,
                headline: self.message_headline,
                date: self.committed_date,
            }
        }
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct RepoName {
        pub name_with_owner: String,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Count {
        pub total_count: u64,
    }

    /// A connection's nodes, without paging.
    #[derive(Deserialize)]
    pub struct Nodes<T> {
        pub nodes: Vec<Option<T>>,
    }

    /// The nodes that aren't null.
    impl<T> IntoIterator for Nodes<T> {
        type Item = T;
        type IntoIter = std::iter::Flatten<std::vec::IntoIter<Option<T>>>;

        fn into_iter(self) -> Self::IntoIter {
            self.nodes.into_iter().flatten()
        }
    }

    /// Some of a connection's nodes, and how many there are in all.
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Counted<T> {
        pub total_count: u64,
        pub nodes: Vec<Option<T>>,
    }

    impl<T> Counted<T> {
        /// The nodes that aren't null, each made an item by `item`.
        pub fn into_capped<U>(self, item: impl FnMut(T) -> Option<U>) -> crate::model::Capped<U> {
            let items = self.nodes.into_iter().flatten().filter_map(item).collect();
            crate::model::Capped::new(items, self.total_count)
        }
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct PageInfo {
        pub has_next_page: bool,
        pub end_cursor: Option<String>,
    }

    /// A page of a connection, and how many there are in all (a search's
    /// count of discussions included).
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Connection<T> {
        #[serde(alias = "discussionCount")]
        pub total_count: u64,
        pub page_info: PageInfo,
        pub nodes: Vec<Option<T>>,
    }

    impl<T> Connection<T> {
        /// The page as results, each node made an item by `item`.
        pub fn into_results<U>(self, mut item: impl FnMut(T) -> U) -> Results<U> {
            self.filter_results(|node| Some(item(node)))
        }

        /// The page as results, the nodes `item` makes nothing of left out.
        pub fn filter_results<U>(self, item: impl FnMut(T) -> Option<U>) -> Results<U> {
            let PageInfo {
                has_next_page,
                end_cursor,
            } = self.page_info;
            Results {
                total: self.total_count,
                items: self.nodes.into_iter().flatten().filter_map(item).collect(),
                next: end_cursor.filter(|_| has_next_page),
            }
        }
    }
}

pub(crate) mod rest_actions {
    use serde::Deserialize;

    pub use super::UserLogin as Actor;

    #[derive(Deserialize)]
    pub struct Run {
        pub id: u64,
        pub name: Option<String>,
        pub display_title: Option<String>,
        pub run_number: u64,
        pub run_attempt: Option<u64>,
        pub event: String,
        pub head_branch: Option<String>,
        pub head_sha: String,
        pub status: Option<String>,
        pub conclusion: Option<String>,
        pub actor: Option<Actor>,
        pub run_started_at: Option<String>,
        pub created_at: Option<String>,
        pub updated_at: Option<String>,
        #[serde(default)]
        pub path: String,
    }

    #[derive(Deserialize)]
    pub struct Runs {
        pub total_count: u64,
        pub workflow_runs: Vec<Run>,
    }

    #[derive(Deserialize)]
    pub struct Step {
        pub number: u32,
        pub name: String,
        pub status: String,
        pub conclusion: Option<String>,
        pub started_at: Option<String>,
        pub completed_at: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct Job {
        pub id: u64,
        pub run_id: u64,
        pub name: String,
        pub status: String,
        pub conclusion: Option<String>,
        pub started_at: Option<String>,
        pub completed_at: Option<String>,
        #[serde(default)]
        pub steps: Vec<Step>,
    }

    #[derive(Deserialize)]
    pub struct Jobs {
        pub jobs: Vec<Job>,
        #[serde(default)]
        pub total_count: u64,
    }

    #[derive(Deserialize)]
    pub struct Workflow {
        pub name: String,
        pub path: String,
        pub state: String,
    }
}

impl rest_actions::Run {
    fn outcome(&self) -> CheckOutcome {
        rest_outcome(
            self.status.as_deref().unwrap_or("queued"),
            self.conclusion.as_deref(),
        )
    }

    pub(crate) fn into_run(self, jobs: Vec<rest_actions::Job>) -> WorkflowRun {
        let outcome = self.outcome();
        WorkflowRun {
            id: self.id,
            name: self.name.unwrap_or_else(|| "Workflow".into()),
            title: self.display_title.unwrap_or_default(),
            number: self.run_number,
            attempt: self.run_attempt.unwrap_or(1),
            event: self.event,
            branch: self.head_branch,
            sha: self.head_sha,
            outcome,
            actor: self.actor.map(|a| a.login),
            started_at: self.run_started_at.or(self.created_at),
            updated_at: self.updated_at,
            path: self.path,
            jobs: jobs
                .into_iter()
                .map(|j| JobSummary {
                    outcome: rest_outcome(&j.status, j.conclusion.as_deref()),
                    id: j.id,
                    name: j.name,
                    started_at: j.started_at,
                    completed_at: j.completed_at,
                })
                .collect(),
        }
    }

    pub(crate) fn into_summary(self) -> RunSummary {
        let outcome = self.outcome();
        RunSummary {
            id: self.id,
            title: self.display_title.unwrap_or_default(),
            number: self.run_number,
            event: self.event,
            branch: self.head_branch,
            outcome,
            actor: self.actor.map(|a| a.login),
            created_at: self.created_at,
        }
    }
}

impl rest_actions::Job {
    pub(crate) fn into_job(self) -> Job {
        Job {
            outcome: rest_outcome(&self.status, self.conclusion.as_deref()),
            id: self.id,
            run_id: self.run_id,
            name: self.name,
            started_at: self.started_at,
            completed_at: self.completed_at,
            steps: self
                .steps
                .into_iter()
                .map(|s| Step {
                    outcome: rest_outcome(&s.status, s.conclusion.as_deref()),
                    number: s.number,
                    name: s.name,
                    started_at: s.started_at,
                    completed_at: s.completed_at,
                })
                .collect(),
        }
    }
}

/// The checks on a commit, by revision.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "RepoVariables"
)]
pub struct CommitChecksQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<RepoCommitChecks>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Repository",
    schema_module = "schema",
    variables = "RepoVariables"
)]
pub struct RepoCommitChecks {
    #[arguments(expression: $expression)]
    pub object: Option<ChecksTarget>,
}

// ---- branches ---------------------------------------------------------------------------------

/// A branch, and its latest commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchInfo {
    pub name: String,
    pub default: bool,
    pub oid: Option<String>,
    pub headline: Option<String>,
    /// `None` when the branch points at no commit, or its commit names nobody.
    pub author: Option<Person>,
    /// ISO 8601: when its latest commit was committed.
    pub date: Option<String>,
    /// Its most recent pull request: number and state.
    pub pr: Option<(u64, IssueState)>,
}

pub(crate) mod wire_branches {
    use serde::Deserialize;

    pub use super::wire::{Nodes, RepoName};
    use super::{GitActor, PullRequestState};

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Commit {
        pub oid: Option<String>,
        pub message_headline: Option<String>,
        pub committed_date: Option<String>,
        pub author: Option<GitActor>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Pr {
        pub number: u64,
        pub state: PullRequestState,
        pub is_draft: bool,
        pub head_ref_name: String,
        pub repository: RepoName,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Branch {
        pub name: String,
        pub target: Option<Commit>,
        pub associated_pull_requests: Nodes<Pr>,
    }
}

impl wire_branches::Branch {
    /// The branch, marked the default if it's `default`, with its newest
    /// pull request in `repo` (GitHub's list also has forks' PRs whose
    /// head is this branch).
    pub(crate) fn into_info(self, repo: &RepoId, default: Option<&str>) -> BranchInfo {
        let (oid, headline, date, author) = match self.target {
            Some(c) => (
                c.oid,
                c.message_headline,
                c.committed_date,
                Some(Person::from(c.author)).filter(|a| *a != Person::Unknown),
            ),
            None => (None, None, None, None),
        };
        let name = self.name.as_str();
        let ours = |p: &wire_branches::Pr| {
            p.head_ref_name == name
                && p.repository
                    .name_with_owner
                    .eq_ignore_ascii_case(&repo.to_string())
        };
        let pr = self
            .associated_pull_requests
            .into_iter()
            .find(ours)
            .map(|p| (p.number, IssueState::pr(p.state, p.is_draft)));
        BranchInfo {
            default: default == Some(self.name.as_str()),
            name: self.name,
            oid,
            headline,
            author,
            date,
            pr,
        }
    }
}

// ---- milestones -------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MilestoneInfo {
    pub number: u64,
    pub title: String,
    pub description: String,
    /// ISO 8601.
    pub due_on: Option<String>,
    pub closed: bool,
    pub closed_at: Option<String>,
    pub updated_at: String,
    /// Its issues and pull requests, open and done.
    pub open: u64,
    pub done: u64,
}

/// A page of milestones, and how many are open and closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MilestoneList {
    pub open: u64,
    pub closed: u64,
    pub results: Results<MilestoneInfo>,
}

/// A milestone and a page of its issues and pull requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MilestoneDetail {
    pub info: MilestoneInfo,
    pub items: Results<IssueSummary>,
    /// Its title has quotes, which GitHub's search can't match, so its
    /// issues and pull requests weren't looked up.
    #[serde(default)]
    pub unsearchable: bool,
}

pub(crate) mod wire_milestones {
    use serde::Deserialize;

    pub use super::wire::Count;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Milestone {
        pub number: u64,
        pub title: String,
        pub description: Option<String>,
        pub due_on: Option<String>,
        pub closed: bool,
        pub closed_at: Option<String>,
        pub updated_at: String,
        pub open_issues: Count,
        pub done_issues: Count,
        pub open_prs: Count,
        pub done_prs: Count,
    }
}

impl wire_milestones::Milestone {
    pub(crate) fn into_info(self) -> MilestoneInfo {
        MilestoneInfo {
            number: self.number,
            title: self.title,
            description: self.description.unwrap_or_default(),
            due_on: self.due_on,
            closed: self.closed,
            closed_at: self.closed_at,
            updated_at: self.updated_at,
            open: self.open_issues.total_count + self.open_prs.total_count,
            done: self.done_issues.total_count + self.done_prs.total_count,
        }
    }
}

// ---- deployments ------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentInfo {
    pub environment: String,
    pub outcome: CheckOutcome,
    /// GitHub's word for its state: `active`, `failure`, `inactive`…
    pub state: String,
    pub created_at: String,
    pub creator: Option<String>,
    pub branch: Option<String>,
    pub oid: String,
    /// Where its log is (usually a workflow run's job).
    pub log_url: Option<String>,
    /// What it deployed to.
    pub environment_url: Option<String>,
}

/// A page of deployments, and the environments to filter by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentList {
    pub environments: Capped<String>,
    pub results: Results<DeploymentInfo>,
}

pub(crate) mod wire_deployments {
    use serde::Deserialize;

    pub use super::{UserLogin, wire::Name};

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Status {
        pub log_url: Option<String>,
        pub environment_url: Option<String>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Deployment {
        pub environment: Option<String>,
        pub state: Option<String>,
        pub created_at: String,
        pub creator: Option<UserLogin>,
        #[serde(rename = "ref")]
        pub branch: Option<Name>,
        pub commit_oid: String,
        pub latest_status: Option<Status>,
    }
}

impl wire_deployments::Deployment {
    pub(crate) fn into_info(self) -> DeploymentInfo {
        let state = self.state.unwrap_or_default().to_lowercase();
        let outcome = match state.as_str() {
            "active" | "success" => CheckOutcome::Success,
            "failure" | "error" => CheckOutcome::Failure,
            "inactive" | "destroyed" => CheckOutcome::Neutral,
            "abandoned" => CheckOutcome::Cancelled,
            _ => CheckOutcome::Pending,
        };
        let (log_url, environment_url) = self
            .latest_status
            .map_or((None, None), |s| (s.log_url, s.environment_url));
        DeploymentInfo {
            environment: self.environment.unwrap_or_default(),
            outcome,
            state: state.replace('_', " "),
            created_at: self.created_at,
            creator: self.creator.map(|c| c.login),
            branch: self.branch.map(|r| r.name),
            oid: self.commit_oid,
            log_url,
            environment_url,
        }
    }
}
// ---- comparisons ------------------------------------------------------------------------------

/// Two revisions compared: the commits from one to the other, and what
/// the diff between them is from and to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comparison {
    /// `diverged`, `ahead`, `behind`, `identical`.
    pub status: String,
    pub ahead: u64,
    pub behind: u64,
    pub total_commits: u64,
    /// Oldest first.
    pub commits: Vec<CommitInfo>,
    /// The diff's ends: the merge base (or, for `a..b`, the base) and the
    /// head, as full commit IDs.
    pub from: String,
    pub to: String,
    pub files: u64,
    /// GitHub lists at most 300 files, so the counts may be short.
    #[serde(default)]
    pub files_capped: bool,
    pub additions: u64,
    pub deletions: u64,
}

/// How many files GitHub lists in a comparison at most.
const COMPARE_FILES: usize = 300;

/// How many commits GitHub lists in a comparison at most (the newest).
pub const COMPARE_COMMITS: u64 = 250;

pub(crate) mod rest_compare {
    use serde::Deserialize;

    pub use super::wire::RestCommit as Commit;

    #[derive(Deserialize)]
    pub struct Sha {
        pub sha: String,
    }

    #[derive(Deserialize)]
    pub struct File {
        #[serde(default)]
        pub additions: u64,
        #[serde(default)]
        pub deletions: u64,
    }

    #[derive(Deserialize)]
    pub struct Compare {
        /// Names the comparison's ends by short SHA:
        /// `…/compare/owner:bcf4368...owner:efd1e47`.
        #[serde(default)]
        pub permalink_url: String,
        pub status: String,
        pub ahead_by: u64,
        pub behind_by: u64,
        pub total_commits: u64,
        pub base_commit: Sha,
        pub merge_base_commit: Sha,
        pub commits: Vec<Commit>,
        #[serde(default)]
        pub files: Vec<File>,
    }
}

impl rest_compare::Compare {
    /// `direct`: `a..b`, the diff from the base itself, not the merge base.
    pub(crate) fn into_comparison(self, direct: bool) -> Comparison {
        // The newest commit listed is the head; with none, the head is
        // behind or at the base, so it's the merge base.
        let to = self
            .commits
            .last()
            .map_or_else(|| self.merge_base_commit.sha.clone(), |c| c.sha.clone());
        let from = if direct {
            self.base_commit.sha
        } else {
            self.merge_base_commit.sha
        };
        Comparison {
            status: self.status,
            ahead: self.ahead_by,
            behind: self.behind_by,
            total_commits: self.total_commits,
            to,
            from,
            files: self.files.len() as u64,
            files_capped: self.files.len() >= COMPARE_FILES,
            additions: self.files.iter().map(|f| f.additions).sum(),
            deletions: self.files.iter().map(|f| f.deletions).sum(),
            commits: self
                .commits
                .into_iter()
                .map(|c| c.into_info(wire::Dated::Authored))
                .collect(),
        }
    }
}
// ---- blame ------------------------------------------------------------------------------------

/// Which commit last changed each line of a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blame {
    pub ranges: Vec<BlameRange>,
}

/// Lines `start..=end` (from 1), last changed by one commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlameRange {
    pub start: u32,
    pub end: u32,
    /// How recent, 1 (newest) to 10 (oldest).
    pub age: u8,
    pub commit: CommitInfo,
}

pub(crate) mod wire_blame {
    use serde::Deserialize;

    use super::wire::Commit;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Range {
        pub starting_line: u32,
        pub ending_line: u32,
        pub age: u8,
        pub commit: Commit,
    }

    #[derive(Deserialize)]
    pub struct Blame {
        pub ranges: Vec<Range>,
    }
}

impl wire_blame::Blame {
    pub(crate) fn into_blame(self) -> Blame {
        Blame {
            ranges: self
                .ranges
                .into_iter()
                .map(|r| BlameRange {
                    start: r.starting_line,
                    end: r.ending_line,
                    age: r.age,
                    commit: r.commit.into_info(),
                })
                .collect(),
        }
    }
}
// ---- gists -------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GistFile {
    pub name: String,
    pub language: Option<String>,
    pub size: u64,
    /// `None` past the API's size limit.
    pub text: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gist {
    pub id: String,
    pub owner: Option<String>,
    pub description: String,
    pub public: bool,
    pub created_at: String,
    pub updated_at: String,
    pub comments: u64,
    pub files: Vec<GistFile>,
}

/// A gist in a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GistSummary {
    pub id: String,
    pub description: String,
    pub files: Vec<String>,
    pub updated_at: String,
    pub stars: u64,
    pub comments: u64,
}

pub(crate) mod rest_gists {
    use std::collections::BTreeMap;

    use serde::Deserialize;

    pub use super::UserLogin;

    #[derive(Deserialize)]
    pub struct File {
        pub language: Option<String>,
        #[serde(default)]
        pub size: u64,
        pub content: Option<String>,
        #[serde(default)]
        pub truncated: bool,
    }

    #[derive(Deserialize)]
    pub struct Gist {
        pub id: String,
        pub owner: Option<UserLogin>,
        pub description: Option<String>,
        pub public: bool,
        pub created_at: String,
        pub updated_at: String,
        #[serde(default)]
        pub comments: u64,
        pub files: BTreeMap<String, File>,
    }
}

impl rest_gists::Gist {
    pub(crate) fn into_gist(self) -> Gist {
        Gist {
            id: self.id,
            owner: self.owner.map(|o| o.login),
            description: self.description.unwrap_or_default(),
            public: self.public,
            created_at: self.created_at,
            updated_at: self.updated_at,
            comments: self.comments,
            files: self
                .files
                .into_iter()
                .map(|(name, f)| GistFile {
                    name,
                    language: f.language,
                    size: f.size,
                    text: f.content,
                    truncated: f.truncated,
                })
                .collect(),
        }
    }
}

pub(crate) mod wire_gists {
    use serde::Deserialize;

    pub use super::wire::Count;

    #[derive(Deserialize)]
    pub struct Name {
        pub name: Option<String>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Gist {
        pub name: String,
        pub description: Option<String>,
        pub updated_at: String,
        pub stargazer_count: u64,
        pub files: Option<Vec<Option<Name>>>,
        pub comments: Count,
    }
}

impl wire_gists::Gist {
    pub(crate) fn into_summary(self) -> GistSummary {
        GistSummary {
            id: self.name,
            description: self.description.unwrap_or_default(),
            files: self
                .files
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|f| f.name)
                .collect(),
            updated_at: self.updated_at,
            stars: self.stargazer_count,
            comments: self.comments.total_count,
        }
    }
}
// ---- teams -------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamSummary {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub secret: bool,
    pub members: u64,
    pub repos: u64,
}

/// A repository a team has access to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamRepo {
    pub repo: RepoId,
    pub description: String,
    pub stars: u64,
}

/// A team: its members, repositories and child teams (the first 50 of
/// each).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamDetail {
    pub team: TeamSummary,
    pub parent: Option<TeamSummary>,
    pub members: Capped<UserSummary>,
    pub repos: Capped<TeamRepo>,
    pub children: Capped<TeamSummary>,
}

pub(crate) mod wire_teams {
    use serde::Deserialize;

    pub use super::wire::{Count, Counted};

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Team {
        pub slug: String,
        pub name: String,
        pub description: Option<String>,
        pub privacy: Option<String>,
        pub members: Option<Count>,
        pub repositories: Option<Count>,
    }

    #[derive(Deserialize)]
    pub struct Member {
        pub login: String,
        pub name: Option<String>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Repo {
        pub name_with_owner: String,
        pub description: Option<String>,
        pub stargazer_count: u64,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Detail {
        #[serde(flatten)]
        pub team: Team,
        pub parent_team: Option<Team>,
        #[serde(rename = "memberList")]
        pub member_list: Counted<Member>,
        #[serde(rename = "repoList")]
        pub repo_list: Counted<Repo>,
        pub child_teams: Counted<Team>,
    }
}

impl wire_teams::Team {
    pub(crate) fn into_summary(self) -> TeamSummary {
        TeamSummary {
            slug: self.slug,
            name: self.name,
            description: self.description.unwrap_or_default(),
            secret: self.privacy.as_deref() == Some("SECRET"),
            members: self.members.map_or(0, |c| c.total_count),
            repos: self.repositories.map_or(0, |c| c.total_count),
        }
    }
}

impl wire_teams::Detail {
    pub(crate) fn into_detail(self) -> TeamDetail {
        TeamDetail {
            team: self.team.into_summary(),
            parent: self.parent_team.map(wire_teams::Team::into_summary),
            members: self.member_list.into_capped(|m| {
                Some(UserSummary {
                    login: m.login,
                    name: m.name,
                    bio: None,
                    is_org: false,
                })
            }),
            repos: self.repo_list.into_capped(|r| {
                Some(TeamRepo {
                    repo: RepoId::parse(&r.name_with_owner)?,
                    description: r.description.unwrap_or_default(),
                    stars: r.stargazer_count,
                })
            }),
            children: self.child_teams.into_capped(|t| Some(t.into_summary())),
        }
    }
}
// ---- security advisories ----------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvisoryPackage {
    pub ecosystem: String,
    pub name: String,
    pub vulnerable: Option<String>,
    pub patched: Option<String>,
}

/// A security advisory: GitHub's reviewed database's, or a repository's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Advisory {
    pub ghsa: String,
    pub cve: Option<String>,
    pub summary: String,
    pub description: String,
    /// `low`, `medium`, `high`, `critical`.
    pub severity: String,
    pub published_at: Option<String>,
    pub updated_at: Option<String>,
    pub withdrawn_at: Option<String>,
    /// The CVSS score (as GitHub writes it) and vector.
    pub cvss: Option<(String, String)>,
    /// `CWE-79 Cross-site Scripting`…
    pub cwes: Vec<String>,
    pub packages: Vec<AdvisoryPackage>,
    pub credits: Vec<String>,
    pub references: Vec<String>,
}

pub(crate) mod rest_advisories {
    use serde::Deserialize;

    pub use super::UserLogin;

    #[derive(Deserialize)]
    pub struct Package {
        pub ecosystem: Option<String>,
        pub name: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct Vulnerability {
        pub package: Option<Package>,
        pub vulnerable_version_range: Option<String>,
        #[serde(alias = "first_patched_version")]
        pub patched_versions: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct Cvss {
        pub score: Option<f64>,
        pub vector_string: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct Cwe {
        pub cwe_id: String,
        pub name: String,
    }

    #[derive(Deserialize)]
    pub struct Credit {
        pub login: Option<String>,
        pub user: Option<UserLogin>,
    }

    #[derive(Deserialize)]
    pub struct Advisory {
        pub ghsa_id: String,
        pub cve_id: Option<String>,
        pub summary: String,
        pub description: Option<String>,
        pub severity: Option<String>,
        pub published_at: Option<String>,
        pub updated_at: Option<String>,
        pub withdrawn_at: Option<String>,
        pub cvss: Option<Cvss>,
        #[serde(default)]
        pub cwes: Option<Vec<Cwe>>,
        #[serde(default)]
        pub vulnerabilities: Option<Vec<Vulnerability>>,
        #[serde(default)]
        pub credits: Option<Vec<Credit>>,
        #[serde(default)]
        pub references: Option<Vec<String>>,
    }
}

impl rest_advisories::Advisory {
    pub(crate) fn into_advisory(self) -> Advisory {
        Advisory {
            ghsa: self.ghsa_id,
            cve: self.cve_id,
            summary: self.summary,
            description: self.description.unwrap_or_default(),
            severity: self.severity.unwrap_or_else(|| "unknown".into()),
            published_at: self.published_at,
            updated_at: self.updated_at,
            withdrawn_at: self.withdrawn_at,
            cvss: self
                .cvss
                .and_then(|c| Some((format!("{:.1}", c.score?), c.vector_string?))),
            cwes: self
                .cwes
                .into_iter()
                .flatten()
                .map(|c| format!("{} {}", c.cwe_id, c.name))
                .collect(),
            packages: self
                .vulnerabilities
                .into_iter()
                .flatten()
                .map(|v| {
                    let package = v.package;
                    AdvisoryPackage {
                        ecosystem: package
                            .as_ref()
                            .and_then(|p| p.ecosystem.clone())
                            .unwrap_or_default(),
                        name: package.and_then(|p| p.name).unwrap_or_default(),
                        vulnerable: v.vulnerable_version_range,
                        patched: v.patched_versions,
                    }
                })
                .collect(),
            credits: self
                .credits
                .into_iter()
                .flatten()
                .filter_map(|c| c.login.or(c.user.map(|u| u.login)))
                .collect(),
            references: self.references.unwrap_or_default(),
        }
    }
}
// ---- wikis -------------------------------------------------------------------------------------

/// A wiki page (or, without one, the wiki's list of pages), read from
/// the wiki's git repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WikiPage {
    /// Its title (the file's name, without its extension).
    pub title: Option<String>,
    pub text: Option<String>,
    /// Whether the text is Markdown (wikis may use other markups).
    pub markdown: bool,
    /// Every page's title, in order.
    pub pages: Vec<String>,
    pub sidebar: Option<String>,
}
// ---- search: discussions, commits, code -------------------------------------------------------

pub(crate) mod rest_search {
    use serde::Deserialize;

    use super::wire::RestCommit;

    #[derive(Deserialize)]
    pub struct Repo {
        pub full_name: String,
    }

    #[derive(Deserialize)]
    pub struct Commit {
        pub repository: Repo,
        #[serde(flatten)]
        pub commit: RestCommit,
    }

    #[derive(Deserialize)]
    pub struct Code {
        pub path: String,
        pub sha: String,
        pub repository: Repo,
    }

    #[derive(Deserialize)]
    pub struct Page<T> {
        pub total_count: u64,
        /// The search timed out and found only some of them.
        #[serde(default)]
        pub incomplete_results: bool,
        pub items: Vec<T>,
    }
}

impl rest_search::Commit {
    pub(crate) fn into_hit(self) -> Option<CommitHit> {
        Some(CommitHit {
            repo: RepoId::parse(&self.repository.full_name)?,
            commit: self.commit.into_info(wire::Dated::Committed),
        })
    }
}

impl rest_search::Code {
    pub(crate) fn into_hit(self) -> Option<CodeHit> {
        Some(CodeHit {
            repo: RepoId::parse(&self.repository.full_name)?,
            path: self.path,
            sha: self.sha,
        })
    }
}
// ---- discussions ------------------------------------------------------------------------------

/// Whose discussions: a repository's, or an organization's (which GitHub
/// keeps in one of its repositories).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DiscussionsOf {
    Repo(RepoId),
    Org(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscussionCategory {
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscussionSummary {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub category: String,
    pub comments: u64,
    pub answered: bool,
    pub upvotes: u64,
    /// ISO 8601.
    pub updated_at: String,
}

/// A page of discussions, and the categories to filter by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscussionList {
    pub categories: Capped<DiscussionCategory>,
    pub results: Results<DiscussionSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscussionDetail {
    /// The repository it's in.
    pub repo: RepoId,
    pub number: u64,
    pub title: String,
    pub body: String,
    pub author: String,
    pub created_at: String,
    pub category: String,
    pub answered: bool,
    pub upvotes: u64,
    pub comments: Vec<DiscussionComment>,
    /// How many comments there are (the first 50 are here).
    #[serde(default)]
    pub total_comments: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscussionComment {
    pub comment: Comment,
    pub upvotes: u64,
    /// The answer to a question.
    pub answer: bool,
    pub replies: Vec<Comment>,
    /// How many replies there are (the first 30 are here).
    #[serde(default)]
    pub total_replies: u64,
}

pub(crate) mod wire_discussions {
    use serde::Deserialize;

    pub use super::wire::{Count, Name, RepoName};
    pub use crate::queries::Actor;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Summary {
        pub number: u64,
        pub title: String,
        pub author: Option<Actor>,
        pub category: Name,
        pub comments: Count,
        pub is_answered: Option<bool>,
        pub upvote_count: u64,
        pub updated_at: String,
    }

    #[derive(Deserialize)]
    pub struct Category {
        pub id: String,
        pub name: String,
        pub slug: String,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Reply {
        pub database_id: Option<u64>,
        pub author: Option<Actor>,
        pub body: String,
        pub created_at: String,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Replies {
        #[serde(default)]
        pub total_count: u64,
        pub nodes: Vec<Option<Reply>>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct WireComment {
        pub database_id: Option<u64>,
        pub author: Option<Actor>,
        pub body: String,
        pub created_at: String,
        pub is_answer: bool,
        pub upvote_count: u64,
        pub replies: Replies,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Comments {
        #[serde(default)]
        pub total_count: u64,
        pub nodes: Vec<Option<WireComment>>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Detail {
        pub number: u64,
        pub title: String,
        pub body: String,
        pub author: Option<Actor>,
        pub created_at: String,
        pub category: Name,
        pub is_answered: Option<bool>,
        pub upvote_count: u64,
        pub comments: Comments,
    }

    /// A discussion a search found, and where.
    #[derive(Deserialize)]
    pub struct Hit {
        pub repository: RepoName,
        #[serde(flatten)]
        pub summary: Summary,
    }
}

impl wire_discussions::Hit {
    pub(crate) fn into_hit(self) -> Option<DiscussionHit> {
        Some(DiscussionHit {
            repo: RepoId::parse(&self.repository.name_with_owner)?,
            summary: self.summary.into_summary(),
        })
    }
}

impl wire_discussions::Summary {
    pub(crate) fn into_summary(self) -> DiscussionSummary {
        DiscussionSummary {
            number: self.number,
            title: self.title,
            author: author(self.author),
            category: self.category.name,
            comments: self.comments.total_count,
            answered: self.is_answered.unwrap_or(false),
            upvotes: self.upvote_count,
            updated_at: self.updated_at,
        }
    }
}

impl wire_discussions::Detail {
    pub(crate) fn into_detail(self, repo: RepoId) -> DiscussionDetail {
        let comment = |id, who, body, created_at| Comment {
            id,
            author: author(who),
            body,
            created_at,
        };
        DiscussionDetail {
            repo,
            number: self.number,
            title: self.title,
            body: self.body,
            author: author(self.author),
            created_at: self.created_at,
            category: self.category.name,
            answered: self.is_answered.unwrap_or(false),
            upvotes: self.upvote_count,
            total_comments: self.comments.total_count,
            comments: self
                .comments
                .nodes
                .into_iter()
                .flatten()
                .map(|c| DiscussionComment {
                    upvotes: c.upvote_count,
                    answer: c.is_answer,
                    total_replies: c.replies.total_count,
                    replies: c
                        .replies
                        .nodes
                        .into_iter()
                        .flatten()
                        .map(|r| comment(r.database_id, r.author, r.body, r.created_at))
                        .collect(),
                    comment: comment(c.database_id, c.author, c.body, c.created_at),
                })
                .collect(),
        }
    }
}

// ---- profiles ------------------------------------------------------------------------

#[derive(cynic::Scalar, Debug, Clone)]
#[cynic(graphql_type = "Date", schema_module = "schema")]
pub struct Date(pub String);

/// A profile README: `<login>/<login>`'s for a user, `.github`'s
/// `profile/README.md` for an organization.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Repository", schema_module = "schema")]
pub struct UserReadmeRepo {
    #[arguments(expression: "HEAD:README.md")]
    pub object: Option<ReadmeObject>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Repository", schema_module = "schema")]
pub struct OrgReadmeRepo {
    #[arguments(expression: "HEAD:profile/README.md")]
    pub object: Option<ReadmeObject>,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "GitObject", schema_module = "schema")]
pub enum ReadmeObject {
    Blob(ReadmeBlob),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Blob", schema_module = "schema")]
pub struct ReadmeBlob {
    pub text: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "UserStatus", schema_module = "schema")]
pub struct WireStatus {
    pub emoji: Option<String>,
    pub message: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "SocialAccountConnection", schema_module = "schema")]
pub struct SocialList {
    pub nodes: Option<Vec<Option<WireSocial>>>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "SocialAccount", schema_module = "schema")]
pub struct WireSocial {
    pub display_name: String,
    pub url: Uri,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "OrganizationConnection", schema_module = "schema")]
pub struct OrgLogins {
    pub total_count: i32,
    pub nodes: Option<Vec<Option<OrgLogin>>>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Organization", schema_module = "schema")]
pub struct OrgLogin {
    pub login: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "OrganizationMemberConnection",
    schema_module = "schema"
)]
pub struct MemberList {
    pub total_count: i32,
    pub nodes: Option<Vec<Option<UserLogin>>>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "StarredRepositoryConnection", schema_module = "schema")]
pub struct StarCount {
    pub total_count: i32,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "ContributionsCollection", schema_module = "schema")]
pub struct WireContributions {
    pub contribution_calendar: WireCalendar,
    /// The 10 repositories with the most, and how many there are.
    #[arguments(maxRepositories: 10)]
    pub commit_contributions_by_repository: Vec<WireRepoCommits>,
    pub total_repositories_with_contributed_commits: i32,
    // These lists are newest first, so `first` is the newest.
    #[arguments(first: 20)]
    pub pull_request_contributions: PrContributions,
    #[arguments(first: 20)]
    pub issue_contributions: IssueContributions,
    #[arguments(first: 20)]
    pub pull_request_review_contributions: ReviewContributions,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "ContributionCalendar", schema_module = "schema")]
pub struct WireCalendar {
    pub total_contributions: i32,
    pub weeks: Vec<WireWeek>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "ContributionCalendarWeek", schema_module = "schema")]
pub struct WireWeek {
    pub contribution_days: Vec<WireDay>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "ContributionCalendarDay", schema_module = "schema")]
pub struct WireDay {
    pub date: Date,
    pub contribution_level: ContributionLevel,
}

#[derive(cynic::Enum, Debug, Clone, Copy)]
#[cynic(graphql_type = "ContributionLevel", schema_module = "schema")]
pub enum ContributionLevel {
    None,
    FirstQuartile,
    SecondQuartile,
    ThirdQuartile,
    FourthQuartile,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "CommitContributionsByRepository",
    schema_module = "schema"
)]
pub struct WireRepoCommits {
    pub repository: RepositoryName,
    #[arguments(first: 100)]
    pub contributions: CommitContributions,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "CreatedCommitContributionConnection",
    schema_module = "schema"
)]
pub struct CommitContributions {
    pub total_count: i32,
    pub nodes: Option<Vec<Option<WireCommitContribution>>>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "CreatedCommitContribution", schema_module = "schema")]
pub struct WireCommitContribution {
    pub occurred_at: DateTime,
    pub commit_count: i32,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "CreatedPullRequestContributionConnection",
    schema_module = "schema"
)]
pub struct PrContributions {
    pub total_count: i32,
    pub nodes: Option<Vec<Option<WirePrContribution>>>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "CreatedPullRequestContribution",
    schema_module = "schema"
)]
pub struct WirePrContribution {
    pub occurred_at: DateTime,
    pub pull_request: ContributedPr,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "CreatedIssueContributionConnection",
    schema_module = "schema"
)]
pub struct IssueContributions {
    pub total_count: i32,
    pub nodes: Option<Vec<Option<WireIssueContribution>>>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "CreatedIssueContribution", schema_module = "schema")]
pub struct WireIssueContribution {
    pub occurred_at: DateTime,
    pub issue: ContributedIssue,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "CreatedPullRequestReviewContributionConnection",
    schema_module = "schema"
)]
pub struct ReviewContributions {
    pub total_count: i32,
    pub nodes: Option<Vec<Option<WireReviewContribution>>>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "CreatedPullRequestReviewContribution",
    schema_module = "schema"
)]
pub struct WireReviewContribution {
    pub occurred_at: DateTime,
    pub pull_request: ContributedPr,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequest", schema_module = "schema")]
pub struct ContributedPr {
    pub number: i32,
    pub title: String,
    pub repository: RepositoryName,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Issue", schema_module = "schema")]
pub struct ContributedIssue {
    pub number: i32,
    pub title: String,
    pub repository: RepositoryName,
}

/// How many months of activity a profile lists.
const ACTIVITY_MONTHS: usize = 3;

impl WireContributions {
    pub(crate) fn into_contributions(self) -> Contributions {
        let calendar = self.contribution_calendar;
        let weeks = calendar
            .weeks
            .into_iter()
            .filter_map(|w| {
                let start = w.contribution_days.first()?.date.0.clone();
                let days = w
                    .contribution_days
                    .iter()
                    .map(|d| match d.contribution_level {
                        ContributionLevel::None => 0,
                        ContributionLevel::FirstQuartile => 1,
                        ContributionLevel::SecondQuartile => 2,
                        ContributionLevel::ThirdQuartile => 3,
                        ContributionLevel::FourthQuartile => 4,
                    })
                    .collect();
                Some(Week { start, days })
            })
            .collect();
        let month = |at: &DateTime| at.0.get(..7).unwrap_or_default().to_owned();
        let mut months: Vec<MonthActivity> = Vec::new();
        let mut found = Vec::new();
        let mut short = Short::default();
        // A list cut short has all of its kind from its oldest month on;
        // in that month and before, there may be more.
        let cut = |total: i32, list: &[DateTime]| {
            let oldest = list.iter().min_by(|a, b| a.0.cmp(&b.0));
            oldest
                .filter(|_| usize::try_from(total).unwrap_or_default() > list.len())
                .map(month)
        };
        let later = |a: Option<String>, b: Option<String>| a.max(b);
        if usize::try_from(self.total_repositories_with_contributed_commits).unwrap_or_default()
            > self.commit_contributions_by_repository.len()
        {
            // Another repository's commits could be in any month.
            short.commits = Some(Short::EVERY_MONTH.to_owned());
        }
        for repo in self.commit_contributions_by_repository {
            let contributions: Vec<WireCommitContribution> =
                nodes(repo.contributions.nodes).collect();
            let at: Vec<DateTime> = contributions
                .iter()
                .map(|c| c.occurred_at.clone())
                .collect();
            short.commits = later(short.commits, cut(repo.contributions.total_count, &at));
            for c in contributions {
                found.push((
                    month(&c.occurred_at),
                    Event::Commits(
                        repo.repository.name_with_owner.clone(),
                        count(c.commit_count),
                    ),
                ));
            }
        }
        let item = |repo: RepositoryName, number: i32, title: String| Contributed {
            repo: repo.name_with_owner,
            number: count(number),
            title,
        };
        let pulls: Vec<_> = nodes(self.pull_request_contributions.nodes).collect();
        let issues: Vec<_> = nodes(self.issue_contributions.nodes).collect();
        let reviews: Vec<_> = nodes(self.pull_request_review_contributions.nodes).collect();
        short.pulls = cut(
            self.pull_request_contributions.total_count,
            &pulls
                .iter()
                .map(|p| p.occurred_at.clone())
                .collect::<Vec<_>>(),
        );
        short.issues = cut(
            self.issue_contributions.total_count,
            &issues
                .iter()
                .map(|i| i.occurred_at.clone())
                .collect::<Vec<_>>(),
        );
        short.reviews = cut(
            self.pull_request_review_contributions.total_count,
            &reviews
                .iter()
                .map(|r| r.occurred_at.clone())
                .collect::<Vec<_>>(),
        );
        for p in pulls {
            let pr = p.pull_request;
            found.push((
                month(&p.occurred_at),
                Event::Pull(item(pr.repository, pr.number, pr.title)),
            ));
        }
        for i in issues {
            let issue = i.issue;
            found.push((
                month(&i.occurred_at),
                Event::Issue(item(issue.repository, issue.number, issue.title)),
            ));
        }
        for r in reviews {
            let pr = r.pull_request;
            found.push((
                month(&r.occurred_at),
                Event::Review(item(pr.repository, pr.number, pr.title)),
            ));
        }
        for (key, event) in found {
            let i = match months.iter().position(|m| m.month == key) {
                Some(i) => i,
                None => {
                    months.push(MonthActivity {
                        month: key,
                        ..MonthActivity::default()
                    });
                    months.len() - 1
                }
            };
            let Some(m) = months.get_mut(i) else {
                continue;
            };
            match event {
                Event::Commits(repo, n) => match m.commits.iter_mut().find(|(r, _)| *r == repo) {
                    Some((_, total)) => *total += n,
                    None => m.commits.push((repo, n)),
                },
                Event::Pull(c) => m.pulls.push(c),
                Event::Issue(c) => m.issues.push(c),
                Event::Review(c) => {
                    if !m
                        .reviews
                        .iter()
                        .any(|r| r.repo == c.repo && r.number == c.number)
                    {
                        m.reviews.push(c);
                    }
                }
            }
        }
        months.sort_by(|a, b| b.month.cmp(&a.month));
        months.truncate(ACTIVITY_MONTHS);
        for m in &mut months {
            m.commits
                .sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        }
        Contributions {
            total: count(calendar.total_contributions),
            weeks,
            activity: months,
            short,
        }
    }
}

/// One contribution, before it's filed under its month.
enum Event {
    Commits(String, u64),
    Pull(Contributed),
    Issue(Contributed),
    Review(Contributed),
}

#[derive(cynic::QueryVariables, Debug)]
pub struct ProfileVariables {
    pub login: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "ProfileVariables"
)]
pub struct ProfileQuery {
    #[arguments(login: $login)]
    pub user: Option<UserFull>,
    #[arguments(login: $login)]
    pub organization: Option<OrgFull>,
    #[cynic(rename = "repository", alias)]
    #[arguments(owner: $login, name: $login)]
    pub user_readme: Option<UserReadmeRepo>,
    #[cynic(rename = "repository", alias)]
    #[arguments(owner: $login, name: ".github")]
    pub org_readme: Option<OrgReadmeRepo>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "User", schema_module = "schema")]
pub struct UserFull {
    pub login: String,
    pub name: Option<String>,
    pub bio: Option<String>,
    pub company: Option<String>,
    pub location: Option<String>,
    pub website_url: Option<Uri>,
    pub followers: FollowCount,
    pub following: FollowingCount,
    #[arguments(first: 6, types: [REPOSITORY])]
    pub pinned_items: Pinned,
    #[arguments(first: 30, orderBy: { field: PUSHED_AT, direction: DESC }, ownerAffiliations: [OWNER])]
    pub repositories: RepoList,
    pub starred_repositories: StarCount,
    pub status: Option<WireStatus>,
    pub pronouns: Option<String>,
    #[arguments(first: 10)]
    pub social_accounts: SocialList,
    #[arguments(first: 20)]
    pub organizations: OrgLogins,
    pub contributions_collection: WireContributions,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Organization", schema_module = "schema")]
pub struct OrgFull {
    pub login: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub website_url: Option<Uri>,
    #[arguments(first: 6, types: [REPOSITORY])]
    pub pinned_items: Pinned,
    #[arguments(first: 30, orderBy: { field: PUSHED_AT, direction: DESC })]
    pub repositories: RepoList,
    pub is_verified: bool,
    #[arguments(first: 12)]
    pub members_with_role: MemberList,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "PinnableItem", schema_module = "schema")]
pub enum PinnedItem {
    Repository(RepoCard),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RepositoryConnection", schema_module = "schema")]
pub struct RepoList {
    pub total_count: i32,
    pub nodes: Option<Vec<Option<RepoCard>>>,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct ListVariables {
    pub owner: String,
    pub name: String,
    pub after: Option<String>,
}

/// A repository's forks, most starred first.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "ListVariables"
)]
pub struct ForksQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<RepoForks>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Repository",
    schema_module = "schema",
    variables = "ListVariables"
)]
pub struct RepoForks {
    #[arguments(first: 30, after: $after, orderBy: { field: STARGAZERS, direction: DESC })]
    pub forks: PagedRepos,
}

/// A page of repositories.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RepositoryConnection", schema_module = "schema")]
pub struct PagedRepos {
    pub total_count: i32,
    pub page_info: PageInfo,
    pub nodes: Option<Vec<Option<RepoCard>>>,
}

impl PagedRepos {
    pub(crate) fn into_results(self) -> Results<RepoSummary> {
        Results {
            total: count(self.total_count),
            items: nodes(self.nodes)
                .filter_map(RepoCard::into_summary)
                .collect(),
            next: self.page_info.next(),
        }
    }
}

#[derive(cynic::Enum, Debug, Clone, Copy)]
#[cynic(graphql_type = "RepositoryOrderField", schema_module = "schema")]
pub enum RepositoryOrderField {
    CreatedAt,
    Name,
    PushedAt,
    Stargazers,
    UpdatedAt,
}

#[derive(cynic::Enum, Debug, Clone, Copy)]
#[cynic(graphql_type = "OrderDirection", schema_module = "schema")]
pub enum OrderDirection {
    Asc,
    Desc,
}

#[derive(cynic::InputObject, Debug)]
#[cynic(graphql_type = "RepositoryOrder", schema_module = "schema")]
pub struct RepositoryOrder {
    pub field: RepositoryOrderField,
    pub direction: OrderDirection,
}

impl From<RepoSort> for RepositoryOrder {
    fn from(sort: RepoSort) -> Self {
        let (field, direction) = match sort {
            RepoSort::Updated => (RepositoryOrderField::PushedAt, OrderDirection::Desc),
            RepoSort::Name => (RepositoryOrderField::Name, OrderDirection::Asc),
            RepoSort::Stars => (RepositoryOrderField::Stargazers, OrderDirection::Desc),
        };
        RepositoryOrder { field, direction }
    }
}

#[derive(cynic::QueryVariables, Debug)]
pub struct OwnerReposVariables {
    pub login: String,
    pub after: Option<String>,
    pub order: RepositoryOrder,
}

/// A user's or organization's own repositories.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "OwnerReposVariables"
)]
pub struct OwnerReposQuery {
    #[arguments(login: $login)]
    pub repository_owner: Option<OwnerRepos>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RepositoryOwner",
    schema_module = "schema",
    variables = "OwnerReposVariables"
)]
pub struct OwnerRepos {
    #[arguments(first: 30, after: $after, orderBy: $order, ownerAffiliations: [OWNER])]
    pub repositories: PagedRepos,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct LoginPageVariables {
    pub login: String,
    pub after: Option<String>,
}

/// What a user starred, most recently first.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "LoginPageVariables"
)]
pub struct StarredQuery {
    #[arguments(login: $login)]
    pub user: Option<UserStarred>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "User",
    schema_module = "schema",
    variables = "LoginPageVariables"
)]
pub struct UserStarred {
    #[arguments(first: 30, after: $after, orderBy: { field: STARRED_AT, direction: DESC })]
    pub starred_repositories: PagedStars,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "StarredRepositoryConnection", schema_module = "schema")]
pub struct PagedStars {
    pub total_count: i32,
    pub page_info: PageInfo,
    pub nodes: Option<Vec<Option<RepoCard>>>,
}

impl PagedStars {
    pub(crate) fn into_results(self) -> Results<RepoSummary> {
        Results {
            total: count(self.total_count),
            items: nodes(self.nodes)
                .filter_map(RepoCard::into_summary)
                .collect(),
            next: self.page_info.next(),
        }
    }
}

// ---- releases and tags ---------------------------------------------------------------------

/// A repository's releases, newest first.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "ListVariables"
)]
pub struct ReleasesQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<RepoReleases>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Repository",
    schema_module = "schema",
    variables = "ListVariables"
)]
pub struct RepoReleases {
    #[arguments(first: 20, after: $after, orderBy: { field: CREATED_AT, direction: DESC })]
    pub releases: ReleaseList,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "ReleaseConnection", schema_module = "schema")]
pub struct ReleaseList {
    pub total_count: i32,
    pub page_info: PageInfo,
    pub nodes: Option<Vec<Option<WireRelease>>>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Release", schema_module = "schema")]
pub struct WireRelease {
    pub name: Option<String>,
    pub tag_name: String,
    pub published_at: Option<DateTime>,
    pub is_prerelease: bool,
    pub is_draft: bool,
    pub is_latest: bool,
    pub author: Option<UserLogin>,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct ReleaseVariables {
    pub owner: String,
    pub name: String,
    pub tag: String,
}

/// One release, with its notes and assets.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "ReleaseVariables"
)]
pub struct ReleaseQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<RepoRelease>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Repository",
    schema_module = "schema",
    variables = "ReleaseVariables"
)]
pub struct RepoRelease {
    #[arguments(tagName: $tag)]
    pub release: Option<WireReleaseFull>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Release", schema_module = "schema")]
pub struct WireReleaseFull {
    #[cynic(spread)]
    pub card: WireRelease,
    pub description: Option<String>,
    #[arguments(first: 50)]
    pub release_assets: AssetList,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "ReleaseAssetConnection", schema_module = "schema")]
pub struct AssetList {
    pub total_count: i32,
    pub nodes: Option<Vec<Option<WireAsset>>>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "ReleaseAsset", schema_module = "schema")]
pub struct WireAsset {
    pub name: String,
    pub size: i32,
    pub download_count: i32,
    pub download_url: Uri,
}

/// A repository's tags, most recently committed first.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "ListVariables"
)]
pub struct TagsQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<RepoTags>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Repository",
    schema_module = "schema",
    variables = "ListVariables"
)]
pub struct RepoTags {
    #[arguments(refPrefix: "refs/tags/", first: 30, after: $after, orderBy: { field: TAG_COMMIT_DATE, direction: DESC })]
    pub refs: Option<TagRefs>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RefConnection", schema_module = "schema")]
pub struct TagRefs {
    pub total_count: i32,
    pub page_info: PageInfo,
    pub nodes: Option<Vec<Option<TagRef>>>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Ref", schema_module = "schema")]
pub struct TagRef {
    pub name: String,
    pub target: Option<TagTarget>,
}

/// A lightweight tag points at a commit; an annotated one at a tag
/// object that points at it.
#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "GitObject", schema_module = "schema")]
pub enum TagTarget {
    Commit(TagCommit),
    Tag(AnnotatedTag),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Commit", schema_module = "schema")]
pub struct TagCommit {
    pub oid: GitObjectId,
    pub committed_date: DateTime,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Tag", schema_module = "schema")]
pub struct AnnotatedTag {
    pub target: TaggedObject,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "GitObject", schema_module = "schema")]
pub enum TaggedObject {
    Commit(TagCommit),
    #[cynic(fallback)]
    Other,
}

impl WireRelease {
    fn into_release(self) -> Release {
        Release {
            name: self
                .name
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| self.tag_name.clone()),
            tag: self.tag_name,
            published_at: self.published_at.map(|d| d.0),
            prerelease: self.is_prerelease,
            draft: self.is_draft,
            latest: self.is_latest,
            author: self.author.map(|a| a.login),
            notes: None,
            assets: Vec::new().into(),
        }
    }
}

impl ReleaseList {
    pub(crate) fn into_results(self) -> Results<Release> {
        Results {
            total: count(self.total_count),
            items: nodes(self.nodes).map(WireRelease::into_release).collect(),
            next: self.page_info.next(),
        }
    }
}

impl WireReleaseFull {
    pub(crate) fn into_release(self) -> Release {
        Release {
            notes: self.description.filter(|d| !d.trim().is_empty()),
            assets: Capped::from_nodes(
                self.release_assets.total_count,
                self.release_assets.nodes,
                |a| Asset {
                    name: a.name,
                    size: count(a.size),
                    downloads: count(a.download_count),
                    url: a.download_url.0,
                },
            ),
            ..self.card.into_release()
        }
    }
}

impl TagRefs {
    pub(crate) fn into_results(self) -> Results<TagInfo> {
        let items = nodes(self.nodes)
            .map(|r| {
                let commit = match r.target {
                    Some(TagTarget::Commit(c)) => Some(c),
                    Some(TagTarget::Tag(t)) => match t.target {
                        TaggedObject::Commit(c) => Some(c),
                        TaggedObject::Other => None,
                    },
                    _ => None,
                };
                TagInfo {
                    name: r.name,
                    oid: commit.as_ref().map(|c| c.oid.0.clone()),
                    date: commit.map(|c| c.committed_date.0),
                }
            })
            .collect();
        Results {
            total: count(self.total_count),
            items,
            next: self.page_info.next(),
        }
    }
}

// ---- branches ------------------------------------------------------------------------------

#[derive(cynic::QueryVariables, Debug)]
pub struct BranchesVariables {
    pub owner: String,
    pub name: String,
}

/// A repository's branches (by name) and tags (newest first), and how
/// many there are of each.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refs {
    pub branches: Vec<String>,
    pub tags: Vec<String>,
    #[serde(default)]
    pub branch_total: u64,
    #[serde(default)]
    pub tag_total: u64,
}

// ---- the viewer's repositories ---------------------------------------------------------

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Query", schema_module = "schema")]
pub struct ViewerReposQuery {
    pub viewer: ViewerRepos,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "User", schema_module = "schema")]
pub struct ViewerRepos {
    #[arguments(
        first: 20,
        orderBy: { field: PUSHED_AT, direction: DESC },
        affiliations: [OWNER, COLLABORATOR, ORGANIZATION_MEMBER],
        ownerAffiliations: [OWNER, COLLABORATOR, ORGANIZATION_MEMBER]
    )]
    pub repositories: RepoList,
}

// ---- comments --------------------------------------------------------------------------

#[derive(cynic::QueryVariables, Debug)]
pub struct AddCommentVariables {
    pub subject: cynic::Id,
    pub body: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Mutation",
    schema_module = "schema",
    variables = "AddCommentVariables"
)]
pub struct AddComment {
    #[arguments(input: { subjectId: $subject, body: $body })]
    pub add_comment: Option<AddCommentPayload>,
}

// ---- starring -----------------------------------------------------------------------------

#[derive(cynic::QueryVariables, Debug)]
pub struct StarVariables {
    pub starrable: cynic::Id,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Mutation",
    schema_module = "schema",
    variables = "StarVariables"
)]
pub struct AddStar {
    #[arguments(input: { starrableId: $starrable })]
    pub add_star: Option<StarPayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Mutation",
    schema_module = "schema",
    variables = "StarVariables"
)]
pub struct RemoveStar {
    #[arguments(input: { starrableId: $starrable })]
    pub remove_star: Option<UnstarPayload>,
}

/// Cache keys for browsed pages.
pub mod keys {
    use super::SearchKind;
    use crate::model::{PrRef, RepoId};

    pub fn repo(repo: &RepoId) -> String {
        format!("repo:{repo}")
    }
    pub fn readme(repo: &RepoId) -> String {
        format!("readme:{repo}")
    }
    pub fn tree(repo: &RepoId, rev: &str, path: &str) -> String {
        format!("tree:{repo}:{rev}:{path}")
    }
    pub fn search(kind: SearchKind, query: &str) -> String {
        format!("search:{kind:?}:{query}")
    }
    pub fn issue(repo: &RepoId, number: u64) -> String {
        format!("issue:{repo}#{number}")
    }
    pub fn pr_activity(pr: &PrRef) -> String {
        format!("pr-activity:{pr}")
    }
    pub fn profile(login: &str) -> String {
        format!("profile:{}", login.to_lowercase())
    }
    pub fn last_commits(repo: &RepoId, rev: &str, path: &str) -> String {
        format!("last-commits:{repo}:{rev}:{path}")
    }
    pub fn refs(repo: &RepoId) -> String {
        format!("refs:{repo}")
    }
    pub fn commit(repo: &RepoId, oid: &str) -> String {
        format!("commit:{repo}@{oid}")
    }
    pub fn run(repo: &RepoId, run: u64, attempt: Option<u64>) -> String {
        format!("run:{repo}:{run}:{attempt:?}")
    }
    pub fn job(repo: &RepoId, job: u64) -> String {
        format!("job:{repo}:{job}")
    }
    pub fn workflow(repo: &RepoId, file: &str) -> String {
        format!("workflow:{repo}:{file}")
    }
    pub fn workflow_runs(repo: &RepoId, file: &str) -> String {
        format!("workflow-runs:{repo}:{file}")
    }
    pub fn commit_checks(repo: &RepoId, rev: &str) -> String {
        format!("commit-checks:{repo}@{rev}")
    }
    pub fn discussions(of: &super::DiscussionsOf, category: Option<&str>) -> String {
        format!("discussions:{of:?}:{category:?}")
    }
    pub fn discussion(of: &super::DiscussionsOf, number: u64) -> String {
        format!("discussion:{of:?}#{number}")
    }
    pub fn pr_checks(pr: &PrRef) -> String {
        format!("pr-checks:{pr}")
    }
    pub fn branch_checks(repo: &RepoId) -> String {
        format!("branch-checks:{repo}")
    }
    pub fn users(list: &super::UserList) -> String {
        format!("users:{list:?}")
    }
    pub fn releases(repo: &RepoId) -> String {
        format!("releases:{repo}")
    }
    pub fn release(repo: &RepoId, tag: &str) -> String {
        format!("release:{repo}:{tag}")
    }
    pub fn tags(repo: &RepoId) -> String {
        format!("tags:{repo}")
    }
    pub fn branches(repo: &RepoId) -> String {
        format!("branches:{repo}")
    }
    pub fn advisories(repo: Option<&RepoId>) -> String {
        format!("advisories:{repo:?}")
    }
    pub fn advisory(repo: Option<&RepoId>, ghsa: &str) -> String {
        format!("advisory:{repo:?}:{ghsa}")
    }
    pub fn teams(org: &str) -> String {
        format!("teams:{org}")
    }
    pub fn team(org: &str, slug: &str) -> String {
        format!("team:{org}/{slug}")
    }
    pub fn gist(id: &str) -> String {
        format!("gist:{id}")
    }
    pub fn gists(login: &str) -> String {
        format!("gists:{login}")
    }
    pub fn blame(repo: &RepoId, rev: &str, path: &str) -> String {
        format!("blame:{repo}:{rev}:{path}")
    }
    pub fn compare(repo: &RepoId, spec: &str) -> String {
        format!("compare:{repo}:{spec}")
    }
    pub fn deployments(repo: &RepoId, environment: Option<&str>) -> String {
        format!("deployments:{repo}:{environment:?}")
    }
    pub fn milestones(repo: &RepoId, closed: bool) -> String {
        format!("milestones:{repo}:{closed}")
    }
    pub fn milestone(repo: &RepoId, number: u64) -> String {
        format!("milestone:{repo}#{number}")
    }
    pub fn owner_repos(login: &str, sort: super::RepoSort) -> String {
        format!("owner-repos:{}:{sort:?}", login.to_lowercase())
    }
    pub fn starred(login: &str) -> String {
        format!("starred:{}", login.to_lowercase())
    }
    pub fn forks(repo: &RepoId) -> String {
        format!("forks:{repo}")
    }
    pub fn history(repo: &RepoId, rev: &str, path: &str) -> String {
        format!("history:{repo}:{rev}:{path}")
    }
    pub const VISITS: &str = "visits";
    /// The tabs open when ghtui last quit.
    pub const TABS: &str = "tabs";
    pub const VIEWER_REPOS: &str = "viewer-repos";
}

// ---- conversions ---------------------------------------------------------------------

impl IssueState {
    /// A pull request's state: the only place one is made, so none forgets
    /// that an open draft is a draft.
    pub(crate) fn pr(state: PullRequestState, draft: bool) -> Self {
        match state {
            PullRequestState::Open if draft => Self::Draft,
            PullRequestState::Open => Self::Open,
            PullRequestState::Closed => Self::Closed,
            PullRequestState::Merged => Self::Merged,
        }
    }
}

fn issue_state(state: WireIssueState, reason: Option<StateReason>) -> IssueState {
    match (state, reason) {
        (WireIssueState::Open, _) => IssueState::Open,
        (WireIssueState::Closed, Some(StateReason::NotPlanned | StateReason::Duplicate)) => {
            IssueState::NotPlanned
        }
        (WireIssueState::Closed, _) => IssueState::Closed,
    }
}

impl WireCommit {
    pub(crate) fn into_detail(self) -> CommitDetail {
        let (headline, body) = self.message.split_once('\n').unwrap_or((&self.message, ""));
        let author = Person::from(self.author);
        let committer = Some(Person::from(self.committer)).filter(|c| {
            c.name() != author.name() && *c != Person::Unknown && c.login() != Some("web-flow")
        });
        CommitDetail {
            oid: self.oid.0,
            headline: headline.trim().to_owned(),
            body: body.trim().to_owned(),
            author,
            authored_at: self.authored_date.0,
            committer,
            committed_at: self.committed_date.0,
            parents: Capped::from_nodes(self.parents.total_count, self.parents.nodes, |p| p.oid.0),
            additions: count(self.additions),
            deletions: count(self.deletions),
            changed_files: self.changed_files_if_available.map(count),
            verified: self.signature.map(|s| s.is_valid),
        }
    }
}

impl CommitCard {
    fn into_info(self) -> CommitInfo {
        CommitInfo {
            author: self.author.into(),
            oid: self.oid.0,
            headline: self.message.lines().next().unwrap_or_default().to_owned(),
            date: self.committed_date.0,
        }
    }
}

impl RepoCard {
    pub fn into_summary(self) -> Option<RepoSummary> {
        Some(RepoSummary {
            repo: RepoId::parse(&self.name_with_owner)?,
            description: self.description.filter(|d| !d.trim().is_empty()),
            stars: count(self.stargazer_count),
            forks: count(self.fork_count),
            language: self.primary_language.as_ref().map(|l| l.name.clone()),
            language_color: self.primary_language.and_then(|l| l.color),
            pushed_at: self.pushed_at.map(|d| d.0),
            private: self.is_private,
            fork: self.is_fork,
            archived: self.is_archived,
        })
    }
}

pub(crate) fn entries(tree: Tree) -> Vec<TreeEntry> {
    let mut out: Vec<TreeEntry> = tree
        .entries
        .unwrap_or_default()
        .into_iter()
        .map(|e| {
            let kind = match (e.kind.as_str(), e.mode) {
                ("tree", _) => EntryKind::Dir,
                ("commit", _) => EntryKind::Submodule,
                (_, 0o120000) => EntryKind::Symlink,
                _ => EntryKind::File,
            };
            TreeEntry {
                path: e.path.unwrap_or_else(|| e.name.clone()),
                name: e.name,
                size: (kind == EntryKind::File).then(|| count(e.size)),
                kind,
            }
        })
        .collect();
    // Directories first, then by name (case-insensitively), like GitHub.
    out.sort_by_cached_key(|e| (e.kind != EntryKind::Dir, e.name.to_lowercase()));
    out
}

impl RepoFull {
    pub(crate) fn into_overview(self) -> Option<RepoOverview> {
        let (default_branch, head) = self.default_branch_ref.map(|r| (r.name, r.target)).unzip();
        let (last_commit, commits) = match head.flatten() {
            Some(RefTarget::Commit(c)) => {
                (Some(c.commit.into_info()), count(c.history.total_count))
            }
            _ => (None, 0),
        };
        let entries = match self.object {
            Some(GitObject::Tree(t)) => entries(t),
            _ => Vec::new(),
        };
        Some(RepoOverview {
            summary: self.card.into_summary()?,
            homepage: self.homepage_url.map(|u| u.0).filter(|u| !u.is_empty()),
            watchers: count(self.watchers.total_count),
            open_issues: count(self.issues.total_count),
            open_prs: count(self.pull_requests.total_count),
            closed_issues: count(self.closed_issues.total_count),
            closed_prs: count(self.closed_pull_requests.total_count),
            license: self
                .license_info
                .map(|l| l.spdx_id.filter(|s| s != "NOASSERTION").unwrap_or(l.name)),
            topics: Capped::from_nodes(
                self.repository_topics.total_count,
                self.repository_topics.nodes,
                |t| t.topic.name,
            ),
            default_branch,
            last_commit,
            commits,
            parent: self.parent.map(|p| p.name_with_owner),
            starred: self.viewer_has_starred,
            id: self.id.into(),
            has_issues: self.has_issues_enabled,
            has_discussions: self.has_discussions_enabled,
            has_wiki: self.has_wiki_enabled,
            branches: self.branch_count.map_or(0, |c| count(c.total_count)),
            tags: self.tag_count.map_or(0, |c| count(c.total_count)),
            entries,
        })
    }
}

impl BrowseItem {
    pub(crate) fn into_issue(self) -> Option<IssueSummary> {
        match self {
            BrowseItem::Issue(i) => Some(IssueSummary {
                repo: RepoId::parse(&i.repository.name_with_owner)?,
                number: count(i.number),
                title: i.title,
                is_pr: false,
                state: issue_state(i.state, i.state_reason),
                author: author(i.author),
                updated_at: i.updated_at.0,
                comments: count(i.comments.total_count),
                labels: labels(i.labels),
                created_at: i.created_at.0,
                review: None,
                checks: None,
            }),
            BrowseItem::PullRequest(p) => Some(IssueSummary {
                repo: RepoId::parse(&p.repository.name_with_owner)?,
                number: count(p.number),
                title: p.title,
                is_pr: true,
                state: IssueState::pr(p.state, p.is_draft),
                author: author(p.author),
                updated_at: p.updated_at.0,
                comments: count(p.comments.total_count),
                labels: labels(p.labels),
                created_at: p.created_at.0,
                review: crate::model::review(p.review_decision),
                checks: None,
            }),
            _ => None,
        }
    }

    pub(crate) fn into_user(self) -> Option<UserSummary> {
        match self {
            BrowseItem::User(u) => Some(UserSummary {
                login: u.login,
                name: u.name,
                bio: u.bio.filter(|b| !b.trim().is_empty()),
                is_org: false,
            }),
            BrowseItem::Organization(o) => Some(UserSummary {
                login: o.login,
                name: o.name,
                bio: o.description.filter(|b| !b.trim().is_empty()),
                is_org: true,
            }),
            _ => None,
        }
    }
}

impl IssueFull {
    pub(crate) fn into_detail(self) -> Option<IssueDetail> {
        Some(IssueDetail {
            repo: RepoId::parse(&self.repository.name_with_owner)?,
            number: count(self.number),
            title: self.title,
            body: self.body,
            state: issue_state(self.state, self.state_reason),
            author: author(self.author),
            created_at: self.created_at.0,
            labels: labels(self.labels),
            assignees: Capped::from_nodes(self.assignees.total_count, self.assignees.nodes, |u| {
                u.login
            }),
            milestone: self.milestone.map(MilestoneRef::from_wire),
            total_comments: count(self.comments.total_count),
            comments: comments(self.comments),
            id: self.id.into(),
        })
    }
}

fn comments(c: IssueComments) -> Vec<Comment> {
    nodes(c.nodes)
        .map(|c| Comment {
            id: c.full_database_id.and_then(|id| id.0.parse().ok()),
            author: author(c.author),
            body: c.body,
            created_at: c.created_at.0,
        })
        .collect()
}

impl WirePrActivity {
    pub(crate) fn into_activity(self) -> PrActivity {
        PrActivity {
            id: self.id.into(),
            total_comments: count(self.comments.total_count),
            total_reviews: self.reviews.as_ref().map_or(0, |r| count(r.total_count)),
            total_commits: count(self.commits.total_count),
            comments: comments(self.comments),
            reviews: nodes(self.reviews.and_then(|r| r.nodes))
                .filter(|r| r.state != ReviewState::Pending)
                .map(|r| ReviewSummary {
                    author: author(r.author),
                    state: match r.state {
                        ReviewState::Approved => "approved",
                        ReviewState::ChangesRequested => "requested changes",
                        ReviewState::Commented => "reviewed",
                        ReviewState::Dismissed => "review dismissed",
                        ReviewState::Pending => "pending",
                    }
                    .to_owned(),
                    body: r.body,
                    submitted_at: r.submitted_at.map(|d| d.0).unwrap_or_default(),
                })
                .collect(),
            commits: nodes(self.commits.nodes)
                .map(|n| n.commit.into_info())
                .collect(),
        }
    }
}

fn pinned(p: Pinned) -> Vec<RepoSummary> {
    nodes(p.nodes)
        .filter_map(|i| match i {
            PinnedItem::Repository(r) => r.into_summary(),
            PinnedItem::Other => None,
        })
        .collect()
}

pub(crate) fn repo_list(list: RepoList) -> (Vec<RepoSummary>, u64) {
    let total = count(list.total_count);
    let repos = nodes(list.nodes)
        .filter_map(RepoCard::into_summary)
        .collect();
    (repos, total)
}

fn readme_text(object: Option<ReadmeObject>) -> Option<String> {
    match object? {
        ReadmeObject::Blob(b) => b.text.filter(|t| !t.trim().is_empty()),
        ReadmeObject::Other => None,
    }
}

impl ProfileQuery {
    pub(crate) fn into_profile(self) -> Option<Profile> {
        if let Some(u) = self.user {
            let (repos, repo_count) = repo_list(u.repositories);
            let status = u.status.and_then(|s| {
                let text = [s.emoji, s.message]
                    .into_iter()
                    .flatten()
                    .filter(|t| !t.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                (!text.is_empty()).then_some(text)
            });
            return Some(Profile {
                readme: readme_text(self.user_readme.and_then(|r| r.object)),
                status,
                pronouns: u.pronouns.filter(|p| !p.trim().is_empty()),
                socials: nodes(u.social_accounts.nodes)
                    .map(|s| (s.display_name, s.url.0))
                    .collect(),
                orgs: Capped::from_nodes(u.organizations.total_count, u.organizations.nodes, |o| {
                    o.login
                }),
                verified: false,
                people: Vec::new(),
                people_count: 0,
                contributions: Some(u.contributions_collection.into_contributions()),
                login: u.login,
                name: u.name.filter(|n| !n.is_empty()),
                bio: u.bio.filter(|b| !b.trim().is_empty()),
                company: u.company.filter(|c| !c.is_empty()),
                location: u.location.filter(|l| !l.is_empty()),
                website: u.website_url.map(|w| w.0).filter(|w| !w.is_empty()),
                followers: Some(count(u.followers.total_count)),
                following: Some(count(u.following.total_count)),
                is_org: false,
                pinned: pinned(u.pinned_items),
                repos,
                repo_count,
                star_count: count(u.starred_repositories.total_count),
            });
        }
        let o = self.organization?;
        let (repos, repo_count) = repo_list(o.repositories);
        Some(Profile {
            login: o.login,
            name: o.name.filter(|n| !n.is_empty()),
            bio: o.description.filter(|b| !b.trim().is_empty()),
            company: None,
            location: o.location.filter(|l| !l.is_empty()),
            website: o.website_url.map(|w| w.0).filter(|w| !w.is_empty()),
            followers: None,
            following: None,
            is_org: true,
            pinned: pinned(o.pinned_items),
            repos,
            repo_count,
            star_count: 0,
            readme: readme_text(self.org_readme.and_then(|r| r.object)),
            status: None,
            pronouns: None,
            socials: Vec::new(),
            orgs: Vec::new().into(),
            verified: o.is_verified,
            people: nodes(o.members_with_role.nodes).map(|u| u.login).collect(),
            people_count: count(o.members_with_role.total_count),
            contributions: None,
        })
    }
}
