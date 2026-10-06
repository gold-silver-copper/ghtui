//! Browsing GitHub: repositories (overview, directories, files, README),
//! issues, profiles, search and pull request conversations.
//!
//! The wire types are typed against GitHub's schema; the models after them
//! are what the app renders and caches.

use serde::{Deserialize, Serialize};

use crate::model::{Label, NodeId, RepoId, author, count, labels};
use crate::queries::{
    Actor, AddCommentPayload, CommentCount, CommitCount, DateTime, FollowCount, FollowingCount,
    GitObjectId, IssueCount, LabelConnection, NumberVariablesFields, PageInfo, PrCount,
    PullRequestReviewDecision, PullRequestState, RepositoryName, ReviewState, StarPayload,
    StatusState, UnstarPayload, Uri, UserCount, fragments, nodes,
};
use ghtui_schema::schema;

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
    pub author: String,
    /// ISO 8601.
    pub authored_at: String,
    /// Who committed it, when that isn't the author (rebased, applied).
    pub committer: Option<String>,
    pub committed_at: String,
    pub parents: Vec<String>,
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
    pub author: String,
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
    pub topics: Vec<String>,
    pub default_branch: Option<String>,
    pub last_commit: Option<CommitInfo>,
    pub commits: u64,
    /// The repository this one was forked from.
    pub parent: Option<String>,
    pub starred: bool,
    /// Node ID, for starring.
    pub id: NodeId,
    pub has_issues: bool,
    /// Root directory at the default branch (empty for empty repos).
    pub entries: Vec<TreeEntry>,
    /// README text, if there is one.
    pub readme: Option<Readme>,
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
    pub labels: Vec<Label>,
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
    pub assets: Vec<Asset>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SearchKind {
    Repos,
    /// Issues and pull requests (unless the query says `is:issue` or `is:pr`).
    Issues,
    /// Pull requests only.
    Pulls,
    Users,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchResults {
    Repos(Results<RepoSummary>),
    Issues(Results<IssueSummary>),
    Users(Results<UserSummary>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
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
    pub labels: Vec<Label>,
    pub assignees: Vec<String>,
    pub comments: Vec<Comment>,
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
    pub comments: Vec<Comment>,
    pub reviews: Vec<ReviewSummary>,
    pub commits: Vec<CommitInfo>,
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
    pub stars: Vec<RepoSummary>,
    #[serde(default)]
    pub star_count: u64,
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

// Lists of nodes, and their nodes' types.
fragments! {
    nodes:
    Topics = "RepositoryTopicConnection" => RepoTopic,
    Assignees = "UserConnection" => UserLogin,
    IssueComments = "IssueCommentConnection" => WireComment,
    Reviews = "PullRequestReviewConnection" => WireReview,
    PrCommits = "PullRequestCommitConnection" => PrCommitNode,
    Pinned = "PinnableItemConnection" => PinnedItem,
    RefNames = "RefConnection" => RefName,
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
    #[arguments(first: 100)]
    pub comments: IssueComments,
    pub repository: RepositoryName,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "IssueComment", schema_module = "schema")]
pub struct WireComment {
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
    #[arguments(first: 100)]
    pub comments: IssueComments,
    #[arguments(first: 50)]
    pub reviews: Option<Reviews>,
    #[arguments(first: 100)]
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
    pub nodes: Option<Vec<Option<RollupContext>>>,
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
        let items = nodes(contexts.and_then(|c| c.nodes))
            .filter_map(|c| match c {
                RollupContext::CheckRun(run) => Some(run.into_item()),
                RollupContext::StatusContext(s) => Some(s.into_item()),
                RollupContext::Other => None,
            })
            .collect();
        Checks {
            oid: self.oid.0,
            items,
            total,
        }
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

// ---- profiles ------------------------------------------------------------------------

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
    #[arguments(first: 30, orderBy: { field: STARRED_AT, direction: DESC })]
    pub starred_repositories: StarList,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "StarredRepositoryConnection", schema_module = "schema")]
pub struct StarList {
    pub total_count: i32,
    pub nodes: Option<Vec<Option<RepoCard>>>,
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
            assets: Vec::new(),
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
            assets: nodes(self.release_assets.nodes)
                .map(|a| Asset {
                    name: a.name,
                    size: count(a.size),
                    downloads: count(a.download_count),
                    url: a.download_url.0,
                })
                .collect(),
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

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "BranchesVariables"
)]
pub struct BranchesQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<RepoBranches>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Repository", schema_module = "schema")]
pub struct RepoBranches {
    #[arguments(refPrefix: "refs/heads/", first: 100, orderBy: { field: TAG_COMMIT_DATE, direction: DESC })]
    pub refs: Option<RefNames>,
    #[cynic(rename = "refs", alias)]
    #[arguments(refPrefix: "refs/tags/", first: 50, orderBy: { field: TAG_COMMIT_DATE, direction: DESC })]
    pub tags: Option<RefNames>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Ref", schema_module = "schema")]
pub struct RefName {
    pub name: String,
}

/// A repository's branches and tags, newest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refs {
    pub branches: Vec<String>,
    pub tags: Vec<String>,
}

