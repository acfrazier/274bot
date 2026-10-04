//! Raw Ice Dungeon connectivity is independent of the ordinary ice-warrior zone refusal.
use std::path::PathBuf;

use api::snapshot::WorldTile;
use nav::router::{find_with, FindOptions, Leg};
use nav::world::NavWorld;
use nav::zones::ZoneExempt;
use nav::WorldState;

#[test]
#[ignore = "requires NAV_QUEST_DOORS_PACK naming a freshly baked real 289 pack"]
fn ice_dungeon_ladder_and_blurite_share_the_raw_walk_surface() {
    let pack = PathBuf::from(std::env::var_os("NAV_QUEST_DOORS_PACK").expect("explicit pack"));
    let world = NavWorld::load_pack(&pack).expect("real 289 pack");
    let from = WorldTile {
        x: 3008,
        z: 9551,
        level: 0,
    };
    // This is deliberately geometry-only, not a product policy workaround.
    // At combat 51, the ordinary ice-warrior zones correctly refuse transit.
    let options = FindOptions {
        zones: ZoneExempt::all(),
        ..FindOptions::default()
    };
    let state = WorldState::empty();
    for (x, z) in [(3048, 9567), (3060, 9580)] {
        let to = WorldTile { x, z, level: 0 };
        let route = find_with(&world.collision, &world.graph, from, to, options, &state)
            .unwrap_or_else(|error| panic!("raw Ice Dungeon {from:?} -> {to:?}: {error:?}"));
        assert_eq!(route.dest, to);
        assert!(
            matches!(route.legs.as_slice(), [Leg::Walk { .. }]),
            "the cave is connected by walking, not by an invented transport: {:?}",
            route.legs
        );
    }
}
