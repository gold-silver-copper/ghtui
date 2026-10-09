//! Typed GraphQL operations, validated against GitHub's schema at compile
//! time. These mirror the wire format; [`crate::model`] holds what the rest of
//! the app uses.

use ghtui_schema::schema;

use crate::model::{MergeState, Mergeable, ReviewDecision, ReviewState, Side, ViewedState};

#[derive(cynic::Scalar, Debug, Clone)]
#[cynic(graphql_type = "DateTime", schema_module = "schema")]
pub struct DateTime(pub String);

#[derive(cynic::Scalar, Debug, Clone)]
#[cynic(graphql_type = "URI", schema_module = "schema")]
pub struct Uri(pub String);

#[derive(cynic::Scalar, Debug, Clone)]
#[cynic(graphql_type = "GitObjectID", schema_module = "schema")]
pub struct GitObjectId(pub String);

/// A number too big for GraphQL's `Int`, as GitHub writes it: a string.
#[derive(cynic::Scalar, Debug, Clone)]
#[cynic(graphql_type = "BigInt", schema_module = "schema")]
pub struct BigInt(pub String);

/// The non-null nodes of a GraphQL list.
pub(crate) fn nodes<T>(list: Option<Vec<Option<T>>>) -> impl Iterator<Item = T> {
    list.into_iter().flatten().flatten()
}

/// Fragments with the same single field on several GraphQL types. Field
/// types are written out in the arms: cynic's derive fails on a type passed
/// in whole.
macro_rules! fragments {
    ($field:ident: Option<$ty:ident>; $($name:ident = $graphql:literal),* $(,)?) => {$(
        #[derive(cynic::QueryFragment, Debug)]
        #[cynic(graphql_type = $graphql, schema_module = "schema")]
        pub struct $name {
            pub $field: Option<$ty>,
        }
    )*};
    (total_count: i32; $($name:ident = $graphql:literal),* $(,)?) => {$(
        #[derive(cynic::QueryFragment, Debug)]
        #[cynic(graphql_type = $graphql, schema_module = "schema")]
        pub struct $name {
            pub total_count: i32,
        }
    )*};
    (id: cynic::Id; $($name:ident = $graphql:literal),* $(,)?) => {$(
        #[derive(cynic::QueryFragment, Debug)]
        #[cynic(graphql_type = $graphql, schema_module = "schema")]
        pub struct $name {
            pub id: cynic::Id,
        }
    )*};
    // A page of nodes and how many there are in all.
    (counted: $($name:ident = $graphql:literal => $ty:ident),* $(,)?) => {$(
        #[derive(cynic::QueryFragment, Debug)]
        #[cynic(graphql_type = $graphql, schema_module = "schema")]
        pub struct $name {
            pub total_count: i32,
            pub nodes: Option<Vec<Option<$ty>>>,
        }
    )*};
    (nodes: $($name:ident = $graphql:literal => $ty:ident),* $(,)?) => {$(
        #[derive(cynic::QueryFragment, Debug)]
        #[cynic(graphql_type = $graphql, schema_module = "schema")]
        pub struct $name {
            pub nodes: Option<Vec<Option<$ty>>>,
        }
    )*};
}
pub(crate) use fragments;

// Connection sizes.
fragments! {
    total_count: i32;
    CommentCount = "IssueCommentConnection",
    UserCount = "UserConnection",
    IssueCount = "IssueConnection",
    PrCount = "PullRequestConnection",
    CommitCount = "CommitHistoryConnection",
    ReviewCommentCount = "PullRequestReviewCommentConnection",
    FollowCount = "FollowerConnection",
    FollowingCount = "FollowingConnection",
    RefCount = "RefConnection",
}

// Results of mutations that only need to succeed.
fragments! {
    client_mutation_id: Option<String>;
    MarkFileAsViewedPayload = "MarkFileAsViewedPayload",
    UnmarkFileAsViewedPayload = "UnmarkFileAsViewedPayload",
    AddCommentPayload = "AddCommentPayload",
    StarPayload = "AddStarPayload",
    UnstarPayload = "RemoveStarPayload",
    MergePayload = "MergePullRequestPayload",
    ClosePrPayload = "ClosePullRequestPayload",
    ReopenPrPayload = "ReopenPullRequestPayload",
    CloseIssuePayload = "CloseIssuePayload",
    ReopenIssuePayload = "ReopenIssuePayload",
    ReadyPayload = "MarkPullRequestReadyForReviewPayload",
    UpdateBranchPayload = "UpdatePullRequestBranchPayload",
}

