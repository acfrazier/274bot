//! Live: two headless slots in one `run_with_io` — `test` walks through the
//! Catherby range-house door while `test2` is a tick-perfect closer.
//!
//! After the mainland hop, both cheat-tele to Catherby. The walker stages
//! outside at (2813,3436,0) then arms dest (2817,3443,0) through door
//! 1530 @ (2816,3438,0). The closer, on every `player_info`, `op_loc`s the
//! door whenever the loc is not the closed id 1530 (try 1530 first, then
//! the open id the client currently shows). PASS when the walker is
//! chebyshev ≤ 1 of dest within 120 s of dest-arm. FAIL on timeout,
//! traveller Budget, or spin (same tile 30 ticks, door still closed, no
//! `op_loc`).
//!
//! Run with the engine up:
//! `LIVE=1 cargo test -p e2e --test nav_door -- --ignored --test-threads=1 --nocapture`

mod common;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use api::interact::{cheat, op_loc};
use common::{fail, live, options, profiles, wait_ingame};
use host_play::run_with_io;
use nav::pack::load_pack;
use nav::router::find;
use nav::tile::{chebyshev, Tile};
use nav::traveller::{NavStatus, Traveller};

/// Closed Catherby range-house door (loc 1530).
const DOOR: Tile = Tile {
    x: 2816,
    z: 3438,
    level: 0,
};
const CLOSED_ID: i32 = 1530;
/// Briefed outside stand (west of pack origin 2816; on-pack fallback is
/// (2816,3436), the walkable tile south of the door).
const OUTSIDE: Tile = Tile {
    x: 2813,
    z: 3436,
    level: 0,
};
const OUTSIDE_PACK: Tile = Tile {
    x: 2816,
    z: 3436,
    level: 0,
};
/// Inside stand, north of the door.
const DEST: Tile = Tile {
    x: 2817,
    z: 3443,
    level: 0,
};
/// `::tele` to OUTSIDE (level, mx, mz, lx, lz).
const WALKER_TELE: &str = "tele 0,43,53,61,44";
/// Inside, diagonal to the door (2817,3439) — off the 2816 corridor.
const CLOSER_TELE: &str = "tele 0,44,53,1,47";

struct Shared {
    walker: Slot,
    closer: Slot,
    traveller: Traveller,
    loc_id: Option<i32>,
    dest_armed: bool,
    arrived: bool,
    failed: Option<String>,
    same_tile_ticks: u32,
    last_here: Option<Tile>,
}

struct Slot {
    last_gen: u64,
    here: Option<Tile>,
    scene_state: i32,
    base: (i32, i32),
    tele_sent: bool,
    /// Closer: 1530 was already tried on this open door.
    tried_1530: bool,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            walker: Slot::default(),
            closer: Slot::default(),
            traveller: Traveller::new(),
            loc_id: None,
            dest_armed: false,
            arrived: false,
            failed: None,
            same_tile_ticks: 0,
            last_here: None,
        }
    }
}

impl Default for Slot {
    fn default() -> Self {
        Self {
            last_gen: 0,
            here: None,
            scene_state: 0,
            base: (0, 0),
            tele_sent: false,
            tried_1530: false,
        }
    }
}

