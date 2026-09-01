//! Live: prove the baked whole-world nav pack's level-0 collision matches
//! the live client's scene `CollisionMap` for every scene tile 0..104.
//! The mainland hop lands in the Lumbridge courtyard (base near
//! (3168, 3168)); the observe hook snapshots
//! `client.collision[minusedlevel]` plus the map-build bases once the
//! mainland scene is fully built (`ingame && scene_state == 2`), then the
//! test compares each scene tile's walk-relevant flags
//! (`WR_GRND | WALK_SCENERY | W_* | V_*`) against the pack's level-0 word
//! at `(base_x + lx, base_z + lz)`.
//!
//! Two scene-relative client behaviors make the border band incomparable,
//! so it is excluded: `CollisionMap.reset` masks the 104 edge with
//! `_BOUNDS`, and `loadLocations` drops any loc whose anchor tile sits on
//! that edge (a 3-wide border tree's footprint reaches 2 tiles inward).
//! The 4-tile band (`0..=3` and `100..=103`) covers both. A loc on a
//! LINK_BELOW tile is stamped on `level - 1` by the client, never its
//! placement plane — the pack mirrors that (the bake's `loadLocations`
//! parity), so those tiles compare normally and any residual mismatch is
//! dumped with a LINK_BELOW marker.
//!
//! Requires a **v5** pack (four collision planes, the v4 wire plus the
//! `worn_req` list): the stale v3 file
//! fails the load with a "rebake" hint. Run with the engine up:
//! `LIVE=1 cargo test -p e2e --test nav_collision -- --ignored --test-threads=1 --nocapture`

mod common;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use client::dash3d::CollisionFlag;
use common::{fail, live, options, profiles, wait_ingame};
use host_play::run_with_io;
use nav::world::NavWorld;

/// Walk-relevant raw flags compared across pack and live: the client's
/// directional wall bits (`W_*`), the scenery footprint, the ground block,
/// and the blockrange vis bits (`V_*`) the client sets with `add_wall`.
/// Visibility-only bits (`VIS_SCENERY`) and the `_BOUNDS` edge mask are
/// not walk-relevant.
const WALK_MASK: i32 = CollisionFlag::WR_GRND
    | CollisionFlag::WALK_SCENERY
    | CollisionFlag::WALK_BLOCK_FLAGS
    | CollisionFlag::VIS_BLOCK_FLAGS;

/// The scene build area is 104x104 tiles.
const SCENE: usize = 104;

/// Scene tiles within this distance of the border are not comparable: the
/// client's `_BOUNDS` reset masks the edge and `loadLocations` drops locs
/// anchored on it (their footprints reach a few tiles inward — 3 wide in
/// the Lumbridge courtyard).
const BORDER_BAND: usize = 4;

/// The client's `MapFlag::LINK_BELOW` bit (level-1 map flags).
const LINK_BELOW: u8 = 0x2;

/// The live collision snapshot the observe hook takes once the mainland
/// scene is fully built.
#[derive(Clone)]
struct Captured {
    base_x: i32,
    base_z: i32,
    minusedlevel: i32,
    /// Live `collision[minusedlevel].flags`, scene-local `flags[lx][lz]`.
    flags: Vec<[i32; SCENE]>,
    /// `mapl[1]` LINK_BELOW mask, scene-local `[lx][lz]`.
    link_below: Vec<[bool; SCENE]>,
}

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
#[ignore = "requires a local 274 engine, a v5 nav pack, and LIVE=1"]
fn nav_collision() {
    if !live() {
        return;
    }

    // The pack must be v5 (four planes, like v4): a stale v3 file fails to load.
    let path = pack_path();
    let world = match NavWorld::load_pack(&path) {
        Ok(w) => w,
        Err(e) => fail(&format!(
            "nav_collision: cannot load nav pack {} ({e}); \
             rebake with `cargo run -p nav --bin nav-pack` (v5)",
            path.display()
        )),
    };

    let mut opts = options();
    opts.mainland = true;
    let captured: Arc<Mutex<Option<Captured>>> = Arc::new(Mutex::new(None));
    let cap = Arc::clone(&captured);
    let play = run_with_io(&opts, profiles(&[("test", "test")]), |_| (None, None), {
        move |c, _name, _hold| {
            // The tutorial island also reaches scene 2; only capture once
            // the mainland hop has landed and the Lumbridge scene is up.
            if c.ingame && c.scene_state == 2 && !c.within_tutorial_island {
                let mut guard = cap.lock().unwrap();
                if guard.is_none() {
                    let level = c.minusedlevel as usize;
                    let mut link_below = vec![[false; SCENE]; SCENE];
                    for (lx, row) in link_below.iter_mut().enumerate() {
                        for (lz, cell) in row.iter_mut().enumerate() {
                            *cell = c.mapl[1][lx][lz] & LINK_BELOW != 0;
                        }
                    }
                    *guard = Some(Captured {
                        base_x: c.map_build_base_x,
                        base_z: c.map_build_base_z,
                        minusedlevel: c.minusedlevel,
                        flags: c.collision[level].flags.clone(),
                        link_below,
                    });
                }
            }
        }
    });

    // The tutorial island reaches scene 2 first; the slot's mainland hop
    // fires on that edge, so give the capture generous deadlines.
    wait_ingame(&play, 1, Duration::from_secs(150), "nav_collision");
    let deadline = Instant::now() + Duration::from_secs(150);
    let cap_data = loop {
        if let Some(c) = captured.lock().unwrap().clone() {
            break c;
        }
        if Instant::now() >= deadline {
            fail("nav_collision: never captured the mainland scene collision");
        }
        std::thread::sleep(Duration::from_millis(250));
    };

    println!(
        "PASS: nav_collision: scene base=({},{}) minusedlevel={}",
        cap_data.base_x, cap_data.base_z, cap_data.minusedlevel
    );

    // Walk every scene tile inside the border band: pack level-0 word vs
    // the live collision[minusedlevel] word, both masked to walk-relevant
    // bits.
    let mut mismatches: Vec<(i32, i32, i32, i32)> = Vec::new();
    let mut link_below_mismatches = 0usize;
    for lz in BORDER_BAND..SCENE - BORDER_BAND {
        for lx in BORDER_BAND..SCENE - BORDER_BAND {
            let wx = cap_data.base_x + lx as i32;
            let wz = cap_data.base_z + lz as i32;
            let pack = (world.collision.flag(wx, wz, 0) & WALK_MASK as u32) as i32;
            let live = cap_data.flags[lx][lz] & WALK_MASK;
            if pack != live {
                mismatches.push((wx, wz, pack, live));
                if cap_data.link_below[lx][lz] {
                    link_below_mismatches += 1;
                }
            }
        }
    }

    if mismatches.is_empty() {
        println!("PASS: pack vs live collision matches on all scene tiles");
        return;
    }

    for (wx, wz, pack, live) in &mismatches {
        let lb =
            cap_data.link_below[(wx - cap_data.base_x) as usize][(wz - cap_data.base_z) as usize];
        println!("FAIL: pack vs live ({wx},{wz}) pack={pack:#x} live={live:#x}");
        if lb {
            println!("  (LINK_BELOW on mapl[1]: the client shifts this loc's plane)");
        }
    }
    fail(&format!(
        "nav_collision: {} mismatched tiles pack vs live \
         ({} on LINK_BELOW tiles)",
        mismatches.len(),
        link_below_mismatches
    ));
}
