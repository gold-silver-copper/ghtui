//! GitHub API access: auth, GraphQL and REST, rate limits, retries, caching.

pub mod auth;
pub mod browse;
pub mod change;
mod client;
pub mod code_search;
pub mod commit_comments;
pub mod corpus;
pub mod create;
pub mod discussion;
pub mod edit;
pub mod enterprise;
pub mod explore;
#[doc(hidden)]
pub mod fuzz;
pub mod gist;
pub mod housekeeping;
pub mod insights;
pub mod lists;
pub mod manage;
pub mod mention;
pub mod model;
pub mod notifications;
pub mod packages;
mod person;
pub mod project;
pub mod queries;
#[cfg(test)]
mod query_check;
pub mod rate_limit;
mod raw;
pub mod reach;
pub mod release;
pub mod replies;
pub mod security;
pub mod settings;
pub mod timeline;
pub mod watch;
pub mod workflows;

pub use client::{ApiError, GitHub, Polled};

/// The `gh` command that adds `scope` to the token, naming an Enterprise
/// host.
pub fn refresh(scope: &str) -> String {
    match crate::host::current() {
        h if h.is_enterprise() => format!("gh auth refresh --hostname {} -s {scope}", h.name()),
        _ => format!("gh auth refresh -s {scope}"),
    }
}

/// What to say when the token lacks the `scope` that `what` takes.
pub fn scope_missing(what: &str, scope: &str) -> String {
    format!(
        "{what}: your token lacks the {scope} scope; `{}` adds it",
        refresh(scope)
    )
}

/// Whether a classic token's scopes (GitHub's `x-oauth-scopes`, comma
/// separated) include `scope`.
pub fn has_scope(scopes: &str, scope: &str) -> bool {
    scopes.split(',').any(|s| s.trim() == scope)
}
