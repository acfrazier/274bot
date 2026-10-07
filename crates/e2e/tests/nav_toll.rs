//! Pack and live checks for Al Kharid toll crossings and both Shantay henge
//! branches on the rebaked 289 nav pack. Al Kharid's toll gates expose the
//! 10-coin `consumed_req` and content-proven Prince Ali Rescue waiver.
//! The Shantay doorway (loc 4031, m51_48 (38,44) = (3302,3116)) carries
//! exactly two content-derived Door edges: the paid desert hop consumes one
//! pass from `consumed_req`, while the free exit derives `to` from its stand
//! by the script's +3z telejump. Neither branch is a plain walk; both interact
//! with the henge.
//!
//! Run on Engine A at the normal 600ms tick with the rebaked pack and isolated
//! cache paths: `LIVE=1 BOT_CPU=1 BOT_LIVE_NAME_PREFIX=<prefix> ENGINE_DIR=<engine>
//! BOT_NAV_SNAPSHOT_ROOT=<evidence-cache> NAV_PACK=<rebaked-pack> isohome cargo
//! test -p e2e --test nav_toll -- --ignored --test-threads=1 --nocapture`
//!
//! `nav_shantay_follow` is the execute twin of `script_nav_shantay`: an exact
//! WalkTo from Irena to the bank chest uses the free exit, then an exact
//! WalkTo back to Irena uses the paid branch with a Shantay pass.

mod common;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use api::snapshot::WorldTile;
use common::{fail, live, mint_seed, profiles, wait_ingame};
use host_play::{run_with_template, ProfileOptions, SharedClientTemplate};
use nav::router::{find_with, FindOptions, Leg};
use nav::transport::TransportKind;
use nav::world::NavWorld;
use nav::WorldState;
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
/// The free desert exit's stand, one tile south of the placement.
const SHANTAY_DESERT_AT: WorldTile = WorldTile {
    x: 3302,
    z: 3115,
    level: 0,
};
/// The free desert exit's landing: the script's +3z telejump from its stand.
const SHANTAY_DESERT_TO: WorldTile = WorldTile {
    x: 3302,
    z: 3118,
    level: 0,
};
const SHANTAY_DESERT_START: WorldTile = WorldTile {
    x: 3302,
    z: 3114,
    level: 0,
};
const SHANTAY_BANK_CHEST: WorldTile = WorldTile {
    x: 3308,
    z: 3120,
    level: 0,
};

const ENGINE_A_GAME_PORT: u16 = 44594;
const ENGINE_A_HTTP_PORT: u16 = 1080;

fn engine_a_template() -> Arc<SharedClientTemplate> {
    let engine_dir = std::env::var_os("ENGINE_DIR")
        .map(PathBuf::from)
        .expect("set ENGINE_DIR to the Engine A root for live tests");
    let unpack_dir = std::env::var_os("BOT_NAV_SNAPSHOT_ROOT")
        .map(PathBuf::from)
        .expect("set BOT_NAV_SNAPSHOT_ROOT to an evidence directory");
    let nav_pack = std::env::var_os("NAV_PACK")
        .map(PathBuf::from)
        .expect("set NAV_PACK to the freshly baked 289 navigation pack");
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .expect("isohome must set an isolated HOME");
    ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(ENGINE_A_GAME_PORT),
        http_port: Some(ENGINE_A_HTTP_PORT),
        engine_dir: Some(engine_dir.clone()),
        cache_dir: Some(engine_dir.join("data/pack/client")),
        unpack_dir: Some(unpack_dir),
        nav_pack: Some(nav_pack),
        vault_path: Some(home.join("nav-shantay-live-vault")),
        ..ProfileOptions::default()
    }
    .resolve(None)
    .expect("resolve explicit Engine A local-289 profile")
    .prepare_template()
    .expect("prepare Engine A local-289 template")
}

