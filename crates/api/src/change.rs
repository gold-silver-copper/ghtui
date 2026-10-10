//! What ghtui changes on GitHub, other than a review: every such write is
//! a [`Change`], sent by [`GitHub::change`].

use ghtui_schema::schema;

use crate::client::{ApiError, GitHub};
use crate::model::{NodeId, RepoId};
use crate::queries::{
    AddCommentPayload, CloseIssuePayload, ClosePrPayload, GitObjectId, MergePayload, ReadyPayload,
    ReopenIssuePayload, ReopenPrPayload, StarPayload, UnstarPayload, UpdateBranchPayload,
};

/// A change to make on GitHub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Comment on an issue or pull request.
    Comment {
        subject: NodeId,
        body: String,
    },
    /// Reply to a review thread right away (outside any pending review).
    Reply {
        thread: NodeId,
        body: String,
    },
    Star {
        repo: RepoId,
        id: NodeId,
        starred: bool,
    },
    /// Merge a pull request, if its head is still `head`.
    Merge {
        pr: NodeId,
        method: MergeMethod,
        head: String,
    },
    /// Close (`open: false`) or reopen a pull request.
    SetPrOpen {
        pr: NodeId,
        open: bool,
    },
    CloseIssue {
        issue: NodeId,
        reason: CloseReason,
    },
    ReopenIssue {
        issue: NodeId,
    },
    ReadyForReview {
        pr: NodeId,
    },
    /// Bring a pull request's branch up to date with its base, if its head
    /// is still `head`.
    UpdateBranch {
        pr: NodeId,
        rebase: bool,
        head: String,
    },
    /// Re-run a workflow run, or only its failed jobs (and what they need).
    Rerun {
        repo: RepoId,
        run: u64,
        failed_only: bool,
    },
    RerunJob {
        repo: RepoId,
        job: u64,
    },
    CancelRun {
        repo: RepoId,
        run: u64,
    },
}

#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cynic(graphql_type = "PullRequestMergeMethod", schema_module = "schema")]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

impl MergeMethod {
    pub fn describe(self) -> &'static str {
        match self {
            Self::Merge => "Create a merge commit",
            Self::Squash => "Squash and merge",
            Self::Rebase => "Rebase and merge",
        }
    }
}

/// Why an issue was closed.
#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq)]
#[cynic(graphql_type = "IssueClosedStateReason", schema_module = "schema")]
pub enum CloseReason {
    Completed,
    NotPlanned,
    /// Needs the issue it duplicates; ghtui doesn't offer it.
    Duplicate,
}

#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq)]
#[cynic(
    graphql_type = "PullRequestBranchUpdateMethod",
    schema_module = "schema"
)]
pub enum UpdateMethod {
    Merge,
    Rebase,
}

