//! Typed GraphQL operations, validated against GitHub's schema at compile
//! time. These mirror the wire format; [`crate::model`] holds what the rest of
//! the app uses.

use ghtui_schema::schema;

use crate::model::{Side, ViewedState};

#[derive(cynic::Scalar, Debug, Clone)]
#[cynic(graphql_type = "DateTime", schema_module = "schema")]
pub struct DateTime(pub String);

#[derive(cynic::Scalar, Debug, Clone)]
#[cynic(graphql_type = "URI", schema_module = "schema")]
pub struct Uri(pub String);

#[derive(cynic::Scalar, Debug, Clone)]
#[cynic(graphql_type = "GitObjectID", schema_module = "schema")]
pub struct GitObjectId(pub String);

/// The non-null nodes of a GraphQL list.
pub(crate) fn nodes<T>(list: Option<Vec<Option<T>>>) -> impl Iterator<Item = T> {
    list.into_iter().flatten().flatten()
}

/// Fragments with the same single field on several GraphQL types.
macro_rules! fragments {
    (total_count: $($name:ident = $graphql:literal),* $(,)?) => {$(
        #[derive(cynic::QueryFragment, Debug)]
        #[cynic(graphql_type = $graphql, schema_module = "schema")]
        pub struct $name {
            pub total_count: i32,
        }
    )*};
    (client_mutation_id: $($name:ident = $graphql:literal),* $(,)?) => {$(
        #[derive(cynic::QueryFragment, Debug)]
        #[cynic(graphql_type = $graphql, schema_module = "schema")]
        pub struct $name {
            pub client_mutation_id: Option<String>,
        }
    )*};
}

// Connection sizes.
fragments! {
    total_count:
    CommentCount = "IssueCommentConnection",
    UserCount = "UserConnection",
    IssueCount = "IssueConnection",
    PrCount = "PullRequestConnection",
    CommitCount = "CommitHistoryConnection",
    FollowCount = "FollowerConnection",
    FollowingCount = "FollowingConnection",
}

// Results of mutations that only need to succeed.
fragments! {
    client_mutation_id:
    MarkFileAsViewedPayload = "MarkFileAsViewedPayload",
    UnmarkFileAsViewedPayload = "UnmarkFileAsViewedPayload",
    AddCommentPayload = "AddCommentPayload",
    StarPayload = "AddStarPayload",
    UnstarPayload = "RemoveStarPayload",
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

// ---- inbox: my PRs and review requests ------------------------------------

/// Search results per inbox section. Larger pages with check rollups make
/// GitHub's search time out (502) for busy accounts.
pub const INBOX_PAGE: i32 = 25;

#[derive(cynic::QueryVariables, Debug)]
pub struct SearchVariables {
    pub query: String,
    pub first: i32,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "SearchVariables"
)]
pub struct SearchQuery {
    #[arguments(query: $query, type: ISSUE, first: $first)]
    pub search: SearchConnection,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "SearchResultItemConnection", schema_module = "schema")]
pub struct SearchConnection {
    pub issue_count: i32,
    pub nodes: Option<Vec<Option<SearchItem>>>,
}

#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "SearchResultItem", schema_module = "schema")]
pub enum SearchItem {
    PullRequest(PrSummary),
    #[cynic(fallback)]
    Other,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequest", schema_module = "schema")]
pub struct PrSummary {
    pub number: i32,
    pub title: String,
    pub url: Uri,
    pub is_draft: bool,
    pub state: PullRequestState,
    pub updated_at: DateTime,
    pub author: Option<Actor>,
    pub repository: RepositoryName,
    pub additions: i32,
    pub deletions: i32,
    pub review_decision: Option<PullRequestReviewDecision>,
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
#[cynic(graphql_type = "PullRequestCommitConnection", schema_module = "schema")]
pub struct CommitRollupConnection {
    pub nodes: Option<Vec<Option<CommitRollupNode>>>,
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
#[cynic(graphql_type = "PullRequestReviewDecision", schema_module = "schema")]
pub enum PullRequestReviewDecision {
    Approved,
    ChangesRequested,
    ReviewRequired,
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
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequest", schema_module = "schema")]
pub struct PrDetail {
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
    pub mergeable: MergeableState,
    #[arguments(first: 20)]
    pub labels: Option<LabelConnection>,
}

#[derive(cynic::Enum, Debug, Clone, Copy)]
#[cynic(graphql_type = "MergeableState", schema_module = "schema")]
pub enum MergeableState {
    Conflicting,
    Mergeable,
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "LabelConnection", schema_module = "schema")]
pub struct LabelConnection {
    pub nodes: Option<Vec<Option<Label>>>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Label", schema_module = "schema")]
pub struct Label {
    pub name: String,
    pub color: String,
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
    #[arguments(first: 100)]
    pub comments: ReviewCommentConnection,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "PullRequestReviewCommentConnection",
    schema_module = "schema"
)]
pub struct ReviewCommentConnection {
    pub nodes: Option<Vec<Option<ReviewComment>>>,
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
    pub reviews: Option<ReviewIdConnection>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestReviewConnection", schema_module = "schema")]
pub struct ReviewIdConnection {
    pub nodes: Option<Vec<Option<ReviewId>>>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestReview", schema_module = "schema")]
pub struct ReviewId {
    pub id: cynic::Id,
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

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "AddPullRequestReviewPayload", schema_module = "schema")]
pub struct StartReviewPayload {
    pub pull_request_review: Option<ReviewId>,
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

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "AddPullRequestReviewThreadPayload",
    schema_module = "schema"
)]
pub struct ThreadPayload {
    pub thread: Option<ThreadId>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestReviewThread", schema_module = "schema")]
pub struct ThreadId {
    pub id: cynic::Id,
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

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "SubmitPullRequestReviewPayload",
    schema_module = "schema"
)]
pub struct SubmitReviewPayload {
    pub pull_request_review: Option<ReviewId>,
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

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "AddPullRequestReviewThreadReplyPayload",
    schema_module = "schema"
)]
pub struct ReplyPayload {
    pub comment: Option<CommentId>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestReviewComment", schema_module = "schema")]
pub struct CommentId {
    pub id: cynic::Id,
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
#[cynic(graphql_type = "ResolveReviewThreadPayload", schema_module = "schema")]
pub struct ResolvePayload {
    pub thread: Option<ThreadId>,
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

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "UnresolveReviewThreadPayload",
    schema_module = "schema"
)]
pub struct UnresolvePayload {
    pub thread: Option<ThreadId>,
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
#[cynic(graphql_type = "PullRequestReviewConnection", schema_module = "schema")]
pub struct ReviewCommitConnection {
    pub nodes: Option<Vec<Option<ReviewCommit>>>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestReview", schema_module = "schema")]
pub struct ReviewCommit {
    pub state: ReviewState,
    pub commit: Option<CommitOid>,
}

#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq)]
#[cynic(graphql_type = "PullRequestReviewState", schema_module = "schema")]
pub enum ReviewState {
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
    Pending,
}

#[cfg(test)]
mod tests {
    use super::*;
    use cynic::{MutationBuilder, QueryBuilder};

    #[test]
    fn search_query_shape() {
        let op = SearchQuery::build(SearchVariables {
            query: "is:pr".into(),
            first: INBOX_PAGE,
        });
        assert!(
            op.query
                .contains("search(query: $query, type: ISSUE, first: $first)"),
            "{}",
            op.query
        );
    }

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