#[test]
#[ignore = "requires a freshly baked 289 nav pack and LIVE=1"]
fn nav_toll() {
    if !live() {
        return;
    }

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
        let paid = e.consumed_req.iter().any(|(id, n)| *id == 995 && *n == 10);
        let waived = e.consumed_req.is_empty()
            && e.quest_req.len() == 1
            && e.quest_req[0] == "Prince Ali Rescue";
        if !paid && !waived {
            fail(&format!(
                "nav_toll: toll-gate edge {e:?} lacks its content-derived coin fare or quest waiver"
            ));
        }
    }

    // The Shantay henge carries exactly two edges: the gated desert hop
    // (the only one that consumes a pass) and the free desert exit.
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
        .find(|e| !e.consumed_req.is_empty())
        .unwrap_or_else(|| fail("nav_toll: no Shantay henge edge consumes the pass"));
    let free = henge
        .iter()
        .find(|e| e.consumed_req.is_empty())
        .unwrap_or_else(|| fail("nav_toll: no free Shantay henge edge (desert exit)"));
    if gated.at != SHANTAY_AT || gated.to != SHANTAY_TO {
        fail(&format!(
            "nav_toll: Shantay gated hop is {:?} -> {:?}, expected {SHANTAY_AT:?} -> {SHANTAY_TO:?}",
            gated.at, gated.to
        ));
    }
    if !gated
        .consumed_req
        .iter()
        .any(|(id, n)| *id == 1854 && *n >= 1)
    {
        fail("nav_toll: Shantay gated hop does not consume a Shantay pass");
    }
    if free.at != SHANTAY_DESERT_AT || free.to != SHANTAY_DESERT_TO {
        fail(&format!(
            "nav_toll: Shantay desert exit is {:?} -> {:?}, expected {SHANTAY_DESERT_AT:?} -> \
             {SHANTAY_DESERT_TO:?}",
            free.at, free.to
        ));
    }
    if free.to
        != (WorldTile {
            x: free.at.x,
            z: free.at.z + 3,
            level: free.at.level,
        })
    {
        fail("nav_toll: Shantay free exit does not land at its stand +3z");
    }
    if !world.collision.standable(free.at) {
        fail("nav_toll: Shantay free-exit interaction stand is not standable");
    }

    let members = WorldState::empty().with_map_members(true);
    let route = find_with(
        &world.collision,
        &world.graph,
        SHANTAY_DESERT_START,
        SHANTAY_BANK_CHEST,
        FindOptions::default(),
        &members,
    )
    .unwrap_or_else(|e| {
        fail(&format!(
            "nav_toll: no exact walk route from Irena to the Shantay bank chest: {e:?}"
        ))
    });
    if !route.legs.iter().any(|leg| {
        matches!(
            leg,
            Leg::Transport { edge }
                if edge.loc_id == 4031 && edge.at == free.at && edge.to == free.to
        )
    }) {
        fail("nav_toll: Irena-to-bank route does not use the free Shantay exit");
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
/// exactly like `panel-play --live script_nav_shantay`. One slot; it walks
/// Irena → chest through the free branch, then back to Irena through the
/// paid branch after giving a Shantay pass. PASS is exact WalkTo arrival at
/// the desert start.
#[test]
#[ignore = "requires Engine A (289), nav pack, and LIVE=1"]
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
    let template = engine_a_template();
    runner
        .lock()
        .unwrap()
        .set_map_members(template.profile().map_members());
    let play = run_with_template(template, mainland, profiles(&entries), |_| (None, None), {
        let runner = Arc::clone(&runner);
        move |c, name, frame| {
            let hold = frame.hold;
            let mut r = runner.lock().unwrap();
            if r.drives(name) {
                r.tick_with_hold(c, hold);
            } else if let Some(index) = r.companion_for(name) {
                r.companion_tick(index, c);
            }
        }
    })
    .expect("start Engine A local-289 Play");
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
