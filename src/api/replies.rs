//! Your saved replies, to insert in a comment. GitHub's API reads them
//! (with a classic token's `read:user` scope) but can't make, change or
//! delete them: that's only on github.com's settings.

use serde::{Deserialize, Serialize};

use crate::api::model::Capped;
use crate::api::{ApiError, GitHub};

/// A saved reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedReply {
    pub title: String,
    pub body: String,
}

/// Where GitHub makes, changes and deletes them.
pub fn settings_url() -> String {
    format!("{}/settings/replies", crate::host::current().web())
}

/// What a refusal to read them means.
fn refusal(err: ApiError) -> ApiError {
    match err {
        ApiError::GraphQl(messages) if messages.iter().any(|m| m.contains("read:user")) => {
            ApiError::Denied(crate::api::scope_missing("Saved replies", "read:user"))
        }
        err => err,
    }
}

impl GitHub {
    /// Your saved replies, most recently changed first.
    pub async fn saved_replies(&self) -> Result<Capped<SavedReply>, ApiError> {
        let found: crate::api::browse::wire::Counted<SavedReply> = self
            .graphql_json(
                crate::api::raw::SAVED_REPLIES,
                serde_json::json!({}),
                "your saved replies",
                "/viewer/savedReplies",
            )
            .await
            .map_err(refusal)?;
        Ok(found.into_capped(Some))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_scope_is_named() {
        let err = ApiError::GraphQl(vec![
            "The 'savedReplies' field requires one of the following scopes: ['read:user']".into(),
        ]);
        assert!(
            refusal(err)
                .to_string()
                .contains("`gh auth refresh -s read:user`")
        );
    }
}
