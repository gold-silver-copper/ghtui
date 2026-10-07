//! GitHub API access: auth, GraphQL and REST, rate limits, retries, caching.

pub mod auth;
pub mod browse;
mod client;
pub mod corpus;
#[doc(hidden)]
pub mod fuzz;
pub mod model;
pub mod queries;
#[cfg(test)]
mod query_check;
pub mod rate_limit;
mod raw;

pub use client::{ApiError, GitHub};
pub use ghtui_store::Cached;