// Node IDs, and the mutation results that carry them.
fragments! {
    id: cynic::Id;
    ReviewId = "PullRequestReview",
    ThreadId = "PullRequestReviewThread",
    CommentId = "PullRequestReviewComment",
}
fragments! {
    pull_request_review: Option<ReviewId>;
    StartReviewPayload = "AddPullRequestReviewPayload",
    SubmitReviewPayload = "SubmitPullRequestReviewPayload",
}
fragments! {
    thread: Option<ThreadId>;
    ThreadPayload = "AddPullRequestReviewThreadPayload",
    ResolvePayload = "ResolveReviewThreadPayload",
    UnresolvePayload = "UnresolveReviewThreadPayload",
}

// Lists of nodes and their totals, and their nodes' types.
fragments! {
    counted:
    LabelConnection = "LabelConnection" => Label,
    ReviewCommentConnection = "PullRequestReviewCommentConnection" => ReviewComment,
}

// Lists of nodes, and their nodes' types.
fragments! {
    nodes:
    CommitRollupConnection = "PullRequestCommitConnection" => CommitRollupNode,
    PendingReviewConnection = "PullRequestReviewConnection" => PendingReview,
    ReviewCommitConnection = "PullRequestReviewConnection" => ReviewCommit,
}

/// An issue or pull request by number.
#[derive(cynic::QueryVariables, Debug)]
pub struct NumberVariables {
    pub owner: String,
    pub name: String,
    pub number: i32,
}

/// A page of a pull request's files or threads.
#[derive(cynic::QueryVariables, Debug)]
pub struct PageVariables {
    pub owner: String,
    pub name: String,
    pub number: i32,
    pub after: Option<String>,
}

// ---- checks on the pull requests a search found ------------------------------

/// Pull requests by ID, for their checks: a search finds them, and this
/// reads their checks apart, so neither request runs into GitHub's
/// 10-second limit (see `GitHub::search`).
#[derive(cynic::QueryVariables, Debug)]
pub struct NodesVariables {
    pub ids: Vec<cynic::Id>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "NodesVariables"
)]
pub struct RollupsQuery {
    #[arguments(ids: $ids)]
    pub nodes: Vec<Option<RollupNode>>,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "Node", schema_module = "schema")]
