//! Live: the fence/gate seam — the headless twin of the panel WalkTo
//! Seers street -> rock crabs. `find` (default `FindOptions`, matching
//! WalkTo) must route from Seers street (2725,3485,0) to the rock-crab
//! shore (2710,3720,0) once `gates.loc` joins the door set. This is the
//! live proof of the same assertion the `nav` lib test
//! `seers_street_reaches_rock_crabs_after_gates` pins: if the flood is
//! still two components the test FAILS (exit 1) — the signal to hunt the
//! seam, never to paper over it with a fake corridor or a bank door.
//!
//! Run with the engine up and the rebaked nav pack at the standard path:
//! `LIVE=1 cargo test -p e2e --test nav_gates -- --ignored --test-threads=1 --nocapture`

mod common;

use std::time::Duration;

use api::snapshot::WorldTile;
use common::{fail, live, options, profiles, wait_ingame};
use host_play::run_with_io;
use nav::router::find;
use nav::world::NavWorld;
use scenario::default_pack_path;

/// Seers street, south of the fence seam.
const SEERS_STREET: WorldTile = WorldTile {
    x: 2725,
    z: 3485,
    level: 0,
};
/// The rock-crab shore north of Rellekka.
const ROCK_CRABS: WorldTile = WorldTile {
    x: 2710,
    z: 3720,
    level: 0,
};

#[test]
#[ignore = "requires a local 274 engine, nav pack, and LIVE=1"]
fn nav_gates() {
    if !live() {
        return;
    }

    // Single slot with the mainland hop (lands the Lumbridge courtyard).
    let seed = [("test", "test")];
    let mut opts = options();
    opts.mainland = true;
    let play = run_with_io(&opts, profiles(&seed), |_| (None, None), |_, _, _| {});
    wait_ingame(&play, 1, Duration::from_secs(150), "nav_gates");

    let world = NavWorld::load_pack(&default_pack_path())
        .unwrap_or_else(|e| fail(&format!("nav pack must load for gate routing: {e:?}")));
    find(&world.collision, &world.graph, SEERS_STREET, ROCK_CRABS)
        .map(|route| {
            println!(
                "PASS: nav_gates Seers street -> rock crabs routed ({:.0} ticks, {} legs)",
                route.ticks,
                route.legs.len()
            );
        })
        .unwrap_or_else(|e| {
            fail(&format!(
                "nav_gates: Seers street {SEERS_STREET:?} -> rock crabs {ROCK_CRABS:?} is NoPath \
                 ({e:?}) — the flood is still two components; gates.loc did not bridge the seam"
            ))
        });
}
