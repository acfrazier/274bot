//! Live: Shilo↔Brimhaven cart edges on the wire — the live twin of the nav
//! `derive_transports_emits_shilo_brimhaven_cart` unit test. Two tests:
//!
//! - `nav_cart`: the pack+`find` proof. Waits for the slot
//!   `ingame && scene_state == 2`, loads the process nav pack and asserts
//!   the two Npc cart edges are on it (coins on the fare, the Shilo
//!   Village journal name on the Brim→Shilo hop), and that `find_with`
//!   routes between the two cart driver tiles on the pack for a state
//!   that can pay the fare (200 coins + the Shilo Village quest). This is
//!   a pack proof, not execute.
//! - `nav_cart_follow`: the execute twin — the same `nav_cart` scenario
//!   `panel-play --live script_nav_cart` drives. The scenario cheat-teles
//!   to the Shilo cart driver, `Follow`s to the Brimhaven cart landing (a
//!   destination that requires the cart hop), the traveller answers the
//!   driver's fare dialog, and PASSes on `TravelOutcome::Arrived`.
//!
//! Run with the engine up and the rebaked v6 pack at the standard path:
//! `LIVE=1 cargo test -p e2e --test nav_cart -- --ignored --test-threads=1 --nocapture`

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use api::snapshot::WorldTile;
use common::{fail, live, mint_seed, options, profiles, wait_ingame};
use host_play::run_with_io;
use nav::router::find_with;
use nav::transport::TransportKind;
use nav::world::NavWorld;
use scenario::{default_pack_path, RunnerStatus, ScenarioRunner};

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

/// A state that can take the Brimhaven→Shilo cart: the 200-coin fare in
/// the inventory and the Shilo Village quest complete.
fn fare_state() -> nav::WorldState {
    nav::WorldState {
        inv: HashMap::from([(995, 200)]),
        quests: ["Shilo Village".to_string()].into(),
        ..nav::WorldState::default()
    }
}

#[test]
#[ignore = "requires a local 274 engine, a rebaked v6 nav pack, and LIVE=1"]
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
    // The two carts are reachable on the pack for a state that can pay
    // the fare (find gates every requirement fail-closed since Task 1).
    find_with(
        &world.collision,
        &world.graph,
        BRIM_DRIVER,
        SHILO_DRIVER,
        nav::router::FindOptions::default(),
        &fare_state(),
    )
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

/// The execute twin: the `nav_cart` scenario run headlessly, exactly like
/// `panel-play --live script_nav_cart`. One slot; PASS is the runner's
/// proof (`arrived` at the Brimhaven cart landing).
#[test]
#[ignore = "requires a local 274 engine, nav pack, and LIVE=1"]
fn nav_cart_follow() {
    if !live() {
        return;
    }

    let scenario = scenario::get("nav_cart").expect("nav_cart scenario in registry");
    let mainland = scenario.seed.mainland;
    let n = scenario.seed.profiles.len();
    let runner = Arc::new(Mutex::new(ScenarioRunner::new(scenario)));
    // Mint a fresh per-run account: the engine auto-registers unknown
    // names, so this run never logs the shared `test` save.
    let entries = {
        let mut r = runner.lock().unwrap();
        r.set_shot_sink(Box::new(|_, _| {}));
        mint_seed(&mut r, n)
    };
    let mut opts = options();
    opts.mainland = mainland;
    let play = run_with_io(&opts, profiles(&entries), |_| (None, None), {
        let runner = Arc::clone(&runner);
        move |c, name| {
            let mut r = runner.lock().unwrap();
            if r.drives(name) {
                r.tick(c);
            } else if let Some(index) = r.companion_for(name) {
                r.companion_tick(index, c);
            }
        }
    });
    runner.lock().unwrap().set_obj_names(play.obj_names());

    wait_ingame(&play, 1, Duration::from_secs(150), "nav_cart_follow");

    let deadline = Instant::now() + Duration::from_secs(360);
    loop {
        let (status, evidence) = {
            let r = runner.lock().unwrap();
            (r.status(), r.evidence().cloned())
        };
        let record = evidence.as_ref().map(|ev| ev.to_json()).unwrap_or_default();
        match status {
            RunnerStatus::Passed => {
                println!("PASS: nav_cart_follow {record}");
                return;
            }
            RunnerStatus::Failed(msg) => {
                eprintln!("FAIL: nav_cart_follow {record}");
                fail(&format!("nav_cart_follow: {msg}"));
            }
            other => {
                if Instant::now() >= deadline {
                    fail(&format!(
                        "nav_cart_follow: no terminal status within 360s ({other:?})"
                    ));
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    }
}
