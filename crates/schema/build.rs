fn main() {
    cynic_codegen::register_schema("github")
        .from_sdl_file("github.graphql")
        .expect("GitHub schema is valid SDL")
        .as_default()
        .expect("schema registers");
}
