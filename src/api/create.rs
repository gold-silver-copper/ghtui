//! Making issues and pull requests: what a new one's form offers (labels,
//! assignees, milestones, templates, and what you may set), the commits a
//! pull request would hold, and the mutations that make them.

use crate::schema::schema;

use crate::api::client::{ApiError, GitHub};
use crate::api::model::{Capped, NodeId, RepoId};
use crate::git::local::CommitMessage;

/// What a new issue or pull request in a repository can have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewForm {
    pub repo_id: NodeId,
    pub default_branch: Option<String>,
    /// GitHub lets you open issues here.
    pub can_create_issues: bool,
    pub has_issues: bool,
    pub archived: bool,
    /// Labels, assignees and a milestone take triage access: GitHub drops
    /// them from anyone else's new issue.
    pub can_triage: bool,
    pub labels: Capped<Choice>,
    pub assignees: Capped<Choice>,
    pub milestones: Capped<Choice>,
    /// The files in `.github/ISSUE_TEMPLATE`: names and text.
    pub issue_templates: Vec<(String, String)>,
    /// The pull request template, from wherever GitHub finds it.
    pub pr_template: Option<String>,
}

/// A label, a person or a milestone to pick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub id: NodeId,
    /// A label's or milestone's name, a login.
    pub name: String,
    /// A label's description, a person's name, a milestone's due date.
    pub about: String,
    /// A label's color.
    pub color: Option<String>,
}

/// A new issue, as it's sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewIssue {
    pub repo: NodeId,
    pub title: String,
    pub body: String,
    pub labels: Vec<NodeId>,
    pub assignees: Vec<NodeId>,
    pub milestone: Option<NodeId>,
}

/// A new pull request, as it's sent: `head` is `owner:branch` when it's a
/// fork's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPr {
    pub repo: NodeId,
    pub base: String,
    pub head: String,
    pub title: String,
    pub body: String,
    pub draft: bool,
}

pub(crate) use wire::TextFiles;

mod wire {
    use serde::Deserialize;

    pub use crate::api::browse::wire::{Counted, Name};

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Repo {
        pub id: String,
        pub viewer_can_create_issues: bool,
        pub has_issues_enabled: bool,
        pub is_archived: bool,
        pub viewer_permission: Option<String>,
        pub default_branch_ref: Option<Name>,
        pub labels: Option<Counted<Label>>,
        pub assignable_users: Counted<User>,
        pub milestones: Option<Counted<Milestone>>,
        pub issue_templates: Option<TextFiles>,
        pub pull_request_templates: Option<Vec<Template>>,
    }

    #[derive(Deserialize)]
    pub struct Label {
        pub id: String,
        pub name: String,
        pub color: String,
        pub description: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct User {
        pub id: String,
        pub login: String,
        pub name: Option<String>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Milestone {
        pub id: String,
        pub title: String,
        pub due_on: Option<String>,
    }

    /// A directory's files; anything else (no directory there) has none.
    #[derive(Deserialize)]
    pub(crate) struct TextFiles {
        entries: Option<Vec<Entry>>,
    }

    #[derive(Deserialize)]
    struct Entry {
        name: String,
        object: Option<Blob>,
    }

    #[derive(Deserialize)]
    struct Blob {
        text: Option<String>,
    }

    impl TextFiles {
        /// Each text file's name and text.
        pub(crate) fn files(tree: Option<Self>) -> Vec<(String, String)> {
            (tree.and_then(|t| t.entries).into_iter().flatten())
                .filter_map(|e| Some((e.name, e.object?.text?)))
                .collect()
        }
    }

    #[derive(Deserialize)]
    pub struct Template {
        pub filename: Option<String>,
        pub body: Option<String>,
    }

    /// A REST comparison's commits, with their whole messages.
    #[derive(Deserialize)]
    pub struct Compare {
        pub total_commits: u64,
        pub commits: Vec<Commit>,
    }

    #[derive(Deserialize)]
    pub struct Commit {
        pub sha: String,
        pub commit: Message,
    }

    #[derive(Deserialize)]
    pub struct Message {
        pub message: String,
    }
}

fn capped<T, U>(c: Option<wire::Counted<T>>, mut f: impl FnMut(T) -> U) -> Capped<U> {
    c.map_or_else(Capped::default, |c| c.into_capped(|t| Some(f(t))))
}

impl GitHub {
    /// What a new issue or pull request in `repo` can have.
    pub async fn new_form(&self, repo: &RepoId) -> Result<NewForm, ApiError> {
        let vars = serde_json::json!({ "owner": repo.owner, "name": repo.name });
        let r: wire::Repo =
            (self.graphql_json(crate::api::raw::NEW_FORM, vars, repo, "/repository")).await?;
        let can_triage = matches!(
            r.viewer_permission.as_deref(),
            Some("ADMIN" | "MAINTAIN" | "WRITE" | "TRIAGE")
        );
        let issue_templates = wire::TextFiles::files(r.issue_templates);
        // GitHub's own choice of template, when it finds one: the first.
        let pr_template = (r.pull_request_templates.unwrap_or_default().into_iter())
            .filter(|t| t.filename.is_some())
            .find_map(|t| t.body);
        Ok(NewForm {
            repo_id: NodeId::new(r.id),
            default_branch: r.default_branch_ref.map(|b| b.name),
            can_create_issues: r.viewer_can_create_issues,
            has_issues: r.has_issues_enabled,
            archived: r.is_archived,
            can_triage,
            labels: capped(r.labels, |l| Choice {
                id: NodeId::new(l.id),
                name: l.name,
                about: l.description.unwrap_or_default(),
                color: Some(l.color),
            }),
            assignees: capped(Some(r.assignable_users), |u| Choice {
                id: NodeId::new(u.id),
                name: u.login,
                about: u.name.unwrap_or_default(),
                color: None,
            }),
            milestones: capped(r.milestones, |m| Choice {
                id: NodeId::new(m.id),
                name: m.title,
                about: (m.due_on.as_deref())
                    .and_then(|d| d.get(..10))
                    .map(|d| format!("due {d}"))
                    .unwrap_or_default(),
                color: None,
            }),
            issue_templates,
            pr_template,
        })
    }

