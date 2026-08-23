//! `nav-pack` CLI: bake the Lumbridge (m50_50) and Catherby (m44_53)
//! mapsquares into a nav pack and write it to `$NAV_PACK` or
//! `~/.274bot/274bot.navpack` (default). Usage: `nav-pack [MAPS_DIR]`, where
//! the default maps dir is `/Users/acfrazier/experiments/Server/content/maps`.
//! Missing or failed mapsquares are skipped with a stderr count; the
//! remaining squares still produce a pack.

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use nav::pack::{encode, merge_squares, parse_mapsquare_jm2, walkable_dots, Mapsquare};

const MAPS_DIR: &str = "/Users/acfrazier/experiments/Server/content/maps";
const SQUARES: [(&str, i32, i32); 2] = [("m50_50.jm2", 50, 50), ("m44_53.jm2", 44, 53)];

fn default_out() -> PathBuf {
    match env::var("HOME") {
        Ok(home) => PathBuf::from(format!("{home}/.274bot/274bot.navpack")),
        Err(_) => PathBuf::from(".274bot/274bot.navpack"),
    }
}

fn main() -> ExitCode {
    let maps_dir = env::args().nth(1).unwrap_or_else(|| MAPS_DIR.into());
    let out = env::var("NAV_PACK")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_out());
    let mut squares: Vec<Mapsquare> = Vec::new();
    let mut skipped = 0usize;
    for (name, mx, mz) in SQUARES {
        let path = Path::new(&maps_dir).join(name);
        match parse_mapsquare_jm2(&path, mx, mz) {
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