pub enum RollupNode {
    PullRequest(PrRollup),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequest", schema_module = "schema")]
pub struct PrRollup {
    pub id: cynic::Id,
    #[arguments(last: 1)]
    pub commits: CommitRollupConnection,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequest", schema_module = "schema")]
pub struct PrSummary {
    pub number: i32,
    pub title: String,
    pub url: Uri,
    #[cynic(spread)]
    pub status: crate::model::PrStatus,
    pub updated_at: DateTime,
    pub author: Option<Actor>,
    pub repository: RepositoryName,
    pub additions: i32,
    pub deletions: i32,
    pub review_decision: Option<ReviewDecision>,
    #[arguments(last: 1)]
    pub commits: CommitRollupConnection,
    pub comments: CommentCount,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Actor", schema_module = "schema")]
pub struct Actor {
    pub login: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Repository", schema_module = "schema")]
pub struct RepositoryName {
    pub name_with_owner: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Milestone", schema_module = "schema")]
pub struct MilestoneName {
    pub number: i32,
    pub title: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestCommit", schema_module = "schema")]
pub struct CommitRollupNode {
    pub commit: CommitRollup,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Commit", schema_module = "schema")]
pub struct CommitRollup {
    pub status_check_rollup: Option<StatusCheckRollup>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "StatusCheckRollup", schema_module = "schema")]
pub struct StatusCheckRollup {
    pub state: StatusState,
}

#[derive(cynic::Enum, Debug, Clone, Copy)]
#[cynic(graphql_type = "PullRequestState", schema_module = "schema")]
pub enum PullRequestState {
    Open,
    Closed,
    Merged,
}

#[derive(cynic::Enum, Debug, Clone, Copy)]
#[cynic(graphql_type = "StatusState", schema_module = "schema")]
pub enum StatusState {
    Error,
    Expected,
    Failure,
    Pending,
    Success,
}

// ---- one pull request ------------------------------------------------------

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "NumberVariables"
)]
pub struct PullRequestQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<RepositoryWithPr>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Repository",
    schema_module = "schema",
    variables = "NumberVariables"
)]
pub struct RepositoryWithPr {
    #[arguments(number: $number)]
    pub pull_request: Option<PrDetail>,
    pub merge_commit_allowed: bool,
    pub squash_merge_allowed: bool,
    pub rebase_merge_allowed: bool,
    pub viewer_default_merge_method: crate::change::MergeMethod,
    pub viewer_permission: Option<RepositoryPermission>,
}

/// What you may do in a repository.
#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq)]
#[cynic(graphql_type = "RepositoryPermission", schema_module = "schema")]
pub enum RepositoryPermission {
    Admin,
    Maintain,
    Write,
    TriagePlus,
    Triage,
    Read,
}

impl RepositoryPermission {
    /// It may push, merge, and re-run or cancel workflows.
    pub fn writes(permission: Option<Self>) -> bool {
        matches!(permission, Some(Self::Admin | Self::Maintain | Self::Write))
    }
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequest", schema_module = "schema")]
pub struct PrDetail {
    pub id: cynic::Id,
    #[cynic(spread)]
    pub summary: PrSummary,
    pub body: String,
    pub created_at: DateTime,
    pub base_ref_name: String,
    pub head_ref_name: String,
    pub base_ref_oid: GitObjectId,
    pub head_ref_oid: GitObjectId,
    pub head_repository: Option<RepositoryName>,
    pub changed_files: i32,
    pub mergeable: Mergeable,
    pub merge_state_status: MergeState,
    pub viewer_can_close: bool,
    pub viewer_can_reopen: bool,
    pub viewer_can_update: bool,
    pub viewer_can_merge_as_admin: bool,
    pub viewer_did_author: bool,
    pub maintainer_can_modify: bool,
    pub is_cross_repository: bool,
    #[arguments(first: 20)]
    pub labels: Option<LabelConnection>,
    pub milestone: Option<MilestoneName>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Label", schema_module = "schema")]
pub struct Label {
    pub name: String,
    pub color: String,
}

/// How far a branch is behind another.
#[derive(cynic::QueryVariables, Debug)]
pub struct BehindVariables {
    pub owner: String,
    pub name: String,
    /// `refs/heads/main`.
    pub base: String,
    /// A commit or a branch (`owner:branch` in a fork).
    pub head: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "BehindVariables"
)]
pub struct BehindQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<RepositoryBehind>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Repository",
    schema_module = "schema",
    variables = "BehindVariables"
)]
pub struct RepositoryBehind {
    #[arguments(qualifiedName: $base)]
    #[cynic(rename = "ref")]
    pub base: Option<RefCompare>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Ref",
    schema_module = "schema",
    variables = "BehindVariables"
)]
pub struct RefCompare {
    #[arguments(headRef: $head)]
    pub compare: Option<Behind>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Comparison", schema_module = "schema")]
pub struct Behind {
    pub behind_by: i32,
}

// ---- viewed files ------------------------------------------------------------

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "PageVariables"
)]
pub struct PrFilesQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<RepositoryWithPrFiles>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Repository",
    schema_module = "schema",
    variables = "PageVariables"
)]
pub struct RepositoryWithPrFiles {
    #[arguments(number: $number)]
    pub pull_request: Option<PrFiles>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "PullRequest",
    schema_module = "schema",
    variables = "PageVariables"
)]
pub struct PrFiles {
    pub id: cynic::Id,
    #[arguments(first: 100, after: $after)]
    pub files: Option<PrChangedFiles>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "PullRequestChangedFileConnection",
    schema_module = "schema"
)]
pub struct PrChangedFiles {
    pub page_info: PageInfo,
    pub nodes: Option<Vec<Option<PrChangedFile>>>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PageInfo", schema_module = "schema")]
pub struct PageInfo {
    pub has_next_page: bool,
    pub end_cursor: Option<String>,
}

impl PageInfo {
    /// The cursor for the next page, if there is one.
    pub fn next(self) -> Option<String> {
        self.end_cursor.filter(|_| self.has_next_page)
    }
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestChangedFile", schema_module = "schema")]
pub struct PrChangedFile {
    pub path: String,
    pub viewer_viewed_state: ViewedState,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct ViewedVariables {
    pub pull_request_id: cynic::Id,
    pub path: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Mutation",
    schema_module = "schema",
    variables = "ViewedVariables"
)]
pub struct MarkFileAsViewed {
    #[arguments(input: { pullRequestId: $pull_request_id, path: $path })]
    pub mark_file_as_viewed: Option<MarkFileAsViewedPayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Mutation",
    schema_module = "schema",
    variables = "ViewedVariables"
)]
pub struct UnmarkFileAsViewed {
    #[arguments(input: { pullRequestId: $pull_request_id, path: $path })]
    pub unmark_file_as_viewed: Option<UnmarkFileAsViewedPayload>,
}

// ---- review threads ----------------------------------------------------------

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "PageVariables"
)]
pub struct ThreadsQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<RepositoryWithThreads>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Repository",
    schema_module = "schema",
    variables = "PageVariables"
)]
pub struct RepositoryWithThreads {
    #[arguments(number: $number)]
    pub pull_request: Option<PrThreads>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "PullRequest",
    schema_module = "schema",
    variables = "PageVariables"
)]
pub struct PrThreads {
    #[arguments(first: 100, after: $after)]
    pub review_threads: ThreadConnection,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "PullRequestReviewThreadConnection",
    schema_module = "schema"
)]
pub struct ThreadConnection {
    pub page_info: PageInfo,
    pub nodes: Option<Vec<Option<ReviewThread>>>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestReviewThread", schema_module = "schema")]
pub struct ReviewThread {
    pub id: cynic::Id,
    pub path: String,
    pub diff_side: Side,
    pub start_diff_side: Option<Side>,
    pub line: Option<i32>,
    pub start_line: Option<i32>,
    pub original_line: Option<i32>,
    pub original_start_line: Option<i32>,
    pub is_outdated: bool,
    pub is_resolved: bool,
    pub subject_type: ThreadSubjectType,
    pub viewer_can_reply: bool,
    pub viewer_can_resolve: bool,
    pub viewer_can_unresolve: bool,
    /// The first comment: it's on the line the thread is about.
    #[arguments(first: 1)]
    #[cynic(rename = "comments", alias)]
    pub first_comment: ReviewCommentConnection,
    /// The newest replies, oldest first; with how many there are in all.
    #[arguments(last: 99)]
    pub comments: ReviewCommentConnection,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestReviewComment", schema_module = "schema")]
pub struct ReviewComment {
    pub id: cynic::Id,
    pub author: Option<Actor>,
    pub body: String,
    pub created_at: DateTime,
    pub url: Uri,
    pub original_commit: Option<CommitOid>,
    pub state: ReviewCommentState,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Commit", schema_module = "schema")]
pub struct CommitOid {
    pub oid: GitObjectId,
}

/// Your pending review, and the commit it's on.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestReview", schema_module = "schema")]
pub struct PendingReview {
    pub id: cynic::Id,
    pub commit: Option<CommitOid>,
    /// Comments in it, wherever they were written.
    pub comments: ReviewCommentCount,
}

#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq)]
#[cynic(
    graphql_type = "PullRequestReviewCommentState",
    schema_module = "schema"
)]
pub enum ReviewCommentState {
    Pending,
    Submitted,
}

#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq)]
#[cynic(
    graphql_type = "PullRequestReviewThreadSubjectType",
    schema_module = "schema"
)]
pub enum ThreadSubjectType {
    File,
    Line,
}