#[test]
#[ignore = "requires a local 274 engine, nav pack, two accounts, and LIVE=1"]
fn nav_door() {
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
        profiles(&[("test", "test"), ("test2", "test2")]),
        |_| (None, None),
        {
            let shared = Arc::clone(&shared);
            move |c, name| {
                if name == "test" {
                    walker_frame(c, &shared);
                } else if name == "test2" {
                    closer_frame(c, &shared);
                }
            }
        },
    );

    wait_ingame(&play, 2, Duration::from_secs(150), "nav_door");

    let arm_deadline = Instant::now() + Duration::from_secs(120);
    let mut started: Option<Instant> = None;
    loop {
        let mut s = shared.lock().unwrap();
        if let Some(msg) = s.failed.clone() {
            fail(&msg);
        }
        let here = s.walker.here;
        if s.arrived || here.is_some_and(|h| chebyshev(h, DEST) <= 1 && at_catherby(h)) {
            let elapsed = started.map(|st| st.elapsed()).unwrap_or_default();
            println!("PASS: nav_door walker arrived at {DEST:?} in {elapsed:?} (here={here:?})");
            return;
        }
        if !s.dest_armed {
            if s.walker.scene_state == 2 {
                if let Some(here) = s.walker.here {
                    if at_catherby(here) && outside_ready(here) {
                        let start = if grid.walkable(here) {
                            here
                        } else {
                            OUTSIDE_PACK
                        };
                        match find(&grid, start, DEST) {
                            Ok(route) => {
                                s.traveller.arm(route);
                                s.dest_armed = true;
                                started = Some(Instant::now());
                                println!(
                                    "nav_door: armed dest {start:?} -> {DEST:?} (here={here:?})"
                                );
                            }
                            Err(_) => fail(&format!(
                                "nav_door: no pack path from {start:?} to {DEST:?} (here={here:?})"
                            )),
                        }
                    }
                }
            }
        }
        let now = Instant::now();
        let walk_timed_out = started
            .map(|st| now >= st + Duration::from_secs(120))
            .unwrap_or(false);
        let arm_timed_out = !s.dest_armed && now >= arm_deadline;
        let scene = s.walker.scene_state;
        let loc_id = s.loc_id;
        drop(s);
        if walk_timed_out {
            fail(&format!(
                "nav_door: not at {DEST:?} within 120 s of dest-arm (here={here:?}, loc_id={loc_id:?})"
            ));
        }
        if arm_timed_out {
            fail(&format!(
                "nav_door: never armed dest within 120 s (here={here:?}, scene={scene})"
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn walker_frame(c: &mut client::client::Client, shared: &Arc<Mutex<Shared>>) {
    let Some(lp) = &c.local_player else {
        return;
    };
    let here = Tile {
        x: c.map_build_base_x + lp.route_x[0],
        z: c.map_build_base_z + lp.route_z[0],
        level: 0,
    };
    let loc_id = wall_loc_id(c, DOOR);
    let door_open = loc_id != Some(CLOSED_ID);

    let mut s = shared.lock().unwrap();
    s.walker.scene_state = c.scene_state;
    s.walker.base = (c.map_build_base_x, c.map_build_base_z);
    s.loc_id = loc_id;

    if at_lumbridge(here) && c.scene_state == 2 && !s.walker.tele_sent {
        cheat(c, WALKER_TELE);
        s.walker.tele_sent = true;
        if debug() {
            println!("nav_door walker: tele {WALKER_TELE} from {here:?}");
        }
        return;
    }

    if c.gens.player == s.walker.last_gen && s.walker.here == Some(here) {
        return;
    }
    s.walker.last_gen = c.gens.player;
    s.walker.here = Some(here);

    if !s.dest_armed {
        return;
    }

    if s.last_here == Some(here) {
        s.same_tile_ticks = s.same_tile_ticks.saturating_add(1);
    } else {
        s.same_tile_ticks = 0;
        s.last_here = Some(here);
    }

    let status = s.traveller.tick(c, here, door_open);
    let sent_op = matches!(status, NavStatus::Door);
    if debug() {
        println!(
            "nav_door walker: here={here:?} status={status:?} door_open={door_open} \
             loc_id={loc_id:?} walk_ok={:?} hop={}",
            s.traveller.last_walk_ok(),
            s.same_tile_ticks
        );
    }
    match status {
        NavStatus::Arrived => s.arrived = true,
        NavStatus::Budget => {
            s.failed = Some("nav_door: traveller per-hop budget exceeded".into());
        }
        _ => {}
    }
    if s.same_tile_ticks >= 30 && !door_open && !sent_op {
        s.failed = Some(format!(
            "nav_door: spin — same tile 30 ticks at {here:?}, door closed (loc_id={loc_id:?}), no op_loc"
        ));
    }
}

fn closer_frame(c: &mut client::client::Client, shared: &Arc<Mutex<Shared>>) {
    let Some(lp) = &c.local_player else {
        return;
    };
    let here = Tile {
        x: c.map_build_base_x + lp.route_x[0],
        z: c.map_build_base_z + lp.route_z[0],
        level: 0,
    };
    let mut s = shared.lock().unwrap();
    s.closer.scene_state = c.scene_state;
    s.closer.base = (c.map_build_base_x, c.map_build_base_z);
    s.closer.here = Some(here);

    if at_lumbridge(here) && c.scene_state == 2 && !s.closer.tele_sent {
        cheat(c, CLOSER_TELE);
        s.closer.tele_sent = true;
        if debug() {
            println!("nav_door closer: tele {CLOSER_TELE} from {here:?}");
        }
        return;
    }

    if c.gens.player == s.closer.last_gen {
        return;
    }
    s.closer.last_gen = c.gens.player;

    let loc_id = wall_loc_id(c, DOOR);
    s.loc_id = loc_id;
    let Some(id) = loc_id else {
        return;
    };
    if id == CLOSED_ID {
        s.closer.tried_1530 = false;
        return;
    }
    // Brief: try closed id 1530 first on the open door; if it does not
    // close, use the open id the client currently shows.
    let op_id = if s.closer.tried_1530 { id } else { CLOSED_ID };
    s.closer.tried_1530 = true;
    op_loc(c, DOOR.x, DOOR.z, op_id);
    if debug() {
        println!("nav_door closer: here={here:?} loc_id={id} op_loc {op_id}");
    }
}

fn wall_loc_id(c: &client::client::Client, tile: Tile) -> Option<i32> {
    let sx = tile.x - c.map_build_base_x;
    let sz = tile.z - c.map_build_base_z;
    if !(0..104).contains(&sx) || !(0..104).contains(&sz) {
        return None;
    }
    c.world
        .get_wall(0, sx, sz)
        .map(|w| (w.typecode >> 14) & 0x7fff)
}

fn at_lumbridge(here: Tile) -> bool {
    here.x >= 3200 && here.x < 3264 && here.z >= 3200 && here.z < 3264
}

fn at_catherby(here: Tile) -> bool {
    here.x >= 2800 && here.x < 2860 && here.z >= 3420 && here.z < 3460
}

fn outside_ready(here: Tile) -> bool {
    chebyshev(here, OUTSIDE) <= 1 || (here.z <= 3437 && here.x <= 2818 && here.x >= 2813)
}

fn debug() -> bool {
    std::env::var("BOT_DEBUG").as_deref() == Ok("1")
}

fn pack_path() -> PathBuf {
    std::env::var("NAV_PACK")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_default();
            PathBuf::from(format!("{home}/.274bot/274bot.navpack"))
        })
}
