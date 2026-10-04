//! Browsing GitHub: repositories (overview, directories, files, README),
//! issues, profiles, search and pull request conversations.
//!
//! The wire types are typed against GitHub's schema; the models after them
//! are what the app renders and caches.

use serde::{Deserialize, Serialize};

use crate::model::{Label, RepoId};
use crate::queries::{Actor, DateTime, GitObjectId, PageInfo, Uri};
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
    pub id: String,
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
    pub id: String,
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
    pub id: String,
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

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "LabelConnection", schema_module = "schema")]
pub struct Labels {
    pub nodes: Option<Vec<Option<crate::queries::Label>>>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "IssueCommentConnection", schema_module = "schema")]
pub struct CommentCount {
    pub total_count: i32,
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
    pub name_with_owner: String,
    pub description: Option<String>,
    pub stargazer_count: i32,
    pub fork_count: i32,
    pub primary_language: Option<Language>,
    pub pushed_at: Option<DateTime>,
    pub is_private: bool,
    pub is_fork: bool,
    pub is_archived: bool,
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
    pub parent: Option<RepoName>,
    pub viewer_has_starred: bool,
    pub has_issues_enabled: bool,
    #[arguments(expression: $expression)]
    pub object: Option<GitObject>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "UserConnection", schema_module = "schema")]
pub struct UserCount {
    pub total_count: i32,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "IssueConnection", schema_module = "schema")]
pub struct IssueCount {
    pub total_count: i32,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestConnection", schema_module = "schema")]
pub struct PrCount {
    pub total_count: i32,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "License", schema_module = "schema")]
pub struct License {
    pub name: String,
    pub spdx_id: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RepositoryTopicConnection", schema_module = "schema")]
pub struct Topics {
    pub nodes: Option<Vec<Option<RepoTopic>>>,
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
#[cynic(graphql_type = "Repository", schema_module = "schema")]
pub struct RepoName {
    pub name_with_owner: String,
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
    pub oid: GitObjectId,
    pub message_headline: String,
    pub committed_date: DateTime,
    pub author: Option<GitActor>,
    #[arguments(first: 0)]
    pub history: CommitCount,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "CommitHistoryConnection", schema_module = "schema")]
pub struct CommitCount {
    pub total_count: i32,
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
    pub labels: Option<Labels>,
    pub repository: RepoName,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequest", schema_module = "schema")]
pub struct PrCard {
    pub number: i32,
    pub title: String,
    pub state: crate::queries::PullRequestState,
    pub is_draft: bool,
    pub author: Option<Actor>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
    pub review_decision: Option<crate::queries::PullRequestReviewDecision>,
    pub comments: CommentCount,
    #[arguments(first: 6)]
    pub labels: Option<Labels>,
    pub repository: RepoName,
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

#[derive(cynic::QueryVariables, Debug)]
pub struct IssueVariables {
    pub owner: String,
    pub name: String,
    pub number: i32,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "IssueVariables"
)]
pub struct IssueQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<RepoIssue>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Repository",
    schema_module = "schema",
    variables = "IssueVariables"
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
    pub labels: Option<Labels>,
    #[arguments(first: 10)]
    pub assignees: Assignees,
    #[arguments(first: 100)]
    pub comments: IssueComments,
    pub repository: RepoName,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "UserConnection", schema_module = "schema")]
pub struct Assignees {
    pub nodes: Option<Vec<Option<UserLogin>>>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "IssueCommentConnection", schema_module = "schema")]
