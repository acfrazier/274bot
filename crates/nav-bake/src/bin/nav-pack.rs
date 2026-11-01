#![deny(clippy::disallowed_types)]
#![deny(clippy::disallowed_methods)]
#![deny(clippy::disallowed_macros)]

//! `nav-pack` CLI: bake the whole world — every `maps/*.jm2` mapsquare —
//! into a per-level [`WorldCollision`] (four planes like the client's
//! `collision[4]`), derive the [`TransportGraph`] from
//! the Server content, and write the v17 nav pack (magic `274V`, version
//! byte 17; `encode`) to `$NAV_PACK` or
//! `~/.274bot/274bot.navpack` (default), plus the raw flags sidecar
//! (magic `274F`; `encode_flags_sidecar`) to `$NAV_FLAGS` or the pack
//! path with its extension swapped to `.navflags` (default
//! `~/.274bot/274bot.navflags`). Legacy 274 usage remains:
//! `nav-pack [MAPS_DIR] [DOORS_CONFIG_DIR] [CONFIG_JAG]`, where omitted paths
//! resolve from `BOT_NAV_ENGINE_DIR` or `ENGINE_DIR`: the engine's sibling
//! `content/maps`, matching doors configs under that content tree, and
//! `<engine>/data/pack/config`. Pass all three paths to use a different tree.
//! Revision-bound bakes use explicit inputs:
//! `nav-pack --revision 274|289 --content CONTENT_DIR --cache CACHE_DIR
//! --cache-manifest CACHE_MANIFEST --out NAV_PACK [--flags-out NAV_FLAGS]
//! [--snapshot-root SNAPSHOT_ROOT]`.
//! Runtime-compatible packs require `--snapshot-root`: the parent containing
//! the complete version-keyed decoded snapshot. Omitting it preserves the
//! legacy offline packed-identity format, which runtime binding rejects.
//! This mode verifies every cache archive and the selected config before
//! baking, then emits `<NAV_PACK>.json` using [`nav_planner::manifest::NavManifest`].
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
//! ([`nav_bake::pack::derive_banks`]) bakes from the same content tree: every
//! `bankbooth` loc placement with the Use-quickly op, plus NPC tellers
//! (`category=bank_teller`) from jm2 NPC placements. Rebake whenever the
//! Server content changes — packs older than v17 are rejected with `BadVersion`.
//! The derivation itself lives in [`nav_bake::bake`], shared with the ordinary
//! application build (`host-play`'s build script), which bakes and stages the
//! selected revision automatically; this CLI stays for deliberate
//! developer/custom-input bakes. Export all pack crossings with
//! `nav-pack --export-crossings PACK --out JSON`.
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use api::prereq_steps::CrossingId;
use api::selected::QuestGate;
use nav_bake::bake::{
    self, config_jag_for, content_inputs, verify_cache_manifest, BakeRequest, BakedNav,
};
use nav_planner::manifest::{nav_manifest_path, CacheManifest};
use nav_planner::transport::TransportGraph;

/// Input paths for the standalone crossing projection mode.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ExportInputs {
    pack: PathBuf,
    out: PathBuf,
}

fn parse_export_args(
    args: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<ExportInputs, String> {
    let args: Vec<String> = args.into_iter().map(|s| s.as_ref().to_string()).collect();
    let mut pack = None;
    let mut out = None;
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        if !flag.starts_with("--") {
            return Err(format!(
                "unexpected positional argument {flag:?} in crossing-export mode"
            ));
        }
        let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
        if value.starts_with("--") || value.is_empty() {
            return Err(format!("{flag} needs a value"));
        }
        match flag.as_str() {
            "--export-crossings" if pack.is_none() => pack = Some(PathBuf::from(value)),
            "--out" if out.is_none() => out = Some(PathBuf::from(value)),
            "--export-crossings" | "--out" => {
                return Err(format!("duplicate crossing-export option {flag}"));
            }
            _ => return Err(format!("unknown crossing-export option {flag}")),
        }
    }
    Ok(ExportInputs {
        pack: pack
            .ok_or_else(|| "crossing-export mode requires --export-crossings PACK".to_owned())?,
        out: out.ok_or_else(|| "crossing-export mode requires --out JSON".to_owned())?,
    })
}

#[derive(serde::Serialize)]
struct CrossingExport<'a> {
    schema: u8,
    navpack_sha256: String,
    crossings: Vec<CrossingExportRow<'a>>,
    identical_edge_groups: Vec<Vec<String>>,
}

#[derive(serde::Serialize)]
struct CrossingExportRow<'a> {
    crossing: CrossingId,
    standable: bool,
    admissible: bool,
    quest_gate_alternatives: Vec<&'a [QuestGate]>,
}

