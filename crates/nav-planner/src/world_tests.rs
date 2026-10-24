use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;

use super::{NavWorld, BLOCKED};
use crate::collision::walk_word_from_parts;
use crate::grid::StepGrid;
use crate::pack::PackError;
use crate::router::{find, Leg};
use crate::tile::Tile;

fn tile(x: i32, z: i32, level: i32) -> WorldTile {
    WorldTile { x, z, level }
}

#[test]
fn seers_street_walks_to_rock_crabs_on_foot() {
    // Task 4: Seers street (2725,3485) -> rock crabs (2710,3720) is on
    // foot after the L0 stamper rebake. Requires the pack: run
    // `cargo run -p nav-bake --bin nav-pack` first. GitHub has neither pack
    // nor Server content — skip, do not panic.
    let Some(world) = NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let from = WorldTile {
        x: 2725,
        z: 3485,
        level: 0,
    };
    let to = WorldTile {
        x: 2710,
        z: 3720,
        level: 0,
    };
    let r = crate::router::find_with(
        &world.collision,
        &world.graph,
        from,
        to,
        crate::router::FindOptions {
            zones: crate::zones::ZoneExempt::all(),
            ..Default::default()
        },
        &crate::world_state::WorldState::empty(),
    )
    .expect("on-foot path after stamper rebake");
    assert_eq!(r.dest, to);
    assert!(r.legs.iter().any(|l| matches!(l, Leg::Walk { .. })));
    let walked: Vec<WorldTile> = r
        .legs
        .iter()
        .flat_map(|l| match l {
            Leg::Walk { tiles } => tiles.clone(),
            Leg::Transport { .. } => vec![],
        })
        .collect();
    // No fake hop: every walked tile stays on plane 0 and the walk
    // must span the whole 235-tile north gap (a single invented cliff
    // transport would walk ~nothing).
    assert!(
        walked.iter().all(|t| t.level == 0),
        "every Walk tile stays on plane 0"
    );
    let north = to.z - from.z;
    assert!(north > 0);
    assert!(
        walked.len() >= north as usize,
        "walk spans the north gap ({} walked tiles for {north} north)",
        walked.len()
    );
    // Transports, if any, are road doors the player would Open — never
    // an invented cliff/teleport hop.
    for l in &r.legs {
        if let Leg::Transport { edge } = l {
            assert_eq!(
                edge.kind,
                TransportKind::Door,
                "transport at {:?} is a road door, not a cliff hop",
                edge.at
            );
        }
    }
}

#[test]
fn open_grid_derives_an_all_walkable_world() {
    let w = NavWorld::from_grid(&StepGrid::fixture_open_3x3());
    assert_eq!(w.collision.origin, tile(0, 0, 0));
    assert_eq!(w.collision.width, 3);
    assert_eq!(w.collision.height, 3);
    for (i, wd) in w.collision.walk.iter().enumerate() {
        assert_eq!(*wd, 0, "word {i} stays open");
    }
    assert!(w.graph.edges.is_empty());
}

#[test]
fn transport_name_bound_is_cached_with_selected_names() {
    let mut world = NavWorld::from_grid(&StepGrid::fixture_door_corridor());
    assert_eq!(world.transport_item_name_bytes_bound(), 0);
    let ids = [31, 563, 995];
    world.graph.edges[0].item_req = vec![(ids[0], 1), (ids[1], 1)];
    world.graph.edges[0].consumed_req = vec![(ids[2], 1)];
    let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
    let expected = ids
        .into_iter()
        .filter_map(|id| data.item_by_id(id)?.name.as_ref())
        .map(String::len)
        .max()
        .expect("selected fixture transport items have names");
    world.bind_named_bank_facts(&data).unwrap();
    assert_eq!(world.transport_item_name_bytes_bound(), expected);
    for id in ids {
        assert_eq!(
            world.transport_item_name(id),
            data.item_by_id(id).and_then(|item| item.name.as_deref())
        );
    }
    assert_eq!(world.transport_item_name(i32::MIN), None);
    let measured = allocation_counter::measure(|| {
        for _ in 0..32 {
            std::hint::black_box(world.transport_item_name_bytes_bound());
        }
    });
    assert_eq!(measured.bytes_total, 0);
}

