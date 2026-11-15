// Static scenery observation: consume LOC_DEL/ADD scene gen before the
// queued native apply, then require the loc family to refresh. Player-tick
// distance updates must rewrite only the Chebyshev scalars.

use api::snapshot::{Family, GameSnapshot, LocLayer, WorldTile};
use client::client::{Client, ClientConfig};
use client::config::runtime_377::{CacheFields377, InterfaceFields377};
use client::config::LocType;
use client::config::{Cache, CacheRevision377, LocMorph, VarpType};
use client::dash3d::ClientPlayer;
use client::io::{ClientRevision, ServerProt};
use std::sync::Arc;

fn cfg() -> ClientConfig {
    ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: false,
    }
}

fn scene_typecode(id: i32) -> i32 {
    0x4000_0000 + (id << 14) + 7 + (8 << 7)
}

fn client_with_flax() -> (Client, i32) {
    let mut c = Client::new(cfg());
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c.minusedlevel = 0;
    c.local_player = Some(ClientPlayer::at(20, 12));
    let id = {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        let id = cache.locs.len() as i32;
        cache.locs.push(LocType {
            id,
            name: "Flax".into(),
            op: vec![Some("Pick".into())],
            width: 1,
            length: 1,
            ..Default::default()
        });
        id
    };
    assert!(c.world.add_scenery(
        0,
        7,
        8,
        0,
        scene_typecode(id),
        (3 << 6) + 10,
        1,
        1,
        0,
        0,
        0,
        0,
        0
    ));
    (c, id)
}

fn ground_named(snap: &GameSnapshot, id: i32) -> &api::snapshot::LocView {
    snap.locs()
        .iter()
        .find(|loc| loc.layer == LocLayer::Ground && loc.id == id)
        .expect("ground loc")
}

/// Packet reception bumps gens.scene; the queued native apply does not.
/// After the observer consumes that gen, del_loc/add_scenery must still
/// refresh the loc family.
#[test]
fn static_loc_refreshes_after_consumed_packet_gen() {
    let (mut c, id) = client_with_flax();
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::LOC_DEL);
    assert!(snap.rebuild_family(&c, Family::Loc));
    assert_eq!(
        ground_named(&snap, id).tile,
        WorldTile {
            x: 3207,
            z: 3208,
            level: 0
        }
    );
    assert_eq!(ground_named(&snap, id).name.as_deref(), Some("Flax"));
    assert!(
        !snap.rebuild_family(&c, Family::Loc),
        "packet gen already consumed while the loc is still present"
    );

    c.world.del_loc(0, 7, 8);
    assert!(
        snap.rebuild_family(&c, Family::Loc),
        "native deletion after consumed packet gen must dirty loc"
    );
    assert!(
        snap.locs()
            .iter()
            .all(|loc| loc.layer != LocLayer::Ground || loc.id != id),
        "deleted flax must disappear"
    );
    assert!(!snap.rebuild_family(&c, Family::Loc));

    c.bump_gens(ServerProt::LOC_ADD_CHANGE);
    assert!(snap.rebuild_family(&c, Family::Loc));
    assert!(
        snap.locs()
            .iter()
            .all(|loc| loc.layer != LocLayer::Ground || loc.id != id),
        "add packet gen is consumed before the native respawn"
    );

    assert!(c.world.add_scenery(
        0,
        7,
        8,
        0,
        scene_typecode(id),
        (3 << 6) + 10,
        1,
        1,
        0,
        0,
        0,
        0,
        0
    ));
    assert!(
        snap.rebuild_family(&c, Family::Loc),
        "native add after consumed packet gen must dirty loc"
    );
    assert_eq!(ground_named(&snap, id).name.as_deref(), Some("Flax"));
}

#[test]
fn no_op_delete_and_dynamic_churn_do_not_rebuild_locs() {
    let (mut c, id) = client_with_flax();
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    assert!(snap.rebuild_family(&c, Family::Loc));
    let loc_ptr = snap.locs().as_ptr();
    let name_ptr = ground_named(&snap, id).name.as_ref().map(|s| s.as_ptr());

    c.world.del_loc(0, 4, 4);
    assert!(
        !snap.rebuild_family(&c, Family::Loc),
        "missing-tile del_loc is a no-op"
    );

    assert!(c
        .world
        .add_dynamic(
            0,
            7 * 128 + 64,
            0,
            8 * 128 + 64,
            scene_typecode(2),
            0,
            0,
            false
        )
        .is_some());
    assert!(
        !snap.rebuild_family(&c, Family::Loc),
        "dynamic sprite churn must not rebuild loc"
    );
    assert_eq!(snap.locs().as_ptr(), loc_ptr);
    assert_eq!(
        ground_named(&snap, id).name.as_ref().map(|s| s.as_ptr()),
        name_ptr,
        "no-op/dynamic must not clone loc names"
    );
}

