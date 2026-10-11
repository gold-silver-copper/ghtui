// A panic here fails the build, which is what a bad schema should do.
#![allow(
    clippy::expect_used,
    reason = "a panic fails the build, as a bad schema should"
)]

fn main() {
    // GitHub's GraphQL schema, which the `schema` module is generated from
    // and the queries are checked against. The schema checks read it too.
    let sdl = "src/schema/github.graphql";
    println!("cargo::rerun-if-changed={sdl}");
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    println!("cargo::rustc-env=GITHUB_SDL={dir}/{sdl}");
    cynic_codegen::register_schema("github")
        .from_sdl_file(sdl)
        .expect("GitHub schema is valid SDL")
        .as_default()
        .expect("schema registers");
}
