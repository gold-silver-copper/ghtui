//! Managing an issue, a pull request or a discussion, as GitHub's sidebar
//! and menus do: a pull request's branch after it's merged, back to
//! draft, its base; locking and pinning; hiding a comment; sub-issues and
//! what blocks an issue; closing as a duplicate; closing a discussion.
//! Each is a [`Manage`], sent as
//! [`Change::Manage`](crate::api::change::Change::Manage). What makes
//! something new (a revert, a transfer, a branch, a discussion) is a call
//! of its own, which answers with what it made.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::client::{ApiError, GitHub};
use crate::api::model::{Capped, NodeId, RepoId};
use crate::api::raw;
pub use crate::api::timeline::LockReason;

/// An issue by its repository and number.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct IssueAt {
    pub repo: RepoId,
    pub number: u64,
}

impl IssueAt {
    /// `#12` or `12` (in `here`), `owner/name#12`, or an issue's link.
    pub fn parse(text: &str, here: &RepoId) -> Option<Self> {
        let text = text.trim();
        let number = |s: &str| s.parse::<u64>().ok().filter(|n| *n > 0);
        if let Some(n) = number(text.strip_prefix('#').unwrap_or(text)) {
            return Some(Self {
                repo: here.clone(),
                number: n,
            });
        }
        if let Some((repo, n)) = text.split_once('#')
            && let (Some(repo), Some(number)) = (RepoId::parse(repo), number(n))
        {
            return Some(Self { repo, number });
        }
        // A link: …/owner/name/issues/12 (or pull/12), on any host.
        let path = text.split_once("://").map_or(text, |(_, rest)| rest);
        let parts: Vec<&str> = path.trim_end_matches('/').split('/').collect();
        match parts.as_slice() {
            [.., owner, name, "issues" | "pull", n] => Some(Self {
                repo: RepoId::parse(&format!("{owner}/{name}"))?,
                number: number(n.split(['#', '?']).next().unwrap_or(n))?,
            }),
            _ => None,
        }
    }

    /// `#12` in `here`, else `owner/name#12`.
    pub fn named(&self, here: &RepoId) -> String {
        if self.repo == *here {
            format!("#{}", self.number)
        } else {
            self.to_string()
        }
    }
}

impl std::fmt::Display for IssueAt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}#{}", self.repo, self.number)
    }
}

words! {
    /// Why a comment is hidden, as GitHub's "Hide" menu offers.
    #[derive(serde::Serialize, serde::Deserialize)]
    pub enum HideReason {
        fn wire,
        /// As GitHub's menu says it.
        pub fn label,
        /// GitHub's `minimizedReason` for it ("off-topic"), which differs
        /// from what's sent.
        pub fn said,
    }
    Spam      "SPAM"      "Spam"      "spam";
    Abuse     "ABUSE"     "Abuse"     "abuse";
    OffTopic  "OFF_TOPIC" "Off Topic" "off-topic";
    Outdated  "OUTDATED"  "Outdated"  "outdated";
    Duplicate "DUPLICATE" "Duplicate" "duplicate";
    Resolved  "RESOLVED"  "Resolved"  "resolved";
}

/// A locked conversation, and GitHub's reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Locked {
    pub reason: Option<LockReason>,
}

/// Whether a comment is hidden (and why, in GitHub's words: "off-topic";
/// empty where it doesn't say), and whether you may hide or unhide it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hiding {
    pub reason: Option<String>,
    pub can_hide: bool,
    pub can_unhide: bool,
}

words! {
    /// Why a discussion is closed.
    #[derive(Serialize, Deserialize)]
    pub enum DiscussionReason {
        /// A closed discussion's `stateReason`.
        fn wire -> pub fn from_wire,
        /// "resolved".
        pub fn said,
    }
    Resolved  "RESOLVED"  "resolved";
    Outdated  "OUTDATED"  "outdated";
    Duplicate "DUPLICATE" "duplicate";
}

