//! Writing in a discussion: comments (answers, in a Q&A category) and
//! replies in their threads, editing and deleting them, and marking the
//! answer. Each is a [`Discuss`], sent as a
//! [`Change::Discussion`](crate::api::change::Change::Discussion).
//! Reactions are [`crate::api::edit::Edit::React`], as anywhere.

use crate::api::client::{ApiError, GitHub};
use crate::api::model::NodeId;
use crate::api::raw;

/// A change to a discussion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Discuss {
    /// A comment on `discussion`, or a reply in the thread of `reply_to`
    /// (a top-level comment).
    Comment {
        discussion: NodeId,
        reply_to: Option<NodeId>,
        body: String,
    },
    /// New text for the post (`post`), or for a comment or reply.
    Edit {
        id: NodeId,
        post: bool,
        body: String,
    },
    /// Delete a comment or reply.
    Delete { id: NodeId },
    /// Mark a comment the answer (`on`), or unmark it.
    Answer { comment: NodeId, on: bool },
}

impl GitHub {
    pub(crate) async fn discuss(&self, change: &Discuss) -> Result<(), ApiError> {
        let (query, vars) = match change {
            Discuss::Comment {
                discussion,
                reply_to,
                body,
            } => (
                raw::ADD_DISCUSSION_COMMENT,
                serde_json::json!({
                    "discussion": discussion.as_str(),
                    "replyTo": reply_to.as_ref().map(NodeId::as_str),
                    "body": body,
                }),
            ),
            Discuss::Edit { id, post, body } => (
                if *post {
                    raw::UPDATE_DISCUSSION
                } else {
                    raw::UPDATE_DISCUSSION_COMMENT
                },
                serde_json::json!({ "id": id.as_str(), "body": body }),
            ),
            Discuss::Delete { id } => (
                raw::DELETE_DISCUSSION_COMMENT,
                serde_json::json!({ "id": id.as_str() }),
            ),
            Discuss::Answer { comment, on } => (
                if *on {
                    raw::MARK_ANSWER
                } else {
                    raw::UNMARK_ANSWER
                },
                serde_json::json!({ "id": comment.as_str() }),
            ),
        };
        self.mutate_json(query, vars).await
    }
}
