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

struct SyntheticHunterContent {
    root: PathBuf,
    has_door: bool,
}

impl SyntheticHunterContent {
    fn new(wanderrange: i32, stationary: bool, has_door: bool) -> Self {
        let movement = if stationary {
            "moverestrict=nomove\n"
        } else {
            "defaultmode=normal\n"
        };
        Self::with_thrower_movement(wanderrange, movement, has_door)
    }

    /// The thrower row takes `movement` verbatim (e.g. a `moverestrict=` line).
    fn with_thrower_movement(wanderrange: i32, movement: &str, has_door: bool) -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "274bot-hunter-zones-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let fixture = Self { root, has_door };
        fixture.write(
            "pack/npc.pack",
            "96=white_wolf\n917=draynor_guard\n1101=thrower\n",
        );
        fixture.write(
            "scripts/configs/hunters.npc",
            &format!(
                "[thrower]\nhuntmode=aggressive_ranged\nhuntrange=8\nwanderrange={wanderrange}\nmaxrange=20\nattackrange=8\n{movement}\
                 [white_wolf]\nhuntmode=support\nhuntrange=1\nwanderrange=0\nmaxrange=1\n\
                 [draynor_guard]\nhuntmode=support\nhuntrange=1\nwanderrange=0\nmaxrange=1\n"
            ),
        );
        fixture.write(
            "scripts/configs/hunters.hunt",
            "[aggressive_ranged]\ntype=player\ncheck_nottoostrong=off\nfind_newmode=applayer2\ncheck_vis=lineofsight\n\
             [support]\ntype=player\ncheck_nottoostrong=off\nfind_newmode=opplayer2\n",
        );
        let door = if has_door { "0 26 14: 200 0 1\n" } else { "" };
        fixture.write(
            "maps/m44_56.jm2",
            &format!("==== MAP ====\n==== LOC ====\n{door}==== NPC ====\n0 35 14: 1101\n"),
        );
        fixture.write(
            "maps/m44_54.jm2",
            "==== MAP ====\n==== LOC ====\n==== NPC ====\n0 35 42: 96\n",
        );
        fixture.write(
            "maps/m48_50.jm2",
            "==== MAP ====\n==== LOC ====\n==== NPC ====\n0 28 40: 917\n",
        );
        fixture
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
}