impl RepoBranches {
    pub(crate) fn into_refs(self) -> Refs {
        let names = |r: Option<RefNames>| -> Vec<String> {
            nodes(r.and_then(|r| r.nodes)).map(|n| n.name).collect()
        };
        Refs {
            branches: names(self.refs),
            tags: names(self.tags),
        }
    }
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
    pub fn forks(repo: &RepoId) -> String {
        format!("forks:{repo}")
    }
    pub fn history(repo: &RepoId, rev: &str, path: &str) -> String {
        format!("history:{repo}:{rev}:{path}")
    }
    pub const VISITS: &str = "visits";
    pub const VIEWER_REPOS: &str = "viewer-repos";
}

// ---- conversions ---------------------------------------------------------------------

fn issue_state(state: WireIssueState, reason: Option<StateReason>) -> IssueState {
    match (state, reason) {
        (WireIssueState::Open, _) => IssueState::Open,
        (WireIssueState::Closed, Some(StateReason::NotPlanned | StateReason::Duplicate)) => {
            IssueState::NotPlanned
        }
        (WireIssueState::Closed, _) => IssueState::Closed,
    }
}

fn actor(a: Option<GitActor>) -> Option<String> {
    a.and_then(|a| a.user.map(|u| u.login).or(a.name))
}

impl WireCommit {
    pub(crate) fn into_detail(self) -> CommitDetail {
        let (headline, body) = self.message.split_once('\n').unwrap_or((&self.message, ""));
        let author = actor(self.author).unwrap_or_else(|| "unknown".into());
        let committer = actor(self.committer).filter(|c| *c != author && c != "web-flow");
        CommitDetail {
            oid: self.oid.0,
            headline: headline.trim().to_owned(),
            body: body.trim().to_owned(),
            author,
            authored_at: self.authored_date.0,
            committer,
            committed_at: self.committed_date.0,
            parents: nodes(self.parents.nodes).map(|p| p.oid.0).collect(),
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
            author: self
                .author
                .and_then(|a| a.user.map(|u| u.login).or(a.name))
                .unwrap_or_else(|| "unknown".into()),
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
    pub(crate) fn into_overview(self, readme: Option<Readme>) -> Option<RepoOverview> {
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
            topics: nodes(self.repository_topics.nodes)
                .map(|t| t.topic.name)
                .collect(),
            default_branch,
            last_commit,
            commits,
            parent: self.parent.map(|p| p.name_with_owner),
            starred: self.viewer_has_starred,
            id: self.id.into(),
            has_issues: self.has_issues_enabled,
            entries,
            readme,
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
                state: match p.state {
                    PullRequestState::Open if p.is_draft => IssueState::Draft,
                    PullRequestState::Open => IssueState::Open,
                    PullRequestState::Closed => IssueState::Closed,
                    PullRequestState::Merged => IssueState::Merged,
                },
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
            assignees: nodes(self.assignees.nodes).map(|u| u.login).collect(),
            comments: comments(self.comments),
            id: self.id.into(),
        })
    }
}

fn comments(c: IssueComments) -> Vec<Comment> {
    nodes(c.nodes)
        .map(|c| Comment {
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

impl ProfileQuery {
    pub(crate) fn into_profile(self) -> Option<Profile> {
        if let Some(u) = self.user {
            let (repos, repo_count) = repo_list(u.repositories);
            return Some(Profile {
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
                stars: nodes(u.starred_repositories.nodes)
                    .filter_map(RepoCard::into_summary)
                    .collect(),
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
            stars: Vec::new(),
            star_count: 0,
        })
    }
}