// ---- pending review ---------------------------------------------------------

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "NumberVariables"
)]
pub struct PendingReviewQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<RepositoryWithPendingReview>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Repository",
    schema_module = "schema",
    variables = "NumberVariables"
)]
pub struct RepositoryWithPendingReview {
    #[arguments(number: $number)]
    pub pull_request: Option<PrPendingReview>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequest", schema_module = "schema")]
pub struct PrPendingReview {
    pub id: cynic::Id,
    /// Only the viewer's own pending review is visible.
    #[arguments(states: [PENDING], first: 1)]
    pub reviews: Option<PendingReviewConnection>,
}

// ---- review mutations --------------------------------------------------------

#[derive(cynic::QueryVariables, Debug)]
pub struct StartReviewVariables {
    pub pull_request_id: cynic::Id,
    pub commit: GitObjectId,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Mutation",
    schema_module = "schema",
    variables = "StartReviewVariables"
)]
pub struct StartReview {
    #[arguments(input: { pullRequestId: $pull_request_id, commitOID: $commit })]
    pub add_pull_request_review: Option<StartReviewPayload>,
}

#[derive(cynic::InputObject, Debug)]
#[cynic(
    graphql_type = "AddPullRequestReviewThreadInput",
    schema_module = "schema",
    rename_all = "camelCase"
)]
pub struct AddThreadInput {
    pub pull_request_review_id: Option<cynic::Id>,
    pub path: Option<String>,
    pub body: String,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub line: Option<i32>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub side: Option<Side>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<i32>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub start_side: Option<Side>,
    pub subject_type: Option<ThreadSubjectType>,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct AddThreadVariables {
    pub input: AddThreadInput,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Mutation",
    schema_module = "schema",
    variables = "AddThreadVariables"
)]
pub struct AddThread {
    #[arguments(input: $input)]
    pub add_pull_request_review_thread: Option<ThreadPayload>,
}

