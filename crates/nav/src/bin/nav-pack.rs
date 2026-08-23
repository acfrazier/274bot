//! `nav-pack` CLI: bake the Lumbridge (m50_50) and Catherby (m44_53)
//! mapsquares into a nav pack and write it to `$NAV_PACK` or
//! `~/.274bot/274bot.navpack` (default). Usage:
//! `nav-pack [MAPS_DIR] [DOORS_CONFIG_DIR]`, where the defaults are
//! `/Users/acfrazier/experiments/Server/content/maps` and
//! `/Users/acfrazier/experiments/Server/content/scripts/doors/configs`.
//! Door loc ids come from the `*.loc` door configs; missing or failed
//! mapsquares are skipped with a stderr count, and the remaining squares
//! still produce a pack.

use std::collections::HashSet;
use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use nav::pack::{encode, merge_squares, parse_door_config, parse_mapsquare_jm2, walkable_dots, Mapsquare};

const MAPS_DIR: &str = "/Users/acfrazier/experiments/Server/content/maps";
const DOORS_DIR: &str = "/Users/acfrazier/experiments/Server/content/scripts/doors/configs";
const SQUARES: [(&str, i32, i32); 2] = [("m50_50.jm2", 50, 50), ("m44_53.jm2", 44, 53)];
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

    let mut squares: Vec<Mapsquare> = Vec::new();
    let mut skipped = 0usize;
    for (name, mx, mz) in SQUARES {
        let path = Path::new(&maps_dir).join(name);
        match parse_mapsquare_jm2(&path, mx, mz, &door_ids) {
            Ok(sq) => squares.push(sq),
            Err(e) => {
                eprintln!("nav-pack: skipping {name}: {e}");
                skipped += 1;
            }
        }
    }
    if squares.is_empty() {
        eprintln!("nav-pack: no mapsquares baked (all {skipped} skipped)");
        return ExitCode::FAILURE;
    }
    let grid = merge_squares(&squares);
    let bytes = encode(&grid);
    if let Err(e) = std::fs::write(&out, &bytes) {
        eprintln!("nav-pack: write {}: {e}", out.display());
        return ExitCode::FAILURE;
    }
    eprintln!(
        "nav-pack: {} bytes ({} walkable tiles, {} doors, {} mapsquare skipped) -> {}",
        bytes.len(),
        walkable_dots(&grid, 0).count(),
        grid.doors.len(),
        skipped,
        out.display()
    );
    ExitCode::SUCCESS
}
