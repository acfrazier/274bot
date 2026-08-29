//! Live: the Al Kharid border toll and the Shantay northbound hop —
//! item-gated gate crossings on the baked nav pack. The pack must carry
//! the two Al Kharid toll gates (`border_gate_toll_left`/`_right`, loc
//! 2882/2883, at the m51_50 (4,27)/(4,28) placements = (3268,3227)/
//! (3268,3228)) as Door edges with the 10-coin toll on `item_req`, and
//! the Shantay henge doorway (loc 4031, m51_48 (38,44) = (3302,3116))
//! as exactly one Door edge into the desert — `to` (3304,3115) (the
//! `[queue,shantay_pass_enter]` landing), one Shantay pass on
//! `item_req`. The free desert exit (the same script's `coordz <=
//! loc_coord` teleport-jump) must NOT become an edge. If the pack predates
//! the toll edges the test FAILS (exit 1) with a rebake hint.
//!
//! Run with the engine up and the rebaked nav pack at the standard path:
//! `LIVE=1 cargo test -p e2e --test nav_toll -- --ignored --test-threads=1 --nocapture`

mod common;

use std::time::Duration;

use api::snapshot::WorldTile;
use common::{fail, live, options, profiles, wait_ingame};
use host_play::run_with_io;
use nav::transport::TransportKind;
use nav::world::NavWorld;
use scenario::default_pack_path;

/// The left toll gate placement (m51_50 local (4,27) = (3268,3227)).
const TOLL_LEFT: WorldTile = WorldTile {
    x: 3268,
    z: 3227,
    level: 0,
};
/// The right toll gate placement (m51_50 local (4,28) = (3268,3228)).
const TOLL_RIGHT: WorldTile = WorldTile {
    x: 3268,
    z: 3228,
    level: 0,
};
/// The Shantay henge doorway placement (m51_48 local (38,44) =
/// (3302,3116)).
const SHANTAY_AT: WorldTile = WorldTile {
    x: 3302,
    z: 3116,
    level: 0,
};
/// Its gated northbound landing (`p_teleport(0_51_48_40_46)` +
/// `p_telejump(movecoord(coord,0,0,-3))`).
const SHANTAY_TO: WorldTile = WorldTile {
    x: 3304,
    z: 3115,
    level: 0,
};

#[test]
#[ignore = "requires a local 274 engine, nav pack, and LIVE=1"]
fn nav_toll() {
    if !live() {
        return;
    }

    // Single slot with the mainland hop (lands the Lumbridge courtyard).
    let seed = [("test", "test")];
    let mut opts = options();
    opts.mainland = true;
    let play = run_with_io(&opts, profiles(&seed), |_| (None, None), |_, _| {});
    wait_ingame(&play, 1, Duration::from_secs(150), "nav_toll");

    let world = NavWorld::load_pack(&default_pack_path())
        .unwrap_or_else(|e| fail(&format!("nav pack must load for toll routing: {e:?}")));

    // Both Al Kharid toll gates derive their two crossings, carrying the
    // 10-coin toll.
    let tolls: Vec<_> = world
        .graph
        .edges
        .iter()
        .filter(|e| e.kind == TransportKind::Door && (e.loc_id == 2882 || e.loc_id == 2883))
        .cloned()
        .collect();
    if tolls.len() < 4 {
        fail(&format!(
            "nav_toll: pack carries {} toll-gate edges (locs 2882/2883), need 4 \
             (rebake with `cargo run -p nav --bin nav-pack`)",
            tolls.len()
        ));
    }
    if tolls.iter().filter(|e| e.loc_id == 2882).count() < 2 {
        fail("nav_toll: no crossing edges for the left toll gate (loc 2882)");
    }
    if tolls.iter().filter(|e| e.loc_id == 2883).count() < 2 {
        fail("nav_toll: no crossing edges for the right toll gate (loc 2883)");
    }
    for e in &tolls {
        if e.at != TOLL_LEFT && e.at != TOLL_RIGHT {
            fail(&format!(
                "nav_toll: toll-gate edge at {:?} must be on a gate tile",
                e.at
            ));
        }
        if !e.item_req.iter().any(|(id, n)| *id == 995 && *n >= 10) {
            fail(&format!(
                "nav_toll: toll-gate edge {e:?} lacks the 10-coin toll"
            ));
        }
    }

    // The Shantay henge carries exactly the one gated northbound hop.
    let henge: Vec<_> = world
        .graph
        .edges
        .iter()
        .filter(|e| e.loc_id == 4031)
        .cloned()
        .collect();
    if henge.len() != 1 {
        fail(&format!(
            "nav_toll: pack carries {} Shantay henge edges (loc 4031), need exactly the \
             gated northbound hop (rebake with `cargo run -p nav --bin nav-pack`)",
            henge.len()
        ));
    }
    if henge[0].at != SHANTAY_AT || henge[0].to != SHANTAY_TO {
        fail(&format!(
            "nav_toll: Shantay hop is {:?} -> {:?}, expected {SHANTAY_AT:?} -> {SHANTAY_TO:?}",
            henge[0].at, henge[0].to
        ));
    }
    if !henge[0]
        .item_req
        .iter()
        .any(|(id, n)| *id == 1854 && *n >= 1)
    {
        fail("nav_toll: Shantay northbound hop lacks the Shantay pass on item_req");
    }

    println!(
        "PASS: nav_toll pack carries {} toll-gate edges with the 10-coin toll and the \
         gated Shantay northbound hop ({} edges, {} doors)",
        tolls.len(),
        world.graph.edges.len(),
        world
            .graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Door)
            .count()
    );
}
