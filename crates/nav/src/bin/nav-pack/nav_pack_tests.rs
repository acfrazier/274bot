use super::*;

#[test]
fn explicit_289_inputs_bind_content_cache_config_and_outputs() {
    let inputs = parse_args([
        "--revision",
        "289",
        "--content",
        "/fixture/289/content",
        "--cache",
        "/fixture/289/engine/data/pack/client",
        "--cache-manifest",
        "/fixture/cache-289.json",
        "--out",
        "/tmp/289.navpack",
    ])
    .unwrap();
    assert_eq!(inputs.revision, Some(289));
    assert_eq!(inputs.maps_dir, PathBuf::from("/fixture/289/content/maps"));
    assert_eq!(
        inputs.doors_dir,
        PathBuf::from("/fixture/289/content/scripts/doors/configs")
    );
    assert_eq!(
        inputs.config_jag,
        PathBuf::from("/fixture/289/engine/data/pack/client/config")
    );
    assert_eq!(
        inputs.cache_dir,
        Some(PathBuf::from("/fixture/289/engine/data/pack/client"))
    );
    assert_eq!(
        inputs.cache_manifest,
        Some(PathBuf::from("/fixture/cache-289.json"))
    );
    assert_eq!(inputs.out, PathBuf::from("/tmp/289.navpack"));
    assert_eq!(inputs.flags_out, PathBuf::from("/tmp/289.navflags"));
    assert_eq!(inputs.reach_out, PathBuf::from("/tmp/289.navreach"));
    assert_eq!(inputs.canlight_out, PathBuf::from("/tmp/289.navcanlight"));
    assert_eq!(inputs.pois_out, PathBuf::from("/tmp/289.navpois"));
}

#[test]
fn explicit_274_uses_server_config_next_to_client_cache() {
    let inputs = parse_args([
        "--revision",
        "274",
        "--content",
        "/fixture/274/content",
        "--cache",
        "/fixture/274/engine/data/pack/client",
        "--cache-manifest",
        "/fixture/cache-274.json",
        "--out",
        "/tmp/274.navpack",
    ])
    .unwrap();
    assert_eq!(
        inputs.config_jag,
        PathBuf::from("/fixture/274/engine/data/pack/config")
    );
}

#[test]
fn explicit_mode_rejects_ambiguous_or_incomplete_inputs() {
    for args in [
        vec!["--revision", "289"],
        vec![
            "--revision",
            "377",
            "--content",
            "/content",
            "--cache",
            "/cache",
            "--cache-manifest",
            "/cache.json",
            "--out",
            "/tmp/x",
        ],
        vec![
            "--revision",
            "289",
            "--content",
            "/content",
            "--cache",
            "/cache",
            "--cache-manifest",
            "/cache.json",
            "--out",
            "/tmp/x",
            "positional",
        ],
        vec![
            "--revision",
            "274",
            "--revision",
            "289",
            "--content",
            "/content",
            "--cache",
            "/cache",
            "--cache-manifest",
            "/cache.json",
            "--out",
            "/tmp/x",
        ],
    ] {
        assert!(parse_args(args).is_err());
    }
}

#[test]
fn legacy_274_positionals_remain_unmanifested() {
    let inputs = parse_args(["/legacy/maps", "/legacy/doors", "/legacy/config"]).unwrap();
    assert_eq!(inputs.revision, None);
    assert_eq!(inputs.maps_dir, PathBuf::from("/legacy/maps"));
    assert_eq!(inputs.doors_dir, PathBuf::from("/legacy/doors"));
    assert_eq!(inputs.config_jag, PathBuf::from("/legacy/config"));
    assert!(inputs.cache_dir.is_none());
    assert!(inputs.cache_manifest.is_none());
    // `gates.loc` follows the maps dir's parent, the content root.
    assert_eq!(
        content_inputs(&inputs.content_dir).gates,
        PathBuf::from("/legacy/scripts/general_use/configs/gates.loc")
    );
}

#[test]
fn flags_path_for_swaps_pack_extension() {
    assert_eq!(
        flags_path_for(&PathBuf::from("/tmp/x/274bot.navpack")),
        PathBuf::from("/tmp/x/274bot.navflags")
    );
}

#[test]
fn default_paths_follow_engine_dir() {
    let content = client::bot_target::content_dir();
    assert_eq!(default_maps_dir(), content.join("maps"));
    assert_eq!(default_doors_dir(), content.join("scripts/doors/configs"));
    assert_eq!(
        default_config_jag(),
        client::engine_dir().join("data/pack/config")
    );
}