/// host-play publishes LocView.distance verbatim. A player tile change
/// must refresh those scalars without rebuilding loc definitions.
#[test]
fn player_tile_change_refreshes_loc_distance_without_resweep() {
    let (mut c, id) = client_with_flax();
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    assert!(snap.rebuild(&c));
    let before = ground_named(&snap, id);
    assert_eq!(before.distance, 13);
    let loc_ptr = snap.locs().as_ptr();
    let name_ptr = before.name.as_ref().map(|s| s.as_ptr());

    c.local_player.as_mut().unwrap().route_x[0] = 7;
    c.local_player.as_mut().unwrap().route_z[0] = 8;
    c.bump_gens(ServerProt::PLAYER_INFO);
    assert!(snap.rebuild(&c));
    assert!(
        !snap.rebuild_family(&c, Family::Loc),
        "player tick is not a loc resweep"
    );
    assert_eq!(snap.locs().as_ptr(), loc_ptr);
    let after = ground_named(&snap, id);
    assert_eq!(after.distance, 0);
    assert_eq!(after.name.as_ref().map(|s| s.as_ptr()), name_ptr);
    assert_eq!(after.name.as_deref(), Some("Flax"));

    c.bump_gens(ServerProt::PLAYER_INFO);
    assert!(snap.rebuild_family(&c, Family::Player));
    assert_eq!(snap.locs().as_ptr(), loc_ptr);
    assert_eq!(ground_named(&snap, id).distance, 0);
    assert_eq!(
        ground_named(&snap, id).name.as_ref().map(|s| s.as_ptr()),
        name_ptr,
        "same-tile player tick must not clone loc names"
    );
}

/// T6: consume LOC_* gen, then a later static-scenery write plus collision
/// mutation must still refresh flags on standalone Family::Loc.
#[test]
fn queued_loc_after_consumed_gen_refreshes_collision_flags() {
    let (mut c, id) = client_with_flax();
    c.ingame = true;
    c.scene_state = 2;
    c.collision[0].flags[7][8] = client::dash3d::CollisionFlag::WALK_SCENERY;
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::LOC_DEL);
    assert!(snap.rebuild(&c));
    assert_eq!(
        snap.scene().collision_flags[7 * 104 + 8],
        client::dash3d::CollisionFlag::WALK_SCENERY
    );
    assert!(
        !snap.rebuild_family(&c, Family::Loc),
        "packet gen already consumed"
    );

    c.world.del_loc(0, 7, 8);
    c.collision[0].flags[7][8] = client::dash3d::CollisionFlag::VIS_SCENERY;
    assert!(
        snap.rebuild_family(&c, Family::Loc),
        "standalone Loc after consumed gen must still dirty"
    );
    assert!(snap
        .locs()
        .iter()
        .all(|loc| loc.layer != LocLayer::Ground || loc.id != id));
    assert_eq!(
        snap.scene().collision_flags[7 * 104 + 8],
        client::dash3d::CollisionFlag::VIS_SCENERY,
        "standalone Loc recopies the current plane"
    );
}

/// T7: missing-tile del_loc / add_dynamic are not collision writes.
#[test]
fn no_op_loc_churn_does_not_require_a_client_collision_hook() {
    let (mut c, _id) = client_with_flax();
    c.ingame = true;
    c.scene_state = 2;
    let mut snap = GameSnapshot::new();
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    assert!(snap.rebuild(&c));
    let before = snap.scene().collision_flags.clone();

    c.world.del_loc(0, 4, 4);
    assert!(!snap.rebuild_family(&c, Family::Loc));
    assert!(c
        .world
        .add_dynamic(
            0,
            7 * 128 + 64,
            0,
            8 * 128 + 64,
            scene_typecode(2),
            0,
            0,
            false
        )
        .is_some());
    assert!(!snap.rebuild_family(&c, Family::Loc));
    assert_eq!(snap.scene().collision_flags, before);
}

fn client_with_multiloc(revision: ClientRevision) -> Client {
    let cache = Cache {
        locs: vec![
            LocType {
                id: 0,
                name: "null".into(),
                width: 1,
                length: 1,
                ..Default::default()
            },
            LocType {
                id: 1,
                name: "Flour bin".into(),
                width: 1,
                length: 1,
                ..Default::default()
            },
            LocType {
                id: 2,
                name: "Flour bin".into(),
                op: vec![Some("Empty".into())],
                width: 3,
                length: 2,
                ..Default::default()
            },
        ],
        loc_morphs: vec![(
            0,
            LocMorph {
                varbit: -1,
                varp: 0,
                targets: vec![1, 2, -1],
            },
        )],
        varps: vec![VarpType::default()],
        revision_377: Some(Box::new(CacheRevision377 {
            config: CacheFields377 {
                ..Default::default()
            },
            interfaces: InterfaceFields377 {
                client_code_layers: Default::default(),
                type1_fields: Vec::new(),
                null_string_masks: Vec::new(),
                initial_overlays: Vec::new(),
            },
        })),
        ..Default::default()
    };
    let mut c = Client::from_shared_with_revision(
        cfg(),
        Arc::new(cache),
        Arc::new(Vec::new()),
        Arc::new(Vec::new()),
        revision,
    );
    c.ingame = true;
    c.scene_state = 2;
    c.var = vec![0];
    c.gens.scene = 1;
    c.gens.varp = 1;
    assert!(c.world.add_scenery(
        0,
        7,
        8,
        0,
        scene_typecode(0),
        (1 << 6) + 10,
        1,
        1,
        0,
        0,
        0,
        0,
        0
    ));
    c
}

