//! Live: the Rune Mysteries essence-mine wizard entries on the wire — the
//! live twin of the nav `derive_transports_emits_essence_mine_entries`
//! unit test. Waits for the slot `ingame && scene_state == 2`, then loads
//! the process nav pack and asserts at least four `TransportKind::Npc`
//! entry edges carrying the Rune Mysteries quest name (Aubury, Sedridor,
//! Distentor, Cromperty, Brimstail), each landing on the mine pad
//! (2912,4833), and that `find` still routes from Aubury's tile to the
//! pad on the pack. Either failure exits 1 (rs2b0t style). Route detail
//! prints only under `BOT_DEBUG=1`.
//!
//! Run with the engine up and the rebaked nav pack at the standard path:
//! `LIVE=1 cargo test -p e2e --test nav_essence -- --ignored --test-threads=1 --nocapture`

mod common;

use std::time::Duration;

use api::snapshot::WorldTile;
use common::{fail, live, options, profiles, wait_ingame};
use host_play::run_with_io;
use nav::router::find;
use nav::transport::TransportKind;
use nav::world::NavWorld;
use scenario::default_pack_path;

/// Aubury (npc 553) in the Varrock rune shop (m50_53 local (53,10)).
const AUBURY: WorldTile = WorldTile {
    x: 3253,
    z: 3402,
    level: 0,
};
/// The Rune Essence mine pad (m45_75 local (32,33)): the walkable centre
/// anchor the wizard entries land on (the real landing is randomised).
const MINE_PAD: WorldTile = WorldTile {
    x: 2912,
    z: 4833,
    level: 0,
};

#[test]
#[ignore = "requires a local 274 engine, a rebaked v5 nav pack, and LIVE=1"]
fn nav_essence() {
    if !live() {
        return;
    }

    // Single slot with the mainland hop (lands the Lumbridge courtyard).
    let seed = [("test", "test")];
    let mut opts = options();
    opts.mainland = true;
    let play = run_with_io(&opts, profiles(&seed), |_| (None, None), |_, _| {});
    wait_ingame(&play, 1, Duration::from_secs(150), "nav_essence");

    let world = NavWorld::load_pack(&default_pack_path())
        .unwrap_or_else(|e| fail(&format!("nav_essence: nav pack must load: {e:?}")));
    let ess: Vec<_> = world
        .graph
        .edges
        .iter()
        .filter(|e| {
            e.kind == TransportKind::Npc
                && e.quest_req.iter().any(|q| {
                    q.to_ascii_lowercase().contains("rune mysteries") || q == "runemysteries"
                })
        })
        .cloned()
        .collect();
    if ess.len() < 4 {
        fail(&format!(
            "nav_essence: pack carries {} essence entry edges, need >= 4 \
             (Aubury+Sedridor+…; rebake with `cargo run -p nav --bin nav-pack`)",
            ess.len()
        ));
    }
    for e in &ess {
        if e.at != AUBURY
            && !(e.at.x == 3103 && e.at.z == 9571) // head_wizard (tower cellar)
            && !(e.at.x == 2594 && e.at.z == 3089) // guild_wizard (Yanille)
            && !(e.at.x == 2683 && e.at.z == 3326) // ardounge_wizard (Cromperty)
            && !(e.at.x == 2390 && e.at.z == 9810)
        // gnome_brimstail
        {
            fail(&format!(
                "nav_essence: entry edge {e:?} is not on a known wizard tile \
                 (rebake with `cargo run -p nav --bin nav-pack`)"
            ));
        }
        if e.to != MINE_PAD {
            fail(&format!(
                "nav_essence: entry edge {e:?} must land on the mine pad {MINE_PAD:?} \
                 (rebake with `cargo run -p nav --bin nav-pack`)"
            ));
        }
        if !e
            .quest_req
            .iter()
            .any(|q| q.to_ascii_lowercase().contains("rune mysteries"))
        {
            fail(&format!(
                "nav_essence: entry edge {e:?} lacks the Rune Mysteries quest req"
            ));
        }
    }
    // Aubury's shop tile reaches the enclosed mine through his entry hop.
    find(&world.collision, &world.graph, AUBURY, MINE_PAD)
        .map(|route| {
            if route.dest != MINE_PAD {
                fail(&format!(
                    "nav_essence: route ends at {:?}, not the mine pad {MINE_PAD:?}",
                    route.dest
                ));
            }
            if nav::debug_enabled() {
                println!(
                    "nav_essence: aubury -> essence mine routed ({:.0} ticks, {} legs)",
                    route.ticks,
                    route.legs.len()
                );
            }
        })
        .unwrap_or_else(|e| {
            fail(&format!(
                "nav_essence: aubury {AUBURY:?} -> mine pad {MINE_PAD:?} is NoPath ({e:?})"
            ))
        });
    println!(
        "PASS: nav_essence pack carries {} essence entry edges ({} edges, {} npc hops)",
        ess.len(),
        world.graph.edges.len(),
        world
            .graph
            .edges
            .iter()
            .filter(|e| e.kind == TransportKind::Npc)
            .count()
    );
}
