//! GitHub's public GraphQL schema, which the queries are typed against.
//! Refresh `schema/github.graphql` from
//! <https://docs.github.com/public/fpt/schema.docs.graphql>.

#[cynic::schema("github")]
pub mod schema {}
