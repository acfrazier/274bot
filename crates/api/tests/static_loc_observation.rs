// Static scenery observation: consume LOC_DEL/ADD scene gen before the
// queued native apply, then require the loc family to refresh. Player-tick
// distance updates must rewrite only the Chebyshev scalars.

use api::snapshot::{Family, GameSnapshot, LocLayer, WorldTile};
use client::client::{Client, ClientConfig};
use client::config::LocType;
use client::dash3d::ClientPlayer;
use client::io::ServerProt;
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
