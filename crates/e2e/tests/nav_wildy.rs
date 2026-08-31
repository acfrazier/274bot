//! Live: the wilderness opt-in — the headless twin of the panel WalkTo
//! `allow wilderness` checkbox. A destination inside the wilderness zone
//! (e.g. z=3525 in the main zone) must NOT route through default
//! `find`/WalkTo (`FindOptions::default()`), and `find_with` with
//! `allow_wilderness: true` must produce a route that reaches it. The
//! panel checkbox → `arm_walk_on` → `find_with` path is pinned by the
//! panel unit tests; this test proves the live pack agrees. The pack must
//! also carry the Ardougne wilderness lever hops (locs 1814/1815): the
//! Ardougne→wilderness landing is an enter-wildy hop, so default `find`
//! from the lever tile must be NoPath while `find_with(allow_wilderness)`
//! routes through the lever alone.
//!
//! Run with the engine up and the rebaked nav pack at the standard path:
//! `LIVE=1 cargo test -p e2e --test nav_wildy -- --ignored --test-threads=1 --nocapture`

mod common;

use std::time::Duration;

use api::snapshot::WorldTile;
use common::{fail, live, options, profiles, wait_ingame};
use host_play::run_with_io;
use nav::router::{find, find_with, FindOptions};
use nav::transport::TransportKind;
use nav::wilderness::in_wilderness;
use nav::world::NavWorld;
use scenario::default_pack_path;

/// A wilderness tile in the main zone (surface pair decodes to
/// x 2944..3391, z 3520..6399). z=3525 at x=3100 is a blocked tree tile
/// in the pack, so the case uses the walkable z=3524 neighbour — still
/// north of the z=3520 zone edge.
const WILDY_DEST: WorldTile = WorldTile {
    x: 3100,
    z: 3524,
    level: 0,
};

/// The Ardougne lever (`wildinlever`, loc 1814) placement: m40_51 local
/// (1,47) = (2561,3311,0).
const ARDOUGNE_LEVER: WorldTile = WorldTile {
    x: 2561,
    z: 3311,
    level: 0,
};
/// Its `p_teleport` landing (`^ardougne_to_wilderness_coord` =
/// `0_49_61_18_20` = (3154,3924,0)), inside the surface zone.
const WILDY_LEVER_LANDING: WorldTile = WorldTile {
    x: 3154,
    z: 3924,
    level: 0,
};

#[test]
#[ignore = "requires a local 274 engine, nav pack, and LIVE=1"]
fn nav_wildy() {
    if !live() {
        return;
    }

    // Single slot with the mainland hop (lands the Lumbridge courtyard).
    let seed = [("test", "test")];
    let mut opts = options();
    opts.mainland = true;
    let play = run_with_io(&opts, profiles(&seed), |_| (None, None), |_, _| {});
    wait_ingame(&play, 1, Duration::from_secs(150), "nav_wildy");

    let world = NavWorld::load_pack(&default_pack_path())
        .unwrap_or_else(|e| fail(&format!("nav pack must load for wilderness routing: {e:?}")));
    let (tx, tz) = play
        .statuses()
        .iter()
        .find(|s| s.username == "test")
        .map(|s| (s.tile_x, s.tile_z))
        .expect("test slot reports a player tile");
    let from = WorldTile {
        x: tx,
        z: tz,
        level: 0,
    };
    if in_wilderness(from) {
        fail(&format!(
            "spawn tile {from:?} must be south of the ditch for this case"
        ));
    }
    if !in_wilderness(WILDY_DEST) {
        fail(&format!(
            "dest {WILDY_DEST:?} must be inside the wilderness zone"
        ));
    }

    // Default find / WalkTo: no route into the wilderness.
    if find(&world.collision, &world.graph, from, WILDY_DEST).is_ok() {
        fail("default find must not route into the wilderness");
    }
    // allow_wilderness: the same search enters and reaches the dest.
    let route = find_with(
        &world.collision,
        &world.graph,
        from,
        WILDY_DEST,
        FindOptions {
            allow_teleports: false,
            allow_wilderness: true,
            allow_bank_fetch: false,
        },
        &nav::WorldState::empty(),
    )
    .unwrap_or_else(|e| {
        fail(&format!(
            "allow_wilderness must route into the wilderness from {from:?}: {e:?}"
        ))
    });
    if route.dest != WILDY_DEST {
        fail(&format!(
            "allow_wilderness route must end at {WILDY_DEST:?}, ended at {:?}",
            route.dest
        ));
    }
    println!(
        "PASS: nav_wildy default find refused the wilderness from {from:?}; \
         allow_wilderness routed ({:.0} ticks)",
        route.ticks
    );

    // The pack carries the two Ardougne wilderness lever hops.
    let levers: Vec<_> = world
        .graph
        .edges
        .iter()
        .filter(|e| e.loc_id == 1814 || e.loc_id == 1815)
        .cloned()
        .collect();
    if levers.len() < 2 {
        fail(&format!(
            "nav_wildy: pack carries {} lever edges (locs 1814/1815), need >= 2 \
             (rebake with `cargo run -p nav --bin nav-pack`)",
            levers.len()
        ));
    }
    if !levers
        .iter()
        .any(|e| e.kind == TransportKind::Door && e.option == 1)
    {
        fail("nav_wildy: no lever edge carries the Pull op (Door kind, option 1)");
    }
    if !levers.iter().any(|e| in_wilderness(e.to)) {
        fail("nav_wildy: no lever edge lands inside the wilderness");
    }
    if !levers.iter().any(|e| !in_wilderness(e.to)) {
        fail("nav_wildy: no lever edge lands outside the wilderness");
    }

    // The Ardougne→wilderness hop is enter-wildy: default find from the
    // lever tile is NoPath, and allow_wilderness routes through the lever
    // alone (no other edge is needed for this leg).
    if find(
        &world.collision,
        &world.graph,
        ARDOUGNE_LEVER,
        WILDY_LEVER_LANDING,
    )
    .is_ok()
    {
        fail("nav_wildy: default find must not route the Ardougne→wildy lever");
    }
    find_with(
        &world.collision,
        &world.graph,
        ARDOUGNE_LEVER,
        WILDY_LEVER_LANDING,
        FindOptions {
            allow_teleports: false,
            allow_wilderness: true,
            allow_bank_fetch: false,
        },
        &nav::WorldState::empty(),
    )
    .map(|route| {
        if route.dest != WILDY_LEVER_LANDING {
            fail(&format!(
                "nav_wildy: lever route ends at {:?}, not {:?}",
                route.dest, WILDY_LEVER_LANDING
            ));
        }
        println!(
            "PASS: nav_wildy lever hop gated — default find refused it, \
             allow_wilderness routed ({:.0} ticks)",
            route.ticks
        );
    })
    .unwrap_or_else(|e| {
        fail(&format!(
            "nav_wildy: allow_wilderness must route the Ardougne lever \
             {ARDOUGNE_LEVER:?} -> {WILDY_LEVER_LANDING:?}: {e:?}"
        ))
    });
}
