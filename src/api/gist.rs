//! Making, changing and deleting your gists.

use serde::Deserialize;

use crate::api::client::{ApiError, GitHub, encode_path};

/// A gist to make: its description, whether it's public, and its files'
/// names and text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewGist {
    pub description: String,
    pub public: bool,
    pub files: Vec<(String, String)>,
}

/// A change to one of a gist's files: by the name it had, its new name
/// and text, or `None` to remove it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub was: String,
    pub now: Option<(String, String)>,
}

/// A change to a gist: its description, files changed or removed, and
/// files added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GistChange {
    pub id: String,
    pub description: String,
    pub changed: Vec<FileChange>,
    pub added: Vec<(String, String)>,
}

/// What GitHub made.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MadeGist {
    pub id: String,
    pub owner: Option<Owner>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Owner {
    pub login: String,
}

impl GitHub {
    pub async fn create_gist(&self, new: &NewGist) -> Result<MadeGist, ApiError> {
        let files: serde_json::Map<String, serde_json::Value> = (new.files.iter())
            .map(|(name, text)| (name.clone(), serde_json::json!({ "content": text })))
            .collect();
        let body = serde_json::json!({
            "description": new.description,
            "public": new.public,
            "files": files,
        });
        self.rest_write_json(http::Method::POST, "/gists", Some(&body))
            .await
    }

    /// Changes a gist. GitHub's API can't change whether it's public.
    pub async fn update_gist(&self, change: &GistChange) -> Result<(), ApiError> {
        let mut files = serde_json::Map::new();
        for c in &change.changed {
            let value = match &c.now {
                Some((name, text)) => serde_json::json!({ "filename": name, "content": text }),
                None => serde_json::Value::Null,
            };
            files.insert(c.was.clone(), value);
        }
        for (name, text) in &change.added {
            files.insert(name.clone(), serde_json::json!({ "content": text }));
        }
        let body = serde_json::json!({ "description": change.description, "files": files });
        let path = format!("/gists/{}", encode_path(&change.id));
        self.rest_write(http::Method::PATCH, &path, Some(&body))
            .await
    }

    pub async fn delete_gist(&self, id: &str) -> Result<(), ApiError> {
        let path = format!("/gists/{}", encode_path(id));
        self.rest_write(http::Method::DELETE, &path, None).await
    }
}
