//! Comments on commits: on a commit as a whole, and on its diff's lines.
//! GitHub's GraphQL can read them but not write them, so both go through
//! REST.

use serde::{Deserialize, Serialize};

use crate::api::model::{Capped, NodeId, RepoId, ReviewComment, ReviewThread, Side};
use crate::api::{ApiError, GitHub};

/// A comment on a commit.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitComment {
    /// GitHub's number for it, which its link's anchor uses
    /// (`#commitcomment-…`).
    pub id: u64,
    pub node_id: String,
    pub author: String,
    pub body: String,
    /// ISO 8601.
    pub created_at: String,
    /// The file it's on, if it's on one.
    pub path: Option<String>,
    /// The line of the file (in the commit) it's on, if it's on one.
    pub line: Option<u32>,
    pub url: String,
}

impl CommitComment {
    /// Its anchor on the commit's page.
    pub fn anchor(&self) -> String {
        format!("commitcomment-{}", self.id)
    }

    /// A comment on a line as a thread of one, read-only, for the
    /// commit's diff to show where it is.
    pub fn as_thread(&self) -> Option<ReviewThread> {
        let path = self.path.clone()?;
        let comment = ReviewComment {
            id: NodeId::new(&self.node_id),
            author: self.author.clone(),
            body: self.body.clone(),
            created_at: self.created_at.clone(),
            url: self.url.clone(),
            original_commit: None,
            pending: false,
            editable: crate::api::edit::Editable::default(),
            review: None,
        };
        Some(ReviewThread {
            id: NodeId::new(&self.node_id),
            path,
            side: Side::Right,
            start_side: None,
            line: self.line,
            start_line: None,
            original_line: self.line,
            original_start_line: None,
            outdated: false,
            resolved: false,
            file_level: self.line.is_none(),
            can_reply: false,
            can_resolve: false,
            can_unresolve: false,
            comments: vec![comment].into(),
            diff_hunk: String::new(),
        })
    }
}

/// REST's commit comment.
#[derive(Deserialize)]
pub(crate) struct Wire {
    id: u64,
    node_id: String,
    user: Option<crate::api::browse::UserLogin>,
    body: String,
    created_at: String,
    path: Option<String>,
    line: Option<u32>,
    html_url: String,
}

impl Wire {
    pub(crate) fn into_comment(self) -> CommitComment {
        CommitComment {
            id: self.id,
            node_id: self.node_id,
            author: self.user.map_or_else(|| "ghost".to_owned(), |u| u.login),
            body: self.body,
            created_at: self.created_at,
            path: self.path.filter(|p| !p.is_empty()),
            line: self.line,
            url: self.html_url,
        }
    }
}

/// How many a page of them holds; more than that says it left some out.
const PAGE: usize = 100;

impl GitHub {
    /// A commit's comments, oldest first: the first hundred.
    pub async fn commit_comments(
        &self,
        repo: &RepoId,
        oid: &str,
    ) -> Result<Capped<CommitComment>, ApiError> {
        let path = format!("/repos/{repo}/commits/{oid}/comments?per_page={PAGE}");
        let wire: Vec<Wire> = self
            .rest_json(&path, format!("{repo}@{oid}'s comments"))
            .await?;
        let full = wire.len() == PAGE;
        let comments: Vec<CommitComment> = wire.into_iter().map(Wire::into_comment).collect();
        // A full page may not be all of them.
        let total = comments.len() as u64 + u64::from(full);
        Ok(Capped::new(comments, total))
    }

    /// Comments on a commit as a whole.
    pub(crate) async fn comment_on_commit(
        &self,
        repo: &RepoId,
        oid: &str,
        body: &str,
    ) -> Result<(), ApiError> {
        let path = format!("/repos/{repo}/commits/{oid}/comments");
        let body = serde_json::json!({ "body": body });
        self.rest_write(http::Method::POST, &path, Some(&body))
            .await
    }
}
