//! GitHub's code search, for the files a name may be defined in.

use crate::api::browse::rest_search::{Code, Page};
use crate::api::{ApiError, GitHub};

/// What code search found: the files' paths, best first, how many files
/// it found in all, and whether it gave up before it was done.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodeHits {
    pub paths: Vec<String>,
    pub total: u64,
    pub incomplete: bool,
}

impl GitHub {
    /// The first ten files GitHub's code search finds for `query`. Its
    /// API reads only default branches, and searches text: it has no
    /// `symbol:`. A search that timed out says so here, as the answer.
    pub async fn code_search(&self, query: &str) -> Result<CodeHits, ApiError> {
        let q: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
        let path = format!("/search/code?q={q}&per_page=10");
        let body = self.rest_get(&path, "the code search").await?;
        let page: Page<Code> = serde_json::from_str(&body)?;
        Ok(CodeHits {
            paths: page.items.into_iter().map(|c| c.path).collect(),
            total: page.total_count,
            incomplete: page.incomplete_results,
        })
    }
}