fn crossing_export_json(
    pack_bytes: &[u8],
    collision: &nav_planner::collision::WorldCollision,
    graph: &TransportGraph,
) -> Result<String, String> {
    let edge_count = graph.edges.len();
    let mut crossings = Vec::with_capacity(edge_count + graph.teleports.len());
    let mut crossing_index = HashMap::with_capacity(edge_count + graph.teleports.len());
    for (index, edge) in graph.edges.iter().chain(&graph.teleports).enumerate() {
        let source = crossing_row_source(index, edge_count);
        let crossing = edge.crossing_id().ok_or_else(|| {
            format!(
                "{source} has no representable CrossingId (loc_id {}, option {})",
                edge.loc_id, edge.option
            )
        })?;
        let standable = collision.standable(crossing.takeoff);
        // Any-tile teleports are not stored in `graph.edges` and have no
        // loc-approach predicate. Their exact candidate tile is admissible
        // whenever it is standable; normal crossings use the shared graph
        // predicate below.
        let admissible = if index < edge_count {
            graph.admissible_from(collision, index, crossing.takeoff)
        } else {
            standable
        };
        let gates = edge
            .quest_gates
            .as_ref()
            .map_or(&[][..], |quest_gates| quest_gates.gates());
        match crossing_index.entry(crossing) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(crossings.len());
                crossings.push(CrossingExportRow {
                    crossing,
                    standable,
                    admissible,
                    quest_gate_alternatives: vec![gates],
                });
            }
            std::collections::hash_map::Entry::Occupied(entry) => {
                let row = &mut crossings[*entry.get()];
                // A physical crossing is admissible when any represented
                // source edge admits its exact takeoff.
                row.admissible |= admissible;
                if !row.quest_gate_alternatives.contains(&gates) {
                    row.quest_gate_alternatives.push(gates);
                }
            }
        }
    }

    let export = CrossingExport {
        schema: 2,
        navpack_sha256: nav_planner::manifest::hash_bytes(pack_bytes),
        crossings,
        identical_edge_groups: identical_edge_groups(graph)?,
    };
    serde_json::to_string_pretty(&export)
        .map_err(|error| format!("serialize crossing export: {error}"))
}

fn identical_edge_groups(graph: &TransportGraph) -> Result<Vec<Vec<String>>, String> {
    struct CandidateGroup {
        first_index: usize,
        members: Option<Vec<usize>>,
    }

    let edge_count = graph.edges.len();
    let edges: Vec<_> = graph.edges.iter().chain(&graph.teleports).collect();
    let mut groups_by_crossing: HashMap<CrossingId, Vec<usize>> = HashMap::new();
    let mut groups = Vec::<CandidateGroup>::new();

    for (index, edge) in edges.iter().enumerate() {
        let crossing = edge.crossing_id().ok_or_else(|| {
            let source = crossing_row_source(index, edge_count);
            format!(
                "{source} has no representable CrossingId (loc_id {}, option {})",
                edge.loc_id, edge.option
            )
        })?;
        let matching_group = groups_by_crossing.get(&crossing).and_then(|candidates| {
            candidates
                .iter()
                .copied()
                .find(|candidate| edges[groups[*candidate].first_index] == *edge)
        });
        if let Some(group_index) = matching_group {
            let group = &mut groups[group_index];
            match &mut group.members {
                Some(members) => members.push(index),
                None => group.members = Some(vec![group.first_index, index]),
            }
        } else {
            let group_index = groups.len();
            groups.push(CandidateGroup {
                first_index: index,
                members: None,
            });
            groups_by_crossing
                .entry(crossing)
                .or_default()
                .push(group_index);
        }
    }

    Ok(groups
        .into_iter()
        .filter_map(|group| {
            group.members.map(|members| {
                members
                    .into_iter()
                    .map(|index| crossing_row_source(index, edge_count))
                    .collect()
            })
        })
        .collect())
}

fn crossing_row_source(index: usize, edge_count: usize) -> String {
    if index < edge_count {
        format!("graph.edges[{index}]")
    } else {
        format!("graph.teleports[{}]", index - edge_count)
    }
}

