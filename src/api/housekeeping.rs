//! Repository housekeeping on GitHub: labels, milestones, branches and
//! tags, forks, new repositories, and files committed. What git does in
//! your clone is [`crate::housekeeping::git`]'s; this is what GitHub's API
//! does where there's no clone, and what only GitHub knows.

use serde::{Deserialize, Serialize};

use crate::api::browse::{Results, wire};
use crate::api::client::{ApiError, GitHub, encode_path};
use crate::api::model::{NodeId, RepoId};
use crate::api::raw::raw;
use crate::ui::pages::url;

// ---- labels ---------------------------------------------------------------------------

/// A repository's label, as its labels page lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelInfo {
    pub name: String,
    /// `rrggbb`, as GitHub keeps it.
    pub color: String,
    pub description: String,
    /// Its open issues and pull requests.
    pub open: u64,
}

/// A label as it's to be: made, or what one becomes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelChange {
    pub name: String,
    /// `rrggbb`, without `#`.
    pub color: String,
    pub description: String,
}

raw! {
    /// A repository's labels by name, 100 at a time from `after`, with how
    /// many open issues and pull requests each has.
    LABELS = "query($owner: String!, $name: String!, $after: String) { repository(owner: $owner, name: $name) { labels(first: 100, after: $after, orderBy: {field: NAME, direction: ASC}) { totalCount pageInfo { hasNextPage endCursor } nodes { name color description issues(states: OPEN) { totalCount } pullRequests(states: OPEN) { totalCount } } } } }";

    /// Who you may make a repository for: you, and the organizations that
    /// let you.
    OWNERS = "query { viewer { login organizations(first: 100) { totalCount nodes { login viewerCanCreateRepositories } } } }";

    /// What a file's form starts from: the repository, the branch's tip (or
    /// the commit a tag or SHA names), and the file there.
    FILE_LOOK = "query($owner: String!, $name: String!, $qualified: String!, $rev: String!, $spec: String!) { repository(owner: $owner, name: $name) { id ref(qualifiedName: $qualified) { target { oid } } commit: object(expression: $rev) { ... on Commit { oid } ... on Tag { target { oid } } } file: object(expression: $spec) { ... on Blob { oid text isBinary isTruncated } } } }";

    /// One commit on a branch, made only if the branch is still at
    /// `expectedHeadOid`.
    COMMIT_ON_BRANCH = "mutation($input: CreateCommitOnBranchInput!) { createCommitOnBranch(input: $input) { commit { oid } } }";
}

// ---- milestones --------------------------------------------------------------------

/// A milestone as it's to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MilestoneChange {
    pub title: String,
    pub description: String,
    /// `YYYY-MM-DD`; none for no due date.
    pub due_on: Option<String>,
}

/// What GitHub made: its number.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MadeMilestone {
    pub number: u64,
}

// ---- branches and tags ---------------------------------------------------------------

words! {
    /// Whether a branch is a tag's or a branch's, for its ref's name.
    #[derive(Hash)]
    pub enum RefKind {
        /// Its refs' prefix below `refs/`.
        pub fn prefix,
        pub fn noun,
    }
    Branch "heads" "branch";
    Tag    "tags"  "tag";
}

impl RefKind {
    /// `refs/heads/name`.
    pub fn full(self, name: &str) -> String {
        format!("refs/{}/{name}", self.prefix())
    }
}

/// What GitHub's rules say of deleting a branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchRules {
    /// A branch protection rule covers it.
    pub protected: bool,
    /// A ruleset forbids deleting it.
    pub undeletable: bool,
}

// ---- forks and new repositories ---------------------------------------------------------

/// Who a repository can be made for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owners {
    pub viewer: String,
    /// Organizations, and whether you may make repositories there.
    pub orgs: Vec<(String, bool)>,
    /// Organizations GitHub didn't list (past its first 100).
    pub more: u64,
}

/// GitHub's templates for a new repository: `.gitignore`s by name, and
/// licences by key and name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Templates {
    pub gitignores: Vec<String>,
    pub licenses: Vec<(String, String)>,
}

/// A fork to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewFork {
    pub repo: RepoId,
    /// An organization; none for your account.
    pub org: Option<String>,
    pub name: String,
    pub default_branch_only: bool,
}

/// A repository to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRepo {
    /// An organization; none for your account.
    pub org: Option<String>,
    pub name: String,
    pub description: String,
    pub private: bool,
    pub readme: bool,
    pub gitignore: Option<String>,
    pub license: Option<String>,
}

/// A repository GitHub made (or is making: a fork arrives later).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MadeRepo {
    pub full_name: String,
    pub clone_url: String,
}

