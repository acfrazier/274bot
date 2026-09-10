//! `nav-pack` CLI: bake the whole world — every `maps/*.jm2` mapsquare —
//! into a per-level [`WorldCollision`] (four planes like the client's
//! `collision[4]`), derive the [`TransportGraph`] from
//! the Server content, and write the v8 nav pack (magic `274V`, version
//! byte 8; `encode`) to `$NAV_PACK` or
//! `~/.274bot/274bot.navpack` (default), plus the raw flags sidecar
//! (magic `274F`; `encode_flags_sidecar`) to `$NAV_FLAGS` or the pack
//! path with its extension swapped to `.navflags` (default
//! `~/.274bot/274bot.navflags`). Legacy 274 usage remains:
//! `nav-pack [MAPS_DIR] [DOORS_CONFIG_DIR] [CONFIG_JAG]`, where the defaults
//! are `$ENGINE_DIR/../content/maps` (default
//! `$HOME/experiments/Server/engine` → `.../content/maps`), matching doors
//! under that content tree, and `$ENGINE_DIR/data/pack/config`.
//! Revision-bound bakes use explicit inputs:
//! `nav-pack --revision 274|289 --content CONTENT_DIR --cache CACHE_DIR
//! --cache-manifest CACHE_MANIFEST --out NAV_PACK [--flags-out NAV_FLAGS]`.
//! This mode verifies every cache archive and the selected config before
//! baking, then emits `<NAV_PACK>.json` using [`nav::manifest::NavManifest`].
//! Revision 274 reads the server `data/pack/config` beside the `client/`
//! cache directory; revision 289 reads `data/pack/client/config`.
//! Door loc ids come from the `*.loc` door configs plus
//! `scripts/general_use/configs/gates.loc` (derived from the maps dir's
//! parent, the Server `content/` root); the loc definitions
//! (blockwalk, width/length, active) come from the client cache's `config`
//! jag. Every `.jm2` under the maps dir bakes or the run fails; non-`.jm2`
//! files (`ignore.csv`/`free2play.csv`) are metadata and skipped. The
//! transport graph derives from the Server content tree (the maps dir's
//! parent): door/ladder/stairs/agility edges, with boats and teleport
//! spells counted and skipped on stderr. The bank stand table
//! ([`nav::pack::derive_banks`]) bakes from the same content tree: every
//! `bankbooth` loc placement with the Use-quickly op. Rebake whenever the
//! Server content changes — a stale v7 pack decodes as `BadVersion`.

use std::collections::HashSet;
use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use api::obj_names::LocDefs;
use api::snapshot::WorldTile;
use client::config::Cache;
use client::io::JagFile;
use nav::collision::{bake_from_maps, WorldCollision};
use nav::manifest::{nav_manifest_path, CacheManifest, NavManifest};
use nav::pack::{derive_banks, encode, encode_flags_sidecar};
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
    match client::operator_home() {
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

#[derive(Debug)]
struct BakeInputs {
    revision: Option<u16>,
    maps_dir: PathBuf,
    doors_dir: PathBuf,
    config_jag: PathBuf,
    cache_dir: Option<PathBuf>,
    cache_manifest: Option<PathBuf>,
    out: PathBuf,
    flags_out: PathBuf,
}

fn parse_args(args: impl IntoIterator<Item = impl AsRef<str>>) -> Result<BakeInputs, String> {
    let args: Vec<String> = args.into_iter().map(|s| s.as_ref().to_string()).collect();
    if !args.iter().any(|arg| arg.starts_with("--")) {
        if args.len() > 3 {
            return Err("usage: nav-pack [MAPS_DIR [DOORS_CONFIG_DIR [CONFIG_JAG]]]".into());
        }
        let maps_dir = args
            .first()
            .map(PathBuf::from)
            .unwrap_or_else(default_maps_dir);
        let doors_dir = args
            .get(1)
            .map(PathBuf::from)
            .unwrap_or_else(default_doors_dir);
        let config_jag = args
            .get(2)
            .map(PathBuf::from)
            .unwrap_or_else(default_config_jag);
        let out = env::var("NAV_PACK")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_out());
        let flags_out = flags_out(&out);
        return Ok(BakeInputs {
            revision: None,
            maps_dir,
            doors_dir,
            config_jag,
            cache_dir: None,
            cache_manifest: None,
            out,
            flags_out,
        });
    }

    let mut revision = None;
    let mut content = None;
    let mut cache_dir = None;
    let mut cache_manifest = None;
    let mut out = None;
    let mut explicit_flags = None;
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        if !flag.starts_with("--") {
            return Err(format!(
                "unexpected positional argument {flag:?} in explicit mode"
            ));
        }
        let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
        if value.starts_with("--") || value.is_empty() {
            return Err(format!("{flag} needs a value"));
        }
        match flag.as_str() {
            "--revision" => {
                if revision.is_some() {
                    return Err("duplicate explicit option --revision".into());
                }
                let parsed = value
                    .parse::<u16>()
                    .map_err(|_| "--revision must be 274 or 289".to_string())?;
                if !matches!(parsed, 274 | 289) {
                    return Err("--revision must be 274 or 289".into());
                }
                revision = Some(parsed);
            }
            "--content" if content.is_none() => content = Some(PathBuf::from(value)),
            "--cache" if cache_dir.is_none() => cache_dir = Some(PathBuf::from(value)),
            "--cache-manifest" if cache_manifest.is_none() => {
                cache_manifest = Some(PathBuf::from(value));
            }
            "--out" if out.is_none() => out = Some(PathBuf::from(value)),
            "--flags-out" if explicit_flags.is_none() => {
                explicit_flags = Some(PathBuf::from(value));
            }
            "--content" | "--cache" | "--cache-manifest" | "--out" | "--flags-out" => {
                return Err(format!("duplicate explicit option {flag}"));
            }
            _ => return Err(format!("unknown explicit option {flag}")),
        }
    }
    let revision =
        revision.ok_or_else(|| "explicit mode requires --revision 274|289".to_string())?;
    let content =
        content.ok_or_else(|| "explicit mode requires --content CONTENT_DIR".to_string())?;
    let cache_dir =
        cache_dir.ok_or_else(|| "explicit mode requires --cache CACHE_DIR".to_string())?;
    let cache_manifest = cache_manifest
        .ok_or_else(|| "explicit mode requires --cache-manifest CACHE_MANIFEST".to_string())?;
    let out = out.ok_or_else(|| "explicit mode requires --out NAV_PACK".to_string())?;
    let config_jag = if revision == 289 {
        cache_dir.join("config")
    } else {
        cache_dir
            .parent()
            .ok_or_else(|| "revision 274 cache path has no data/pack parent".to_string())?
            .join("config")
    };
    let flags_out = explicit_flags.unwrap_or_else(|| flags_path_for(&out));
    Ok(BakeInputs {
        revision: Some(revision),
        maps_dir: content.join("maps"),
        doors_dir: content.join("scripts/doors/configs"),
        config_jag,
        cache_dir: Some(cache_dir),
        cache_manifest: Some(cache_manifest),
        out,
        flags_out,
    })
}

