use super::*;

#[test]
fn content_inputs_follow_the_content_root() {
    let inputs = content_inputs(Path::new("/content"));
    assert_eq!(inputs.maps_dir, PathBuf::from("/content/maps"));
    assert_eq!(
        inputs.doors_dir,
        PathBuf::from("/content/scripts/doors/configs")
    );
    assert_eq!(
        inputs.gates,
        PathBuf::from("/content/scripts/general_use/configs/gates.loc")
    );
}

#[test]
fn the_289_config_jag_is_inside_the_cache_and_274_beside_it() {
    assert_eq!(
        config_jag_for(289, Path::new("/289/engine/data/pack/client")).unwrap(),
        PathBuf::from("/289/engine/data/pack/client/config")
    );
    assert_eq!(
        config_jag_for(274, Path::new("/274/engine/data/pack/client")).unwrap(),
        PathBuf::from("/274/engine/data/pack/config")
    );
}

#[test]
fn generator_identity_tracks_the_manual_id_format_and_sources() {
    let base = generator_identity(&[("src/pack.rs", "fn a() {}")]);
    assert_eq!(base.len(), 64);
    assert!(base.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(base, generator_identity(&[("src/pack.rs", "fn b() {}")]));
    assert_ne!(
        base,
        generator_identity(&[("src/collision.rs", "fn a() {}")])
    );
    // Order and labels are part of the identity.
    assert_ne!(
        generator_identity(&[("a.rs", "x"), ("b.rs", "y")]),
        generator_identity(&[("b.rs", "x"), ("a.rs", "y")])
    );
}

#[test]
fn router_source_bytes_invalidate_a_warm_reach_stamp() {
    // bake_reach floods with router::step_ok; a movement change must not
    // keep a warm-stamped 274R sidecar while pack/flags stamps still match.
    assert!(
        GENERATOR_SOURCES.contains(&"src/router.rs"),
        "GENERATOR_SOURCES must include the step_ok-owning source"
    );
    assert!(GENERATOR_SOURCES.contains(&"src/paint.rs"));
    assert!(
        GENERATOR_SOURCES.contains(&"src/canlight.rs"),
        "GENERATOR_SOURCES must include the canlight-owning source"
    );
    assert!(
        GENERATOR_SOURCES.contains(&"src/transport/condparse.rs"),
        "GENERATOR_SOURCES must include the transport condparse-owning source"
    );

    let baseline: Vec<(&str, &str)> = GENERATOR_SOURCES
        .iter()
        .map(|path| (*path, "fn a() {}"))
        .collect();
    let mut router_only = baseline.clone();
    for (label, text) in &mut router_only {
        if *label == "src/router.rs" {
            *text = "fn step_ok_changed() {}";
        }
    }
    assert_eq!(
        router_only
            .iter()
            .find(|(path, _)| *path == "src/paint.rs")
            .map(|(_, text)| *text),
        Some("fn a() {}"),
        "paint.rs bytes stay the same; only router.rs changes"
    );

    let warm = generator_identity(&baseline);
    let after_router = generator_identity(&router_only);
    assert_ne!(
        warm, after_router,
        "router.rs bytes join generator_identity"
    );

    let inputs = [crate::bundle::InputFingerprint {
        path: "/content/maps/m1.jm2".into(),
        bytes: 10,
        modified_nanos: 5,
    }];
    let baked = crate::bundle::BakeStamp {
        content_id: None,
        source_sha256: None,
        generator: warm,
        format: "274V8".into(),
        revision: 289,
        cache_id: "cache-1".into(),
        cache_manifest: None,
        nav_sha256: "ab".repeat(32),
        flags_sha256: "cd".repeat(32),
        reach_sha256: "ef".repeat(32),
        canlight_sha256: "12".repeat(32),
        canlight_identity: "34".repeat(32),
        pack_bytes: 11,
        flags_bytes: 7,
        reach_bytes: 9,
        canlight_bytes: 5,
        relative_pack: "nav/289/274bot.navpack".into(),
        relative_flags: "nav/289/274bot.navflags".into(),
        relative_reach: "nav/289/274bot.navreach".into(),
        relative_canlight: "nav/289/274bot.navcanlight".into(),
        pois_sha256: Some("56".repeat(32)),
        pois_bytes: Some(3),
        relative_pois: Some("nav/289/274bot.navpois".into()),
        pois_generator: Some("pois-gen".into()),
        inputs: inputs.to_vec(),
    };
    let expected = crate::bundle::StampExpectation {
        revision: 289,
        format: "274V8",
        generator: &after_router,
        cache_id: "cache-1",
        inputs: &inputs,
        staged_pack_bytes: Some(11),
        staged_flags_bytes: Some(7),
        staged_reach_bytes: Some(9),
        staged_canlight_bytes: Some(5),
        staged_pois_bytes: Some(3),
        pois_generator: "pois-gen",
    };
    let error = baked
        .covers(&expected)
        .expect_err("router source change must fail covers and force reach rebake");
    assert!(error.contains("generator"), "{error}");
}

#[test]
fn a_missing_canonical_input_fails_the_bake() {
    let request = BakeRequest {
        revision: None,
        maps_dir: Path::new("/nonexistent/content/maps"),
        doors_dir: Path::new("/nonexistent/content/scripts/doors/configs"),
        gates: Path::new("/nonexistent/content/scripts/general_use/configs/gates.loc"),
        config_jag: Path::new("/nonexistent/engine/data/pack/config"),
        cache: None,
        require_all_door_configs: true,
        content_id: None,
    };
    let error = bake_world(&request).err().expect("a bake without inputs");
    assert!(error.contains("doors.loc"), "{error}");
}
