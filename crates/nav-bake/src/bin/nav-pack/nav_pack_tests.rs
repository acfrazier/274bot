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
fn explicit_377_selects_its_own_cache_and_decoded_snapshot() {
    let inputs = parse_args([
        "--revision",
        "377",
        "--content",
        "/fixture/377/content",
        "--cache",
        "/fixture/377/cache",
        "--cache-manifest",
        "/fixture/cache-377.json",
        "--server-loc-dat",
        "/fixture/377/engine/data/pack/server/loc.dat",
        "--snapshot-root",
        "/fixture/377/unpack",
        "--out",
        "/fixture/nav/377.navpack",
    ])
    .unwrap();
    assert_eq!(inputs.revision, Some(377));
    assert_eq!(inputs.maps_dir, PathBuf::from("/fixture/377/content/maps"));
    assert_eq!(
        inputs.config_jag,
        PathBuf::from("/fixture/377/cache/config")
    );
    assert_eq!(
        inputs.snapshot_root,
        Some(PathBuf::from("/fixture/377/unpack"))
    );
    assert_eq!(
        inputs.server_loc_dat_377,
        Some(PathBuf::from("/fixture/377/engine/data/pack/server/loc.dat"))
    );
}

#[test]
fn explicit_377_requires_an_engine_server_table_without_inferring_cache_layout() {
    let inputs = [
        "--revision", "377", "--content", "/content", "--cache", "/published-cache",
        "--cache-manifest", "/cache.json", "--out", "/nav/377.navpack",
    ];
    let error = parse_args(inputs).unwrap_err();
    assert!(error.contains("requires --server-loc-dat"), "{error}");
    for revision in ["274", "289"] {
        let error = parse_args([
            "--revision", revision, "--content", "/content", "--cache", "/cache",
            "--cache-manifest", "/cache.json", "--out", "/nav/output",
            "--server-loc-dat", "/server/loc.dat",
        ]).unwrap_err();
        assert!(error.contains("only supported for revision 377"), "{error}");
    }
    let error = parse_args([
        "--revision", "377", "--content", "/content", "--cache", "/published-cache",
        "--cache-manifest", "/cache.json", "--out", "/nav/377.navpack",
        "--server-loc-dat", "/server/loc.dat", "--server-loc-dat", "/other/loc.dat",
    ]).unwrap_err();
    assert!(error.contains("duplicate explicit option --server-loc-dat"), "{error}");
}

#[test]
fn explicit_mode_rejects_ambiguous_or_incomplete_inputs() {
    for args in [
        vec!["--revision", "289"],
        vec![
            "--revision",
            "378",
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
fn legacy_default_paths_follow_explicit_engine_dir() {
    let engine_dir = PathBuf::from("/fixture/274/engine");
    let (maps, doors, config) = legacy_default_paths(Some(engine_dir)).unwrap();
    assert_eq!(maps, PathBuf::from("/fixture/274/content/maps"));
    assert_eq!(
        doors,
        PathBuf::from("/fixture/274/content/scripts/doors/configs")
    );
    assert_eq!(
        config,
        PathBuf::from("/fixture/274/engine/data/pack/config")
    );
}

#[test]
fn legacy_default_paths_require_an_engine_root() {
    let error = legacy_default_paths(None).unwrap_err();
    assert!(error.contains("BOT_NAV_ENGINE_DIR"));
    assert!(error.contains("ENGINE_DIR"));
}

#[test]
fn cli_bake_rejects_a_missing_door_config_before_baking() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "274bot-nav-pack-strict-{}-{unique}",
        std::process::id()
    ));
    let maps = root.join("content/maps");
    let doors = root.join("content/scripts/doors/configs");
    std::fs::create_dir_all(&maps).unwrap();
    std::fs::create_dir_all(&doors).unwrap();
    std::fs::write(root.join("config.jag"), b"").unwrap();
    std::fs::write(doors.join("doors.loc"), "[loc_100]\nop1=Open\n").unwrap();
    let inputs = parse_args([
        maps.to_string_lossy().into_owned(),
        doors.to_string_lossy().into_owned(),
        root.join("config.jag").to_string_lossy().into_owned(),
    ])
    .unwrap();
    let gates = root.join("content/scripts/general_use/configs/gates.loc");
    let error = bake_world(&inputs, &gates, None)
        .err()
        .expect("missing door config must fail");
    assert!(error.contains("doubledoors.loc"), "{error}");
    std::fs::remove_dir_all(root).unwrap();
}
