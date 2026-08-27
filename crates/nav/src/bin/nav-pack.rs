//! `nav-pack` CLI: bake the whole world — every `maps/*.jm2` mapsquare —
//! into a level-0 [`WorldCollision`] and write the nav pack to `$NAV_PACK`
//! or `~/.274bot/274bot.navpack` (default). Usage:
//! `nav-pack [MAPS_DIR] [DOORS_CONFIG_DIR] [CONFIG_JAG]`, where the defaults
//! are `/Users/acfrazier/experiments/Server/content/maps`,
//! `/Users/acfrazier/experiments/Server/content/scripts/doors/configs`, and
//! the compiled client cache `/Users/acfrazier/experiments/Server/engine/data/pack/config`.
//! Door loc ids come from the `*.loc` door configs; loc `blockwalk=no` /
//! open-door stages come from `content/scripts/**/*.loc`; the loc definitions
//! (blockwalk, width/length, active) come from the client cache's `config`
//! jag. Every `.jm2` under the maps dir bakes or the run fails; non-`.jm2`
//! files (`ignore.csv`/`free2play.csv`) are metadata and skipped.

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use api::obj_names::LocDefs;
use api::snapshot::WorldTile;
use client::config::Cache;
use client::io::JagFile;
use nav::collision::{bake_from_maps, WorldCollision};
use nav::pack::{
    encode, merge_squares, parse_door_config, parse_mapsquare_jm2, parse_passable_locs, Mapsquare,
};

const MAPS_DIR: &str = "/Users/acfrazier/experiments/Server/content/maps";
const DOORS_DIR: &str = "/Users/acfrazier/experiments/Server/content/scripts/doors/configs";
const SCRIPTS_DIR: &str = "/Users/acfrazier/experiments/Server/content/scripts";
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
            Ok(text) => door_ids.extend(parse_door_config(&text)),
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

    let mut passable = HashSet::new();
    visit_loc_files(Path::new(SCRIPTS_DIR), &mut |text| {
        passable.extend(parse_passable_locs(text));
    });

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

    // The nav pack write: one bbox StepGrid on level 0 over every mapsquare.
    let entries = match fs::read_dir(&maps_dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("nav-pack: read {maps_dir}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut squares: Vec<Mapsquare> = Vec::new();
    for ent in entries.flatten() {
        let path = ent.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jm2") {
            continue;
        }
        let Some((mx, mz)) = mapsquare_coords(&path) else {
            eprintln!(
                "nav-pack: bailing on {}: not an m<x>_<z> mapsquare",
                path.display()
            );
            return ExitCode::FAILURE;
        };
        match parse_mapsquare_jm2(&path, mx, mz, &door_ids, &passable) {
            Ok(sq) => squares.push(sq),
            Err(e) => {
                eprintln!("nav-pack: bailing on {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
        }
    }
    if squares.is_empty() {
        eprintln!("nav-pack: no mapsquares baked");
        return ExitCode::FAILURE;
    }
    let grid = merge_squares(&squares);
    let bytes = encode(&grid);
    if let Err(e) = std::fs::write(&out, &bytes) {
        eprintln!("nav-pack: write {}: {e}", out.display());
        return ExitCode::FAILURE;
    }
    eprintln!(
        "nav-pack: baked {} mapsquares (0 skipped) into a {}x{} collision grid, {} walkable tiles -> {} bytes ({} doors) -> {}",
        squares.len(),
        collision.width,
        collision.height,
        walkable,
        bytes.len(),
        grid.doors.len(),
        out.display()
    );
    ExitCode::SUCCESS
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

/// `m<x>_<z>.jm2` -> `(x, z)`.
fn mapsquare_coords(path: &Path) -> Option<(i32, i32)> {
    let name = path.file_stem()?.to_str()?;
    let rest = name.strip_prefix('m')?;
    let (x, z) = rest.split_once('_')?;
    Some((x.parse().ok()?, z.parse().ok()?))
}

/// Recursively read every `*.loc` under `dir` and hand the text to `cb`.
fn visit_loc_files(dir: &Path, cb: &mut impl FnMut(&str)) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for ent in entries.flatten() {
        let path = ent.path();
        if path.is_dir() {
            visit_loc_files(&path, cb);
        } else if path.extension().and_then(|s| s.to_str()) == Some("loc") {
            if let Ok(text) = std::fs::read_to_string(&path) {
                cb(&text);
            }
        }
    }
}