#[test]
fn blocked_tiles_stamp_every_direction_mask() {
    let w = NavWorld::from_grid(&StepGrid::fixture_door_corridor());
    let door_tile = tile(2, 0, 0);
    let idx = (door_tile.z - w.collision.origin.z) as usize * w.collision.width
        + (door_tile.x - w.collision.origin.x) as usize;
    let blocked = (w.collision.blocked[idx >> 6] >> (idx & 63)) & 1 != 0;
    assert_eq!(
        walk_word_from_parts(w.collision.walk[idx], blocked),
        BLOCKED,
        "the full directional stamp is in the packed walk surface"
    );
    // The full directional stamp is in the walk block masks, so the
    // router never steps onto it.
    assert_eq!(
        walk_word_from_parts(w.collision.walk[idx], blocked)
            & CollisionFlag::WALK_BLOCK_FLAGS as u32,
        CollisionFlag::WALK_BLOCK_FLAGS as u32
    );
    assert!(w.collision.walkable(tile(0, 0, 0)));
    assert!(!w.collision.walkable(door_tile));
}

#[test]
fn door_edges_become_transport_edges() {
    let w = NavWorld::from_grid(&StepGrid::fixture_door_corridor());
    let fwd = w
        .graph
        .edges
        .iter()
        .find(|e| e.kind == TransportKind::Door && e.at == tile(1, 0, 0))
        .expect("door edge from the corridor's west side");
    assert_eq!(fwd.to, tile(3, 0, 0));
    assert_eq!(fwd.loc_id, 1530);
    assert_eq!(fwd.ticks, 1);
    assert_eq!(w.graph.at.get(&tile(1, 0, 0)).map(Vec::len), Some(1));
    // The fixture corridor is a single directed edge.
    assert_eq!(w.graph.at.get(&tile(3, 0, 0)), None);
}

#[test]
fn find_uses_the_derived_world_across_the_corridor() {
    let w = NavWorld::from_grid(&StepGrid::fixture_door_corridor());
    // The walled tile blocks walking; the door edge crosses it, so the
    // route splits into Walk -> Transport -> Walk legs. The origin is
    // within the door's interact radius of at=(1,0), so the door is
    // taken straight from it.
    let r = find(&w.collision, &w.graph, tile(0, 0, 0), tile(4, 0, 0)).unwrap();
    // The 1-tick door taken from the origin + 1 walk tile (0.5).
    assert_eq!(r.ticks, 1.5);
    assert_eq!(r.legs.len(), 3);
    let (Leg::Walk { .. }, Leg::Transport { edge }, Leg::Walk { .. }) =
        (&r.legs[0], &r.legs[1], &r.legs[2])
    else {
        panic!("expected Walk, Transport, Walk legs");
    };
    assert_eq!(edge.at, tile(1, 0, 0));
    assert_eq!(edge.to, tile(3, 0, 0));
    assert_eq!(edge.loc_id, 1530);
}

#[test]
fn derived_world_routes_across_a_fixture_rect() {
    let w = NavWorld::from_grid(&StepGrid::fixture_rect_at(
        Tile {
            x: 3200,
            z: 3200,
            level: 0,
        },
        64,
        64,
    ));
    let r = find(
        &w.collision,
        &w.graph,
        tile(3200, 3200, 0),
        tile(3263, 3263, 0),
    )
    .unwrap();
    assert_eq!(r.ticks, 31.5); // 63 run steps at 0.5 ticks each
    let Leg::Walk { tiles } = &r.legs[0] else {
        panic!("walk-only route");
    };
    assert_eq!(tiles.first(), Some(&tile(3200, 3200, 0)));
    assert_eq!(tiles.last(), Some(&tile(3263, 3263, 0)));
}

#[test]
fn load_pack_keeps_stale_v5_wire_as_bad_version() {
    // A leftover v5 (pre-v6) `274V` pack is not a 274N grid pack:
    // load must surface BadVersion(5) so the operator rebakes, not
    // fall into the grid decoder and come out as a confusing BadMagic.
    let bytes = b"274V\x05";
    let dir = std::env::temp_dir().join(format!(
        "274bot-navworld-v5-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("fixture-v5.navpack");
    std::fs::write(&path, &bytes).unwrap();
    assert!(matches!(
        NavWorld::load_pack(&path),
        Err(PackError::BadVersion(5))
    ));
    assert!(matches!(
        NavWorld::from_bytes(&bytes),
        Err(PackError::BadVersion(5))
    ));
    let _ = std::fs::remove_dir_all(&dir);
}