impl MadeRepo {
    pub fn repo(&self) -> Option<RepoId> {
        RepoId::parse(&self.full_name)
    }
}

// ---- files -------------------------------------------------------------------------------------

/// Where a file's form starts: the repository's ID, the branch's tip (or
/// the commit a tag or SHA names: then it isn't a branch), and the file
/// there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileLook {
    pub repo_id: NodeId,
    /// The branch's tip, when the revision is a branch.
    pub branch_head: Option<String>,
    /// The commit the revision names.
    pub commit: String,
    pub file: Option<FileAt>,
}

/// A file at a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileAt {
    /// Its blob: what says whether it changed.
    pub oid: String,
    /// `None` for a binary file, or one too big for GitHub's API.
    pub text: Option<String>,
}

/// One commit of files on a branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCommit {
    pub repo: RepoId,
    pub branch: String,
    /// The tip it's made on: GitHub refuses it if the branch moved since.
    pub head: String,
    pub headline: String,
    pub body: String,
    /// Paths and their new text.
    pub additions: Vec<(String, String)>,
    pub deletions: Vec<String>,
}

/// The token's `workflow` scope, which changing a workflow's file takes.
pub fn workflow_scoped(err: ApiError) -> ApiError {
    let lacks = |m: &str| m.contains("workflow") && m.contains("scope");
    let denied = || {
        ApiError::Denied(crate::api::scope_missing(
            "Changing a workflow's file",
            "workflow",
        ))
    };
    match &err {
        ApiError::GraphQl(m) if m.iter().any(|m| lacks(m)) => denied(),
        ApiError::Http { message, .. } if lacks(message) => denied(),
        _ => err,
    }
}

