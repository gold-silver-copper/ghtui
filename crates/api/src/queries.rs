//! Typed GraphQL operations, validated against GitHub's schema at compile
//! time. These mirror the wire format; [`crate::model`] holds what the rest of
//! the app uses.

use ghtui_schema::schema;

#[derive(cynic::Scalar, Debug, Clone)]
#[cynic(graphql_type = "DateTime", schema_module = "schema")]
pub struct DateTime(pub String);

#[derive(cynic::Scalar, Debug, Clone)]
#[cynic(graphql_type = "URI", schema_module = "schema")]
pub struct Uri(pub String);

#[derive(cynic::Scalar, Debug, Clone)]
#[cynic(graphql_type = "GitObjectID", schema_module = "schema")]
pub struct GitObjectId(pub String);

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_module = "schema")]
pub struct RateLimit {
    pub cost: i32,
    pub limit: i32,
    pub remaining: i32,
    pub reset_at: DateTime,
}

// ---- viewer ---------------------------------------------------------------

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Query", schema_module = "schema")]
pub struct ViewerQuery {
    pub viewer: Viewer,
    pub rate_limit: Option<RateLimit>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "User", schema_module = "schema")]
pub struct Viewer {
    pub login: String,
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
    pub rate_limit: Option<RateLimit>,
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
    pub comments: CountOnly,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "IssueCommentConnection", schema_module = "schema")]
pub struct CountOnly {
    pub total_count: i32,
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

#[derive(cynic::QueryVariables, Debug)]
pub struct PullRequestVariables {
    pub owner: String,
    pub name: String,
    pub number: i32,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "PullRequestVariables"
)]
pub struct PullRequestQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<RepositoryWithPr>,
    pub rate_limit: Option<RateLimit>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Repository",
    schema_module = "schema",
    variables = "PullRequestVariables"
)]
pub struct RepositoryWithPr {
    #[arguments(number: $number)]
    pub pull_request: Option<PrDetail>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequest", schema_module = "schema")]
pub struct PrDetail {
    pub number: i32,
    pub title: String,
    pub body: String,
    pub url: Uri,
    pub is_draft: bool,
    pub state: PullRequestState,
    pub created_at: DateTime,
    pub updated_at: DateTime,
    pub author: Option<Actor>,
    pub repository: RepositoryName,
    pub base_ref_name: String,
    pub head_ref_name: String,
    pub base_ref_oid: GitObjectId,
    pub head_ref_oid: GitObjectId,
    pub head_repository: Option<RepositoryName>,
    pub additions: i32,
    pub deletions: i32,
    pub changed_files: i32,
    pub mergeable: MergeableState,
    pub review_decision: Option<PullRequestReviewDecision>,
    #[arguments(first: 20)]
    pub labels: Option<LabelConnection>,
    #[arguments(last: 1)]
    pub commits: CommitRollupConnection,
    pub comments: CountOnly,
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

#[derive(cynic::QueryVariables, Debug)]
pub struct PrFilesVariables {
    pub owner: String,
    pub name: String,
    pub number: i32,
    pub after: Option<String>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Query",
    schema_module = "schema",
    variables = "PrFilesVariables"
)]
pub struct PrFilesQuery {
    #[arguments(owner: $owner, name: $name)]
    pub repository: Option<RepositoryWithPrFiles>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Repository",
    schema_module = "schema",
    variables = "PrFilesVariables"
)]
pub struct RepositoryWithPrFiles {
    #[arguments(number: $number)]
    pub pull_request: Option<PrFiles>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "PullRequest",
    schema_module = "schema",
    variables = "PrFilesVariables"
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

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequestChangedFile", schema_module = "schema")]
pub struct PrChangedFile {
    pub path: String,
    pub viewer_viewed_state: FileViewedState,
}

#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq)]
#[cynic(graphql_type = "FileViewedState", schema_module = "schema")]
pub enum FileViewedState {
    Dismissed,
    Unviewed,
    Viewed,
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
#[cynic(graphql_type = "MarkFileAsViewedPayload", schema_module = "schema")]
pub struct MarkFileAsViewedPayload {
    pub client_mutation_id: Option<String>,
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

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "UnmarkFileAsViewedPayload", schema_module = "schema")]
pub struct UnmarkFileAsViewedPayload {
    pub client_mutation_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use cynic::QueryBuilder;

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
        assert!(op.query.contains("rateLimit"), "{}", op.query);
    }

    #[test]
    fn viewed_mutations_shape() {
        use cynic::MutationBuilder;
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
        let files = PrFilesQuery::build(PrFilesVariables {
            owner: "o".into(),
            name: "r".into(),
            number: 1,
            after: None,
        });
        assert!(files.query.contains("viewerViewedState"), "{}", files.query);
    }

    #[test]
    fn pull_request_query_shape() {
        let op = PullRequestQuery::build(PullRequestVariables {
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
