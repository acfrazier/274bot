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
//! `nav_essence_follow` is the execute twin — the same `nav_essence`
//! scenario `panel-play --live script_nav_essence` drives. The scenario
//! cheat-teles to Aubury, `Follow`s into the mine (the entry hop latches
//! the EssenceSession on any mine landing), then `Follow`s back out
//! through the exit portal to Aubury — not another wizard — and PASSes on
//! `TravelOutcome::Arrived` near Aubury's anchor.
//!
//! Run with the engine up and the rebaked nav pack at the standard path:
//! `LIVE=1 cargo test -p e2e --test nav_essence -- --ignored --test-threads=1 --nocapture`

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use api::snapshot::WorldTile;
use common::{fail, live, mint_seed, options, profiles, wait_ingame};
use host_play::run_with_io;
use nav::router::find_with;
use nav::transport::TransportKind;
use nav::world::NavWorld;
use scenario::{default_pack_path, RunnerStatus, ScenarioRunner};

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

/// A state that can take a wizard's entry hop: the Rune Mysteries quest
/// complete. The gate is the quest journal's row name — "Rune Mysteries
/// Quest" — since Task 1 `find` gates every requirement fail-closed (the
/// perm-scoped `%runemysteries` varp is never transmitted, so only the
/// journal proves it live).
fn ess_state() -> nav::WorldState {
    nav::WorldState {
        quests: ["Rune Mysteries Quest".to_string()].into(),
        ..nav::WorldState::default()
    }
}

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
    // Aubury's shop tile reaches the enclosed mine through his entry hop
    // (the Rune-Mysteries-complete state satisfies the packed quest req).
    find_with(
        &world.collision,
        &world.graph,
        AUBURY,
        MINE_PAD,
        nav::router::FindOptions::default(),
        &ess_state(),
    )
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
    // The scenario teles onto the exit anchor (3253,3401), a tile from
    // the wizard: the entry edge must still be relaxable from there.
    find_with(
        &world.collision,
        &world.graph,
        WorldTile {
            x: 3253,
            z: 3401,
            level: 0,
        },
        MINE_PAD,
        nav::router::FindOptions::default(),
        &ess_state(),
    )
    .map(|route| {
        if route.dest != MINE_PAD {
            fail(&format!(
                "nav_essence: anchor route ends at {:?}, not the mine pad {MINE_PAD:?}",
                route.dest
            ));
        }
    })
    .unwrap_or_else(|e| {
        fail(&format!(
            "nav_essence: aubury anchor (3253,3401) -> mine pad {MINE_PAD:?} is NoPath ({e:?})"
        ))
    });
    // Every mine landing must be able to reach the exit: with a session
    // latched, any interior tile (the teleport landings are random among
    // 22 `essence_mine_teleports` coords) routes to a portal and out.
    let session = nav::essence::essence_session_for_wizard(553).unwrap();
    for &(x, z) in &[(2909, 4834), (2912, 4833), (2935, 4846), (2896, 4809)] {
        find_with(
            &world.collision,
            &world.graph,
            WorldTile { x, z, level: 0 },
            WorldTile {
                x: 3253,
                z: 3401,
                level: 0,
            },
            nav::router::FindOptions {
                essence: Some(session),
                ..nav::router::FindOptions::default()
            },
            &ess_state(),
        )
        .map(|route| {
            let anchor = WorldTile {
                x: 3253,
                z: 3401,
                level: 0,
            };
            if route.dest != anchor {
                fail(&format!(
                    "nav_essence: mine landing ({x},{z}) return ends at {:?}, not the anchor",
                    route.dest
                ));
            }
        })
        .unwrap_or_else(|e| {
            fail(&format!(
                "nav_essence: mine landing ({x},{z}) -> aubury anchor is NoPath ({e:?})"
            ))
        });
    }
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

/// The execute twin: the `nav_essence` scenario run headlessly, exactly
/// like `panel-play --live script_nav_essence`. One slot; PASS is the
/// runner's proof (back near Aubury's anchor after entering the mine and
/// returning through the exit portal).
#[test]
#[ignore = "requires a local 274 engine, nav pack, and LIVE=1"]
fn nav_essence_follow() {
    if !live() {
        return;
    }

    let scenario = scenario::get("nav_essence").expect("nav_essence scenario in registry");
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

    wait_ingame(&play, 1, Duration::from_secs(150), "nav_essence_follow");

    let deadline = Instant::now() + Duration::from_secs(360);
    loop {
        let (status, evidence) = {
            let r = runner.lock().unwrap();
            (r.status(), r.evidence().cloned())
        };
        let record = evidence.as_ref().map(|ev| ev.to_json()).unwrap_or_default();
        match status {
            RunnerStatus::Passed => {
                println!("PASS: nav_essence_follow {record}");
                return;
            }
            RunnerStatus::Failed(msg) => {
                eprintln!("FAIL: nav_essence_follow {record}");
                fail(&format!("nav_essence_follow: {msg}"));
            }
            other => {
                if Instant::now() >= deadline {
                    fail(&format!(
                        "nav_essence_follow: no terminal status within 360s ({other:?})"
                    ));
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    }
}
