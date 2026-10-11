// A panic here fails the build, which is what a bad schema should do.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "a panic fails the build, as a bad schema should"
)]

fn main() {
    // Where the schema is, for the crates that depend on this one (Cargo
    // hands it to their build scripts as DEP_GHTUI_SCHEMA_SDL).
    let sdl =
        std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("github.graphql");
    println!("cargo::metadata=sdl={}", sdl.display());
    cynic_codegen::register_schema("github")
        .from_sdl_file("github.graphql")
        .expect("GitHub schema is valid SDL")
        .as_default()
        .expect("schema registers");
}