impl GitHub {
    /// Makes `change` on GitHub.
    pub async fn change(&self, change: &Change) -> Result<(), ApiError> {
        use cynic::MutationBuilder;
        match change {
            Change::Comment { subject, body } => {
                let op = AddComment::build(AddCommentVariables {
                    subject: subject.gql(),
                    body: body.clone(),
                });
                self.mutate(op).await.map(drop)
            }
            Change::Reply { thread, body } => {
                let op = crate::queries::Reply::build(crate::queries::ReplyVariables {
                    thread_id: thread.gql(),
                    body: body.clone(),
                });
                self.mutate(op).await.map(drop)
            }
            Change::Star { id, starred, .. } => {
                let vars = IdVariables { id: id.gql() };
                if *starred {
                    self.mutate(AddStar::build(vars)).await.map(drop)
                } else {
                    self.mutate(RemoveStar::build(vars)).await.map(drop)
                }
            }
            Change::Merge { pr, method, head } => {
                let op = MergePr::build(MergeVariables {
                    pr: pr.gql(),
                    method: *method,
                    head: GitObjectId(head.clone()),
                });
                self.mutate(op).await.map(drop)
            }
            Change::SetPrOpen { pr, open } => {
                let vars = IdVariables { id: pr.gql() };
                if *open {
                    self.mutate(ReopenPr::build(vars)).await.map(drop)
                } else {
                    self.mutate(ClosePr::build(vars)).await.map(drop)
                }
            }
            Change::CloseIssue { issue, reason } => {
                let op = CloseIssue::build(CloseIssueVariables {
                    issue: issue.gql(),
                    reason: *reason,
                });
                self.mutate(op).await.map(drop)
            }
            Change::ReopenIssue { issue } => {
                let op = ReopenIssue::build(IdVariables { id: issue.gql() });
                self.mutate(op).await.map(drop)
            }
            Change::ReadyForReview { pr } => {
                let op = MarkReady::build(IdVariables { id: pr.gql() });
                self.mutate(op).await.map(drop)
            }
            Change::UpdateBranch { pr, rebase, head } => {
                let op = UpdateBranch::build(UpdateBranchVariables {
                    pr: pr.gql(),
                    method: if *rebase {
                        UpdateMethod::Rebase
                    } else {
                        UpdateMethod::Merge
                    },
                    head: GitObjectId(head.clone()),
                });
                self.mutate(op).await.map(drop)
            }
            Change::Rerun {
                repo,
                run,
                failed_only,
            } => {
                let what = if *failed_only {
                    "rerun-failed-jobs"
                } else {
                    "rerun"
                };
                self.rest_post(&format!("/repos/{repo}/actions/runs/{run}/{what}"))
                    .await
            }
            Change::RerunJob { repo, job } => {
                self.rest_post(&format!("/repos/{repo}/actions/jobs/{job}/rerun"))
                    .await
            }
            Change::CancelRun { repo, run } => {
                self.rest_post(&format!("/repos/{repo}/actions/runs/{run}/cancel"))
                    .await
            }
        }
    }
}

#[derive(cynic::QueryVariables, Debug)]
pub struct IdVariables {
    pub id: cynic::Id,
}

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
#[cynic(
    graphql_type = "Mutation",
    schema_module = "schema",
    variables = "IdVariables"
)]
pub struct AddStar {
    #[arguments(input: { starrableId: $id })]
    pub add_star: Option<StarPayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Mutation",
    schema_module = "schema",
    variables = "IdVariables"
)]
pub struct RemoveStar {
    #[arguments(input: { starrableId: $id })]
    pub remove_star: Option<UnstarPayload>,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct MergeVariables {
    pub pr: cynic::Id,
    pub method: MergeMethod,
    pub head: GitObjectId,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Mutation",
    schema_module = "schema",
    variables = "MergeVariables"
)]
pub struct MergePr {
    #[arguments(input: { pullRequestId: $pr, mergeMethod: $method, expectedHeadOid: $head })]
    pub merge_pull_request: Option<MergePayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Mutation",
    schema_module = "schema",
    variables = "IdVariables"
)]
pub struct ClosePr {
    #[arguments(input: { pullRequestId: $id })]
    pub close_pull_request: Option<ClosePrPayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Mutation",
    schema_module = "schema",
    variables = "IdVariables"
)]
pub struct ReopenPr {
    #[arguments(input: { pullRequestId: $id })]
    pub reopen_pull_request: Option<ReopenPrPayload>,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct CloseIssueVariables {
    pub issue: cynic::Id,
    pub reason: CloseReason,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Mutation",
    schema_module = "schema",
    variables = "CloseIssueVariables"
)]
pub struct CloseIssue {
    #[arguments(input: { issueId: $issue, stateReason: $reason })]
    pub close_issue: Option<CloseIssuePayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Mutation",
    schema_module = "schema",
    variables = "IdVariables"
)]
pub struct ReopenIssue {
    #[arguments(input: { issueId: $id })]
    pub reopen_issue: Option<ReopenIssuePayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Mutation",
    schema_module = "schema",
    variables = "IdVariables"
)]
pub struct MarkReady {
    #[arguments(input: { pullRequestId: $id })]
    pub mark_pull_request_ready_for_review: Option<ReadyPayload>,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct UpdateBranchVariables {
    pub pr: cynic::Id,
    pub method: UpdateMethod,
    pub head: GitObjectId,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "Mutation",
    schema_module = "schema",
    variables = "UpdateBranchVariables"
)]
pub struct UpdateBranch {
    #[arguments(input: { pullRequestId: $pr, updateMethod: $method, expectedHeadOid: $head })]
    pub update_pull_request_branch: Option<UpdateBranchPayload>,
}
