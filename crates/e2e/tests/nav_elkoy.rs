//! Live: Elkoy's Tree Gnome Village maze escorts on the wire — the live
//! twin of the nav `derive_transports_emits_elkoy_escort_both_ways` unit
//! test. Waits for the slot `ingame && scene_state == 2`, then loads the
//! process nav pack and asserts the two `TransportKind::Npc` escort edges
//! are on it: the maze-side Elkoy (npc 473) at (2504,3191) escorts into
//! the village (`to` the maze coord (2515,3159)) and the village Elkoy
//! (npc 474) at (2514,3159) escorts out (`to` the entrance coord
//! (2504,3192)), each carrying the Tree Gnome Village quest req, and that
//! `find` still routes from the maze-side Elkoy's tile to the village on
//! the pack. Either failure exits 1 (rs2b0t style). Route detail prints
//! only under `BOT_DEBUG=1`.
//!
//! Run with the engine up and the rebaked v4 pack at the standard path:
//! `LIVE=1 cargo test -p e2e --test nav_elkoy -- --ignored --test-threads=1 --nocapture`

mod common;

use std::time::Duration;

use api::snapshot::WorldTile;
use common::{fail, live, options, profiles, wait_ingame};
use host_play::run_with_io;
use nav::router::find;
use nav::transport::TransportKind;
use nav::world::NavWorld;
use scenario::default_pack_path;

/// The maze entrance coord (`^elkoy_entrance_coord = 0_39_49_8_56`).
const ENTRANCE: WorldTile = WorldTile {
    x: 2504,
    z: 3192,
    level: 0,
};
/// The village (maze) coord (`^elkoy_maze_coord = 0_39_49_19_23`).
const MAZE: WorldTile = WorldTile {
    x: 2515,
    z: 3159,
    level: 0,
};
/// The maze-side Elkoy (npc 473, m39_49 local (8,55)): one tile south of
/// the entrance coord.
const ELKOY_MAZE_SIDE: WorldTile = WorldTile {
    x: 2504,
    z: 3191,
    level: 0,
};

#[test]
#[ignore = "requires a local 274 engine, a rebaked v4 nav pack, and LIVE=1"]
fn nav_elkoy() {
    if !live() {
        return;
    }

    // Single slot with the mainland hop (lands the Lumbridge courtyard).
    let seed = [("test", "test")];
    let mut opts = options();
    opts.mainland = true;
    let play = run_with_io(&opts, profiles(&seed), |_| (None, None), |_, _| {});
    wait_ingame(&play, 1, Duration::from_secs(150), "nav_elkoy");

    let world = NavWorld::load_pack(&default_pack_path())
        .unwrap_or_else(|e| fail(&format!("nav_elkoy: nav pack must load: {e:?}")));
    let elk: Vec<_> = world
        .graph
        .edges
        .iter()
        .filter(|e| {
            e.kind == TransportKind::Npc && (e.to == ENTRANCE || e.to == MAZE)
        })
        .cloned()
        .collect();
    if elk.len() != 2 {
        fail(&format!(
            "nav_elkoy: pack carries {} Elkoy escort edges, need 2 \
             (rebake with `cargo run -p nav --bin nav-pack`)",
            elk.len()
        ));
    }
    if !elk.iter().any(|e| e.at == ELKOY_MAZE_SIDE && e.to == MAZE) {
        fail(&format!(
            "nav_elkoy: the maze-side Elkoy {ELKOY_MAZE_SIDE:?} -> village {MAZE:?} \
             escort is missing (rebake with `cargo run -p nav --bin nav-pack`)"
        ));
    }
    if !elk.iter().any(|e| e.to == ENTRANCE) {
        fail(&format!(
            "nav_elkoy: the village Elkoy -> entrance {ENTRANCE:?} escort is missing \
             (rebake with `cargo run -p nav --bin nav-pack`)"
        ));
    }
    for e in &elk {
        if e.option != 1 {
            fail(&format!("nav_elkoy: escort edge {e:?} is not Talk-to (op 1)"));
        }
        if !e
            .quest_req
            .iter()
            .any(|q| q == "Tree Gnome Village")
        {
            fail(&format!(
                "nav_elkoy: escort edge {e:?} lacks the Tree Gnome Village quest req"
            ));
        }
    }
    // The maze-side Elkoy's tile reaches the village through his escort
    // hop (the traveller walks no maze tiles).
    find(&world.collision, &world.graph, ELKOY_MAZE_SIDE, MAZE)
        .map(|route| {
            if route.dest != MAZE {
                fail(&format!(
                    "nav_elkoy: route ends at {:?}, not the village {MAZE:?}",
                    route.dest
                ));
            }
            if nav::debug_enabled() {
                println!(
                    "nav_elkoy: maze-side elkoy -> village routed ({:.0} ticks, {} legs)",
                    route.ticks,
                    route.legs.len()
                );
            }
        })
        .unwrap_or_else(|e| {
            fail(&format!(
                "nav_elkoy: maze-side elkoy {ELKOY_MAZE_SIDE:?} -> village {MAZE:?} \
                 is NoPath ({e:?})"
            ))
        });
    println!(
        "PASS: nav_elkoy pack carries {} Elkoy escort edges ({} edges, {} npc hops)",
        elk.len(),
        world.graph.edges.len(),
        world
            .graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Npc)
            .count()
    );
}