impl GitHub {
    /// A repository's labels by name, 100 at a time from `after`.
    pub async fn labels(
        &self,
        repo: &RepoId,
        after: Option<String>,
    ) -> Result<Results<LabelInfo>, ApiError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire {
            name: String,
            color: String,
            description: Option<String>,
            issues: wire::Count,
            pull_requests: wire::Count,
        }
        let vars = serde_json::json!({ "owner": repo.owner, "name": repo.name, "after": after });
        let page: wire::Connection<Wire> = self
            .graphql_json(LABELS, vars, repo, "/repository/labels")
            .await?;
        self.check_connection(&page, 100, format!("{repo}'s labels"));
        let labels = page.into_results(|l| LabelInfo {
            name: l.name,
            color: l.color,
            description: l.description.unwrap_or_default(),
            open: l.issues.total_count + l.pull_requests.total_count,
        });
        Ok(labels)
    }

    pub async fn create_label(&self, repo: &RepoId, label: &LabelChange) -> Result<(), ApiError> {
        let body = serde_json::json!({
            "name": label.name,
            "color": label.color,
            "description": label.description,
        });
        let path = format!("/repos/{repo}/labels");
        self.rest_write(http::Method::POST, &path, Some(&body))
            .await
    }

    /// Changes label `was` into `label`: its name, colour and description.
    pub async fn update_label(
        &self,
        repo: &RepoId,
        was: &str,
        label: &LabelChange,
    ) -> Result<(), ApiError> {
        let body = serde_json::json!({
            "new_name": label.name,
            "color": label.color,
            "description": label.description,
        });
        let path = format!("/repos/{repo}/labels/{}", encode_path(was));
        self.rest_write(http::Method::PATCH, &path, Some(&body))
            .await
    }

    pub async fn delete_label(&self, repo: &RepoId, name: &str) -> Result<(), ApiError> {
        let path = format!("/repos/{repo}/labels/{}", encode_path(name));
        self.rest_write(http::Method::DELETE, &path, None).await
    }

    pub async fn create_milestone(
        &self,
        repo: &RepoId,
        m: &MilestoneChange,
    ) -> Result<MadeMilestone, ApiError> {
        let path = format!("/repos/{repo}/milestones");
        self.rest_write_json(http::Method::POST, &path, Some(&milestone_body(m)))
            .await
    }

    pub async fn update_milestone(
        &self,
        repo: &RepoId,
        number: u64,
        m: &MilestoneChange,
    ) -> Result<(), ApiError> {
        let path = format!("/repos/{repo}/milestones/{number}");
        self.rest_write(http::Method::PATCH, &path, Some(&milestone_body(m)))
            .await
    }

    /// Closes milestone `number`, or reopens it.
    pub async fn set_milestone_closed(
        &self,
        repo: &RepoId,
        number: u64,
        closed: bool,
    ) -> Result<(), ApiError> {
        let state = if closed { "closed" } else { "open" };
        let body = serde_json::json!({ "state": state });
        let path = format!("/repos/{repo}/milestones/{number}");
        self.rest_write(http::Method::PATCH, &path, Some(&body))
            .await
    }

    pub async fn delete_milestone(&self, repo: &RepoId, number: u64) -> Result<(), ApiError> {
        let path = format!("/repos/{repo}/milestones/{number}");
        self.rest_write(http::Method::DELETE, &path, None).await
    }

    /// The commit `rev` (a branch, a tag or a SHA) names, as GitHub has it.
    pub async fn commit_of(&self, repo: &RepoId, rev: &str) -> Result<String, ApiError> {
        #[derive(Deserialize)]
        struct Wire {
            sha: String,
        }
        let path = format!("/repos/{repo}/commits/{}", url::encode_path(rev));
        let wire: Wire = self.rest_json(&path, format!("{rev} in {repo}")).await?;
        Ok(wire.sha)
    }

    /// Makes branch or tag `name` at commit `sha`.
    pub async fn create_ref(
        &self,
        repo: &RepoId,
        kind: RefKind,
        name: &str,
        sha: &str,
    ) -> Result<(), ApiError> {
        let body = serde_json::json!({ "ref": kind.full(name), "sha": sha });
        let path = format!("/repos/{repo}/git/refs");
        self.rest_write(http::Method::POST, &path, Some(&body))
            .await
    }

    pub async fn delete_ref(
        &self,
        repo: &RepoId,
        kind: RefKind,
        name: &str,
    ) -> Result<(), ApiError> {
        let path = format!(
            "/repos/{repo}/git/refs/{}/{}",
            kind.prefix(),
            url::encode_path(name)
        );
        self.rest_write(http::Method::DELETE, &path, None).await
    }

    /// Whether GitHub's rules keep branch `name` from being deleted: a
    /// branch protection rule, or a ruleset's.
    pub async fn branch_rules(&self, repo: &RepoId, name: &str) -> Result<BranchRules, ApiError> {
        #[derive(Deserialize)]
        struct Branch {
            protected: bool,
        }
        #[derive(Deserialize)]
        struct Rule {
            #[serde(rename = "type")]
            kind: String,
        }
        let path = format!("/repos/{repo}/branches/{}", url::encode_path(name));
        let branch: Branch = self.rest_json(&path, name).await?;
        // Rulesets: GitHub lists the rules that apply to the branch.
        let path = format!("/repos/{repo}/rules/branches/{}", url::encode_path(name));
        let rules: Vec<Rule> = match self.rest_json(&path, name).await {
            Ok(rules) => rules,
            // An older Enterprise Server has no rulesets.
            Err(ApiError::NotFound(_)) => Vec::new(),
            Err(err) => return Err(err),
        };
        Ok(BranchRules {
            protected: branch.protected,
            undeletable: rules.iter().any(|r| r.kind == "deletion"),
        })
    }

    /// You, and the organizations you're in.
    pub async fn owners(&self) -> Result<Owners, ApiError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Org {
            login: String,
            viewer_can_create_repositories: bool,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Viewer {
            login: String,
            organizations: wire::Counted<Org>,
        }
        let viewer: Viewer = self
            .graphql_json(OWNERS, serde_json::json!({}), "your account", "/viewer")
            .await?;
        let orgs: Vec<(String, bool)> = (viewer.organizations.nodes.into_iter().flatten())
            .map(|o| (o.login, o.viewer_can_create_repositories))
            .collect();
        let more = (viewer.organizations.total_count).saturating_sub(orgs.len() as u64);
        Ok(Owners {
            viewer: viewer.login,
            orgs,
            more,
        })
    }

    /// GitHub's `.gitignore` and licence templates.
    pub async fn templates(&self) -> Result<Templates, ApiError> {
        #[derive(Deserialize)]
        struct License {
            key: String,
            name: String,
        }
        let (gitignores, licenses) = tokio::try_join!(
            self.rest_json("/gitignore/templates", "templates"),
            self.rest_json::<Vec<License>>("/licenses", "licences"),
        )?;
        Ok(Templates {
            gitignores,
            licenses: licenses.into_iter().map(|l| (l.key, l.name)).collect(),
        })
    }

    pub async fn fork(&self, new: &NewFork) -> Result<MadeRepo, ApiError> {
        let mut body = serde_json::json!({
            "name": new.name,
            "default_branch_only": new.default_branch_only,
        });
        if let (Some(org), Some(map)) = (&new.org, body.as_object_mut()) {
            map.insert("organization".into(), org.clone().into());
        }
        let path = format!("/repos/{}/forks", new.repo);
        self.rest_write_json(http::Method::POST, &path, Some(&body))
            .await
    }

    pub async fn create_repo(&self, new: &NewRepo) -> Result<MadeRepo, ApiError> {
        let mut body = serde_json::json!({
            "name": new.name,
            "description": new.description,
            "private": new.private,
            "auto_init": new.readme,
        });
        if let Some(map) = body.as_object_mut() {
            if let Some(g) = &new.gitignore {
                map.insert("gitignore_template".into(), g.clone().into());
            }
            if let Some(l) = &new.license {
                map.insert("license_template".into(), l.clone().into());
            }
        }
        let path = match &new.org {
            Some(org) => format!("/orgs/{}/repos", encode_path(org)),
            None => "/user/repos".to_owned(),
        };
        self.rest_write_json(http::Method::POST, &path, Some(&body))
            .await
    }

    /// Where a file's form starts: `rev`'s commit (a branch's tip when
    /// it's a branch), and the file at `path` there, if there's one.
    pub async fn file_look(
        &self,
        repo: &RepoId,
        rev: &str,
        path: &str,
    ) -> Result<FileLook, ApiError> {
        #[derive(Deserialize)]
        struct Oid {
            oid: String,
        }
        #[derive(Deserialize)]
        struct Target {
            target: Oid,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Commit {
            Commit(Oid),
            Tag(Target),
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Blob {
            /// None when the path is a folder.
            oid: Option<String>,
            text: Option<String>,
            is_binary: Option<bool>,
            #[serde(default)]
            is_truncated: bool,
        }
        #[derive(Deserialize)]
        struct Wire {
            id: NodeId,
            #[serde(rename = "ref")]
            branch: Option<Target>,
            commit: Option<Commit>,
            file: Option<Blob>,
        }
        let vars = serde_json::json!({
            "owner": repo.owner,
            "name": repo.name,
            "qualified": format!("refs/heads/{rev}"),
            "rev": rev,
            "spec": format!("{rev}:{path}"),
        });
        let wire: Wire = self
            .graphql_json(FILE_LOOK, vars, repo, "/repository")
            .await?;
        let commit = match wire.commit {
            Some(Commit::Commit(c)) => c.oid,
            Some(Commit::Tag(t)) => t.target.oid,
            None => return Err(ApiError::NotFound(format!("{rev} in {repo}"))),
        };
        let file = (wire.file).and_then(|b| {
            Some(FileAt {
                oid: b.oid?,
                text: (b.text).filter(|_| !b.is_truncated && b.is_binary != Some(true)),
            })
        });
        Ok(FileLook {
            repo_id: wire.id,
            branch_head: wire.branch.map(|b| b.target.oid),
            commit,
            file,
        })
    }

    /// Commits files on a branch, as GitHub's website does: only while
    /// the branch is still at the tip it was made on.
    pub async fn commit_files(&self, c: &FileCommit) -> Result<(), ApiError> {
        use base64::Engine;
        let b64 = |text: &str| base64::engine::general_purpose::STANDARD.encode(text);
        let additions: Vec<serde_json::Value> = (c.additions.iter())
            .map(|(path, text)| serde_json::json!({ "path": path, "contents": b64(text) }))
            .collect();
        let deletions: Vec<serde_json::Value> = (c.deletions.iter())
            .map(|path| serde_json::json!({ "path": path }))
            .collect();
        let mut message = serde_json::json!({ "headline": c.headline });
        if !c.body.is_empty()
            && let Some(map) = message.as_object_mut()
        {
            map.insert("body".into(), c.body.clone().into());
        }
        let input = serde_json::json!({
            "branch": {
                "repositoryNameWithOwner": c.repo.to_string(),
                "branchName": c.branch,
            },
            "expectedHeadOid": c.head,
            "message": message,
            "fileChanges": { "additions": additions, "deletions": deletions },
        });
        let vars = serde_json::json!({ "input": input });
        self.mutate_json(COMMIT_ON_BRANCH, vars)
            .await
            .map_err(workflow_scoped)
    }
}

/// A milestone's fields as GitHub's REST API takes them. A due date is
/// sent at noon UTC, which is that day wherever GitHub reads it.
fn milestone_body(m: &MilestoneChange) -> serde_json::Value {
    let due = m.due_on.as_ref().map(|d| format!("{d}T12:00:00Z"));
    serde_json::json!({
        "title": m.title,
        "description": m.description,
        "due_on": due,
    })
}
