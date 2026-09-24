//! `nav-pack` CLI: bake the whole world — every `maps/*.jm2` mapsquare —
//! into a per-level [`WorldCollision`] (four planes like the client's
//! `collision[4]`), derive the [`TransportGraph`] from
//! the Server content, and write the v10 nav pack (magic `274V`, version
//! byte 10; `encode`) to `$NAV_PACK` or
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
//! --cache-manifest CACHE_MANIFEST --out NAV_PACK [--flags-out NAV_FLAGS]
//! [--snapshot-root SNAPSHOT_ROOT]`.
//! Runtime-compatible packs require `--snapshot-root`: the parent containing
//! the complete version-keyed decoded snapshot. Omitting it preserves the
//! legacy offline packed-identity format, which runtime binding rejects.
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
//! Server content changes — a stale v8 or older pack decodes as `BadVersion`.
//! The derivation itself lives in [`nav::bake`], shared with the ordinary
//! application build (`host-play`'s build script), which bakes and stages the
//! selected revision automatically; this CLI stays for deliberate
//! developer/custom-input bakes.

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use nav::bake::{
    self, config_jag_for, content_inputs, verify_cache_manifest, BakeRequest, BakedNav,
};
use nav::manifest::{nav_manifest_path, CacheManifest};

fn default_maps_dir() -> PathBuf {
    content_inputs(&client::bot_target::content_dir()).maps_dir
}

fn default_doors_dir() -> PathBuf {
    content_inputs(&client::bot_target::content_dir()).doors_dir
}

