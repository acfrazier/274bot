//! Live: Shilo↔Brimhaven cart edges on the wire — the live twin of the nav
//! `derive_transports_emits_shilo_brimhaven_cart` unit test. Waits for the
//! slot `ingame && scene_state == 2`, then loads the process nav pack and
//! asserts the two Npc cart edges are on it (coins on the fare, the Shilo
//! Village journal name on the Brim→Shilo hop), and that `find` still
//! routes between the two cart driver tiles on the pack. Either failure
//! exits 1 (rs2b0t style). Route detail prints only under `BOT_DEBUG=1`.
//!
//! Run with the engine up and the rebaked v5 pack at the standard path:
//! `LIVE=1 cargo test -p e2e --test nav_cart -- --ignored --test-threads=1 --nocapture`

mod common;

use std::time::Duration;

use api::snapshot::WorldTile;
use common::{fail, live, options, profiles, wait_ingame};
use host_play::run_with_io;
use nav::router::find;
use nav::transport::TransportKind;
use nav::world::NavWorld;
use scenario::default_pack_path;

/// Hajedy (brimhavencartdriver, npc 510) by the Brimhaven cart
/// (m43_50 local (27,11)).
const BRIM_DRIVER: WorldTile = WorldTile {
    x: 2779,
    z: 3211,
    level: 0,
};
/// Vigroy (shilocartdriver, npc 511) at the Shilo Village cart
/// (m44_46 local (18,10)).
const SHILO_DRIVER: WorldTile = WorldTile {
    x: 2834,
    z: 2954,
    level: 0,
};

#[test]
#[ignore = "requires a local 274 engine, a rebaked v5 nav pack, and LIVE=1"]
fn nav_cart() {
    if !live() {
        return;
    }

    // Single slot with the mainland hop (lands the Lumbridge courtyard).
    let seed = [("test", "test")];
    let mut opts = options();
    opts.mainland = true;
    let play = run_with_io(&opts, profiles(&seed), |_| (None, None), |_, _| {});
    wait_ingame(&play, 1, Duration::from_secs(150), "nav_cart");

    let world = NavWorld::load_pack(&default_pack_path())
        .unwrap_or_else(|e| fail(&format!("nav_cart: nav pack must load: {e:?}")));
    let carts: Vec<_> = world
        .graph
        .edges
        .iter()
        .filter(|e| e.kind == TransportKind::Npc)
        .cloned()
        .collect();
    if carts.len() < 2 {
        fail(&format!(
            "nav_cart: pack carries {} Npc cart edges, need >= 2 \
             (rebake with `cargo run -p nav --bin nav-pack`)",
            carts.len()
        ));
    }
    if !carts.iter().any(|e| !e.item_req.is_empty()) {
        fail("nav_cart: no cart edge carries the coins fare on item_req");
    }
    if !carts
        .iter()
        .any(|e| e.quest_req.iter().any(|q| q.contains("Shilo Village")))
    {
        fail("nav_cart: no cart edge carries the Shilo Village quest req");
    }
    // The two carts are reachable on the pack (walk and/or cart hop).
    find(&world.collision, &world.graph, BRIM_DRIVER, SHILO_DRIVER)
        .map(|route| {
            if route.dest != SHILO_DRIVER {
                fail(&format!(
                    "nav_cart: route ends at {:?}, not the shilo cart {:?}",
                    route.dest, SHILO_DRIVER
                ));
            }
            if nav::debug_enabled() {
                println!(
                    "nav_cart: brimhaven cart -> shilo cart routed ({:.0} ticks, {} legs)",
                    route.ticks,
                    route.legs.len()
                );
            }
        })
        .unwrap_or_else(|e| {
            fail(&format!(
                "nav_cart: brimhaven cart {BRIM_DRIVER:?} -> shilo cart {SHILO_DRIVER:?} \
                 is NoPath ({e:?})"
            ))
        });
}
