//! Live: the Al Kharid border toll and the Shantay-pass edges — item-
//! gated gate crossings on the baked nav pack. The pack must carry the
//! two Al Kharid toll gates (`border_gate_toll_left`/`_right`, loc
//! 2882/2883, at the m51_50 (4,27)/(4,28) placements = (3268,3227)/
//! (3268,3228)) as Door edges with the 10-coin toll on `item_req`, and
//! the Shantay henge doorway (loc 4031, m51_48 (38,44) = (3302,3116))
//! as exactly **two** Door edges, one per `shantay_pass.rs2`
//! `[oploc1,shantay_pass_henge_doorway]` branch — the gated hop into
//! the desert (`at` the placement, `to` (3304,3115) — the
//! `[queue,shantay_pass_enter]` landing — one Shantay pass on
//! `item_req`) and the free desert exit (`at` (3302,3115) on the desert
//! side, `to` (3303,3118) — the `coordz <= loc_coord`
//! `p_telejump(movecoord(coord,0,0,3))` landing — no `item_req`). Only
//! the gated hop carries the pass: the desert exit is **not** a plain
//! walk, it is an `op_loc` interaction with the henge. If the pack
//! predates the toll edges the test FAILS (exit 1) with a rebake hint.
//!
//! Run with the engine up and the rebaked nav pack at the standard path:
//! `LIVE=1 cargo test -p e2e --test nav_toll -- --ignored --test-threads=1 --nocapture`
//!
//! `nav_shantay_follow` is the live twin of the `script_nav_shantay`
//! scenario: desert → pass without a pass in the inventory, then pass →
//! desert with the pass given.

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use api::snapshot::WorldTile;
use common::{fail, live, mint_seed, options, profiles, wait_ingame};
use host_play::run_with_io;
use nav::transport::TransportKind;
use nav::world::NavWorld;
use scenario::{default_pack_path, RunnerStatus, ScenarioRunner};

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
/// The gated hop's landing (`p_teleport(0_51_48_40_46)` +
/// `p_telejump(movecoord(coord,0,0,-3))`).
const SHANTAY_TO: WorldTile = WorldTile {
    x: 3304,
    z: 3115,
    level: 0,
};
/// The free desert exit's stand (`at` one tile south of the placement).
const SHANTAY_DESERT_AT: WorldTile = WorldTile {
    x: 3302,
    z: 3115,
    level: 0,
};
/// The free desert exit's landing (the desert-side
/// `p_telejump(movecoord(coord,0,0,3))` from (3303,3115)).
const SHANTAY_DESERT_TO: WorldTile = WorldTile {
    x: 3303,
    z: 3118,
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
    let play = run_with_io(&opts, profiles(&seed), |_| (None, None), |_, _, _| {});
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

    // The Shantay henge carries exactly two edges: the gated desert hop
    // (the only one with the pass) and the free desert exit.
    let henge: Vec<_> = world
        .graph
        .edges
        .iter()
        .filter(|e| e.loc_id == 4031)
        .cloned()
        .collect();
    if henge.len() != 2 {
        fail(&format!(
            "nav_toll: pack carries {} Shantay henge edges (loc 4031), need exactly 2 \
             (the gated desert hop and the free desert exit; rebake with \
             `cargo run -p nav --bin nav-pack`)",
            henge.len()
        ));
    }
    let gated = henge
        .iter()
        .find(|e| !e.item_req.is_empty())
        .unwrap_or_else(|| fail("nav_toll: no Shantay henge edge carries the pass"));
    let free = henge
        .iter()
        .find(|e| e.item_req.is_empty())
        .unwrap_or_else(|| fail("nav_toll: no free Shantay henge edge (desert exit)"));
    if gated.at != SHANTAY_AT || gated.to != SHANTAY_TO {
        fail(&format!(
            "nav_toll: Shantay gated hop is {:?} -> {:?}, expected {SHANTAY_AT:?} -> {SHANTAY_TO:?}",
            gated.at, gated.to
        ));
    }
    if !gated.item_req.iter().any(|(id, n)| *id == 1854 && *n >= 1) {
        fail("nav_toll: Shantay gated hop lacks the Shantay pass on item_req");
    }
    if free.at != SHANTAY_DESERT_AT || free.to != SHANTAY_DESERT_TO {
        fail(&format!(
            "nav_toll: Shantay desert exit is {:?} -> {:?}, expected {SHANTAY_DESERT_AT:?} -> \
             {SHANTAY_DESERT_TO:?}",
            free.at, free.to
        ));
    }

    println!(
        "PASS: nav_toll pack carries {} toll-gate edges with the 10-coin toll and exactly two \
         Shantay henge edges (the pass-gated desert hop and the free desert exit) ({} edges, {} \
         doors)",
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

/// The execute twin: the `script_nav_shantay` scenario run headlessly,
/// exactly like `panel-play --live script_nav_shantay`. One slot; the
/// scenario drives the desert → pass leg with an empty inventory and the
/// pass → desert leg after `give`-ing the pass. PASS is the runner's
/// proof (`arrived` at the desert dest).
#[test]
#[ignore = "requires a local 274 engine, nav pack, and LIVE=1"]
fn nav_shantay_follow() {
    if !live() {
        return;
    }

    let scenario = scenario::get("nav_shantay").expect("nav_shantay scenario in registry");
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
        move |c, name, hold| {
            let mut r = runner.lock().unwrap();
            if r.drives(name) {
                r.tick_with_hold(c, hold);
            } else if let Some(index) = r.companion_for(name) {
                r.companion_tick(index, c);
            }
        }
    });
    runner.lock().unwrap().set_obj_names(play.obj_names());

    wait_ingame(&play, 1, Duration::from_secs(150), "nav_shantay_follow");

    let deadline = Instant::now() + Duration::from_secs(420);
    loop {
        let (status, evidence) = {
            let r = runner.lock().unwrap();
            (r.status(), r.evidence().cloned())
        };
        let record = evidence.as_ref().map(|ev| ev.to_json()).unwrap_or_default();
        match status {
            RunnerStatus::Passed => {
                println!("PASS: nav_shantay_follow {record}");
                return;
            }
            RunnerStatus::Failed(msg) => {
                eprintln!("FAIL: nav_shantay_follow {record}");
                fail(&format!("nav_shantay_follow: {msg}"));
            }
            other => {
                if Instant::now() >= deadline {
                    fail(&format!(
                        "nav_shantay_follow: no terminal status within 420s ({other:?})"
                    ));
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    }
}
