//! Live: one headless client via host-play with `mainland` on. After login
//! the mainland hop teleports the player into the Lumbridge courtyard
//! (`tele 0,50,50,20,20`); the landing tile can be (3220,3220,0) or
//! (3220,3222,0), so the test arms from the observed walkable `here`. The
//! nav traveller then walks to (3230,3222,0), open ground. PASS only when
//! `arrived(here, dest, true)` fires within 90 s of arming; FAIL on timeout
//! or on the traveller's per-hop budget.
//!
//! Run with the engine up:
//! `LIVE=1 cargo test -p e2e --test nav_walk -- --ignored --test-threads=1 --nocapture`

mod common;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{fail, live, options, profiles, wait_ingame};
use host_play::run_with_io;
use nav::pack::load_pack;
use nav::router::find;
use nav::tile::Tile;
use nav::traveller::{NavStatus, Traveller};

/// Open-courtyard walk destination, 10 tiles east of the mainland landing.
const DEST: Tile = Tile { x: 3230, z: 3222, level: 0 };

/// State shared between the slot's observe hook and this test thread.
/// The hook latches the player-family gen, publishes the player's current
/// tile, and ticks the traveller; the test thread arms the route and polls
/// for arrival.
struct Shared {
    last_gen: u64,
    here: Option<Tile>,
    scene_state: i32,
    base: (i32, i32),
    traveller: Traveller,
    arrived: bool,
    failed: Option<String>,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            last_gen: 0,
            here: None,
            scene_state: 0,
            base: (0, 0),
            traveller: Traveller::new(),
            arrived: false,
            failed: None,
        }
    }
}

#[test]
#[ignore = "requires a local 274 engine, nav pack, and LIVE=1"]
fn nav_walk() {
    if !live() {
        return;
    }

    let grid = load_pack(&pack_path())
        .unwrap_or_else(|e| fail(&format!("no nav pack ({e}) — run nav-pack")));

    let shared = Arc::new(Mutex::new(Shared::default()));
    let mut opts = options();
    opts.mainland = true;

    let play = run_with_io(
        &opts,
        profiles(&[("test", "test")]),
        |_| (None, None),
        {
            let shared = Arc::clone(&shared);
            // Drive the traveller only when a player update landed; the
            // player's tile is the route head, as in `Driver::local_route`.
            move |c, _| {
                let Some(lp) = &c.local_player else {
                    return;
                };
                // The client's route head is scene-relative; the nav grid
                // is in absolute world tiles, so add the build origin.
                let here = Tile {
                    x: c.map_build_base_x + lp.route_x[0],
                    z: c.map_build_base_z + lp.route_z[0],
                    level: 0,
                };
                let mut s = shared.lock().unwrap();
                s.scene_state = c.scene_state;
                s.base = (c.map_build_base_x, c.map_build_base_z);
                // Tick on movement or on a player update: the gen latch
                // re-arms a walk while standing, the tile latch advances
                // the route as the player moves.
                if c.gens.player == s.last_gen && s.here == Some(here) {
                    return;
                }
                s.last_gen = c.gens.player;
                s.here = Some(here);
                let status = s.traveller.tick(c, here, false);
                if std::env::var("BOT_DEBUG").as_deref() == Ok("1") {
                    let (bx, bz) = (c.map_build_base_x, c.map_build_base_z);
                    println!(
                        "nav_walk: here={here:?} status={status:?} walk_ok={:?} \
                         base=({bx},{bz}) scene_dest=({},{}) flag=({},{})",
                        s.traveller.last_walk_ok(),
                        DEST.x - bx,
                        DEST.z - bz,
                        c.minimap_flag_x,
                        c.minimap_flag_z,
                    );
                }
                match status {
                    NavStatus::Arrived => s.arrived = true,
                    NavStatus::Budget => {
                        s.failed = Some("nav_walk: traveller per-hop budget exceeded".into())
                    }
                    _ => {}
                }
            }
        },
    );

    wait_ingame(&play, 1, Duration::from_secs(90), "nav_walk");

    // Arm once the mainland hop has landed the player on the packed
    // courtyard (tutorial island is off-grid, so a walkable tile means the
    // tele arrived). The player is stationary until armed, so the observed
    // tile is a safe route origin even when it is not the expected landing.
    let mut armed = false;
    let mut started: Option<Instant> = None;
    loop {
        let mut s = shared.lock().unwrap();
        if let Some(msg) = s.failed.clone() {
            fail(&msg);
        }
        if s.arrived {
            let elapsed = started.map(|st| st.elapsed()).unwrap_or_default();
            println!("PASS: nav_walk arrived at {DEST:?} in {elapsed:?}");
            return;
        }
        if !armed {
            // Wait for a real mainland scene: scene 2 with a world build
            // origin (>=3000), not the tutorial island or a 0 base.
            if s.scene_state == 2 && s.base.0 >= 3000 && s.base.1 >= 3000 {
                if let Some(here) = s.here {
                    if grid.walkable(here) {
                        match find(&grid, here, DEST) {
                            Ok(route) => {
                                s.traveller.arm(route);
                                armed = true;
                                started = Some(Instant::now());
                                println!("nav_walk: armed route {here:?} -> {DEST:?}");
                            }
                            Err(_) => {
                                fail(&format!(
                                    "nav_walk: no pack path from {here:?} to {DEST:?}"
                                ))
                            }
                        }
                    }
                }
            }
        }
        // The 90 s clock starts at arm, not after wait_ingame, so a slow
        // tutorial->tele hop cannot eat the walk budget.
        let timed_out = started
            .map(|st| Instant::now() >= st + Duration::from_secs(90))
            .unwrap_or(false);
        let here = s.here;
        drop(s);
        if timed_out {
            fail(&format!(
                "nav_walk: not at {DEST:?} within 90 s (armed={armed}, here={here:?})"
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// `$NAV_PACK`, or the nav-pack CLI default (`~/.274bot/274bot.navpack`).
fn pack_path() -> PathBuf {
    std::env::var("NAV_PACK")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_default();
            PathBuf::from(format!("{home}/.274bot/274bot.navpack"))
        })
}
