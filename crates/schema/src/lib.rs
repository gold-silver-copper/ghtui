//! GitHub's public GraphQL schema, compiled once.
//!
//! The generated module is large; keeping it in its own crate means query
//! changes in `ghtui-api` never recompile it. Refresh `github.graphql` from
//! <https://docs.github.com/public/fpt/schema.docs.graphql>.

#[cynic::schema("github")]
pub mod schema {}
