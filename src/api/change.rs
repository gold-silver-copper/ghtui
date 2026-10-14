//! The changes ghtui makes to issues, pull requests, discussions, runs,
//! notifications, projects, alerts and a repository's settings: each is a
//! [`Change`], sent by [`GitHub::change`]; the writes the offline queue
//! keeps until GitHub is back are some of them. Reviews, releases, gists,
//! wikis, labels, milestones, files and the like are written by their own
//! jobs.

use crate::schema::schema;

use crate::api::client::{ApiError, GitHub};
use crate::api::model::{NodeId, RepoId};
use crate::api::notifications::ThreadAt;
use crate::api::queries::{
    AddCommentPayload, AutoMergePayload, CloseIssuePayload, ClosePrPayload, DequeuePayload,
    EnqueuePayload, GitObjectId, MergePayload, NoAutoMergePayload, ReadyPayload,
    ReopenIssuePayload, ReopenPrPayload, StarPayload, UnstarPayload, UpdateBranchPayload,
};

/// A change to make on GitHub.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Change {
    /// Comment on an issue or pull request.
    Comment {
        subject: NodeId,
        body: String,
    },
    /// Comment on a commit as a whole.
    CommitComment {
        repo: RepoId,
        oid: String,
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
    /// Merge a pull request once every requirement is met (with a merge
    /// queue, add it to the queue then), if its head is still `head`.
    EnableAutoMerge {
        pr: NodeId,
        method: MergeMethod,
        head: String,
    },
    DisableAutoMerge {
        pr: NodeId,
    },
    /// Add a pull request to its base branch's merge queue, if its head
    /// is still `head`.
    Enqueue {
        pr: NodeId,
        head: String,
    },
    Dequeue {
        pr: NodeId,
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
    /// Mark a notification's thread read.
    MarkRead {
        thread: String,
    },
    /// Mark a notification's thread done: out of the inbox (and kept out
    /// of it here until GitHub updates it again).
    MarkDone {
        thread: ThreadAt,
    },
    /// Mark read every notification updated by `last_read_at`: all of
    /// them, or one repository's.
    MarkAllRead {
        repo: Option<RepoId>,
        last_read_at: String,
    },
    /// Mark each of these threads read, or done.
    MarkEach {
        threads: Vec<ThreadAt>,
        done: bool,
    },
    /// Edit a comment, a title, a reaction or a sidebar.
    Edit(crate::api::edit::Edit),
    /// Write in a discussion, or mark its answer.
    Discussion(crate::api::discussion::Discuss),
    /// Add, archive or restore a project's item, or edit a draft's text.
    Project(crate::api::project::ProjectChange),
    /// Set a project item's field to a value, or clear it (`None`); `said`
    /// is what it sets, as people say it ("Status to Done").
    SetProjectField {
        project: NodeId,
        item: NodeId,
        field: NodeId,
        value: Option<crate::api::project::FieldValue>,
        said: String,
    },
    /// Dismiss (for a secret, resolve) a security alert, or reopen it.
    Alert(crate::api::security::AlertChange),
    /// Lock, pin, hide, a branch, sub-issues and the like
    /// ([`crate::api::manage`]).
    Manage(crate::api::manage::Manage),
    /// A repository's settings ([`crate::api::settings`]).
    Settings(crate::api::settings::SettingsChange),
}

#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cynic(graphql_type = "PullRequestMergeMethod")]
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
#[cynic(graphql_type = "IssueClosedStateReason")]
pub enum CloseReason {
    Completed,
    NotPlanned,
    /// Needs the issue it duplicates; ghtui doesn't offer it.
    Duplicate,
}

#[derive(cynic::Enum, Debug, Clone, Copy, PartialEq, Eq)]
#[cynic(graphql_type = "PullRequestBranchUpdateMethod")]
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
            Change::CommitComment { repo, oid, body } => {
                self.comment_on_commit(repo, oid, body).await
            }
            Change::Reply { thread, body } => {
                let op = crate::api::queries::Reply::build(crate::api::queries::ReplyVariables {
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
            Change::EnableAutoMerge { pr, method, head } => {
                let op = EnableAutoMerge::build(MergeVariables {
                    pr: pr.gql(),
                    method: *method,
                    head: GitObjectId(head.clone()),
                });
                self.mutate(op).await.map(drop)
            }
            Change::DisableAutoMerge { pr } => {
                let op = DisableAutoMerge::build(IdVariables { id: pr.gql() });
                self.mutate(op).await.map(drop)
            }
            Change::Enqueue { pr, head } => {
                let op = Enqueue::build(EnqueueVariables {
                    pr: pr.gql(),
                    head: GitObjectId(head.clone()),
                });
                self.mutate(op).await.map(drop)
            }
            Change::Dequeue { pr } => {
                let op = Dequeue::build(IdVariables { id: pr.gql() });
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
                self.rest_write(
                    http::Method::POST,
                    &format!("/repos/{repo}/actions/runs/{run}/{what}"),
                    None,
                )
                .await
            }
            Change::RerunJob { repo, job } => {
                self.rest_write(
                    http::Method::POST,
                    &format!("/repos/{repo}/actions/jobs/{job}/rerun"),
                    None,
                )
                .await
            }
            Change::CancelRun { repo, run } => {
                self.rest_write(
                    http::Method::POST,
                    &format!("/repos/{repo}/actions/runs/{run}/cancel"),
                    None,
                )
                .await
            }
            Change::MarkRead { thread } => self.mark(thread, false).await,
            Change::MarkDone { thread } => {
                self.mark(&thread.id, true).await?;
                self.remember_done(std::slice::from_ref(thread)).await;
                Ok(())
            }
            Change::MarkAllRead { repo, last_read_at } => {
                let path = match repo {
                    Some(repo) => format!("/repos/{repo}/notifications"),
                    None => "/notifications".to_owned(),
                };
                let body = serde_json::json!({ "last_read_at": last_read_at, "read": true });
                self.rest_write(http::Method::PUT, &path, Some(&body)).await
            }
            // GitHub marks threads done one at a time.
            Change::MarkEach { threads, done } => {
                let mut marked = 0;
                let mut sent = Ok(());
                for thread in threads {
                    if let Err(err) = self.mark(&thread.id, *done).await {
                        sent = Err(match err {
                            ApiError::Http { status, message } if marked > 0 => ApiError::Http {
                                status,
                                message: format!("{message} (after {marked} of {})", threads.len()),
                            },
                            err => err,
                        });
                        break;
                    }
                    marked += 1;
                }
                // Those marked done before a failure are done.
                if *done {
                    self.remember_done(threads.get(..marked).unwrap_or_default())
                        .await;
                }
                sent
            }
            Change::Edit(edit) => self.edit(edit).await,
            Change::Discussion(change) => self.discuss(change).await,
            Change::Project(change) => self.change_project(change).await,
            Change::SetProjectField {
                project,
                item,
                field,
                value,
                ..
            } => {
                self.set_project_field(project, item, field, value.as_ref())
                    .await
            }
            Change::Alert(change) => self.set_alert(change).await,
            Change::Manage(change) => self.manage(change).await,
            Change::Settings(change) => self.change_setting(change).await,
        }
    }
}

impl GitHub {
    /// Marks a notification's thread read, or done.
    async fn mark(&self, thread: &str, done: bool) -> Result<(), ApiError> {
        let path = format!("/notifications/threads/{thread}");
        let method = if done {
            http::Method::DELETE
        } else {
            http::Method::PATCH
        };
        self.rest_write(method, &path, None).await
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
#[cynic(graphql_type = "Mutation", variables = "AddCommentVariables")]
pub struct AddComment {
    #[arguments(input: { subjectId: $subject, body: $body })]
    pub add_comment: Option<AddCommentPayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "IdVariables")]
pub struct AddStar {
    #[arguments(input: { starrableId: $id })]
    pub add_star: Option<StarPayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "IdVariables")]
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
#[cynic(graphql_type = "Mutation", variables = "MergeVariables")]
pub struct MergePr {
    #[arguments(input: { pullRequestId: $pr, mergeMethod: $method, expectedHeadOid: $head })]
    pub merge_pull_request: Option<MergePayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "MergeVariables")]