fn export_crossings(inputs: &ExportInputs) -> Result<(), String> {
    let pack_bytes = std::fs::read(&inputs.pack)
        .map_err(|error| format!("read {}: {error}", inputs.pack.display()))?;
    let (mut collision, graph, _) = nav_planner::pack::decode(&pack_bytes)
        .map_err(|error| format!("decode {}: {error}", inputs.pack.display()))?;
    let flags_path = flags_path_for(&inputs.pack);
    let flags = nav_planner::pack::read_flags_sidecar(&flags_path, false)
        .map_err(|error| format!("read flags sidecar {}: {error}", flags_path.display()))?;
    if flags.origin != collision.origin
        || flags.width != collision.width
        || flags.height != collision.height
    {
        return Err(format!(
            "flags sidecar {} grid {:?} {}x{} does not match pack grid {:?} {}x{}",
            flags_path.display(),
            flags.origin,
            flags.width,
            flags.height,
            collision.origin,
            collision.width,
            collision.height
        ));
    }
    let expected_flag_count = collision
        .width
        .checked_mul(collision.height)
        .and_then(|cells| cells.checked_mul(nav_planner::collision::LEVELS))
        .ok_or_else(|| "pack collision dimensions overflow the flags sidecar size".to_owned())?;
    if flags.flags.len() != expected_flag_count {
        return Err(format!(
            "flags sidecar {} has {} words; pack grid requires {expected_flag_count}",
            flags_path.display(),
            flags.flags.len()
        ));
    }
    let raw_flags = std::sync::Arc::try_unwrap(flags.flags).map_err(|_| {
        format!(
            "flags sidecar {} is unexpectedly shared",
            flags_path.display()
        )
    })?;
    collision.attach_flags(raw_flags);

    let json = crossing_export_json(&pack_bytes, &collision, &graph)?;
    std::fs::write(&inputs.out, json)
        .map_err(|error| format!("write {}: {error}", inputs.out.display()))
}

fn legacy_default_paths(
    engine_dir: Option<PathBuf>,
) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    let engine_dir = engine_dir.ok_or_else(|| {
        "legacy positional defaults require BOT_NAV_ENGINE_DIR or ENGINE_DIR; pass all three input paths to avoid defaults".to_owned()
    })?;
    let content_dir = engine_dir
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("content");
    let inputs = content_inputs(&content_dir);
    Ok((
        inputs.maps_dir,
        inputs.doors_dir,
        engine_dir.join("data/pack/config"),
    ))
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
    pois_out: PathBuf,
}

fn parse_args(args: impl IntoIterator<Item = impl AsRef<str>>) -> Result<BakeInputs, String> {
    let args: Vec<String> = args.into_iter().map(|s| s.as_ref().to_string()).collect();
    if !args.iter().any(|arg| arg.starts_with("--")) {
        if args.len() > 3 {
            return Err("usage: nav-pack [MAPS_DIR [DOORS_CONFIG_DIR [CONFIG_JAG]]]".into());
        }
        let defaults = if args.len() < 3 {
            let engine_dir = env::var_os("BOT_NAV_ENGINE_DIR")
                .or_else(|| env::var_os("ENGINE_DIR"))
                .map(PathBuf::from);
            Some(legacy_default_paths(engine_dir)?)
        } else {
            None
        };
        let maps_dir = args
            .first()
            .map(PathBuf::from)
            .or_else(|| defaults.as_ref().map(|(maps_dir, _, _)| maps_dir.clone()))
            .ok_or_else(|| "missing maps path; pass it explicitly".to_owned())?;
        let doors_dir = args
            .get(1)
            .map(PathBuf::from)
            .or_else(|| defaults.as_ref().map(|(_, doors_dir, _)| doors_dir.clone()))
            .ok_or_else(|| "missing doors path; pass it explicitly".to_owned())?;
        let config_jag = args
            .get(2)
            .map(PathBuf::from)
            .or_else(|| {
                defaults
                    .as_ref()
                    .map(|(_, _, config_jag)| config_jag.clone())
            })
            .ok_or_else(|| "missing config cache path; pass it explicitly".to_owned())?;
        let out = env::var("NAV_PACK")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_out());
        let flags_out = flags_out(&out);
        let reach_out = out.with_extension("navreach");
        let canlight_out = out.with_extension("navcanlight");
        let pois_out = out.with_extension("navpois");
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
            pois_out,
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
    let pois_out = out.with_extension("navpois");
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
        pois_out,
    })
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--export-crossings") {
        let inputs = match parse_export_args(&args) {
            Ok(inputs) => inputs,
            Err(error) => {
                eprintln!("nav-pack: {error}");
                return ExitCode::FAILURE;
            }
        };
        return match export_crossings(&inputs) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("nav-pack: {error}");
                ExitCode::FAILURE
            }
        };
    }
    let inputs = match parse_args(args) {
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
        require_all_door_configs: true,
        input_fingerprints: None,
        content_id: decoded.as_deref(),
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
    if let Some(pois) = &baked.pois {
        if let Err(e) = std::fs::write(&inputs.pois_out, pois) {
            eprintln!("nav-pack: write {}: {e}", inputs.pois_out.display());
            return ExitCode::FAILURE;
        }
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
            "nav-pack: bound revision {} cache {} nav {} flags {} reach {} canlight {} pois {}",
            manifest.revision,
            manifest.cache_id,
            manifest.nav_sha256,
            manifest.flags_sha256.as_deref().unwrap_or("none"),
            manifest.reach_sha256.as_deref().unwrap_or("none"),
            manifest.canlight_sha256.as_deref().unwrap_or("none"),
            manifest.pois_sha256.as_deref().unwrap_or("none")
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
