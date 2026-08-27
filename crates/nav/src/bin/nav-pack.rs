//! `nav-pack` CLI: bake the whole world — every `maps/*.jm2` mapsquare —
//! into a level-0 [`WorldCollision`], derive the [`TransportGraph`] from
//! the Server content, and write the v2 nav pack to `$NAV_PACK` or
//! `~/.274bot/274bot.navpack` (default). Usage:
//! `nav-pack [MAPS_DIR] [DOORS_CONFIG_DIR] [CONFIG_JAG]`, where the defaults
//! are `/Users/acfrazier/experiments/Server/content/maps`,
//! `/Users/acfrazier/experiments/Server/content/scripts/doors/configs`, and
//! the compiled client cache `/Users/acfrazier/experiments/Server/engine/data/pack/config`.
//! Door loc ids come from the `*.loc` door configs; the loc definitions
//! (blockwalk, width/length, active) come from the client cache's `config`
//! jag. Every `.jm2` under the maps dir bakes or the run fails; non-`.jm2`
//! files (`ignore.csv`/`free2play.csv`) are metadata and skipped. The
//! transport graph derives from the Server content tree (the maps dir's
//! parent): door/ladder/stairs/agility edges, with boats and teleport
//! spells counted and skipped on stderr.

use std::collections::HashSet;
use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use api::obj_names::LocDefs;
use api::snapshot::WorldTile;
use client::config::Cache;
use client::io::JagFile;
use nav::collision::{bake_from_maps, WorldCollision};
use nav::pack::encode_v2;
use nav::transport::derive_transports;

const MAPS_DIR: &str = "/Users/acfrazier/experiments/Server/content/maps";
const DOORS_DIR: &str = "/Users/acfrazier/experiments/Server/content/scripts/doors/configs";
const CONFIG_JAG: &str = "/Users/acfrazier/experiments/Server/engine/data/pack/config";
const DOOR_CONFIGS: [&str; 3] = ["doors.loc", "doubledoors.loc", "opened_doors.loc"];

fn default_out() -> PathBuf {
    match env::var("HOME") {
        Ok(home) => PathBuf::from(format!("{home}/.274bot/274bot.navpack")),
        Err(_) => PathBuf::from(".274bot/274bot.navpack"),
    }
}

fn main() -> ExitCode {
    let mut it = env::args().skip(1);
    let maps_dir = it.next().unwrap_or_else(|| MAPS_DIR.into());
    let doors_dir = it.next().unwrap_or_else(|| DOORS_DIR.into());
    let config_jag = it.next().unwrap_or_else(|| CONFIG_JAG.into());
    let out = env::var("NAV_PACK")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_out());

    // Openable wall door loc ids from the Server door configs.
    let mut door_ids = HashSet::new();
    let mut config_failed = 0usize;
    for name in DOOR_CONFIGS {
        let path = Path::new(&doors_dir).join(name);
        match std::fs::read_to_string(&path) {
            Ok(text) => door_ids.extend(nav::pack::parse_door_config(&text)),
            Err(e) => {
                eprintln!("nav-pack: skipping {name}: {e}");
                config_failed += 1;
            }
        }
    }
    if config_failed == DOOR_CONFIGS.len() {
        eprintln!(
            "nav-pack: no door configs parsed (need {} in {doors_dir})",
            DOOR_CONFIGS.join(", ")
        );
        return ExitCode::FAILURE;
    }

    // Loc definitions (blockwalk, width/length, active) from the client
    // cache: the same table the game client builds its collision from.
    let loc_defs = match std::fs::read(&config_jag) {
        Ok(bytes) => {
            let cache = Cache::unpack(&JagFile::new(bytes));
            LocDefs::from_locs(&cache.locs)
        }
        Err(e) => {
            eprintln!(
                "nav-pack: cannot load loc defs from {}: {e}",
                Path::new(&config_jag).display()
            );
            return ExitCode::FAILURE;
        }
    };

    // Whole-world collision bake (the walkability source of truth).
    let collision = match bake_from_maps(Path::new(&maps_dir), &loc_defs, &door_ids) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("nav-pack: collision bake failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    let walkable = walkable_tiles(&collision);

    // The transport graph from the Server content tree (maps/scripts/pack
    // all live under the maps dir's parent).
    let content_root = Path::new(&maps_dir)
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let graph = derive_transports(content_root, &loc_defs);

    // The v2 pack write: collision flags + transport edges.
    let bytes = encode_v2(&collision, &graph);
    if let Err(e) = std::fs::write(&out, &bytes) {
        eprintln!("nav-pack: write {}: {e}", out.display());
        return ExitCode::FAILURE;
    }
    eprintln!(
        "nav-pack: baked {} mapsquares into a {}x{} collision grid, {} walkable tiles, {} transport edges -> {} bytes -> {}",
        squares_baked(&maps_dir),
        collision.width,
        collision.height,
        walkable,
        graph.edges.len(),
        bytes.len(),
        out.display()
    );
    ExitCode::SUCCESS
}

/// Count `.jm2` files under `maps_dir` (for the summary line).
fn squares_baked(maps_dir: &str) -> usize {
    std::fs::read_dir(maps_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("jm2"))
                .count()
        })
        .unwrap_or(0)
}

/// Count tiles with no walk-blocking flag on the bake's level-0 plane.
fn walkable_tiles(c: &WorldCollision) -> usize {
    (0..c.height)
        .flat_map(|z| {
            (0..c.width).map(move |x| (c.origin.x + x as i32, c.origin.z + z as i32))
        })
        .filter(|(x, z)| c.walkable(WorldTile { x: *x, z: *z, level: 0 }))
        .count()
}
