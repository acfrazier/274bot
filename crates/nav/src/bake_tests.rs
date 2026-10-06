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

    let input_sha = "56".repeat(32);
    let pack_sha = "ab".repeat(32);
    let flags_sha = "cd".repeat(32);
    let reach_sha = "ef".repeat(32);
    let canlight_sha = "12".repeat(32);
    let manifest_sha = "13".repeat(32);
    let inputs = [crate::bundle::InputFingerprint {
        path: "/content/maps/m1.jm2".into(),
        bytes: 10,
        modified_nanos: 5,
        sha256: input_sha.clone(),
    }];
    let baked = crate::bundle::BakeStamp {
        content_id: None,
        source_sha256: None,
        generator: warm,
        format: "274V16".into(),
        revision: 289,
        cache_id: "cache-1".into(),
        cache_manifest: None,
        nav_sha256: pack_sha.clone(),
        flags_sha256: flags_sha.clone(),
        reach_sha256: reach_sha.clone(),
        canlight_sha256: canlight_sha.clone(),
        canlight_identity: "34".repeat(32),
        pack_bytes: 11,
        flags_bytes: 7,
        reach_bytes: 9,
        canlight_bytes: 5,
        manifest_sha256: Some(manifest_sha.clone()),
        manifest_bytes: Some(13),
        relative_pack: "nav/289/274bot.navpack".into(),
        relative_flags: "nav/289/274bot.navflags".into(),
        relative_reach: "nav/289/274bot.navreach".into(),
        relative_canlight: "nav/289/274bot.navcanlight".into(),
        relative_manifest: Some("nav/289/274bot.navpack.json".into()),
        pois_sha256: Some("56".repeat(32)),
        pois_bytes: Some(3),
        relative_pois: Some("nav/289/274bot.navpois".into()),
        pois_generator: Some("pois-gen".into()),
        inputs: inputs.to_vec(),
    };
    let expected = crate::bundle::StampExpectation {
        revision: 289,
        format: "274V16",
        generator: &after_router,
        cache_id: "cache-1",
        inputs: &inputs,
        staged_pack_bytes: Some(11),
        staged_pack_sha256: Some(&pack_sha),
        staged_flags_bytes: Some(7),
        staged_flags_sha256: Some(&flags_sha),
        staged_reach_bytes: Some(9),
        staged_reach_sha256: Some(&reach_sha),
        staged_canlight_bytes: Some(5),
        staged_canlight_sha256: Some(&canlight_sha),
        staged_manifest_bytes: Some(13),
        staged_manifest_sha256: Some(&manifest_sha),
        staged_pois_bytes: Some(3),
        staged_pois_sha256: Some(
            "5656565656565656565656565656565656565656565656565656565656565656",
        ),
        pois_generator: "pois-gen",
    };
    let error = baked
        .covers(&expected)
        .expect_err("router source change must fail covers and force reach rebake");
    assert!(error.contains("generator"), "{error}");
}

#[test]
fn a_missing_door_config_fails_the_bake() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "274bot-nav-missing-door-{}-{unique}",
        std::process::id()
    ));
    let maps = root.join("content/maps");
    let doors = root.join("content/scripts/doors/configs");
    let config_jag = root.join("engine/data/pack/config");
    std::fs::create_dir_all(&doors).unwrap();
    std::fs::create_dir_all(config_jag.parent().unwrap()).unwrap();
    std::fs::write(doors.join("doors.loc"), "[loc_100]\nop1=Open\n").unwrap();
    std::fs::write(&config_jag, b"").unwrap();
    let gates = root.join("content/scripts/general_use/configs/gates.loc");
    let request = BakeRequest {
        revision: None,
        maps_dir: &maps,
        doors_dir: &doors,
        gates: &gates,
        config_jag: &config_jag,
        cache: None,
        require_all_door_configs: true,
        input_fingerprints: None,
        content_id: None,
    };
    let error = bake_world(&request)
        .err()
        .expect("a bake without all door inputs");
    assert!(error.contains("doubledoors.loc"), "{error}");
    std::fs::remove_dir_all(root).unwrap();
}