/// A change to an issue, a pull request or a discussion.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Manage {
    /// Delete a pull request's head branch.
    DeleteBranch {
        repo: RepoId,
        branch: String,
    },
    /// Make it again at the commit it was at.
    RestoreBranch {
        repo: RepoId,
        branch: String,
        oid: String,
    },
    ConvertToDraft {
        pr: NodeId,
    },
    /// Point a pull request at another base branch.
    ChangeBase {
        pr: NodeId,
        base: String,
    },
    /// Lock an issue's, a pull request's or a discussion's conversation.
    Lock {
        subject: NodeId,
        reason: Option<LockReason>,
    },
    Unlock {
        subject: NodeId,
    },
    /// Pin an issue to its repository's Issues tab (`on`), or unpin it.
    Pin {
        issue: NodeId,
        on: bool,
    },
    Hide {
        comment: NodeId,
        reason: HideReason,
    },
    Unhide {
        comment: NodeId,
    },
    /// Close an issue as a duplicate of `of` (`None`: one to choose).
    CloseAsDuplicate {
        issue: NodeId,
        of: Option<IssueAt>,
    },
    /// Revert a merged pull request: a branch and a pull request of it.
    Revert {
        pr: NodeId,
    },
    /// Move an issue to another repository (`to`, by its node ID).
    Transfer {
        issue: NodeId,
        to: NodeId,
        repo: RepoId,
    },
    /// Add `child` to `parent`'s sub-issues, or remove it.
    SubIssue {
        parent: NodeId,
        child: IssueAt,
        add: bool,
    },
    /// Move a sub-issue to just before (or after) another.
    MoveSubIssue {
        parent: NodeId,
        child: IssueAt,
        sibling: IssueAt,
        before: bool,
    },
    /// Mark `issue` blocked by `by`, or no longer.
    BlockedBy {
        issue: NodeId,
        by: IssueAt,
        add: bool,
    },
    /// Make a branch for an issue, at its repository's default branch,
    /// linked to it (its Development).
    LinkBranch {
        issue: NodeId,
        repo: RepoId,
        name: String,
    },
    CloseDiscussion {
        discussion: NodeId,
        reason: DiscussionReason,
    },
    ReopenDiscussion {
        discussion: NodeId,
    },
}

/// A repository an issue can move to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub repo: RepoId,
    pub id: NodeId,
}

/// A repository's discussion category, and the form a new discussion
/// in it starts from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    pub id: NodeId,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub answerable: bool,
    /// Its form (`.github/DISCUSSION_TEMPLATE/<slug>.yml`), if it has one.
    pub form: Option<String>,
}

/// Where new discussions go: the repository's ID, and its categories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Categories {
    pub repo: NodeId,
    pub categories: Capped<Category>,
}

#[derive(Deserialize)]
struct Id {
    id: NodeId,
}

impl GitHub {
    /// An issue's node ID.
    async fn issue_id(&self, at: &IssueAt) -> Result<NodeId, ApiError> {
        let vars = json!({ "owner": at.repo.owner, "name": at.repo.name, "number": at.number });
        let found: Id = (self.graphql_json(
            raw::ISSUE_OR_PR_ID,
            vars,
            at,
            "/repository/issueOrPullRequest",
        ))
        .await?;
        Ok(found.id)
    }