#[test]
fn revision_377_multiloc_observes_current_name_actions_and_base_size() {
    let mut c = client_with_multiloc(ClientRevision::R377);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    let empty = ground_named(&snap, 0);
    assert_eq!(empty.name.as_deref(), Some("Flour bin"));
    assert!(empty.actions.is_empty());
    assert_eq!((empty.width, empty.length), (1, 1));

    // Only the varp packet changes. Full rebuild consumes Varp before Loc.
    c.var[0] = 1;
    c.gens.varp += 1;
    snap.rebuild(&c);
    let full = ground_named(&snap, 0);
    assert_eq!(
        full.id, 0,
        "the interaction retains the placed base typecode"
    );
    assert_eq!(full.name.as_deref(), Some("Flour bin"));
    assert_eq!(full.actions, vec![Some("Empty".into())]);
    assert_eq!((full.width, full.length), (1, 1));
    assert_eq!((full.footprint_width, full.footprint_length), (1, 1));
    assert!(!snap.rebuild_family(&c, Family::Loc));

    c.var[0] = 2;
    c.gens.varp += 1;
    assert!(snap.rebuild_family(&c, Family::Loc));
    let absent = ground_named(&snap, 0);
    assert_eq!(absent.name, None, "a -1 morph has no displayed target");
    assert!(absent.actions.is_empty());
}

#[test]
fn revision_377_multiloc_and_null_form_preserve_all_base_placement_fields() {
    let mut c = client_with_multiloc(ClientRevision::R377);
    let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
    // Fenkenstrain's table has this base/form footprint mismatch.
    cache.locs[0] = LocType {
        id: 0,
        name: "Base table".into(),
        desc: "Base description".into(),
        op: vec![Some("Base action".into())],
        width: 4,
        length: 2,
        blockwalk: false,
        blockrange: false,
        active: true,
        forceapproach: 5,
        anim: 17,
        mapfunction: 23,
        mapscene: 29,
        ..Default::default()
    };
    cache.locs[1] = LocType {
        id: 1,
        name: "Morphed table".into(),
        desc: "Form description".into(),
        op: vec![Some("Search".into())],
        width: 1,
        length: 1,
        blockwalk: true,
        blockrange: true,
        active: false,
        forceapproach: 0,
        anim: -1,
        mapfunction: -1,
        mapscene: -1,
        ..Default::default()
    };
    let mut snap = GameSnapshot::new();
    for selector in [0, 2] {
        c.var[0] = selector;
        c.gens.varp += 1;
        assert!(snap.rebuild_family(&c, Family::Loc));
        let loc = ground_named(&snap, 0);
        assert_eq!(loc.id, 0);
        assert_eq!((loc.width, loc.length), (4, 2));
        assert_eq!((loc.footprint_width, loc.footprint_length), (2, 4));
        assert!(!loc.block_walk);
        assert!(!loc.block_range);
        assert!(loc.active);
        assert_eq!(loc.force_approach, 5);
        assert_eq!(loc.animation, 17);
        assert_eq!(loc.map_function, 23);
        assert_eq!(loc.map_scene, 29);
        if selector == 0 {
            assert_eq!(loc.name.as_deref(), Some("Morphed table"));
            assert_eq!(loc.description.as_deref(), Some("Form description"));
            assert_eq!(loc.actions, vec![Some("Search".into())]);
        } else {
            assert_eq!(loc.name, None);
            assert_eq!(loc.description, None);
            assert!(loc.actions.is_empty());
        }
    }
}

#[test]
fn legacy_loc_views_stay_byte_identical_even_with_morph_metadata() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let mut c = client_with_multiloc(revision);
        let mut snap = GameSnapshot::new();
        snap.rebuild(&c);
        assert_eq!(ground_named(&snap, 0).name.as_deref(), Some("null"));
        let before = serde_json::to_vec(snap.locs()).unwrap();
        c.var[0] = 1;
        c.gens.varp += 1;
        snap.rebuild(&c);
        assert_eq!(serde_json::to_vec(snap.locs()).unwrap(), before);
        assert!(!snap.rebuild_family(&c, Family::Loc));
    }
}
