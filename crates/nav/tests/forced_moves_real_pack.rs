//! Original Hero Ice Queen city routes plus the real, gated loc 2634 Mine crossing.
use std::path::PathBuf;

use api::selected::ClientRevision;
use api::snapshot::{stat_name, stat_used, GameSnapshot, StatView};
use api::WorldTile;
use nav::router::{find_first_with, FindOptions};
use nav::transport::TransportKind;
use nav::world::NavWorld;
use nav::world_state::WorldState;

fn tile(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

/// Reproduce the original diagnostic's planned profile, not live TestedStats.
fn planned_state(floor: i32, pickaxe: i32) -> WorldState {
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_stats((0..25).map(|index| {
        let level = match index {
            0..=4 | 6 => floor,
            5 => 43,
            7 | 10 => 53,
            14 => 50,
            15 => 25,
            _ => if stat_used(index) { 1 } else { 0 },
        };
        StatView {
            index: index as i32,
            name: stat_name(index).into(),
            base: level,
            effective: level,
            xp: 0,
            used: stat_used(index),
        }
    }).collect());
    let mut state = WorldState::from_snapshot(&snapshot).with_map_members(true);
    state.inv.insert(pickaxe, 1);
    state
}

fn queen_stands(world: &NavWorld) -> Vec<WorldTile> {
    let npc = tile(2866, 9956);
    (-3..=3)
        .flat_map(|dx| (-3..=3).map(move |dz| tile(npc.x + dx, npc.z + dz)))
        .filter(|stand| world.collision.standable(*stand))
        .collect()
}

fn rockslide_edges(world: &NavWorld) -> Vec<&nav::transport::TransportEdge> {
    world
        .graph
        .edges
        .iter()
        .filter(|edge| edge.loc_id == 2634 && edge.option == 2)
        .collect()
}

#[test]
#[ignore = "requires NAV_QUEST_DOORS_PACK naming a freshly baked real 289 pack"]
fn hero_ice_queen_routes_and_rockslide_require_real_mine_gates() {
    let pack = PathBuf::from(std::env::var_os("NAV_QUEST_DOORS_PACK").expect("explicit pack"));
    let mut world = NavWorld::load_pack(&pack).expect("real 289 pack");
    let selected =
        api::game_data::for_revision(ClientRevision::R289).expect("selected 289 game data");
    let pickaxe = selected
        .item_by_alias("bronze_pickaxe")
        .expect("selected Bronze pickaxe")
        .id;
    let unrelated = selected.item_by_alias("coins").expect("selected coins").id;

    let edges = rockslide_edges(&world);
    assert_eq!(
        edges.len(),
        2,
        "the real loc 2634 op2 Mine has both directed source crossings"
    );
    for edge in &edges {
        assert_eq!(edge.kind, TransportKind::AgilityShortcut);
        assert_eq!(edge.at, tile(2838, 3517), "retain the actual loc interaction anchor");
        assert_eq!(
            edge.player_delta, None,
            "forced movement is represented by the exact landing"
        );
        assert_eq!(
            edge.takeoff,
            Some(edge_takeoff(edge)),
            "forced move is tied to its source takeoff"
        );
        assert!(edge.skill_req.contains(&(14, 50)), "Mine requires Mining 50");
        assert!(
            edge.item_req
                .iter()
                .any(|(id, count)| *id == pickaxe && *count > 0)
                || edge.worn_req.contains(&pickaxe),
            "Mine must require the selected usable pickaxe"
        );
    }
    assert!(edges.iter().any(|edge| {
        edge.takeoff == Some(tile(2840, 3517)) && edge.to == tile(2837, 3518)
    }), "east-side Mine movement must land at the source-derived (-3,+1) endpoint");
    assert!(edges.iter().any(|edge| {
        edge.takeoff == Some(tile(2837, 3518)) && edge.to == tile(2840, 3517)
    }), "west-side Mine movement must land at the source-derived (+3,-1) endpoint");

    for edge in &edges {
        let legal = planned_state(40, pickaxe);
        assert!(
            legal.allows(edge),
            "carried selected pickaxe and Mining 50 authorize {edge:?}"
        );

        let mut mining_49 = legal.clone();
        mining_49.stats.insert(14, 49);
        assert!(!mining_49.allows(edge), "Mining 49 cannot authorize {edge:?}");

        let mut no_pickaxe = planned_state(40, unrelated);
        no_pickaxe.inv.clear();
        no_pickaxe.inv.insert(unrelated, 1);
        no_pickaxe.stats.insert(14, 50);
        assert!(
            !no_pickaxe.allows(edge),
            "Mining 50 without a held or worn pickaxe cannot authorize {edge:?}"
        );

        let mut unrelated_item = planned_state(40, unrelated);
        unrelated_item.stats.insert(14, 50);
        assert!(
            !unrelated_item.allows(edge),
            "an unrelated held item cannot authorize {edge:?}"
        );

        let mut unknown_mining = planned_state(40, pickaxe);
        unknown_mining.stats.remove(&14);
        assert!(
            !unknown_mining.allows(edge),
            "unknown Mining cannot authorize {edge:?}"
        );

        let mut worn_pickaxe = planned_state(40, pickaxe);
        worn_pickaxe.inv.clear();
        worn_pickaxe.worn.insert(pickaxe);
        assert!(
            worn_pickaxe.allows(edge),
            "a worn selected usable pickaxe and Mining 50 authorize {edge:?}"
        );
    }

    let stands = queen_stands(&world);
    assert_eq!(stands.len(), 40, "preserve the original NPC795 radius-three stand candidates");
    let origins = [
        ("Varrock", tile(3253, 3420)),
        ("Heroes Guild", tile(2894, 3507)),
        ("Catherby", tile(2808, 3441)),
    ];
    for (label, from) in origins {
        assert!(
            world.collision.standable(from),
            "original {label} origin remains standable"
        );
        for floor in [40, 60] {
            let state = planned_state(floor, pickaxe);
            let result = find_first_with(
                &world.collision,
                &world.graph,
                from,
                &stands,
                FindOptions::default(),
                &state,
            );
            let route = result.route().unwrap_or_else(|error| {
                panic!(
                    "original {label} -> NPC795 route under normal zone policy, planned combat floor {floor}: {error:?}"
                )
            });
            assert!(stands.contains(&route.dest), "route ends at an NPC795 stand candidate");
            println!(
                "Hero {label} -> Ice Queen, normal zones, planned floor {floor}: {} legs, {} ticks",
                route.legs.len(),
                route.ticks
            );
        }
    }

    // The source report also ran a separately labelled geometry-only search;
    // disabling packed danger zones is diagnostic, never a product route policy.
    let saved_zones = world.graph.zones.take();
    for (label, from) in origins {
        for floor in [40, 60] {
            let state = planned_state(floor, pickaxe);
            let result = find_first_with(
                &world.collision,
                &world.graph,
                from,
                &stands,
                FindOptions::default(),
                &state,
            );
            let route = result.route().unwrap_or_else(|error| {
                panic!(
                    "geometry-only diagnostic {label} -> NPC795, planned combat floor {floor}: {error:?}"
                )
            });
            assert!(stands.contains(&route.dest));
            println!(
                "Hero {label} -> Ice Queen, GEOMETRY-ONLY diagnostic, planned floor {floor}: {} legs, {} ticks",
                route.legs.len(),
                route.ticks
            );
        }
    }
    world.graph.zones = saved_zones;
}

// The expected source takeoff is one of the two exact branches; this helper
// makes the metadata assertion explicit without inferring a takeoff from `at`.
fn edge_takeoff(edge: &nav::transport::TransportEdge) -> WorldTile {
    match edge.to {
        WorldTile {
            x: 2837,
            z: 3518,
            level: 0,
        } => tile(2840, 3517),
        WorldTile {
            x: 2840,
            z: 3517,
            level: 0,
        } => tile(2837, 3518),
        other => panic!("unexpected source-derived loc 2634 Mine landing: {other:?}"),
    }
}