#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq)]
#[cynic(graphql_type = "PullRequestReviewEvent", schema_module = "schema")]
pub enum ReviewEvent {
    Approve,
    Comment,
    Dismiss,
    RequestChanges,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct SubmitReviewVariables {
    pub review_id: cynic::Id,
    pub event: ReviewEvent,
    pub body: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Mutation",
    schema_module = "schema",
    variables = "SubmitReviewVariables"
)]
pub struct SubmitReview {
    #[arguments(input: { pullRequestReviewId: $review_id, event: $event, body: $body })]
    pub submit_pull_request_review: Option<SubmitReviewPayload>,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct ReplyVariables {
    pub thread_id: cynic::Id,
    pub body: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Mutation",
    schema_module = "schema",
    variables = "ReplyVariables"
)]
pub struct Reply {
    #[arguments(input: { pullRequestReviewThreadId: $thread_id, body: $body })]
    pub add_pull_request_review_thread_reply: Option<ReplyPayload>,
}

fragments! {
    comment: Option<CommentId>;
    ReplyPayload = "AddPullRequestReviewThreadReplyPayload",
}

#[derive(cynic::QueryVariables, Debug)]
pub struct ThreadIdVariables {
    pub thread_id: cynic::Id,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Mutation",
    schema_module = "schema",
    variables = "ThreadIdVariables"
)]
pub struct Resolve {
    #[arguments(input: { threadId: $thread_id })]
    pub resolve_review_thread: Option<ResolvePayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Mutation",
    schema_module = "schema",
    variables = "ThreadIdVariables"
)]
pub struct Unresolve {
    #[arguments(input: { threadId: $thread_id })]
    pub unresolve_review_thread: Option<UnresolvePayload>,
}

// ---- the viewer's last review --------------------------------------------------

#[derive(cynic::QueryVariables, Debug)]
pub struct LastReviewVariables {
    pub owner: String,
    pub name: String,
    pub number: i32,
    pub login: String,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "LastReviewVariables"
)]
pub struct LastReviewQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<RepositoryWithReviews>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Repository",
    schema_module = "schema",
    variables = "LastReviewVariables"
)]
pub struct RepositoryWithReviews {
    #[arguments(number: $number)]
    pub pull_request: Option<PrReviews>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "PullRequest",
    schema_module = "schema",
    variables = "LastReviewVariables"
)]
pub struct PrReviews {
    #[arguments(author: $login, last: 20)]
    pub reviews: Option<ReviewCommitConnection>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestReview", schema_module = "schema")]
pub struct ReviewCommit {
    pub state: ReviewState,
    pub commit: Option<CommitOid>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use cynic::{MutationBuilder, QueryBuilder};

    #[test]
    fn viewed_mutations_shape() {
        let op = MarkFileAsViewed::build(ViewedVariables {
            pull_request_id: cynic::Id::new("PR_1"),
            path: "a.rs".into(),
        });
        assert!(
            op.query
                .contains("markFileAsViewed(input: {pullRequestId: $pullRequestId, path: $path})"),
            "{}",
            op.query
        );
        let files = PrFilesQuery::build(PageVariables {
            owner: "o".into(),
            name: "r".into(),
            number: 1,
            after: None,
        });
        assert!(files.query.contains("viewerViewedState"), "{}", files.query);
    }

    #[test]
    fn pull_request_query_shape() {
        let op = PullRequestQuery::build(NumberVariables {
            owner: "o".into(),
            name: "r".into(),
            number: 1,
        });
        assert!(
            op.query.contains("pullRequest(number: $number)"),
            "{}",
            op.query
        );
        assert!(op.query.contains("headRefOid"), "{}", op.query);
    }
}
