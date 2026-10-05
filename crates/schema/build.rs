// A panic here fails the build, which is what a bad schema should do.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "a panic fails the build, as a bad schema should"
)]

fn main() {
    cynic_codegen::register_schema("github")
        .from_sdl_file("github.graphql")
        .expect("GitHub schema is valid SDL")
        .as_default()
        .expect("schema registers");
}
