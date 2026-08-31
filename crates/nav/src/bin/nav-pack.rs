//! `nav-pack` CLI: bake the whole world — every `maps/*.jm2` mapsquare —
//! into a per-level [`WorldCollision`] (four planes like the client's
//! `collision[4]`), derive the [`TransportGraph`] from
//! the Server content, and write the v7 nav pack (magic `274V`, version
//! byte 7; `encode`) to `$NAV_PACK` or
//! `~/.274bot/274bot.navpack` (default), plus the raw flags sidecar
//! (magic `274F`; `encode_flags_sidecar`) to `$NAV_FLAGS` or the pack
//! path with its extension swapped to `.navflags` (default
//! `~/.274bot/274bot.navflags`). Usage:
//! `nav-pack [MAPS_DIR] [DOORS_CONFIG_DIR] [CONFIG_JAG]`, where the defaults
//! are `$ENGINE_DIR/../content/maps` (default
//! `$HOME/experiments/Server/engine` → `.../content/maps`), matching doors
//! under that content tree, and `$ENGINE_DIR/data/pack/config`.
//! Door loc ids come from the `*.loc` door configs plus
//! `scripts/general_use/configs/gates.loc` (derived from the maps dir's
//! parent, the Server `content/` root); the loc definitions
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
use nav::pack::{encode, encode_flags_sidecar};
use nav::transport::derive_transports;

const DOOR_CONFIGS: [&str; 3] = ["doors.loc", "doubledoors.loc", "opened_doors.loc"];

fn default_maps_dir() -> PathBuf {
    client::bot_target::content_dir().join("maps")
}

fn default_doors_dir() -> PathBuf {
    client::bot_target::content_dir().join("scripts/doors/configs")
}

fn default_config_jag() -> PathBuf {
    client::bot_target::config_jag()
}

/// `gates.loc` lives under the Server `content/` tree, sibling of `maps/`.
fn gates_loc(maps_dir: &Path) -> PathBuf {
    let content_root = maps_dir.parent().unwrap_or(maps_dir);
    content_root.join("scripts/general_use/configs/gates.loc")
}

fn default_out() -> PathBuf {
    match env::var("HOME") {
        Ok(home) => PathBuf::from(format!("{home}/.274bot/274bot.navpack")),
        Err(_) => PathBuf::from(".274bot/274bot.navpack"),
    }
}

/// Where the flags sidecar goes: `$NAV_FLAGS` if set, else the pack path
/// with its extension swapped to `.navflags`.
fn flags_out(out: &Path) -> PathBuf {
    env::var("NAV_FLAGS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| flags_path_for(out))
}

/// The default sidecar path next to a pack path: `274bot.navpack` ->
/// `274bot.navflags`.
fn flags_path_for(out: &Path) -> PathBuf {
    out.with_extension("navflags")
}

fn main() -> ExitCode {
    let mut it = env::args().skip(1);
    let maps_dir = it
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(default_maps_dir);
    let doors_dir = it
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(default_doors_dir);
    let config_jag = it
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(default_config_jag);
    let gates = gates_loc(&maps_dir);
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
    // Fence gates (`scripts/general_use/configs/gates.loc`) join the door
    // set so their tiles do not stamp blocked in the bake; the transport
    // graph derives the same set itself in `door_edges`.
    match std::fs::read_to_string(&gates) {
        Ok(text) => door_ids.extend(nav::pack::parse_door_config(&text)),
        Err(e) => {
            eprintln!("nav-pack: skipping gates.loc: {e}");
            config_failed += 1;
        }
    }
    if config_failed == DOOR_CONFIGS.len() + 1 {
        eprintln!(
            "nav-pack: no door configs parsed (need {} in {} plus {})",
            DOOR_CONFIGS.join(", "),
            doors_dir.display(),
            gates.display()
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
                config_jag.display()
            );
            return ExitCode::FAILURE;
        }
    };

    // Whole-world collision bake (the walkability source of truth).
    let mut collision = match bake_from_maps(&maps_dir, &loc_defs, &door_ids) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("nav-pack: collision bake failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    let walkable = walkable_tiles(&collision);

    // The transport graph from the Server content tree (maps/scripts/pack
    // all live under the maps dir's parent); door edge from/to snap to the
    // nearest walkable tile on the collision just baked.
    let content_root = maps_dir.parent().unwrap_or_else(|| Path::new("."));
    let graph = derive_transports(content_root, &loc_defs, &collision);

    // The raw baked flags ride in the sidecar; the v7 pack carries only
    // the packed walk surface (the router's resident form).
    let flags = collision
        .flags
        .take()
        .expect("bake_from_maps always stamps raw flags");
    let flags_bytes =
        encode_flags_sidecar(collision.origin, collision.width, collision.height, &flags);
    let flags_path = flags_out(&out);

    // The pack write: packed walk surface + transport edges.
    let bytes = encode(&collision, &graph);
    if let Err(e) = std::fs::write(&out, &bytes) {
        eprintln!("nav-pack: write {}: {e}", out.display());
        return ExitCode::FAILURE;
    }
    if let Err(e) = std::fs::write(&flags_path, &flags_bytes) {
        eprintln!("nav-pack: write {}: {e}", flags_path.display());
        return ExitCode::FAILURE;
    }
    eprintln!(
        "nav-pack: baked {} mapsquares into a {}x{} collision grid, {} walkable tiles, {} transport edges -> {} bytes -> {}; {} flag bytes -> {}",
        squares_baked(&maps_dir),
        collision.width,
        collision.height,
        walkable,
        graph.edges.len(),
        bytes.len(),
        out.display(),
        flags_bytes.len(),
        flags_path.display()
    );
    ExitCode::SUCCESS
}

/// Count `.jm2` files under `maps_dir` (for the summary line).
fn squares_baked(maps_dir: &Path) -> usize {
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
        .flat_map(|z| (0..c.width).map(move |x| (c.origin.x + x as i32, c.origin.z + z as i32)))
        .filter(|(x, z)| {
            c.walkable(WorldTile {
                x: *x,
                z: *z,
                level: 0,
            })
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gates_loc_sits_under_content_next_to_maps() {
        let maps = Path::new("/tmp/Server/content/maps");
        assert_eq!(
            gates_loc(maps),
            PathBuf::from("/tmp/Server/content/scripts/general_use/configs/gates.loc")
        );
    }

    #[test]
    fn flags_path_for_swaps_pack_extension() {
        assert_eq!(
            flags_path_for(&PathBuf::from("/tmp/x/274bot.navpack")),
            PathBuf::from("/tmp/x/274bot.navflags")
        );
    }

    #[test]
    fn default_paths_follow_engine_dir() {
        let engine = client::engine_dir();
        let content = client::bot_target::content_dir();
        assert_eq!(default_maps_dir(), content.join("maps"));
        assert_eq!(default_doors_dir(), content.join("scripts/doors/configs"));
        assert_eq!(default_config_jag(), engine.join("data/pack/config"));
        assert_eq!(
            gates_loc(&default_maps_dir()),
            content.join("scripts/general_use/configs/gates.loc")
        );
    }
}
