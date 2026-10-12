//! GitHub API access: auth, GraphQL and REST, rate limits, retries, caching.

pub mod auth;
pub mod browse;
pub mod change;
mod client;
pub mod code_search;
pub mod corpus;
pub mod create;
pub mod discussion;
pub mod edit;
#[doc(hidden)]
pub mod fuzz;
pub mod mention;
pub mod model;
pub mod notifications;
mod person;
pub mod project;
pub mod queries;
#[cfg(test)]
mod query_check;
pub mod rate_limit;
mod raw;
pub mod security;

pub use crate::store::Cached;
pub use client::{ApiError, GitHub, Polled};
