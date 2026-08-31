//! Live: the packed jewellery Teleport edge executes on the wire — the
//! live twin of the traveller's
//! `follow_jewellery_teleport_rubs_the_packed_item_and_arrives` unit
//! test. Waits for the slot `ingame && scene_state == 2`, then loads the
//! process nav pack and asserts the charged dueling ring's rub edge
//! (obj 2552, `opheld4` Rub) is on it: `to` the Al Kharid Duel Arena
//! (3315,3235), carrying the charged item as its `item_req`, and that
//! the layer routes the arena through that rub only while the ring is
//! held (`find_allow_teleports` routes the rub edge; without the item
//! the edge stays refused — the 2-tick rub always beats the walk in the
//! baked cost model). Either failure exits 1 (rs2b0t style). Route
//! detail prints only under `BOT_DEBUG=1`.
//!
//! `nav_tele_follow` is the execute twin — the same `nav_tele` scenario
//! `panel-play --live script_nav_tele` drives. The scenario cheat-gives
//! the charged dueling ring, `Follow`s the packed rub edge to the Duel
//! Arena with `allow_teleports` on, and PASSes on
//! `TravelOutcome::Arrived` within the packed landing's scatter radius —
//! the traveller rubs the held item (`OpTarget::Item` + option 4), never
//! the WalkTo `::tele` cheat.
//!
//! Run with the engine up and the rebaked nav pack at the standard path:
//! `LIVE=1 cargo test -p e2e --test nav_tele -- --ignored --test-threads=1 --nocapture`

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use api::snapshot::WorldTile;
use common::{fail, live, mint_seed, options, profiles, wait_ingame};
use host_play::run_with_io;
use nav::router::{find_allow_teleports, find_with, FindOptions, Leg};
use nav::transport::TransportKind;
use nav::world::NavWorld;
use scenario::{default_pack_path, RunnerStatus, ScenarioRunner};

/// The Al Kharid Duel Arena (m51_50 local (51,35)): the packed
/// `ring_of_dueling_8` rub edge's `to` — the landing anchor the live
/// `map_findsquare` scatter lands within chebyshev 2 of.
const DUEL_ARENA: WorldTile = WorldTile {
    x: 3315,
    z: 3235,
    level: 0,
};
/// The mainland hop's landing tile: Lumbridge courtyard (m50_50 local
/// (20,20)) — where the scenario's Follow arms from.
const COURTYARD: WorldTile = WorldTile {
    x: 3220,
    z: 3220,
    level: 0,
};

/// The charged dueling ring obj id (`ring_of_dueling_8`).
const RING: i32 = 2552;

/// A state holding the charged ring: the packed rub edge's `item_req`.
fn ring_state() -> nav::WorldState {
    nav::WorldState {
        inv: HashMap::from([(RING, 1)]),
        ..nav::WorldState::default()
    }
}