impl Drop for SyntheticHunterContent {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn synthetic_hunter_npc(id: i32) -> client::config::NpcType {
    client::config::NpcType {
        id,
        name: format!("Synthetic hunter {id}"),
        vislevel: 67,
        size: 1,
        ..client::config::NpcType::default()
    }
}

fn collision_with_hunter_ridge() -> WorldCollision {
    let mut collision = collision_with_flags(64, 64, &[]);
    collision.origin = WorldTile {
        x: 2828,
        z: 3575,
        level: 0,
    };
    let wall_x = usize::try_from(2847 - collision.origin.x).unwrap();
    let mut flags = vec![0u32; 4 * 64 * 64];
    for z in 0..64 {
        flags[z * 64 + wall_x] = CollisionFlag::V_E as u32;
        flags[z * 64 + wall_x + 1] = CollisionFlag::V_W as u32;
    }
    collision.attach_flags(flags);
    collision
}

fn derive_synthetic_hunter_zones(
    content: &SyntheticHunterContent,
    collision: &WorldCollision,
) -> DerivedZones {
    let door_ids = if content.has_door {
        HashSet::from([200])
    } else {
        HashSet::new()
    };
    derive_synthetic_hunter_zones_with(content, collision, &door_ids, &HashSet::new())
}

fn derive_synthetic_hunter_zones_with(
    content: &SyntheticHunterContent,
    collision: &WorldCollision,
    door_ids: &HashSet<i32>,
    opened_door_ids: &HashSet<i32>,
) -> DerivedZones {
    let npc_types = [
        synthetic_hunter_npc(96),
        synthetic_hunter_npc(917),
        synthetic_hunter_npc(1101),
    ];
    derive_zone_table(
        &content.root,
        collision,
        &crate::transport::TransportGraph::default(),
        &npc_types,
        door_ids,
        opened_door_ids,
    )
    .unwrap()
}

fn synthetic_thrower_zone_index(table: &ZoneTable) -> usize {
    table
        .zones()
        .iter()
        .position(|zone| zone.spawn_x == 2851 && zone.spawn_z == 3598)
        .expect("synthetic thrower zone")
}

#[test]
fn derive_zone_table_keeps_full_rectangle_for_wandering_los_hunter() {
    let content = SyntheticHunterContent::new(4, false, false);
    let collision = collision_with_hunter_ridge();
    let derived = derive_synthetic_hunter_zones(&content, &collision);
    let zone_index = synthetic_thrower_zone_index(&derived.table);
    let zone = &derived.table.zones()[zone_index];

    assert_eq!(
        (zone.min_x, zone.min_z, zone.max_x, zone.max_z),
        (2839, 3586, 2863, 3610),
        "wandering hunter keeps its full range-derived rectangle"
    );
    assert!(
        derived
            .table
            .carves()
            .iter()
            .all(|(index, _)| usize::from(*index) != zone_index),
        "a wandering LOS hunter walks past a sight-only ridge, so nothing is carved"
    );
    assert_eq!(
        derived
            .table
            .at(WorldTile {
                x: 2839,
                z: 3598,
                level: 0,
            })
            .count(),
        1,
        "the sight-only ridge does not remove any tile from the wandering hunter zone"
    );
}

#[test]
fn derive_zone_table_suppresses_all_carves_for_nearby_openable_door() {
    let content = SyntheticHunterContent::new(0, true, true);
    let collision = collision_with_hunter_ridge();
    let derived = derive_synthetic_hunter_zones(&content, &collision);
    let zone_index = synthetic_thrower_zone_index(&derived.table);
    let zone = &derived.table.zones()[zone_index];

    assert_eq!(
        (zone.min_x, zone.min_z, zone.max_x, zone.max_z),
        (2843, 3590, 2859, 3606),
        "the openable door is at x=min_x-1, inside the expanded envelope"
    );
    assert!(
        derived
            .table
            .carves()
            .iter()
            .all(|(index, _)| usize::from(*index) != zone_index),
        "one nearby openable door suppresses every visibility carve for this zone"
    );
    assert_eq!(
        derived
            .table
            .at(WorldTile {
                x: 2843,
                z: 3598,
                level: 0,
            })
            .count(),
        1,
        "the blocked boundary tile remains in the conservative zone"
    );
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

/// A walk- and sight-blocking scenery ridge along x=2847, covering the whole
/// grid so a hunter cannot walk around either end of it.
fn collision_with_scenery_ridge() -> WorldCollision {
    let mut collision = collision_with_flags(128, 128, &[]);
    collision.origin = WorldTile {
        x: 2800,
        z: 3550,
        level: 0,
    };
    let ridge_x = usize::try_from(2847 - collision.origin.x).unwrap();
    let mut flags = vec![0u32; 4 * 128 * 128];
    for z in 0..128 {
        flags[z * 128 + ridge_x] =
            CollisionFlag::WALK_SCENERY as u32 | CollisionFlag::VIS_SCENERY as u32;
    }
    collision.attach_flags(flags);
    collision
}

fn thrower_tile_count(table: &ZoneTable, x: i32) -> usize {
    table
        .at(WorldTile {
            x,
            z: 3598,
            level: 0,
        })
        .count()
}

#[test]
fn derive_zone_table_carves_mobile_hunter_tiles_it_cannot_reach_or_see() {
    let content = SyntheticHunterContent::new(4, false, false);
    let collision = collision_with_scenery_ridge();
    let derived = derive_synthetic_hunter_zones(&content, &collision);
    let zone_index = synthetic_thrower_zone_index(&derived.table);
    let zone = &derived.table.zones()[zone_index];
    assert_eq!(
        (zone.min_x, zone.min_z, zone.max_x, zone.max_z),
        (2839, 3586, 2863, 3610),
        "the range-derived rectangle is unchanged; only membership narrows"
    );
    assert!(derived
        .table
        .carves()
        .iter()
        .any(|(index, _)| usize::from(*index) == zone_index));
    for x in 2839..=2847 {
        assert_eq!(
            thrower_tile_count(&derived.table, x),
            0,
            "x={x} lies behind a ridge the hunter can neither cross nor see through"
        );
    }
    for x in 2848..=2863 {
        assert_eq!(thrower_tile_count(&derived.table, x), 1, "x={x}");
    }
}

#[test]
fn derive_zone_table_keeps_mobile_rectangle_near_any_door_state() {
    let collision = collision_with_scenery_ridge();
    let closed = SyntheticHunterContent::new(4, false, true);
    let opened = SyntheticHunterContent::new(4, false, true);
    for derived in [
        derive_synthetic_hunter_zones_with(
            &closed,
            &collision,
            &HashSet::from([200]),
            &HashSet::new(),
        ),
        derive_synthetic_hunter_zones_with(
            &opened,
            &collision,
            &HashSet::new(),
            &HashSet::from([200]),
        ),
    ] {
        let zone_index = synthetic_thrower_zone_index(&derived.table);
        assert!(
            derived
                .table
                .carves()
                .iter()
                .all(|(index, _)| usize::from(*index) != zone_index),
            "a door a player can open or close keeps the whole rectangle"
        );
        assert_eq!(thrower_tile_count(&derived.table, 2846), 1);
    }
}

#[test]
fn npc_steps_follow_engine_take_step_for_large_npcs() {
    // A scenery column at x=6 with a one-tile gap at z=5.
    let mut flags = vec![0u32; 4 * 12 * 12];
    for z in (0..12).filter(|z| *z != 5) {
        flags[z * 12 + 6] = CollisionFlag::WALK_SCENERY as u32;
    }
    let flag = |x: i32, z: i32| {
        if (0..12).contains(&x) && (0..12).contains(&z) {
            flags[(z * 12 + x) as usize]
        } else {
            0
        }
    };
    let step = |x, z, dir, size| npc_step_ok(&flag, x, z, dir, size, StepStrategy::Normal);
    assert!(step(5, 5, (1, 0), 1), "a size-1 NPC fits the gap");
    for z in 3..=6 {
        assert!(
            !step(4, z, (1, 0), 2),
            "a size-2 NPC never fits a one-tile gap (south-west z={z})"
        );
    }
    assert!(step(1, 1, (1, 1), 1));
    assert!(
        !step(1, 1, (1, 1), 2),
        "takeStep moves only width-1 NPCs diagonally"
    );
    assert!(step(1, 1, (1, 0), 2) && step(1, 1, (0, 1), 2));
}

/// Raw 289 level-0 flags around `ghast_invis@3478,3328,0` in the Mort Myre
/// bog, `[z - 3321][x - 3473]`: `0x200000` is blocked ground, `0x20100`
/// scenery that also blocks projectiles.
const BOG_FLAGS: [[u32; 11]; 11] = [
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x200000],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [0x20100, 0, 0, 0, 0, 0, 0, 0, 0x20100, 0, 0x200000],
    [0, 0, 0, 0, 0, 0, 0, 0x200000, 0x200000, 0x200000, 0x200000],
    [
        0x20100, 0x20100, 0, 0, 0, 0x200000, 0x200000, 0x200000, 0x200000, 0x200000, 0x200000,
    ],
    [
        0x20100, 0x20100, 0x20100, 0x20100, 0, 0x200000, 0x200000, 0x220100, 0x200000, 0x220100,
        0x200000,
    ],
    [
        0x20100, 0x20100, 0x20100, 0x20100, 0x200000, 0x200000, 0x200000, 0x200000, 0x200000,
        0x200000, 0x200000,
    ],
    [
        0x20100, 0x20100, 0x20100, 0x20100, 0x200000, 0x200000, 0x200000, 0x200000, 0x200000,
        0x200000, 0x200000,
    ],
    [
        0x20100, 0x20100, 0x20100, 0x20100, 0x200000, 0x200000, 0x200000, 0x200000, 0x200000,
        0x200000, 0x220100,
    ],
    [
        0x20100, 0x20100, 0x20100, 0x20100, 0x200000, 0x200000, 0x200000, 0x220100, 0x200000,
        0x200000, 0x200000,
    ],
    [
        0x20100, 0x20100, 0x20100, 0x20100, 0x200000, 0x200000, 0x200000, 0x200000, 0x200000,
        0x200000, 0x200000,
    ],
];

fn bog_flag(x: i32, z: i32) -> u32 {
    BOG_FLAGS[usize::try_from(z - 3321).unwrap()][usize::try_from(x - 3473).unwrap()]
}

#[test]
fn npc_steps_match_engine_can_travel_per_strategy() {
    // The engine's own `StepValidator.canTravel(flags, 0, x, z, dx, dz, size,
    // 0, strategy)` over `BOG_FLAGS`, for every south-west tile x 3474..=3481,
    // z 3322..=3329 (bit `(z - 3322) * 8 + (x - 3474)`), one word per
    // direction in `DIRS` order; diagonals are zero for size 2 because
    // `takeStep` never tries them. Generated by
    // `274bot-evidence/NAV-WWM-ROUTE-R2/oracle/step-oracle.ts`.
    const DIRS: [(i32, i32); 8] = [
        (0, -1),
        (0, 1),
        (-1, 0),
        (1, 0),
        (-1, -1),
        (-1, 1),
        (1, -1),
        (1, 1),
    ];
    const ORACLE: [(i32, StepStrategy, [u64; 8]); 6] = [
        (
            1,
            StepStrategy::Normal,
            [
                0x0000_080e_3f7f_ffff,
                0x0000_0000_080e_3f7f,
                0x0000_0010_1c7f_feff,
                0x0000_0004_071f_bfff,
                0x0000_0000_1c7e_feff,
                0x0000_0000_000c_3e7e,
                0x0000_0004_071f_bfff,
                0x0000_0000_0006_1f3f,
            ],
        ),
        (
            1,
            StepStrategy::Blocked,
            [
                0xf8f8_b0f0_c000_0000,
                0xb8f8_f8f8_b0f0_c000,
                0xf0f0_f060_e080_0000,
                0xfcfc_fc58_f8e0_0000,
                0xf0f0_2060_8000_0000,
                0x30f0_f060_2080_0000,
                0xf8f8_1050_c000_0000,
                0x98f8_f858_10e0_0000,
            ],
        ),
        (
            1,
            StepStrategy::LineOfSight,
            [
                0xf8f8_b8fe_ff7f_ffff,
                0xb8f8_f8f8_b8fe_ff7f,
                0xf0f0_f070_fcff_feff,
                0xfcfc_fc5c_ffff_bfff,
                0xf0f0_3070_fc7e_feff,
                0x30f0_f070_30fc_fe7e,
                0xf8f8_185c_ff3f_bfff,
                0x98f8_f858_18fe_bf3f,
            ],
        ),
        (
            2,
            StepStrategy::Normal,
            [
                0x0000_0006_1f3f_ffff,
                0x0000_0000_0000_061f,
                0x0000_0000_101c_7efe,
                0x0000_0000_0203_0f5f,
                0,
                0,
                0,
                0,
            ],
        ),
        (
            2,
            StepStrategy::Blocked,
            [
                0xf8f8_10f0_c000_0000,
                0xf898_f8f8_f810_f0c0,
                0x70f0_f060_6080_0000,
                0x6e7e_feac_acf0_8000,
                0,
                0,
                0,
                0,
            ],
        ),
        (
            2,
            StepStrategy::LineOfSight,
            [
                0xf8f8_18fe_ff3f_ffff,
                0xf898_f8f8_f818_feff,
                0x70f0_f070_70fc_fefe,
                0x6e7e_feae_aeff_dfdf,
                0,
                0,
                0,
                0,
            ],
        ),
    ];
    for (size, strategy, words) in ORACLE {
        for (dir, word) in DIRS.into_iter().zip(words) {
            for bit in 0..64 {
                let (x, z) = (3474 + bit % 8, 3322 + bit / 8);
                assert_eq!(
                    npc_step_ok(&bog_flag, x, z, dir, size, strategy),
                    word >> bit & 1 == 1,
                    "size {size} {strategy:?} from ({x},{z}) by {dir:?}"
                );
            }
        }
    }
}

#[test]
fn blocked_normal_ghast_walks_the_bog_its_wander_witness_crosses() {
    use crate::map::services::MoveRestrict;
    // REVIEW-NAV-WWM-ROUTE R1: `ghast_invis` (size 2, `moverestrict=
    // blocked+normal`) wanders from its spawn (3478,3328) to (3477,3325),
    // cardinal-adjacent to the player tile (3476,3325). The engine steps it
    // under LINE_OF_SIGHT; NORMAL refuses the three bog steps.
    assert_eq!(
        StepStrategy::of(MoveRestrict::BlockedNormal),
        Some(StepStrategy::LineOfSight)
    );
    let walk = [
        (3478, 3328),
        (3478, 3327),
        (3478, 3326),
        (3478, 3325),
        (3477, 3325),
    ];
    for (index, pair) in walk.windows(2).enumerate() {
        let [(x, z), (to_x, to_z)] = [pair[0], pair[1]];
        let dir = (to_x - x, to_z - z);
        assert!(
            npc_step_ok(&bog_flag, x, z, dir, 2, StepStrategy::LineOfSight),
            "the ghast steps ({x},{z}) -> ({to_x},{to_z})"
        );
        assert_eq!(
            npc_step_ok(&bog_flag, x, z, dir, 2, StepStrategy::Normal),
            index == 3,
            "NORMAL at ({x},{z}) -> ({to_x},{to_z})"
        );
    }
}

/// Blocked ground (`WR_GRND`, no wall or scenery) along x=2847 across the
/// whole grid: NORMAL walkers stop there, sight and LINE_OF_SIGHT walkers
/// don't.
fn collision_with_ground_strip() -> WorldCollision {
    let mut collision = collision_with_flags(128, 128, &[]);
    collision.origin = WorldTile {
        x: 2800,
        z: 3550,
        level: 0,
    };
    let strip_x = usize::try_from(2847 - collision.origin.x).unwrap();
    let mut flags = vec![0u32; 4 * 128 * 128];
    for z in 0..128 {
        flags[z * 128 + strip_x] = CollisionFlag::WR_GRND as u32;
    }
    collision.attach_flags(flags);
    collision
}

#[test]
fn derive_zone_table_floods_each_hunter_under_its_move_restriction() {
    let collision = collision_with_ground_strip();
    let members = |movement: &str| {
        let content = SyntheticHunterContent::with_thrower_movement(4, movement, false);
        let derived = derive_synthetic_hunter_zones(&content, &collision);
        (2839..=2863)
            .filter(|&x| thrower_tile_count(&derived.table, x) == 1)
            .collect::<Vec<_>>()
    };
    // NORMAL stops at the strip, so x=2839 lies beyond huntrange 8 of every
    // tile it reaches (x >= 2848).
    assert_eq!(
        members("defaultmode=normal\n"),
        (2840..=2863).collect::<Vec<_>>()
    );
    // `blocked+normal` walks over blocked ground and keeps the whole row.
    assert_eq!(
        members("moverestrict=blocked+normal\n"),
        (2839..=2863).collect::<Vec<_>>()
    );
    // `blocked` steps only onto blocked ground; none surrounds this spawn, so
    // it hunts from its spawn tile alone.
    assert_eq!(
        members("moverestrict=blocked\n"),
        (2843..=2859).collect::<Vec<_>>()
    );
}
