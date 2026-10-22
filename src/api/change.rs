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
    /// An organization's people and teams, or an invitation sent to you
    /// ([`crate::api::org`]).
    Org(crate::api::org::OrgChange),
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
        match change {
            Change::Comment { subject, body } => {
                self.run::<AddComment, _>(AddCommentVariables {
                    subject: subject.gql(),
                    body: body.clone(),
                })
                .await
            }
            Change::CommitComment { repo, oid, body } => {
                self.comment_on_commit(repo, oid, body).await
            }
            Change::Reply { thread, body } => {
                self.run::<crate::api::queries::Reply, _>(crate::api::queries::ReplyVariables {
                    thread_id: thread.gql(),
                    body: body.clone(),
                })
                .await
            }
            Change::Star {
                id, starred: true, ..
            } => self.run_on::<AddStar>(id).await,
            Change::Star { id, .. } => self.run_on::<RemoveStar>(id).await,
            Change::Merge { pr, method, head } | Change::EnableAutoMerge { pr, method, head } => {
                let vars = MergeVariables {
                    pr: pr.gql(),
                    method: *method,
                    head: GitObjectId(head.clone()),
                };
                if let Change::Merge { .. } = change {
                    self.run::<MergePr, _>(vars).await
                } else {
                    self.run::<EnableAutoMerge, _>(vars).await
                }
            }
            Change::DisableAutoMerge { pr } => self.run_on::<DisableAutoMerge>(pr).await,
            Change::Enqueue { pr, head } => {
                self.run::<Enqueue, _>(EnqueueVariables {
                    pr: pr.gql(),
                    head: GitObjectId(head.clone()),
                })
                .await
            }
            Change::Dequeue { pr } => self.run_on::<Dequeue>(pr).await,
            Change::SetPrOpen { pr, open: true } => self.run_on::<ReopenPr>(pr).await,
            Change::SetPrOpen { pr, .. } => self.run_on::<ClosePr>(pr).await,
            Change::CloseIssue { issue, reason } => {
                self.run::<CloseIssue, _>(CloseIssueVariables {
                    issue: issue.gql(),
                    reason: *reason,
                })
                .await
            }
            Change::ReopenIssue { issue } => self.run_on::<ReopenIssue>(issue).await,
            Change::ReadyForReview { pr } => self.run_on::<MarkReady>(pr).await,
            Change::UpdateBranch { pr, rebase, head } => {
                self.run::<UpdateBranch, _>(UpdateBranchVariables {
                    pr: pr.gql(),
                    method: if *rebase {
                        UpdateMethod::Rebase
                    } else {
                        UpdateMethod::Merge
                    },
                    head: GitObjectId(head.clone()),
                })
                .await
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
                self.post(&format!("/repos/{repo}/actions/runs/{run}/{what}"))
                    .await
            }
            Change::RerunJob { repo, job } => {
                self.post(&format!("/repos/{repo}/actions/jobs/{job}/rerun"))
                    .await
            }
            Change::CancelRun { repo, run } => {
                self.post(&format!("/repos/{repo}/actions/runs/{run}/cancel"))
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
            Change::Org(change) => self.change_org(change).await,
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

    /// POSTs to `path` with no body.
    async fn post(&self, path: &str) -> Result<(), ApiError> {
        self.rest_write(http::Method::POST, path, None).await
    }

    /// Sends mutation `M`, which only has to succeed.
    pub(crate) async fn run<M, V>(&self, vars: V) -> Result<(), ApiError>
    where
        M: cynic::MutationBuilder<V> + serde::de::DeserializeOwned,
        V: serde::Serialize,
    {
        self.mutate(M::build(vars)).await.map(drop)
    }

    /// Sends mutation `M` on the node `id`.
    pub(crate) async fn run_on<M>(&self, id: &NodeId) -> Result<(), ApiError>
    where
        M: cynic::MutationBuilder<IdVariables> + serde::de::DeserializeOwned,
    {
        self.run::<M, _>(IdVariables { id: id.gql() }).await
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

/// Mutations, one row each: the struct, its variables, and the field it
/// asks for with that field's payload and input. [`GitHub::run`] sends one
/// that only has to succeed. `MUTATIONS` counts the rows, for the test
/// that every mutation is checked.
macro_rules! mutations {
    ($($(#[$doc:meta])* $name:ident($vars:literal) $field:ident: $payload:ident { $($input:tt)* })*) => {
        $(
            $(#[$doc])*
            #[derive(cynic::QueryFragment, Debug)]
            #[cynic(graphql_type = "Mutation", variables = $vars)]
            pub struct $name {
                #[arguments(input: { $($input)* })]
                pub $field: Option<$payload>,
            }
        )*
        #[cfg(test)]
        pub(crate) const MUTATIONS: usize = [$(stringify!($name)),*].len();
    };
}

pub(crate) use mutations;

mutations! {
    AddComment("AddCommentVariables") add_comment: AddCommentPayload { subjectId: $subject, body: $body }
    AddStar("IdVariables") add_star: StarPayload { starrableId: $id }
    RemoveStar("IdVariables") remove_star: UnstarPayload { starrableId: $id }
    MergePr("MergeVariables") merge_pull_request: MergePayload { pullRequestId: $pr, mergeMethod: $method, expectedHeadOid: $head }
    EnableAutoMerge("MergeVariables") enable_pull_request_auto_merge: AutoMergePayload { pullRequestId: $pr, mergeMethod: $method, expectedHeadOid: $head }
    DisableAutoMerge("IdVariables") disable_pull_request_auto_merge: NoAutoMergePayload { pullRequestId: $id }
    Enqueue("EnqueueVariables") enqueue_pull_request: EnqueuePayload { pullRequestId: $pr, expectedHeadOid: $head }
    Dequeue("IdVariables") dequeue_pull_request: DequeuePayload { id: $id }
    ClosePr("IdVariables") close_pull_request: ClosePrPayload { pullRequestId: $id }
    ReopenPr("IdVariables") reopen_pull_request: ReopenPrPayload { pullRequestId: $id }
    CloseIssue("CloseIssueVariables") close_issue: CloseIssuePayload { issueId: $issue, stateReason: $reason }
    ReopenIssue("IdVariables") reopen_issue: ReopenIssuePayload { issueId: $id }
    MarkReady("IdVariables") mark_pull_request_ready_for_review: ReadyPayload { pullRequestId: $id }
    UpdateBranch("UpdateBranchVariables") update_pull_request_branch: UpdateBranchPayload { pullRequestId: $pr, updateMethod: $method, expectedHeadOid: $head }
}

#[derive(cynic::QueryVariables, Debug)]
pub struct MergeVariables {
    pub pr: cynic::Id,
    pub method: MergeMethod,
    pub head: GitObjectId,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct EnqueueVariables {
    pub pr: cynic::Id,
    pub head: GitObjectId,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct CloseIssueVariables {
    pub issue: cynic::Id,
    pub reason: CloseReason,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct UpdateBranchVariables {
    pub pr: cynic::Id,
    pub method: UpdateMethod,
    pub head: GitObjectId,
}