#[test]
#[ignore = "requires a local 274 engine, a rebaked v5 nav pack, and LIVE=1"]
fn nav_tele() {
    if !live() {
        return;
    }

    // Single slot with the mainland hop (lands the Lumbridge courtyard).
    let seed = [("test", "test")];
    let mut opts = options();
    opts.mainland = true;
    let play = run_with_io(&opts, profiles(&seed), |_| (None, None), |_, _| {});
    wait_ingame(&play, 1, Duration::from_secs(150), "nav_tele");

    let world = NavWorld::load_pack(&default_pack_path())
        .unwrap_or_else(|e| fail(&format!("nav_tele: nav pack must load: {e:?}")));
    let ring: Vec<_> = world
        .graph
        .teleports
        .iter()
        .filter(|e| e.kind == TransportKind::Teleport && e.loc_id == RING)
        .cloned()
        .collect();
    if ring.len() != 1 {
        fail(&format!(
            "nav_tele: pack carries {} dueling-ring rub edges, need 1 \
             (rebake with `cargo run -p nav --bin nav-pack`)",
            ring.len()
        ));
    }
    let e = &ring[0];
    if e.to != DUEL_ARENA {
        fail(&format!(
            "nav_tele: ring rub edge lands at {:?}, not the Duel Arena {DUEL_ARENA:?} \
             (rebake with `cargo run -p nav --bin nav-pack`)",
            e.to
        ));
    }
    if e.option != 4 {
        fail(&format!("nav_tele: ring rub edge is not Rub (op 4): {e:?}"));
    }
    if !e.item_req.iter().any(|&(id, n)| id == RING && n >= 1) {
        fail(&format!(
            "nav_tele: ring rub edge lacks the charged item req: {e:?}"
        ));
    }
    // The arena is walkable in the bake, so the layer's contribution is
    // the rub leg itself: without the ring the gated rub edge must never
    // route (the search falls back to the plain walk), and with the ring
    // the 2-tick rub always beats the walk.
    let empty = find_allow_teleports(
        &world.collision,
        &world.graph,
        COURTYARD,
        DUEL_ARENA,
        &nav::WorldState::default(),
    )
    .unwrap_or_else(|e| {
        fail(&format!(
            "nav_tele: courtyard -> Duel Arena without the ring is NoPath ({e:?})"
        ))
    });
    if empty.legs.iter().any(|l| {
        matches!(
            l,
            Leg::Transport { edge } if edge.kind == TransportKind::Teleport
        )
    }) {
        fail(&format!(
            "nav_tele: the rub edge routed without the ring ({:?}) — item_req not enforced",
            empty.legs
        ));
    }
    let routed = find_allow_teleports(
        &world.collision,
        &world.graph,
        COURTYARD,
        DUEL_ARENA,
        &ring_state(),
    )
    .unwrap_or_else(|e| {
        fail(&format!(
            "nav_tele: courtyard -> Duel Arena with the ring held is NoPath ({e:?})"
        ))
    });
    if routed.dest != DUEL_ARENA {
        fail(&format!(
            "nav_tele: route ends at {:?}, not the Duel Arena {DUEL_ARENA:?}",
            routed.dest
        ));
    }
    let rub = routed
        .legs
        .iter()
        .find(|l| {
            matches!(
                l,
                Leg::Transport { edge }
                    if edge.kind == TransportKind::Teleport && edge.loc_id == RING
            )
        })
        .expect("the routed legs include the ring rub edge");
    let _ = rub;
    if nav::debug_enabled() {
        println!(
            "nav_tele: courtyard -> Duel Arena routed ({:.0} ticks, {} legs)",
            routed.ticks,
            routed.legs.len()
        );
    }
    // A default find (layer off) never uses a packed teleport — the
    // arena may be walkable in the bake, but never through the layer.
    // Walk-unreachable with the layer off is fine (the arena then
    // requires allow_teleports); the routed-tele check is above.
    if let Ok(route) = find_with(
        &world.collision,
        &world.graph,
        COURTYARD,
        DUEL_ARENA,
        FindOptions::default(),
        &ring_state(),
    ) {
        if route.legs.iter().any(|l| {
            matches!(
                l,
                Leg::Transport { edge } if edge.kind == TransportKind::Teleport
            )
        }) {
            fail(&format!(
                "nav_tele: default find used a packed teleport: {:?}",
                route.legs
            ));
        }
    }
    println!(
        "PASS: nav_tele pack carries the dueling-ring rub edge ({} edges, {} teleports)",
        world.graph.edges.len(),
        world.graph.teleports.len()
    );
}

/// The execute twin: the `nav_tele` scenario run headlessly, exactly
/// like `panel-play --live script_nav_tele`. One slot; PASS is the
/// runner's proof — `arrived_near` the packed Duel Arena landing after
/// the traveller executes the ring's packed rub.
#[test]
#[ignore = "requires a local 274 engine, nav pack, and LIVE=1"]
fn nav_tele_follow() {
    if !live() {
        return;
    }

    let scenario = scenario::get("nav_tele").expect("nav_tele scenario in registry");
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

    wait_ingame(&play, 1, Duration::from_secs(150), "nav_tele_follow");

    let deadline = Instant::now() + Duration::from_secs(360);
    loop {
        let (status, evidence) = {
            let r = runner.lock().unwrap();
            (r.status(), r.evidence().cloned())
        };
        let record = evidence.as_ref().map(|ev| ev.to_json()).unwrap_or_default();
        match status {
            RunnerStatus::Passed => {
                println!("PASS: nav_tele_follow {record}");
                return;
            }
            RunnerStatus::Failed(msg) => {
                eprintln!("FAIL: nav_tele_follow {record}");
                fail(&format!("nav_tele_follow: {msg}"));
            }
            other => {
                if Instant::now() >= deadline {
                    fail(&format!(
                        "nav_tele_follow: no terminal status within 360s ({other:?})"
                    ));
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    }
}
