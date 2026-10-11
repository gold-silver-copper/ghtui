//! A corpus of real GitHub responses, recorded by the contract job for
//! the requests ghtui makes and replayed by offline tests, so they check
//! ghtui against what GitHub really sends rather than what a test's author
//! believes it sends.
//!
//! Each response is a JSON file named by its request's [`key`]: the
//! method, the path (with its query) and, for a POST, its body. Responses
//! are kept whole, but for one thing: a comparison's files lose their
//! `patch`, which ghtui doesn't read and which makes a big comparison
//! megabytes.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One recorded response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recorded {
    /// `GET /repos/o/r` or `POST /graphql`, for reading.
    pub request: String,
    /// A POST's body (a GraphQL query and its variables).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
    pub status: u16,
    /// The `Link` header, which pages some lists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    pub response: String,
}

/// A request's file name: a stable hash (FNV-1a) of its method, path and
/// body, so a replay finds what a recording wrote.
pub fn key(method: &str, path: &str, body: Option<&serde_json::Value>) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let body = body.map(serde_json::Value::to_string).unwrap_or_default();
    for byte in [method, " ", path, " ", &body].concat().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}.json")
}

/// Where responses are recorded, when they are.
#[derive(Debug, Clone)]
pub struct Recorder(pub PathBuf);

impl Recorder {
    /// Writes a response, logging (not failing) if it can't.
    pub(crate) fn record(&self, recorded: &Recorded, body: Option<&serde_json::Value>) {
        let mut recorded = recorded.clone();
        if recorded.request.contains("/compare/")
            && let Ok(mut json) = serde_json::from_str::<serde_json::Value>(&recorded.response)
        {
            for file in json
                .get_mut("files")
                .and_then(serde_json::Value::as_array_mut)
                .into_iter()
                .flatten()
            {
                if let Some(file) = file.as_object_mut() {
                    file.remove("patch");
                }
            }
            recorded.response = json.to_string();
        }
        let recorded = &recorded;
        let (method, path) = recorded
            .request
            .split_once(' ')
            .unwrap_or(("GET", &recorded.request));
        let file = self.0.join(key(method, path, body));
        let written = serde_json::to_string_pretty(recorded)
            .map_err(|e| e.to_string())
            .and_then(|json| std::fs::write(&file, json).map_err(|e| e.to_string()));
        if let Err(err) = written {
            tracing::warn!(err, file = %file.display(), "couldn't record a response");
        }
    }
}

/// Reads a recorded response.
pub fn read(dir: &Path, file: &str) -> Option<Recorded> {
    let text = std::fs::read_to_string(dir.join(file)).ok()?;
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The key mustn't change between Rust versions or runs: the corpus
    /// is committed under it.
    #[test]
    fn keys_are_stable() {
        // FNV-1a of "GET /user ", worked out apart from this code.
        assert_eq!(key("GET", "/user", None), "46fbc3d342fd9773.json");
        let body = serde_json::json!({"query": "q", "variables": {"b": 1, "a": 2}});
        assert_eq!(
            key("POST", "/graphql", Some(&body)),
            key("POST", "/graphql", Some(&body.clone()))
        );
        assert_ne!(key("GET", "/a", None), key("GET", "/b", None));
    }
}
