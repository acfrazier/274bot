#![deny(clippy::disallowed_types)]
#![deny(clippy::disallowed_methods)]
#![deny(clippy::disallowed_macros)]

//! `nav-pack` CLI: bake local Server content and client config into the v19 nav
//! pack and its local sidecars. Revision selection chooses the game-data
//! schema; it does not authenticate local content or cache files.
//!
//! Bake with explicit inputs:
//! `nav-pack --revision 274|289|377 --content CONTENT_DIR --cache CACHE_DIR
//! --game-data-dir GAME_DATA_DIR --out NAV_PACK [--flags-out NAV_FLAGS]`.
//! Revision 274 reads the server `data/pack/config` beside the `client/`
//! cache directory. Revisions 289 and 377 read `config` inside the cache directory.
//! Door loc ids come from the `*.loc` door configs plus
//! `scripts/general_use/configs/gates.loc` under CONTENT_DIR; loc definitions
//! (blockwalk, width/length, active) come from the selected config jag.
//! Every `.jm2` under `content/maps` bakes or the run fails; non-`.jm2`
//! files (`ignore.csv`/`free2play.csv`) are metadata and skipped. The
//! transport graph and bank stands derive from local Server content.
//! Rebake whenever Server content changes — older packs fail with
//! `BadVersion`. The derivation lives in [`nav_bake::bake`], shared with the
//! ordinary application build. Export crossing facts with
//! `nav-pack --export-crossings PACK --out JSON`.
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use api::prereq_steps::CrossingId;
use api::selected::QuestGate;
use nav_bake::bake::{self, config_jag_for, content_inputs, BakeRequest, BakedNav};
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
        schema: 3,
        crossings,
        identical_edge_groups: identical_edge_groups(graph)?,
    };
    serde_json::to_string_pretty(&export)
        .map_err(|error| format!("serialize crossing export: {error}"))
}

fn crossing_row_source(index: usize, edge_count: usize) -> String {
    if index < edge_count {
        format!("graph.edges[{index}]")
    } else {
        format!("graph.teleports[{}]", index - edge_count)
    }
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

fn export_crossings(inputs: &ExportInputs) -> Result<(), String> {
    let pack_bytes = std::fs::read(&inputs.pack)
        .map_err(|error| format!("read {}: {error}", inputs.pack.display()))?;
    let (mut collision, graph, _) = nav_planner::pack::decode(&pack_bytes)
        .map_err(|error| format!("decode {}: {error}", inputs.pack.display()))?;
    let flags_path = flags_path_for(&inputs.pack);
    let flags = nav_planner::pack::read_flags_sidecar(&flags_path)
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

    let json = crossing_export_json(&collision, &graph)?;
    std::fs::write(&inputs.out, json)
        .map_err(|error| format!("write {}: {error}", inputs.out.display()))
}

fn flags_path_for(out: &Path) -> PathBuf {
    out.with_extension("navflags")
}

#[derive(Debug)]
struct BakeInputs {
    revision: u16,
    content_dir: PathBuf,
    maps_dir: PathBuf,
    doors_dir: PathBuf,
    config_jag: PathBuf,
    game_data_dir: PathBuf,
    out: PathBuf,
    flags_out: PathBuf,
    reach_out: PathBuf,
    canlight_out: PathBuf,
    pois_out: PathBuf,
}
fn parse_args(args: impl IntoIterator<Item = impl AsRef<str>>) -> Result<BakeInputs, String> {
    let args: Vec<String> = args.into_iter().map(|s| s.as_ref().to_string()).collect();
    let mut revision = None;
    let mut content = None;
    let mut cache_dir = None;
    let mut game_data_dir = None;
    let mut out = None;
    let mut explicit_flags = None;
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        if !flag.starts_with("--") {
            return Err(format!(
                "unexpected positional argument {flag:?}; use named nav-pack inputs"
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
                    .map_err(|_| "--revision must be 274, 289 or 377".to_string())?;
                if !matches!(parsed, 274 | 289 | 377) {
                    return Err("--revision must be 274, 289 or 377".into());
                }
                revision = Some(parsed);
            }
            "--content" if content.is_none() => content = Some(PathBuf::from(value)),
            "--cache" if cache_dir.is_none() => cache_dir = Some(PathBuf::from(value)),
            "--game-data-dir" if game_data_dir.is_none() => {
                game_data_dir = Some(PathBuf::from(value))
            }
            "--out" if out.is_none() => out = Some(PathBuf::from(value)),
            "--flags-out" if explicit_flags.is_none() => {
                explicit_flags = Some(PathBuf::from(value));
            }
            "--content" | "--cache" | "--game-data-dir" | "--out" | "--flags-out" => {
                return Err(format!("duplicate explicit option {flag}"));
            }
            _ => return Err(format!("unknown explicit option {flag}")),
        }
    }
    let revision =
        revision.ok_or_else(|| "explicit mode requires --revision 274|289|377".to_string())?;
    let content =
        content.ok_or_else(|| "explicit mode requires --content CONTENT_DIR".to_string())?;
    let cache_dir =
        cache_dir.ok_or_else(|| "explicit mode requires --cache CACHE_DIR".to_string())?;
    let game_data_dir = game_data_dir
        .ok_or_else(|| "explicit mode requires --game-data-dir GAME_DATA_DIR".to_string())?;
    let out = out.ok_or_else(|| "explicit mode requires --out NAV_PACK".to_string())?;
    let inputs = content_inputs(&content);
    let config_jag = config_jag_for(revision, &cache_dir).map_err(|error| error.to_string())?;
    let flags_out = explicit_flags.unwrap_or_else(|| flags_path_for(&out));
    let reach_out = out.with_extension("navreach");
    let canlight_out = out.with_extension("navcanlight");
    let pois_out = out.with_extension("navpois");
    Ok(BakeInputs {
        revision,
        content_dir: content,
        maps_dir: inputs.maps_dir,
        doors_dir: inputs.doors_dir,
        config_jag,
        game_data_dir,
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

    let baked = match bake_world(&inputs, &gates) {
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

fn bake_world(inputs: &BakeInputs, gates: &Path) -> Result<BakedNav, nav_bake::GenerationError> {
    bake::bake_world(&BakeRequest {
        revision: Some(inputs.revision),
        game_data_dir: &inputs.game_data_dir,
        maps_dir: &inputs.maps_dir,
        doors_dir: &inputs.doors_dir,
        gates,
        config_jag: &inputs.config_jag,
        require_all_door_configs: true,
    })
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
