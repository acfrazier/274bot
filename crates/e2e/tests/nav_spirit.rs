//! Live: spirit-tree edges on the wire — the live twin of the nav
//! `derive_transports_emits_spirit_tree_edges` unit test. Waits for the
//! slot `ingame && scene_state == 2`, then loads the process nav pack and
//! asserts the SpiritTree kind is on it: at least 8 directed hops (the
//! stronghold tree → village/varrock/khazard, the village tree → the
//! three, the young tree → village across its two placements), and that
//! `find` still routes between two spirit-tree tiles on the pack. Either
//! failure exits 1 (rs2b0t style). Route detail prints only under
//! `BOT_DEBUG=1`.
//!
//! Run with the engine up and the rebaked v4 pack at the standard path:
//! `LIVE=1 cargo test -p e2e --test nav_spirit -- --ignored --test-threads=1 --nocapture`

mod common;

use std::time::Duration;

use api::snapshot::WorldTile;
use common::{fail, live, options, profiles, wait_ingame};
use host_play::run_with_io;
use nav::router::find;
use nav::transport::TransportKind;
use nav::world::NavWorld;
use scenario::default_pack_path;

/// The stronghold spirit tree (loc 1293) placement tile.
const STRONGHOLD_TREE: WorldTile = WorldTile {
    x: 2460,
    z: 3445,
    level: 0,
};
/// The varrock tree landing (`^varrock_tree`).
const VARROCK_TREE: WorldTile = WorldTile {
    x: 3179,
    z: 3507,
    level: 0,
};

#[test]
#[ignore = "requires a local 274 engine, a rebaked v4 nav pack, and LIVE=1"]
fn nav_spirit() {
    if !live() {
        return;
    }

    // Single slot with the mainland hop (lands the Lumbridge courtyard).
    let seed = [("test", "test")];
    let mut opts = options();
    opts.mainland = true;
    let play = run_with_io(&opts, profiles(&seed), |_| (None, None), |_, _| {});
    wait_ingame(&play, 1, Duration::from_secs(150), "nav_spirit");

    let world = NavWorld::load_pack(&default_pack_path())
        .unwrap_or_else(|e| fail(&format!("nav_spirit: nav pack must load: {e:?}")));
    let trees: Vec<_> = world
        .graph
        .edges
        .iter()
        .filter(|e| e.kind == TransportKind::SpiritTree)
        .collect();
    if trees.len() < 8 {
        fail(&format!(
            "nav_spirit: pack carries {} SpiritTree edges, need >= 8 \
             (rebake with `cargo run -p nav --bin nav-pack`)",
            trees.len()
        ));
    }
    // The two trees are reachable on the pack (walk and/or spirit-tree hop).
    find(&world.collision, &world.graph, STRONGHOLD_TREE, VARROCK_TREE)
        .map(|route| {
            if route.dest != VARROCK_TREE {
                fail(&format!(
                    "nav_spirit: route ends at {:?}, not the varrock tree {:?}",
                    route.dest, VARROCK_TREE
                ));
            }
            if nav::debug_enabled() {
                println!(
                    "nav_spirit: stronghold tree -> varrock tree routed ({:.0} ticks, {} legs)",
                    route.ticks,
                    route.legs.len()
                );
            }
        })
        .unwrap_or_else(|e| {
            fail(&format!(
                "nav_spirit: stronghold tree {STRONGHOLD_TREE:?} -> varrock tree {VARROCK_TREE:?} \
                 is NoPath ({e:?})"
            ))
        });
}