    pub(crate) async fn manage(&self, change: &Manage) -> Result<(), ApiError> {
        let (query, vars) = match change {
            Manage::DeleteBranch { repo, branch } => {
                let path = format!("/repos/{repo}/git/refs/heads/{branch}");
                return self.rest_write(http::Method::DELETE, &path, None).await;
            }
            Manage::RestoreBranch { repo, branch, oid } => {
                let body = json!({ "ref": format!("refs/heads/{branch}"), "sha": oid });
                let path = format!("/repos/{repo}/git/refs");
                return self
                    .rest_write(http::Method::POST, &path, Some(&body))
                    .await;
            }
            Manage::ConvertToDraft { pr } => (raw::CONVERT_TO_DRAFT, json!({ "id": pr.as_str() })),
            Manage::ChangeBase { pr, base } => {
                (raw::CHANGE_BASE, json!({ "id": pr.as_str(), "base": base }))
            }
            Manage::Lock { subject, reason } => (
                raw::LOCK,
                json!({ "id": subject.as_str(), "reason": reason.map(LockReason::wire) }),
            ),
            Manage::Unlock { subject } => (raw::UNLOCK, json!({ "id": subject.as_str() })),
            Manage::Pin { issue, on: true } => (raw::PIN_ISSUE, json!({ "id": issue.as_str() })),
            Manage::Pin { issue, .. } => (raw::UNPIN_ISSUE, json!({ "id": issue.as_str() })),
            Manage::Hide { comment, reason } => (
                raw::HIDE_COMMENT,
                json!({ "id": comment.as_str(), "reason": reason.wire() }),
            ),
            Manage::Unhide { comment } => (raw::UNHIDE_COMMENT, json!({ "id": comment.as_str() })),
            Manage::Revert { pr } => return self.revert(pr).await.map(drop),
            Manage::Transfer { issue, to, .. } => return self.transfer(issue, to).await.map(drop),
            Manage::CloseAsDuplicate { issue, of } => {
                let of = of
                    .as_ref()
                    .ok_or_else(|| ApiError::Internal("no issue it duplicates".into()))?;
                let of = self.issue_id(of).await?;
                (
                    raw::CLOSE_AS_DUPLICATE,
                    json!({ "id": issue.as_str(), "of": of.as_str() }),
                )
            }
            Manage::SubIssue { parent, child, add } => {
                let child = self.issue_id(child).await?;
                let query = if *add {
                    raw::ADD_SUB_ISSUE
                } else {
                    raw::REMOVE_SUB_ISSUE
                };
                (
                    query,
                    json!({ "id": parent.as_str(), "child": child.as_str() }),
                )
            }
            Manage::MoveSubIssue {
                parent,
                child,
                sibling,
                before,
            } => {
                let child = self.issue_id(child).await?;
                let sibling = self.issue_id(sibling).await?;
                let (before, after) = if *before {
                    (Some(sibling.as_str()), None)
                } else {
                    (None, Some(sibling.as_str()))
                };
                (
                    raw::MOVE_SUB_ISSUE,
                    json!({ "id": parent.as_str(), "child": child.as_str(), "before": before, "after": after }),
                )
            }
            Manage::BlockedBy { issue, by, add } => {
                let by = self.issue_id(by).await?;
                let query = if *add {
                    raw::ADD_BLOCKED_BY
                } else {
                    raw::REMOVE_BLOCKED_BY
                };
                (query, json!({ "id": issue.as_str(), "by": by.as_str() }))
            }
            Manage::LinkBranch { issue, repo, name } => {
                let vars = json!({ "owner": repo.owner, "name": repo.name });
                let oid: String = (self.graphql_json(
                    raw::DEFAULT_BRANCH_OID,
                    vars,
                    format!("{repo}'s default branch"),
                    "/repository/defaultBranchRef/target/oid",
                ))
                .await?;
                (
                    raw::LINK_BRANCH,
                    json!({ "id": issue.as_str(), "oid": oid, "name": name }),
                )
            }
            Manage::CloseDiscussion { discussion, reason } => (
                raw::CLOSE_DISCUSSION,
                json!({ "id": discussion.as_str(), "reason": reason.wire() }),
            ),
            Manage::ReopenDiscussion { discussion } => {
                (raw::REOPEN_DISCUSSION, json!({ "id": discussion.as_str() }))
            }
        };
        self.mutate_json(query, vars).await
    }

    /// Reverts a merged pull request: GitHub makes a branch reverting it
    /// and a pull request of it, whose number this answers.
    pub async fn revert(&self, pr: &NodeId) -> Result<u64, ApiError> {
        let made: Value = self
            .graphql_data(raw::REVERT_PR, json!({ "id": pr.as_str() }), false)
            .await?;
        made.pointer("/revertPullRequest/revertPullRequest/number")
            .and_then(Value::as_u64)
            .ok_or_else(|| ApiError::Internal("GitHub didn't say which pull request".into()))
    }

    /// Moves an issue to another repository: where it is now.
    pub async fn transfer(&self, issue: &NodeId, to: &NodeId) -> Result<IssueAt, ApiError> {
        let vars = json!({ "id": issue.as_str(), "repo": to.as_str() });
        let made: Value = self.graphql_data(raw::TRANSFER_ISSUE, vars, false).await?;
        let issue = made.pointer("/transferIssue/issue");
        let repo = (issue.and_then(|i| i.pointer("/repository/nameWithOwner")))
            .and_then(Value::as_str)
            .and_then(RepoId::parse);
        let number = issue.and_then(|i| i.get("number")).and_then(Value::as_u64);
        match (repo, number) {
            (Some(repo), Some(number)) => Ok(IssueAt { repo, number }),
            _ => Err(ApiError::Internal(
                "GitHub didn't say where the issue went".into(),
            )),
        }
    }

    /// Starts a discussion: its number.
    pub async fn start_discussion(
        &self,
        repo: &NodeId,
        category: &NodeId,
        title: &str,
        body: &str,
    ) -> Result<u64, ApiError> {
        let vars = json!({ "repo": repo.as_str(), "category": category.as_str(), "title": title, "body": body });
        let made: Value = self
            .graphql_data(raw::START_DISCUSSION, vars, false)
            .await?;
        made.pointer("/createDiscussion/discussion/number")
            .and_then(Value::as_u64)
            .ok_or_else(|| ApiError::Internal("GitHub didn't say which discussion".into()))
    }

