//! Live: Seers street → rock crabs on foot — the live twin of the nav
//! `seers_street_walks_to_rock_crabs_on_foot` unit test. Waits for the
//! slot `ingame && scene_state == 2`, then runs `nav::router::find` for
//! the same OD (Seers street (2725,3485) → rock crabs (2710,3720), both
//! level 0) on the pack the process loaded. `NoPath` fails the harness
//! with exit 1 (rs2b0t style). The character is not walked the whole way
//! this task: the pack `find` is the proof, and the same anti-hop
//! invariants as the unit twin hold (every walked tile stays on plane 0,
//! the walk spans the north gap, transports are road doors only).
//!
//! Run with the engine up and a v4 pack baked:
//! `LIVE=1 cargo test -p e2e --test nav_seers_crabs -- --ignored --test-threads=1 --nocapture`

mod common;

use std::path::PathBuf;
use std::time::Duration;

use api::snapshot::WorldTile;
use common::{fail, live, options, profiles, wait_ingame};
use host_play::run_with_io;
use nav::router::{find, Leg};
use nav::world::NavWorld;

/// The task OD, verbatim: Seers street → rock crabs, both level 0.
const FROM: WorldTile = WorldTile {
    x: 2725,
    z: 3485,
    level: 0,
};
const TO: WorldTile = WorldTile {
    x: 2710,
    z: 3720,
    level: 0,
};

fn pack_path() -> PathBuf {
    match std::env::var("NAV_PACK") {
        Ok(p) => PathBuf::from(p),
        Err(_) => {
            let home = std::env::var("HOME").expect("HOME set");
            PathBuf::from(format!("{home}/.274bot/274bot.navpack"))
        }
    }
}

#[test]
#[ignore = "requires a local 274 engine, a rebaked v4 nav pack, and LIVE=1"]
fn nav_seers_crabs() {
    if !live() {
        return;
    }

    // The pack must be v4 (the Task-3 rebake): a stale file fails to load.
    let path = pack_path();
    let world = match NavWorld::load_pack(&path) {
        Ok(w) => w,
        Err(e) => fail(&format!(
            "nav_seers_crabs: cannot load nav pack {} ({e}); \
             rebake with `cargo run -p nav --bin nav-pack` (v4)",
            path.display()
        )),
    };

    // Wait ingame && scene_state == 2 (the mainland hop lands in the
    // Lumbridge courtyard; the pack route is scene-independent, the slot
    // is the live-gate the brief requires).
    let mut opts = options();
    opts.mainland = true;
    let play = run_with_io(&opts, profiles(&[("test", "test")]), |_| (None, None), |_, _| {});
    wait_ingame(&play, 1, Duration::from_secs(150), "nav_seers_crabs");

    // Find the same OD on the pack the process loaded.
    let r = match find(&world.collision, &world.graph, FROM, TO) {
        Ok(r) => r,
        Err(e) => fail(&format!(
            "nav_seers_crabs: find Seers street {FROM:?} -> rock crabs {TO:?}: {e:?}"
        )),
    };
    assert_eq!(r.dest, TO);
    if !r.legs.iter().any(|l| matches!(l, Leg::Walk { .. })) {
        fail("nav_seers_crabs: route has no Walk leg");
    }

    // Anti-hop invariants, mirroring the unit twin.
    let walked: Vec<WorldTile> = r
        .legs
        .iter()
        .flat_map(|l| match l {
            Leg::Walk { tiles } => tiles.clone(),
            Leg::Transport { .. } => vec![],
        })
        .collect();
    if !walked.iter().all(|t| t.level == 0) {
        fail("nav_seers_crabs: a Walk leg leaves plane 0");
    }
    let north = TO.z - FROM.z;
    if walked.len() < north as usize {
        fail(&format!(
            "nav_seers_crabs: walk spans only {} tiles of {north} north \
             (a fake hop walked ~nothing)",
            walked.len()
        ));
    }
    for l in &r.legs {
        if let Leg::Transport { edge } = l {
            if edge.kind != nav::transport::TransportKind::Door {
                fail(&format!(
                    "nav_seers_crabs: transport at {:?} is not a road door \
                     (invented cliff/teleport hop)",
                    edge.at
                ));
            }
        }
    }

    println!(
        "PASS: nav_seers_crabs: Seers street -> rock crabs on foot: \
         {} legs, {:.1} ticks, {} walked tiles, all level 0",
        r.legs.len(),
        r.ticks,
        walked.len()
    );
}