pub struct IssueComments {
    pub nodes: Option<Vec<Option<WireComment>>>,
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
    variables = "IssueVariables"
)]
pub struct PrActivityQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<RepoPrActivity>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Repository",
    schema_module = "schema",
    variables = "IssueVariables"
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
#[cynic(graphql_type = "PullRequestReviewConnection", schema_module = "schema")]
pub struct Reviews {
    pub nodes: Option<Vec<Option<WireReview>>>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestReview", schema_module = "schema")]
pub struct WireReview {
    pub author: Option<Actor>,
    pub state: crate::queries::ReviewState,
    pub body: String,
    pub submitted_at: Option<DateTime>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestCommitConnection", schema_module = "schema")]
pub struct PrCommits {
    pub nodes: Option<Vec<Option<PrCommitNode>>>,
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
    pub message_headline: String,
    pub committed_date: DateTime,
    pub author: Option<GitActor>,
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

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "FollowerConnection", schema_module = "schema")]
pub struct FollowCount {
    pub total_count: i32,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "FollowingConnection", schema_module = "schema")]
pub struct FollowingCount {
    pub total_count: i32,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PinnableItemConnection", schema_module = "schema")]
pub struct Pinned {
    pub nodes: Option<Vec<Option<PinnedItem>>>,
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
#[cynic(graphql_type = "RefConnection", schema_module = "schema")]
pub struct RefNames {
    pub nodes: Option<Vec<Option<RefName>>>,
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
            r.and_then(|r| r.nodes)
                .into_iter()
                .flatten()
                .flatten()
                .map(|n| n.name)
                .collect()
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

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "AddCommentPayload", schema_module = "schema")]
pub struct AddCommentPayload {
    pub client_mutation_id: Option<String>,
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

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "AddStarPayload", schema_module = "schema")]
pub struct StarPayload {
    pub client_mutation_id: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RemoveStarPayload", schema_module = "schema")]
pub struct UnstarPayload {
    pub client_mutation_id: Option<String>,
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
    pub fn files(repo: &RepoId, rev: &str) -> String {
        format!("files:{repo}:{rev}")
    }
    pub fn last_commits(repo: &RepoId, rev: &str, path: &str) -> String {
        format!("last-commits:{repo}:{rev}:{path}")
    }
    pub fn refs(repo: &RepoId) -> String {
        format!("refs:{repo}")
    }
    pub const VISITS: &str = "visits";
    pub const VIEWER_REPOS: &str = "viewer-repos";
}

// ---- conversions ---------------------------------------------------------------------

fn count(n: i32) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

fn login(actor: &Option<Actor>) -> String {
    actor
        .as_ref()
        .map_or_else(|| "ghost".to_owned(), |a| a.login.clone())
}

fn labels(l: Option<Labels>) -> Vec<Label> {
    l.and_then(|l| l.nodes)
        .into_iter()
        .flatten()
        .flatten()
        .map(|l| Label {
            name: l.name,
            color: l.color,
        })
        .collect()
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

fn git_author(a: &Option<GitActor>) -> String {
    a.as_ref()
        .and_then(|a| {
            a.user
                .as_ref()
                .map(|u| u.login.clone())
                .or_else(|| a.name.clone())
        })
        .unwrap_or_else(|| "unknown".into())
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
    out.sort_by(|a, b| {
        (a.kind != EntryKind::Dir)
            .cmp(&(b.kind != EntryKind::Dir))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

impl RepoFull {
    pub(crate) fn into_overview(self, readme: Option<Readme>) -> Option<RepoOverview> {
        let (default_branch, last_commit, commits) = match self.default_branch_ref {
            Some(r) => match r.target {
                Some(RefTarget::Commit(c)) => (
                    Some(r.name),
                    Some(CommitInfo {
                        author: git_author(&c.author),
                        oid: c.oid.0,
                        headline: c.message_headline,
                        date: c.committed_date.0,
                    }),
                    count(c.history.total_count),
                ),
                _ => (Some(r.name), None, 0),
            },
            None => (None, None, 0),
        };
        let entries = match self.object {
            Some(GitObject::Tree(t)) => entries(t),
            _ => Vec::new(),
        };
        Some(RepoOverview {
            summary: RepoCard {
                name_with_owner: self.name_with_owner,
                description: self.description,
                stargazer_count: self.stargazer_count,
                fork_count: self.fork_count,
                primary_language: self.primary_language,
                pushed_at: self.pushed_at,
                is_private: self.is_private,
                is_fork: self.is_fork,
                is_archived: self.is_archived,
            }
            .into_summary()?,
            homepage: self.homepage_url.map(|u| u.0).filter(|u| !u.is_empty()),
            watchers: count(self.watchers.total_count),
            open_issues: count(self.issues.total_count),
            open_prs: count(self.pull_requests.total_count),
            closed_issues: count(self.closed_issues.total_count),
            closed_prs: count(self.closed_pull_requests.total_count),
            license: self
                .license_info
                .map(|l| l.spdx_id.filter(|s| s != "NOASSERTION").unwrap_or(l.name)),
            topics: self
                .repository_topics
                .nodes
                .into_iter()
                .flatten()
                .flatten()
                .map(|t| t.topic.name)
                .collect(),
            default_branch,
            last_commit,
            commits,
            parent: self.parent.map(|p| p.name_with_owner),
            starred: self.viewer_has_starred,
            id: self.id.into_inner(),
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
                author: login(&i.author),
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
                    crate::queries::PullRequestState::Open if p.is_draft => IssueState::Draft,
                    crate::queries::PullRequestState::Open => IssueState::Open,
                    crate::queries::PullRequestState::Closed => IssueState::Closed,
                    crate::queries::PullRequestState::Merged => IssueState::Merged,
                },
                author: login(&p.author),
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
            author: login(&self.author),
            created_at: self.created_at.0,
            labels: labels(self.labels),
            assignees: self
                .assignees
                .nodes
                .into_iter()
                .flatten()
                .flatten()
                .map(|u| u.login)
                .collect(),
            comments: comments(self.comments),
            id: self.id.into_inner(),
        })
    }
}

fn comments(c: IssueComments) -> Vec<Comment> {
    c.nodes
        .into_iter()
        .flatten()
        .flatten()
        .map(|c| Comment {
            author: login(&c.author),
            body: c.body,
            created_at: c.created_at.0,
        })
        .collect()
}

impl WirePrActivity {
    pub(crate) fn into_activity(self) -> PrActivity {
        PrActivity {
            id: self.id.into_inner(),
            comments: comments(self.comments),
            reviews: self
                .reviews
                .and_then(|r| r.nodes)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|r| r.state != crate::queries::ReviewState::Pending)
                .map(|r| ReviewSummary {
                    author: login(&r.author),
                    state: match r.state {
                        crate::queries::ReviewState::Approved => "approved",
                        crate::queries::ReviewState::ChangesRequested => "requested changes",
                        crate::queries::ReviewState::Commented => "reviewed",
                        crate::queries::ReviewState::Dismissed => "review dismissed",
                        crate::queries::ReviewState::Pending => "pending",
                    }
                    .to_owned(),
                    body: r.body,
                    submitted_at: r.submitted_at.map(|d| d.0).unwrap_or_default(),
                })
                .collect(),
            commits: self
                .commits
                .nodes
                .into_iter()
                .flatten()
                .flatten()
                .map(|n| CommitInfo {
                    author: git_author(&n.commit.author),
                    oid: n.commit.oid.0,
                    headline: n.commit.message_headline,
                    date: n.commit.committed_date.0,
                })
                .collect(),
        }
    }
}

fn pinned(p: Pinned) -> Vec<RepoSummary> {
    p.nodes
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|i| match i {
            PinnedItem::Repository(r) => r.into_summary(),
            PinnedItem::Other => None,
        })
        .collect()
}

pub(crate) fn repo_list(list: RepoList) -> (Vec<RepoSummary>, u64) {
    let total = count(list.total_count);
    let repos = list
        .nodes
        .into_iter()
        .flatten()
        .flatten()
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
                stars: u
                    .starred_repositories
                    .nodes
                    .into_iter()
                    .flatten()
                    .flatten()
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