    /// The repositories of `owner` an issue can move to: those you can
    /// write to with issues on, most recently pushed first, of the first
    /// 100.
    pub async fn transfer_targets(&self, owner: &str) -> Result<Capped<Target>, ApiError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Repo {
            id: NodeId,
            name_with_owner: String,
            has_issues_enabled: bool,
            is_archived: bool,
            viewer_permission: Option<String>,
        }
        let found: crate::api::browse::wire::Counted<Repo> = (self.graphql_json(
            raw::TRANSFER_TARGETS,
            json!({ "login": owner }),
            format!("{owner}'s repositories"),
            "/repositoryOwner/repositories",
        ))
        .await?;
        let total = found.total_count;
        let items = (found.nodes.into_iter().flatten())
            .filter(|r| {
                let writes = matches!(
                    r.viewer_permission.as_deref(),
                    Some("ADMIN" | "MAINTAIN" | "WRITE")
                );
                writes && r.has_issues_enabled && !r.is_archived
            })
            .filter_map(|r| {
                Some(Target {
                    repo: RepoId::parse(&r.name_with_owner)?,
                    id: r.id,
                })
            })
            .collect::<Vec<_>>();
        // Those left out by permission aren't "more": only those past the
        // first page may be.
        let more = total.saturating_sub(100);
        let n = items.len() as u64;
        Ok(Capped::new(items, n + more))
    }

    /// A repository's open issues, most recently updated first: number and
    /// title.
    pub async fn open_issues(&self, repo: &RepoId) -> Result<Capped<(u64, String)>, ApiError> {
        #[derive(Deserialize)]
        struct Issue {
            number: u64,
            title: String,
        }
        let found: crate::api::browse::wire::Counted<Issue> = (self.graphql_json(
            raw::OPEN_ISSUES,
            json!({ "owner": repo.owner, "name": repo.name }),
            repo,
            "/repository/issues",
        ))
        .await?;
        Ok(found.into_capped(|i| Some((i.number, i.title))))
    }

    /// A repository's discussion categories, with their forms.
    pub async fn categories(&self, repo: &RepoId) -> Result<Categories, ApiError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire {
            id: NodeId,
            discussion_categories: crate::api::browse::wire::Counted<WireCategory>,
            templates: Option<crate::api::create::TextFiles>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct WireCategory {
            id: NodeId,
            name: String,
            slug: String,
            description: Option<String>,
            is_answerable: bool,
        }
        let found: Wire = (self.graphql_json(
            raw::DISCUSSION_FORMS,
            json!({ "owner": repo.owner, "name": repo.name }),
            repo,
            "/repository",
        ))
        .await?;
        let files = crate::api::create::TextFiles::files(found.templates);
        let form = |slug: &str| {
            (files.iter())
                .find(|(name, _)| {
                    name.rsplit_once('.').is_some_and(|(stem, ext)| {
                        stem.eq_ignore_ascii_case(slug) && matches!(ext, "yml" | "yaml")
                    })
                })
                .map(|(_, text)| text.clone())
        };
        let categories = found.discussion_categories.into_capped(|c| {
            Some(Category {
                form: form(&c.slug),
                id: c.id,
                name: c.name,
                slug: c.slug,
                description: c.description.unwrap_or_default(),
                answerable: c.is_answerable,
            })
        });
        Ok(Categories {
            repo: found.id,
            categories,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_issue_is_named_as_people_write_it() {
        let here = RepoId::new("o", "r");
        let at = |n| {
            Some(IssueAt {
                repo: here.clone(),
                number: n,
            })
        };
        assert_eq!(IssueAt::parse("#12", &here), at(12));
        assert_eq!(IssueAt::parse(" 7 ", &here), at(7));
        assert_eq!(
            IssueAt::parse("a/b#3", &here),
            Some(IssueAt {
                repo: RepoId::new("a", "b"),
                number: 3
            })
        );
        assert_eq!(
            IssueAt::parse("https://github.com/a/b/issues/9#issuecomment-1", &here),
            Some(IssueAt {
                repo: RepoId::new("a", "b"),
                number: 9
            })
        );
        assert_eq!(IssueAt::parse("#0", &here), None);
        assert_eq!(IssueAt::parse("fix it", &here), None);
    }
}
