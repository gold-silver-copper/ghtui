//! GitHub API access: auth, GraphQL and REST, rate limits, retries, caching.

pub mod auth;
mod client;
pub mod model;
pub mod queries;
pub mod rate_limit;

pub use client::{ApiError, GitHub, install_crypto_provider};
pub use ghtui_store::Cached;
