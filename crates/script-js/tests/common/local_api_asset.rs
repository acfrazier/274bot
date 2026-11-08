pub fn local_api_asset(relative_path: &str, description: &str) -> bool {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../api")
        .join(relative_path);
    let path = path.to_string_lossy();
    api::test_support::local_asset_path("", &path, description).is_some()
}
