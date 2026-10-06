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
    snapshot.seed_stats(
        (0..25)
            .map(|index| {
                let level = match index {
                    0..=4 | 6 => floor,
                    5 => 43,
                    7 | 10 => 53,
                    14 => 50,
                    15 => 25,
                    _ => {
                        if stat_used(index) {
                            1
                        } else {
                            0
                        }
                    }
                };
                StatView {
                    index: index as i32,
                    name: stat_name(index).into(),
                    base: level,
                    effective: level,
                    xp: 0,
                    used: stat_used(index),
                }
            })
            .collect(),
    );
    let mut state = WorldState::from_snapshot(&snapshot).with_map_members(true);
    state.inv.insert(pickaxe, 1);
    let selected =
        api::game_data::for_revision(ClientRevision::R289).expect("selected 289 planned inventory");
    state.inv.insert(
        selected.item_by_alias("coins").expect("selected coins").id,
        408,
    );
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
        // This loc type is also placed elsewhere in the real map. The
        // original Hero repro names this particular mountain rockslide.
        .filter(|edge| edge.loc_id == 2634 && edge.option == 2 && edge.at == tile(2838, 3517))
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
    assert!(
        !edges.is_empty(),
        "real loc 2634 op2 Mine must have proved crossings"
    );
    for edge in &edges {
        assert_eq!(edge.kind, TransportKind::AgilityShortcut);
        assert_eq!(
            edge.at,
            tile(2838, 3517),
            "retain the actual loc interaction anchor"
        );
        assert_eq!(
            edge.player_delta, None,
            "forced movement is represented by the exact landing"
        );
        let start = edge
            .takeoff
            .expect("forced movement requires an exact operation stand");
        let east = start.x > edge.at.x;
        assert_eq!(edge.to.x - start.x, if east { -3 } else { 3 });
        assert_eq!(edge.to.z - start.z, if east { 1 } else { -1 });
        assert_eq!(
            edge.ticks, 8,
            "both force-walk steps and both p_delay(1) waits"
        );
        assert!(
            edge.skill_req.contains(&(14, 50)),
            "Mine requires Mining 50"
        );
        assert!(
            edge.worn_req.is_empty(),
            "a strict worn tool is never auto-equipped"
        );
        assert!(
            matches!(edge.item_req.as_slice(), [(_, 1)]) && edge.worn_all_req.is_empty()
                || edge.item_req.is_empty() && edge.worn_all_req.len() == 1,
            "Mine requires one carried OR one strictly worn usable tool: {edge:?}"
        );
    }
    assert!(
        edges
            .iter()
            .any(|edge| { edge.takeoff == Some(tile(2840, 3517)) && edge.to == tile(2837, 3518) }),
        "east-side Mine movement must land at the source-derived (-3,+1) endpoint"
    );
    assert!(
        edges
            .iter()
            .any(|edge| { edge.takeoff == Some(tile(2837, 3518)) && edge.to == tile(2840, 3517) }),
        "west-side Mine movement must land at the source-derived (+3,-1) endpoint"
    );

    let carried = planned_state(40, pickaxe);
    let mut worn = carried.clone();
    worn.inv.clear();
    worn.worn.insert(pickaxe);
    assert!(
        edges.iter().any(|edge| carried.allows(edge)),
        "a carried Bronze pickaxe is usable"
    );
    assert!(
        edges.iter().any(|edge| worn.allows(edge)),
        "a strictly worn Bronze pickaxe is usable"
    );
    for edge in &edges {
        let mut mining_49 = carried.clone();
        mining_49.stats.insert(14, 49);
        assert!(
            !mining_49.allows(edge),
            "Mining 49 cannot authorize {edge:?}"
        );
        let mut no_pickaxe = planned_state(40, unrelated);
        no_pickaxe.inv.clear();
        assert!(
            !no_pickaxe.allows(edge),
            "Mining 50 without a held or worn pickaxe: {edge:?}"
        );
        let unrelated_item = planned_state(40, unrelated);
        assert!(
            !unrelated_item.allows(edge),
            "an unrelated item cannot authorize {edge:?}"
        );
        let mut unknown_mining = carried.clone();
        unknown_mining.stats.remove(&14);
        assert!(
            !unknown_mining.allows(edge),
            "unknown Mining cannot authorize {edge:?}"
        );
    }

    let stands = queen_stands(&world);
    assert_eq!(
        stands.len(),
        40,
        "preserve the original NPC795 radius-three stand candidates"
    );
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
            assert_eq!(
                result.route().err(),
                Some(nav::router::RouteError::NoPath),
                "the original {label} floor {floor} cannot bypass unsafe zones or the locked Guild"
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
            if label == "Heroes Guild" {
                assert_eq!(
                    result.route().err(),
                    Some(nav::router::RouteError::NoPath),
                    "the original interior Guild profile has not completed Hero's Quest"
                );
                continue;
            }
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

    // Qualify the same three origins with honest permissions: max combat,
    // completed Hero's Quest for the interior Guild, and the existing named
    // mountain encounter grant. The zone table stays enabled.
    let mut qualified = planned_state(99, pickaxe);
    qualified.stats.insert(5, 99);
    qualified.combat_level = Some(126);
    qualified.quests.insert("Hero's Quest".to_owned());
    let mut qualified_worn = qualified.clone();
    qualified_worn.inv.remove(&pickaxe);
    qualified_worn.worn.insert(pickaxe);
    let zones = world.graph.zones.as_ref().expect("real danger zone table");
    let options = FindOptions {
        zones: nav::zones::ZoneExempt::named(&[zones
            .resolve("white-wolf-mountain")
            .expect("named mountain grant")])
        .expect("one bounded encounter grant"),
        ..FindOptions::default()
    };
    for (mode, state) in [("carried", qualified), ("strictly worn", qualified_worn)] {
        for (label, from) in origins {
            let result = find_first_with(
                &world.collision,
                &world.graph,
                from,
                &stands,
                options,
                &state,
            );
            let route = result.route().unwrap_or_else(|error| {
                panic!("qualified {label} -> NPC795 with {mode} pickaxe: {error:?}")
            });
            assert!(stands.contains(&route.dest));
            assert!(
                route.legs.iter().any(|leg| matches!(leg,
                    nav::router::Leg::Transport { edge }
                        if edge.loc_id == 2634 && edge.option == 2
                            && edge.at == tile(2838, 3517)
                )),
                "the qualified route must use the real gated Mine crossing"
            );
            println!("Hero qualified {label}, {mode} tool, normal table + named mountain grant: {} legs, {} ticks",
                route.legs.len(), route.ticks);
        }
    }
}