    /// The commits `head` has that `base` doesn't, oldest first, with
    /// their messages (the newest 250, as GitHub compares).
    pub async fn commit_messages(
        &self,
        repo: &RepoId,
        base: &str,
        head: &str,
    ) -> Result<Capped<CommitMessage>, ApiError> {
        use crate::ui::pages::url::encode_path;
        // A fork's branch is `owner:branch`.
        let head = match head.split_once(':') {
            Some((owner, branch)) => format!("{}:{}", encode_path(owner), encode_path(branch)),
            None => encode_path(head),
        };
        let path = format!(
            "/repos/{}/{}/compare/{}...{head}",
            repo.owner,
            repo.name,
            encode_path(base)
        );
        let what = format!("{repo}'s comparison {base}...{head}");
        let wire: wire::Compare = serde_json::from_str(&self.rest_get(&path, what).await?)?;
        let commits = (wire.commits.into_iter())
            .map(|c| CommitMessage {
                oid: c.sha,
                message: c.commit.message,
            })
            .collect();
        Ok(Capped::new(commits, wire.total_commits))
    }

    /// Opens an issue; its number.
    pub async fn create_issue(&self, new: &NewIssue) -> Result<u64, ApiError> {
        use cynic::MutationBuilder;
        let gql = |ids: &[NodeId]| Some(ids.iter().map(NodeId::gql).collect());
        let op = CreateIssue::build(CreateIssueVariables {
            repo: new.repo.gql(),
            title: new.title.clone(),
            body: new.body.clone(),
            labels: gql(&new.labels),
            assignees: gql(&new.assignees),
            milestone: new.milestone.as_ref().map(NodeId::gql),
        });
        let made = self.mutate(op).await?;
        let issue = made.create_issue.and_then(|p| p.issue);
        number(issue.map(|i| i.number), "issue")
    }

    /// Opens a pull request; its number.
    pub async fn create_pull_request(&self, new: &NewPr) -> Result<u64, ApiError> {
        use cynic::MutationBuilder;
        let op = CreatePr::build(CreatePrVariables {
            repo: new.repo.gql(),
            base: new.base.clone(),
            head: new.head.clone(),
            title: new.title.clone(),
            body: new.body.clone(),
            draft: new.draft,
        });
        let made = self.mutate(op).await?;
        let pr = made.create_pull_request.and_then(|p| p.pull_request);
        number(pr.map(|p| p.number), "pull request")
    }
}

/// The number GitHub gave what it made.
fn number(n: Option<i32>, what: &str) -> Result<u64, ApiError> {
    let n = n.ok_or_else(|| ApiError::Decode(format!("GitHub didn't say which {what} it made")))?;
    u64::try_from(n).map_err(|_| ApiError::Decode(format!("a {what} numbered {n}")))
}

#[derive(cynic::QueryVariables, Debug)]
pub struct CreateIssueVariables {
    pub repo: cynic::Id,
    pub title: String,
    pub body: String,
    pub labels: Option<Vec<cynic::Id>>,
    pub assignees: Option<Vec<cynic::Id>>,
    pub milestone: Option<cynic::Id>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "CreateIssueVariables")]
pub struct CreateIssue {
    #[arguments(input: { repositoryId: $repo, title: $title, body: $body, labelIds: $labels, assigneeIds: $assignees, milestoneId: $milestone })]
    pub create_issue: Option<CreateIssuePayload>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct CreateIssuePayload {
    pub issue: Option<Numbered>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Issue")]
pub struct Numbered {
    pub number: i32,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct CreatePrVariables {
    pub repo: cynic::Id,
    pub base: String,
    pub head: String,
    pub title: String,
    pub body: String,
    pub draft: bool,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Mutation", variables = "CreatePrVariables")]
pub struct CreatePr {
    #[arguments(input: { repositoryId: $repo, baseRefName: $base, headRefName: $head, title: $title, body: $body, draft: $draft })]
    pub create_pull_request: Option<CreatePrPayload>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "CreatePullRequestPayload")]
pub struct CreatePrPayload {
    pub pull_request: Option<NumberedPr>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "PullRequest")]
pub struct NumberedPr {
    pub number: i32,
}
