use api::obj_names::LocDefs;
use api::snapshot::WorldTile;
use client::config::LocType;
use nav_bake::collision::bake_from_maps;
use nav_planner::movement::step_ok;
use nav_planner::router::{find, Leg};
use nav_planner::transport::TransportGraph;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

fn tile(x: i32, z: i32, level: i32) -> WorldTile {
    WorldTile { x, z, level }
}
/// A scratch mapsquare directory for one fixture, removed on drop.
struct FixDir(PathBuf);

impl FixDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("274bot-nav-router-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        FixDir(dir)
    }
}

impl Drop for FixDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A one-loc `LocDefs` table.
fn defs(locs: &[LocType]) -> LocDefs {
    LocDefs::from_locs(locs)
}
/// The wall-tile fixture from the live `nav_door` trace: wall 980
/// (WALL_STRAIGHT, south) at (2816,3437) and door 1530 (WALL_STRAIGHT,
/// north) at (2816,3438) — m44_53 `0 0 45: 980 0 3` and
/// `0 0 46: 1530 0 1`. `step_ok` must reject every step into the wall
/// tile (the east step the live walker took) and the closed door, while
/// genuinely open neighbours still pass, and the router never routes
/// onto the wall tile.
#[test]
fn wall_tile_blocks_through_wall_steps_and_the_router_avoids_it() {
    let fix = FixDir::new("wall-980-door-1530");
    fs::write(
        fix.0.join("m43_53.jm2"),
        "==== MAP ====\n0 63 44: h1 u50\n0 63 45: h1 u50\n0 63 46: h1 u50\n==== LOC ====\n",
    )
    .unwrap();
    fs::write(
            fix.0.join("m44_53.jm2"),
            "==== MAP ====\n0 0 43: h1 u50\n0 0 44: h10 u50\n0 0 45: h19 o10 u48\n0 0 46: h30 o10 u48\n0 0 47: h30 o5 f4 u50\n==== LOC ====\n0 0 45: 980 0 3\n0 0 46: 1530 0 1\n",
        )
        .unwrap();
    let locs = defs(&[
        LocType {
            id: 980,
            blockwalk: true,
            ..LocType::default()
        },
        LocType {
            id: 1530,
            blockwalk: true,
            ..LocType::default()
        },
    ]);
    let mut door_ids = HashSet::new();
    door_ids.insert(1530);
    let wc = bake_from_maps(&fix.0, &locs, &door_ids).unwrap();
    let g = TransportGraph::default();

    // South face of wall 980: cannot enter that tile from the south.
    // Entering it from the west is a walk along the wall, not through it.
    assert!(!step_ok(&wc, tile(2816, 3436, 0), (0, 1)));
    assert!(step_ok(&wc, tile(2815, 3437, 0), (1, 0)));
    assert!(!step_ok(&wc, tile(2816, 3438, 0), (0, 1)));
    // A genuinely open neighbour still passes.
    assert!(step_ok(&wc, tile(2815, 3437, 0), (0, 1)));
    assert!(step_ok(&wc, tile(2815, 3437, 0), (-1, 0)));

    let r = find(&wc, &g, tile(2813, 3436, 0), tile(2815, 3438, 0)).unwrap();
    let Leg::Walk { tiles } = &r.legs[0] else {
        panic!("walk-only route");
    };
    // Crossing the wall's south face is still rejected; the path stays
    // on the open side.
    assert!(!tiles.contains(&tile(2816, 3436, 0)));
}
