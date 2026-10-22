//! Following people and organizations, and your own status: what a
//! profile's Follow button and the website's avatar menu change.

use serde_json::json;

use crate::api::client::{ApiError, GitHub};
use crate::api::model::NodeId;
use crate::api::raw;

/// A status to set: what the avatar menu's dialog takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewStatus {
    /// A glyph or a `:name:`.
    pub emoji: Option<String>,
    pub message: String,
    /// Busy: you may be slow to answer.
    pub busy: bool,
    /// When it clears itself (`2026-10-21T12:00:00Z`), if it does.
    pub expires_at: Option<String>,
}

impl GitHub {
    /// Follows the user (or, `org`, the organization) `id`, or unfollows
    /// them.
    pub async fn follow(&self, id: &NodeId, org: bool, on: bool) -> Result<(), ApiError> {
        let query = match (org, on) {
            (false, true) => raw::FOLLOW_USER,
            (false, false) => raw::UNFOLLOW_USER,
            (true, true) => raw::FOLLOW_ORG,
            (true, false) => raw::UNFOLLOW_ORG,
        };
        self.mutate_json(query, json!({ "id": id })).await
    }

    /// Sets your status to `status`, or clears it (`None`).
    pub async fn set_status(&self, status: Option<&NewStatus>) -> Result<(), ApiError> {
        let vars = match status {
            Some(s) => json!({
                "emoji": s.emoji,
                "message": s.message,
                "busy": s.busy,
                "expires": s.expires_at,
            }),
            None => json!({ "emoji": null, "message": null, "busy": false, "expires": null }),
        };
        self.mutate_json(raw::CHANGE_STATUS, vars).await
    }
}
