//! The land route around the back of White Wolf Mountain. The mountain's pack
//! wolves and sentries (`vislevel=25`, hunt mode `cowardly` with
//! `check_nottoostrong=outside_wilderness`) wander the back ridge but never
//! hunt a player above combat 50; every other hunter there must be avoided by
//! its collision-flood membership.
use std::path::PathBuf;

use api::WorldTile;
use nav::router::{find_blocking_zones, find_with, FindOptions, Leg};
use nav::world::NavWorld;
use nav::zones::ZoneClass;
use nav::WorldState;

fn tile(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

fn state(combat_level: Option<i32>) -> WorldState {
    let mut state = WorldState::empty().with_map_members(true);
    state.combat_level = combat_level;
    state
}

/// `(name, from x, from z, to x, to z)`.
type Route = (&'static str, i32, i32, i32, i32);

const ROUTES: [Route; 2] = [
    ("Taverley->Catherby", 2895, 3450, 2809, 3440),
    ("Lumbridge->Ardougne", 3220, 3211, 2661, 3301),
];

#[test]
#[ignore = "requires NAV_WWM_PACK naming a freshly baked real 289 pack"]
fn combat_51_walks_around_the_back_of_white_wolf_mountain() {
    let pack = PathBuf::from(std::env::var_os("NAV_WWM_PACK").expect("explicit pack"));
    let world = NavWorld::load_pack(&pack).expect("real 289 pack");
    let table = world.graph.zones.as_ref().expect("baked zones");
    let combat = 51;
    for (name, from_x, from_z, to_x, to_z) in ROUTES {
        let (from, to) = (tile(from_x, from_z), tile(to_x, to_z));
        let route = find_with(
            &world.collision,
            &world.graph,
            from,
            to,
            FindOptions::default(),
            &state(Some(combat)),
        )
        .unwrap_or_else(|error| panic!("{name}: {error:?}"));
        assert_eq!(route.dest, to);
        let mut tiles = Vec::new();
        for leg in &route.legs {
            match leg {
                Leg::Walk { tiles: walk } => tiles.extend(walk.iter().copied()),
                Leg::Transport { edge } => tiles.extend([edge.at, edge.to]),
            }
        }
        let mut inactive = std::collections::BTreeSet::new();
        for step in &tiles {
            for index in table.at(*step) {
                let zone = &table.zones()[usize::from(index)];
                let kind = &table.kinds()[usize::from(zone.kind)];
                let active = zone.class == ZoneClass::Always || i32::from(zone.cap) >= combat;
                assert!(
                    !active,
                    "{name}: {step:?} is inside the membership of {}@{},{} (cap {})",
                    kind.id, zone.spawn_x, zone.spawn_z, zone.cap
                );
                let group = (zone.group != nav::zones::NO_GROUP)
                    .then(|| table.groups()[usize::from(zone.group)].id.as_ref());
                assert!(
                    group != Some("white-wolf-mountain") || (kind.vislevel == 25 && zone.cap == 50),
                    "{name}: only the vislevel-25 wolves may be crossed on the mountain, found {}",
                    kind.id
                );
                inactive.insert(format!("{}@{},{}", kind.id, zone.spawn_x, zone.spawn_z));
            }
        }
        let max_z = tiles
            .iter()
            .filter(|t| (2817..=2860).contains(&t.x))
            .map(|t| t.z)
            .max();
        assert!(
            max_z.is_some_and(|z| z >= 3520),
            "{name}: crosses the back of the mountain, not its southern pass"
        );
        assert!(
            tiles
                .iter()
                .any(|t| t.x <= 2800 && (3480..=3530).contains(&t.z)),
            "{name}: comes down the mountain's west side"
        );
        println!(
            "{name}: {} tiles, {} ticks, inside only combat-capped memberships {:?}",
            tiles.len(),
            route.ticks,
            inactive
        );
    }
}

#[test]
#[ignore = "requires NAV_WWM_PACK naming a freshly baked real 289 pack"]
fn unknown_or_low_combat_is_refused_at_white_wolf_mountain() {
    let pack = PathBuf::from(std::env::var_os("NAV_WWM_PACK").expect("explicit pack"));
    let world = NavWorld::load_pack(&pack).expect("real 289 pack");
    let table = world.graph.zones.as_ref().expect("baked zones");
    for combat in [None, Some(3), Some(50)] {
        for (name, from_x, from_z, to_x, to_z) in ROUTES {
            let (from, to) = (tile(from_x, from_z), tile(to_x, to_z));
            assert!(
                find_with(
                    &world.collision,
                    &world.graph,
                    from,
                    to,
                    FindOptions::default(),
                    &state(combat),
                )
                .is_err(),
                "{name} combat {combat:?}: the vislevel-25 wolves walk the back ridge"
            );
            let names: Vec<String> = find_blocking_zones(
                &world.collision,
                &world.graph,
                from,
                to,
                FindOptions::default(),
                &state(combat),
                &[],
            )
            .expect("a zone-attributed refusal")
            .into_iter()
            .map(|key| table.name(key))
            .collect();
            assert!(
                names.iter().any(|name| name == "white-wolf-mountain"),
                "{name} combat {combat:?}: {names:?}"
            );
        }
    }
}
