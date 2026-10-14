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

pub use crate::store::Cached;
pub use client::{ApiError, GitHub, Polled};