fn collision_with_flags(
    width: usize,
    height: usize,
    entries: &[(usize, usize, u32)],
) -> WorldCollision {
    let mut flags = vec![0u32; 4 * width * height];
    for &(x, z, face) in entries {
        flags[z * width + x] |= face;
    }
    let (walk, blocked) = crate::collision::pack_walk(&flags);
    WorldCollision {
        origin: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        width,
        height,
        walk,
        blocked,
        flags: None,
    }
}

#[test]
fn stationary_melee_shape_uses_exact_wall_faces_and_open_door_faces() {
    let spawn = WorldTile {
        x: 4,
        z: 3,
        level: 0,
    };
    let north_wall = collision_with_flags(8, 8, &[(4, 3, CollisionFlag::W_N as u32)]);
    let north_wall_shape = stationary_melee_shape(&north_wall, spawn, 1, &HashSet::new()).unwrap();
    assert_ne!(north_wall_shape & (1u64 << 3), 0);
    assert_eq!(north_wall_shape & (1u64 << 7), 0);

    let west_wall = collision_with_flags(8, 8, &[(4, 3, CollisionFlag::W_W as u32)]);
    let closed_shape = stationary_melee_shape(&west_wall, spawn, 1, &HashSet::new()).unwrap();
    assert_eq!(closed_shape & (1u64 << 3), 0);

    let door_faces = openable_door_faces(&[crate::map::services::OpenableDoor {
        x: 3,
        z: 3,
        level: 0,
        shape: LocShape::WALL_STRAIGHT as u8,
        rotation: LocAngle::EAST as u8,
    }])
    .unwrap();
    let open_shape = stationary_melee_shape(&west_wall, spawn, 1, &door_faces).unwrap();
    assert_ne!(open_shape & (1u64 << 3), 0);
}

#[test]
fn fixed_ranged_hunter_carves_occluded_tiles_but_retains_visible_range() {
    let spawn = WorldTile {
        x: 12,
        z: 12,
        level: 0,
    };
    let mut collision = collision_with_flags(24, 24, &[]);
    // A permanent range-blocking ridge, unlike walk-only ground collision.
    let mut flags = vec![0u32; 4 * 24 * 24];
    for z in 0..24 {
        flags[z * 24 + 8] = CollisionFlag::V_E as u32;
        flags[z * 24 + 9] = CollisionFlag::V_W as u32;
    }
    collision.attach_flags(flags);
    let zone = Zone::npc(spawn, 8, ZoneClass::Always, u16::MAX, 0);
    let mut carves = Vec::new();
    append_ranged_visibility_carves(&collision, &zone, 0, &mut carves).unwrap();
    let table = ZoneTable::from_parts(
        vec![zone],
        vec![ZoneKind::new(
            "thrower",
            "Thrower Troll",
            1101,
            67,
            true,
            false,
        )],
        vec![],
        carves,
        vec![],
        collision.origin,
        24,
        24,
        &crate::transport::WildernessRules::default(),
    )
    .unwrap();
    assert_eq!(
        table.at(spawn).count(),
        1,
        "the NPC and visible plateau stay dangerous"
    );
    assert_eq!(table.at(WorldTile { x: 9, ..spawn }).count(), 1);
    assert_eq!(
        table.at(WorldTile { x: 8, ..spawn }).count(),
        0,
        "the ridge blocks acquisition"
    );
    assert_eq!(table.at(WorldTile { x: 4, ..spawn }).count(), 0);
    assert_eq!(
        table.at(WorldTile { x: 20, ..spawn }).count(),
        1,
        "inclusive hunt range remains eight"
    );
    assert_eq!(table.at(WorldTile { x: 21, ..spawn }).count(), 0);
    collision.drop_flags();
    assert!(append_ranged_visibility_carves(&collision, &zone, 0, &mut Vec::new()).is_err());
}