fn default_config_jag() -> PathBuf {
    client::bot_target::config_jag()
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
    content_dir: PathBuf,
    maps_dir: PathBuf,
    doors_dir: PathBuf,
    config_jag: PathBuf,
    cache_dir: Option<PathBuf>,
    cache_manifest: Option<PathBuf>,
    snapshot_root: Option<PathBuf>,
    out: PathBuf,
    flags_out: PathBuf,
    reach_out: PathBuf,
    canlight_out: PathBuf,
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
        let reach_out = out.with_extension("navreach");
        let canlight_out = out.with_extension("navcanlight");
        let content_dir = maps_dir
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        return Ok(BakeInputs {
            revision: None,
            content_dir,
            maps_dir,
            doors_dir,
            config_jag,
            cache_dir: None,
            cache_manifest: None,
            snapshot_root: None,
            out,
            flags_out,
            reach_out,
            canlight_out,
        });
    }

    let mut revision = None;
    let mut content = None;
    let mut cache_dir = None;
    let mut cache_manifest = None;
    let mut snapshot_root = None;
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
            "--snapshot-root" if snapshot_root.is_none() => {
                snapshot_root = Some(PathBuf::from(value))
            }
            "--out" if out.is_none() => out = Some(PathBuf::from(value)),
            "--flags-out" if explicit_flags.is_none() => {
                explicit_flags = Some(PathBuf::from(value));
            }
            "--content" | "--cache" | "--cache-manifest" | "--snapshot-root" | "--out"
            | "--flags-out" => {
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
    let inputs = content_inputs(&content);
    let config_jag = config_jag_for(revision, &cache_dir)?;
    let flags_out = explicit_flags.unwrap_or_else(|| flags_path_for(&out));
    let reach_out = out.with_extension("navreach");
    let canlight_out = out.with_extension("navcanlight");
    Ok(BakeInputs {
        revision: Some(revision),
        content_dir: content,
        maps_dir: inputs.maps_dir,
        doors_dir: inputs.doors_dir,
        config_jag,
        cache_dir: Some(cache_dir),
        cache_manifest: Some(cache_manifest),
        snapshot_root,
        out,
        flags_out,
        reach_out,
        canlight_out,
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
    let gates = content_inputs(&inputs.content_dir).gates;

    let verified_cache = match (
        inputs.revision,
        inputs.cache_dir.as_deref(),
        inputs.cache_manifest.as_deref(),
    ) {
        (Some(revision), Some(cache_dir), Some(manifest_path)) => {
            match verify_cache_manifest(revision, cache_dir, manifest_path, &inputs.config_jag) {
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

    let baked = match bake_world(&inputs, &gates, verified_cache.as_ref()) {
        Ok(baked) => baked,
        Err(error) => {
            eprintln!("nav-pack: {error}");
            return ExitCode::FAILURE;
        }
    };
    for note in &baked.notes {
        eprintln!("nav-pack: {note}");
    }
    write_outputs(&inputs, &baked)
}

fn bake_world(
    inputs: &BakeInputs,
    gates: &Path,
    cache: Option<&CacheManifest>,
) -> Result<BakedNav, String> {
    let decoded = match (
        inputs.revision,
        inputs.cache_dir.as_deref(),
        inputs.snapshot_root.as_deref(),
    ) {
        (Some(revision), Some(cache), Some(root)) => {
            Some(bake::decoded_identity(revision, cache, root)?)
        }
        _ => None,
    };
    let mut baked = bake::bake_world(&BakeRequest {
        revision: inputs.revision,
        maps_dir: &inputs.maps_dir,
        doors_dir: &inputs.doors_dir,
        gates,
        config_jag: &inputs.config_jag,
        cache,
        require_all_door_configs: false,
    })?;
    if let (Some(revision), Some(cache_dir), Some(root), Some(id)) = (
        inputs.revision,
        inputs.cache_dir.as_deref(),
        inputs.snapshot_root.as_deref(),
        decoded.as_deref(),
    ) {
        if bake::decoded_identity(revision, cache_dir, root)? != id {
            return Err("cache changed during navigation bake".into());
        }
        baked.manifest.as_mut().expect("bound bake").content_id = Some(id.into());
    }
    Ok(baked)
}

fn write_outputs(inputs: &BakeInputs, baked: &BakedNav) -> ExitCode {
    if let Err(e) = std::fs::write(&inputs.out, &baked.pack) {
        eprintln!("nav-pack: write {}: {e}", inputs.out.display());
        return ExitCode::FAILURE;
    }
    if let Err(e) = std::fs::write(&inputs.flags_out, &baked.flags) {
        eprintln!("nav-pack: write {}: {e}", inputs.flags_out.display());
        return ExitCode::FAILURE;
    }
    if let Err(e) = std::fs::write(&inputs.reach_out, &baked.reach) {
        eprintln!("nav-pack: write {}: {e}", inputs.reach_out.display());
        return ExitCode::FAILURE;
    }
    if let Err(e) = std::fs::write(&inputs.canlight_out, &baked.canlight) {
        eprintln!("nav-pack: write {}: {e}", inputs.canlight_out.display());
        return ExitCode::FAILURE;
    }
    if let Some(manifest) = &baked.manifest {
        let manifest_path = nav_manifest_path(&inputs.out);
        let bytes = match serde_json::to_vec_pretty(manifest) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("nav-pack: manifest: {e}");
                return ExitCode::FAILURE;
            }
        };
        if let Err(e) = std::fs::write(&manifest_path, bytes) {
            eprintln!("nav-pack: write {}: {e}", manifest_path.display());
            return ExitCode::FAILURE;
        }
        eprintln!(
            "nav-pack: bound revision {} cache {} nav {} flags {} reach {} canlight {}",
            manifest.revision,
            manifest.cache_id,
            manifest.nav_sha256,
            manifest.flags_sha256.as_deref().unwrap_or("none"),
            manifest.reach_sha256.as_deref().unwrap_or("none"),
            manifest.canlight_sha256.as_deref().unwrap_or("none")
        );
    }
    let summary = baked.summary;
    eprintln!(
        "nav-pack: baked {} mapsquares into a {}x{} collision grid, {} walkable tiles, {} transport edges, {} bank stands -> {} bytes -> {}; {} flag bytes -> {}; {} reach bytes -> {}; {} canlight bytes -> {}",
        summary.mapsquares,
        summary.width,
        summary.height,
        summary.walkable,
        summary.edges,
        summary.banks,
        baked.pack.len(),
        inputs.out.display(),
        baked.flags.len(),
        inputs.flags_out.display(),
        baked.reach.len(),
        inputs.reach_out.display(),
        baked.canlight.len(),
        inputs.canlight_out.display()
    );
    ExitCode::SUCCESS
}

#[cfg(test)]
#[path = "nav-pack/nav_pack_tests.rs"]
mod tests;
