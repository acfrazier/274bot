//! Live: the Zanaris shed door on the wire — the live twin of the nav
//! `derive_transports_emits_zanaris_shed_door_with_worn_dramen` unit
//! test. Waits for the slot `ingame && scene_state == 2`, then loads the
//! process nav pack and asserts the shed `TransportKind::Door` edge is on
//! it: non-empty `worn_req` (the Dramen staff) and the Zanaris landing
//! (`0_50_149_20_56` = (3220,9592), `to.x > 3000 && to.z > 9000` — not the
//! Lumbridge swamp), and that `find` still routes from the shed door's
//! tile to the landing on the pack. Either failure exits 1 (rs2b0t style).
//! Route detail prints only under `BOT_DEBUG=1`.
//!
//! Run with the engine up and the rebaked v5 pack at the standard path:
//! `LIVE=1 cargo test -p e2e --test nav_zanaris -- --ignored --test-threads=1 --nocapture`

mod common;

use std::time::Duration;

use api::snapshot::WorldTile;
use common::{fail, live, options, profiles, wait_ingame};
use host_play::run_with_io;
use nav::router::find;
use nav::transport::TransportKind;
use nav::world::NavWorld;
use scenario::default_pack_path;

/// The Lumbridge swamp shed door (`zanarisdoor`, loc 2406, m50_49 local
/// (2,33)): the door loc tile the Open op is used on.
const SHED_DOOR: WorldTile = WorldTile {
    x: 3202,
    z: 3169,
    level: 0,
};
/// The Zanaris landing (`0_50_149_20_56` = (3220,9592)): in the 6400-cellar
/// band, `to.x > 3000 && to.z > 9000`.
const ZANARIS: WorldTile = WorldTile {
    x: 3220,
    z: 9592,
    level: 0,
};
/// The Dramen staff (`pack/obj.pack` 772) the door's open script requires
/// worn (`inv_total(worn, dramen_staff) > 0`).
const DRAMEN_STAFF: i32 = 772;

#[test]
#[ignore = "requires a local 274 engine, a rebaked v5 nav pack, and LIVE=1"]
fn nav_zanaris() {
    if !live() {
        return;
    }

    // Single slot with the mainland hop (lands the Lumbridge courtyard).
    let seed = [("test", "test")];
    let mut opts = options();
    opts.mainland = true;
    let play = run_with_io(&opts, profiles(&seed), |_| (None, None), |_, _| {});
    wait_ingame(&play, 1, Duration::from_secs(150), "nav_zanaris");

    let world = NavWorld::load_pack(&default_pack_path())
        .unwrap_or_else(|e| fail(&format!("nav_zanaris: nav pack must load: {e:?}")));
    let shed: Vec<_> = world
        .graph
        .edges
        .iter()
        .filter(|e| e.kind == TransportKind::Door && !e.worn_req.is_empty())
        .cloned()
        .collect();
    if shed.is_empty() {
        fail(&format!(
            "nav_zanaris: pack carries no shed Door edge with a worn req \
             (rebake with `cargo run -p nav --bin nav-pack`)"
        ));
    }
    let e = &shed[0];
    if e.worn_req.is_empty() {
        fail("nav_zanaris: shed door carries an empty worn_req");
    }
    if !(e.to.x > 3000 && e.to.z > 9000) {
        fail(&format!(
            "nav_zanaris: shed door lands at {:?}, not the Zanaris landing \
             ({ZANARIS:?}); a rebake is needed",
            e.to
        ));
    }
    if e.to != ZANARIS {
        fail(&format!(
            "nav_zanaris: shed door lands at {:?}, expected the Zanaris \
             landing {ZANARIS:?}",
            e.to
        ));
    }
    if e.loc_id != 2406 {
        fail(&format!("nav_zanaris: shed door loc is {}, expected zanarisdoor 2406", e.loc_id));
    }
    if !e.worn_req.contains(&DRAMEN_STAFF) {
        fail(&format!(
            "nav_zanaris: shed door worn_req {:?} lacks the Dramen staff {DRAMEN_STAFF}",
            e.worn_req
        ));
    }
    // The shed door's tile reaches the Zanaris landing through the hop.
    find(&world.collision, &world.graph, SHED_DOOR, ZANARIS)
        .map(|route| {
            if route.dest != ZANARIS {
                fail(&format!(
                    "nav_zanaris: route ends at {:?}, not the landing {ZANARIS:?}",
                    route.dest
                ));
            }
            if nav::debug_enabled() {
                println!(
                    "nav_zanaris: shed door -> Zanaris routed ({:.0} ticks, {} legs)",
                    route.ticks,
                    route.legs.len()
                );
            }
        })
        .unwrap_or_else(|e| {
            fail(&format!(
                "nav_zanaris: shed door {SHED_DOOR:?} -> Zanaris {ZANARIS:?} \
                 is NoPath ({e:?})"
            ))
        });
    println!(
        "PASS: nav_zanaris pack carries the shed door ({} edges, {} doors)",
        world.graph.edges.len(),
        world
            .graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Door)
            .count()
    );
}