fn main() -> ExitCode {
    let inputs = match parse_args(env::args().skip(1)) {
        Ok(inputs) => inputs,
        Err(error) => {
            eprintln!("nav-pack: {error}");
            return ExitCode::FAILURE;
        }
    };
    let maps_dir = &inputs.maps_dir;
    let doors_dir = &inputs.doors_dir;
    let config_jag = &inputs.config_jag;
    let gates = gates_loc(maps_dir);

    let verified_cache = match (
        inputs.revision,
        inputs.cache_dir.as_deref(),
        inputs.cache_manifest.as_deref(),
    ) {
        (Some(revision), Some(cache_dir), Some(manifest_path)) => {
            let result = (|| {
                let bytes = std::fs::read(manifest_path)
                    .map_err(|e| format!("cache manifest {}: {e}", manifest_path.display()))?;
                let manifest: CacheManifest = serde_json::from_slice(&bytes)
                    .map_err(|e| format!("cache manifest {}: {e}", manifest_path.display()))?;
                manifest.verify(revision, cache_dir)?;
                let expected_config = manifest
                    .archives
                    .get("config")
                    .ok_or_else(|| "cache manifest has no config archive".to_string())?;
                let actual_config = nav::manifest::hash_file(config_jag)?;
                if &actual_config != expected_config {
                    return Err(format!(
                        "selected config {} does not belong to the verified revision {revision} cache",
                        config_jag.display()
                    ));
                }
                Ok(manifest)
            })();
            match result {
                Ok(manifest) => Some(manifest),
                Err(error) => {
                    eprintln!("nav-pack: {error}");
                    return ExitCode::FAILURE;
                }
            }
        }
        (None, None, None) => None,
        _ => unreachable!("parse_args keeps explicit resource inputs together"),
    };

    // Openable wall door loc ids from the Server door configs.
    let mut door_ids = HashSet::new();
    let mut config_failed = 0usize;
    for name in DOOR_CONFIGS {
        let path = doors_dir.join(name);
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
    let loc_defs = match std::fs::read(config_jag) {
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
    let mut collision = match bake_from_maps(maps_dir, &loc_defs, &door_ids) {
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

    // The bank stand table from the same content tree (every `bankbooth`
    // placement, Use-quickly op).
    let banks = derive_banks(content_root);

    // The raw baked flags ride in the sidecar; the v8 pack carries only
    // the packed walk surface (the router's resident form).
    let flags = collision
        .flags
        .take()
        .expect("bake_from_maps always stamps raw flags");
    let flags_bytes =
        encode_flags_sidecar(collision.origin, collision.width, collision.height, &flags);
    let flags_path = &inputs.flags_out;

    // The pack write: packed walk surface + transport edges + bank stands.
    let bytes = encode(&collision, &graph, &banks);
    let manifest = match (inputs.revision, verified_cache.as_ref()) {
        (Some(revision), Some(cache)) => {
            match NavManifest::capture(revision, cache, &bytes, Some(&flags_bytes)).and_then(
                |manifest| {
                    serde_json::to_vec_pretty(&manifest)
                        .map(|bytes| (manifest, bytes))
                        .map_err(|e| e.to_string())
                },
            ) {
                Ok(manifest) => Some(manifest),
                Err(error) => {
                    eprintln!("nav-pack: manifest: {error}");
                    return ExitCode::FAILURE;
                }
            }
        }
        (None, None) => None,
        _ => unreachable!("verified cache accompanies an explicit revision"),
    };
    if let Err(e) = std::fs::write(&inputs.out, &bytes) {
        eprintln!("nav-pack: write {}: {e}", inputs.out.display());
        return ExitCode::FAILURE;
    }
    if let Err(e) = std::fs::write(flags_path, &flags_bytes) {
        eprintln!("nav-pack: write {}: {e}", flags_path.display());
        return ExitCode::FAILURE;
    }
    if let Some((manifest, manifest_bytes)) = manifest {
        if let Err(e) = std::fs::write(nav_manifest_path(&inputs.out), &manifest_bytes) {
            eprintln!(
                "nav-pack: write {}: {e}",
                nav_manifest_path(&inputs.out).display()
            );
            return ExitCode::FAILURE;
        }
        eprintln!(
            "nav-pack: bound revision {} cache {} nav {} flags {}",
            manifest.revision,
            manifest.cache_id,
            manifest.nav_sha256,
            manifest.flags_sha256.as_deref().unwrap_or("none")
        );
    }
    eprintln!(
        "nav-pack: baked {} mapsquares into a {}x{} collision grid, {} walkable tiles, {} transport edges, {} bank stands -> {} bytes -> {}; {} flag bytes -> {}",
        squares_baked(maps_dir),
        collision.width,
        collision.height,
        walkable,
        graph.edges.len(),
        banks.len(),
        bytes.len(),
        inputs.out.display(),
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
    fn explicit_289_inputs_bind_content_cache_config_and_outputs() {
        let inputs = parse_args([
            "--revision",
            "289",
            "--content",
            "/fixture/289/content",
            "--cache",
            "/fixture/289/engine/data/pack/client",
            "--cache-manifest",
            "/fixture/cache-289.json",
            "--out",
            "/tmp/289.navpack",
        ])
        .unwrap();
        assert_eq!(inputs.revision, Some(289));
        assert_eq!(inputs.maps_dir, PathBuf::from("/fixture/289/content/maps"));
        assert_eq!(
            inputs.doors_dir,
            PathBuf::from("/fixture/289/content/scripts/doors/configs")
        );
        assert_eq!(
            inputs.config_jag,
            PathBuf::from("/fixture/289/engine/data/pack/client/config")
        );
        assert_eq!(
            inputs.cache_dir,
            Some(PathBuf::from("/fixture/289/engine/data/pack/client"))
        );
        assert_eq!(
            inputs.cache_manifest,
            Some(PathBuf::from("/fixture/cache-289.json"))
        );
        assert_eq!(inputs.out, PathBuf::from("/tmp/289.navpack"));
        assert_eq!(inputs.flags_out, PathBuf::from("/tmp/289.navflags"));
    }

    #[test]
    fn explicit_274_uses_server_config_next_to_client_cache() {
        let inputs = parse_args([
            "--revision",
            "274",
            "--content",
            "/fixture/274/content",
            "--cache",
            "/fixture/274/engine/data/pack/client",
            "--cache-manifest",
            "/fixture/cache-274.json",
            "--out",
            "/tmp/274.navpack",
        ])
        .unwrap();
        assert_eq!(
            inputs.config_jag,
            PathBuf::from("/fixture/274/engine/data/pack/config")
        );
    }

    #[test]
    fn explicit_mode_rejects_ambiguous_or_incomplete_inputs() {
        for args in [
            vec!["--revision", "289"],
            vec![
                "--revision",
                "377",
                "--content",
                "/content",
                "--cache",
                "/cache",
                "--cache-manifest",
                "/cache.json",
                "--out",
                "/tmp/x",
            ],
            vec![
                "--revision",
                "289",
                "--content",
                "/content",
                "--cache",
                "/cache",
                "--cache-manifest",
                "/cache.json",
                "--out",
                "/tmp/x",
                "positional",
            ],
            vec![
                "--revision",
                "274",
                "--revision",
                "289",
                "--content",
                "/content",
                "--cache",
                "/cache",
                "--cache-manifest",
                "/cache.json",
                "--out",
                "/tmp/x",
            ],
        ] {
            assert!(parse_args(args).is_err());
        }
    }

    #[test]
    fn legacy_274_positionals_remain_unmanifested() {
        let inputs = parse_args(["/legacy/maps", "/legacy/doors", "/legacy/config"]).unwrap();
        assert_eq!(inputs.revision, None);
        assert_eq!(inputs.maps_dir, PathBuf::from("/legacy/maps"));
        assert_eq!(inputs.doors_dir, PathBuf::from("/legacy/doors"));
        assert_eq!(inputs.config_jag, PathBuf::from("/legacy/config"));
        assert!(inputs.cache_dir.is_none());
        assert!(inputs.cache_manifest.is_none());
    }

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