pub struct EnableAutoMerge {
    #[arguments(input: { pullRequestId: $pr, mergeMethod: $method, expectedHeadOid: $head })]
    pub enable_pull_request_auto_merge: Option<AutoMergePayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "IdVariables")]
pub struct DisableAutoMerge {
    #[arguments(input: { pullRequestId: $id })]
    pub disable_pull_request_auto_merge: Option<NoAutoMergePayload>,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct EnqueueVariables {
    pub pr: cynic::Id,
    pub head: GitObjectId,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "EnqueueVariables")]
pub struct Enqueue {
    #[arguments(input: { pullRequestId: $pr, expectedHeadOid: $head })]
    pub enqueue_pull_request: Option<EnqueuePayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "IdVariables")]
pub struct Dequeue {
    #[arguments(input: { id: $id })]
    pub dequeue_pull_request: Option<DequeuePayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "IdVariables")]
pub struct ClosePr {
    #[arguments(input: { pullRequestId: $id })]
    pub close_pull_request: Option<ClosePrPayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "IdVariables")]
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
#[cynic(graphql_type = "Mutation", variables = "CloseIssueVariables")]
pub struct CloseIssue {
    #[arguments(input: { issueId: $issue, stateReason: $reason })]
    pub close_issue: Option<CloseIssuePayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "IdVariables")]
pub struct ReopenIssue {
    #[arguments(input: { issueId: $id })]
    pub reopen_issue: Option<ReopenIssuePayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "IdVariables")]
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
#[cynic(graphql_type = "Mutation", variables = "UpdateBranchVariables")]
pub struct UpdateBranch {
    #[arguments(input: { pullRequestId: $pr, updateMethod: $method, expectedHeadOid: $head })]
    pub update_pull_request_branch: Option<UpdateBranchPayload>,
}
