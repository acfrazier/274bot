//! Frozen-route and end-to-end P1-C corpus exporter.
//!
//! This is deliberately ignored by ordinary test runs. It uses only the
//! pre-P1-C public surface, so the identical harness runs against approved
//! P1-B and P1-C binaries; model identity is detected from RouteAssessment's
//! Debug representation rather than selected with a product flag.

use api::condition::PoisonState;
use api::selected::ClientRevision;
use api::snapshot::WorldTile;
use host_play::admission::{self, Admission, RiskInput, RiskPolicy};
use host_play::WalkGlobals;
use nav_planner::router::{
    find_blocking_zones, find_first_with, find_many_with, find_with_avoid, FindOptions, Leg, Route,
    RouteError,
};
use nav_planner::world::NavWorld;
use nav_planner::world_state::WorldState;
use nav_planner::zones::{ZoneExempt, ZoneKey};
use script::combat::risk::{self, RiskTables, Style};
use script::combat::CombatTables;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "survivable_corpus/qualification.rs"]
mod qualification;
use qualification::GuideQualificationFixture;

const DEFAULT_PACK: &str =
    "/Volumes/dev-scratch/274bot-evidence/NAV-GATED-ENTRANCES/reference-v17/274bot-289-v17.navpack";
const DEFAULT_QUERY: &str = "/Volumes/dev-scratch/274bot-evidence/SEED-CALC/queries.jsonl";
const DEFAULT_GUIDE_25: &str = "/Volumes/dev-scratch-2/274bot-worktrees/survivable-accuracy-src";
const DEFAULT_GUIDE_67: &str = "/Volumes/dev-scratch-2/274bot-worktrees/quest-paths";
const DEFAULT_OUTPUT: &str = "/Volumes/dev-scratch/274bot-evidence/SURVIVABLE-P1C/corpus";
const GUIDE_25_PIN: &str = "fcc930b755794f0c7d02d024a2c450731a16ce14";
const GUIDE_67_PIN_PREFIX: &str = "764a49853";

#[derive(Clone)]
struct Loadout {
    id: &'static str,
    attack: u8,
    strength: u8,
    defence: u8,
    hp: u8,
    prayer: u8,
    aliases: &'static [&'static str],
    lobster_count: u8,
    map_members: Option<bool>,
    hp_variants: &'static [(&'static str, u8)],
}

#[derive(Clone)]
struct FrozenRoute {
    id: String,
    target: WorldTile,
    route: Option<Route>,
    search_error: Option<String>,
    route_source: String,
}

#[derive(Clone)]
struct CorpusInput {
    id: String,
    source: Value,
    kind: String,
    raw: Value,
    from: Option<WorldTile>,
    targets: Vec<WorldTile>,
    state: WorldState,
    state_json: Value,
    route_options: FindOptions,
    grant_names: Vec<String>,
    frozen: Vec<FrozenRoute>,
    witness_target: Option<WorldTile>,
    status_without_route: Option<String>,
    provenance: Value,
}

#[test]
#[ignore = "explicit evidence export; run on both approved P1-B and P1-C worktrees"]
fn export_survivable_p1c_corpus_and_diff() {
    if !api::test_support::game_data_available() {
        return;
    }
    export().unwrap_or_else(|error| panic!("survivability corpus export failed: {error}"));
}

fn export() -> Result<(), String> {
    let output = env_path("SURVIVABLE_CORPUS_OUT_DIR", DEFAULT_OUTPUT);
    let Some(pack_path) = api::test_support::local_asset_path(
        "NAV_PACK",
        DEFAULT_PACK,
        "real navigation pack for the survivability corpus",
    ) else {
        return Ok(());
    };
    if api::test_support::local_asset_path(
        "",
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../api/data/game-data/289.json"
        ),
        "revision 289 game data for the survivability corpus",
    )
    .is_none()
    {
        return Ok(());
    }
    let Some(query_path) = api::test_support::local_asset_path(
        "SURVIVABLE_QUERY_FILE",
        DEFAULT_QUERY,
        "real survivability query corpus",
    ) else {
        return Ok(());
    };
    let Some(guide_25_root) = api::test_support::local_asset_path(
        "SURVIVABLE_GUIDE_25_ROOT",
        DEFAULT_GUIDE_25,
        "real 25-guide source checkout",
    ) else {
        return Ok(());
    };
    let Some(guide_67_root) = api::test_support::local_asset_path(
        "SURVIVABLE_GUIDE_67_ROOT",
        DEFAULT_GUIDE_67,
        "real 67-guide source checkout",
    ) else {
        return Ok(());
    };
    let Some(design_r2_path) = api::test_support::local_asset_path(
        "",
        "/Volumes/dev-scratch/274bot-evidence/SURVIVABLE-ACCURACY/design-r2.md",
        "survivability design R2 document",
    ) else {
        return Ok(());
    };
    let Some(design_r4_path) = api::test_support::local_asset_path(
        "",
        "/Volumes/dev-scratch/274bot-evidence/SURVIVABLE-ACCURACY/design-r4.md",
        "survivability design R4 document",
    ) else {
        return Ok(());
    };
    fs::create_dir_all(&output).map_err(|e| format!("create output {}: {e}", output.display()))?;

    let selected = api::game_data::for_revision(ClientRevision::R289)
        .map_err(|e| format!("selected R289 game data: {e}"))?;
    let combat = CombatTables::build(std::sync::Arc::clone(&selected))
        .map_err(|e| format!("build R289 combat tables: {e:?}"))?;
    let risks = RiskTables::build(&combat);
    let mut world = NavWorld::load_pack(&pack_path)
        .map_err(|e| format!("load selected navigation pack: {e:?}"))?;
    world
        .bind_named_bank_facts(&selected)
        .map_err(|e| format!("bind named bank facts: {e:?}"))?;
    if world.graph.zones.is_none() {
        return Err("selected v17 pack has no zone table".into());
    }

    let worktree = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let worktree = fs::canonicalize(&worktree)
        .map_err(|e| format!("canonicalize worktree {}: {e}", worktree.display()))?;
    let product_commit = git_head(&worktree)?;
    let w4_path = worktree.join("crates/nav-planner/examples/perf_corpus.rs");
    let path_schema_path = worktree.join("crates/script/tests/path_schema.rs");
    let w4_source = source_file("w4_perf_corpus", &w4_path)?;
    let path_schema_source = source_file("script_path_schema", &path_schema_path)?;
    let source_roots = load_guide_roots(guide_25_root, guide_67_root)?;
    let query_bytes = fs::read(&query_path)
        .map_err(|e| format!("read calculator query file {}: {e}", query_path.display()))?;
    let query_sha = sha256(&query_bytes);
    let query_source = json!({
        "path": query_path.display().to_string(),
        "sha256": query_sha,
        "line_count": query_bytes.split(|b| *b == b'\n').filter(|line| !line.is_empty()).count(),
    });

    let loadouts = loadouts(&selected)?;
    let mut inputs = Vec::new();
    let zones = world.graph.zones.take();
    let w4 = w4_inputs(&world, &w4_source);
    world.graph.zones = zones;
    inputs.extend(w4?);
    inputs.extend(query_inputs(
        &world,
        &query_path,
        &query_bytes,
        &query_source,
    )?);
    inputs.extend(live_inputs(&world, &path_schema_source)?);
    inputs.extend(path_schema_probes(
        &world,
        &source_roots,
        &path_schema_source,
    )?);
    inputs.extend(guide_inputs(&world, &source_roots, &selected)?);
    write_qualification_fixture(&output, &inputs)?;

    let mut records = Vec::new();
    let mut source_rows: BTreeMap<String, usize> = BTreeMap::new();
    let mut loadout_rows: BTreeMap<String, usize> = BTreeMap::new();
    for input in &inputs {
        for loadout in &loadouts {
            let variants = profile_variants(loadout, &input.id);
            let record = export_record(
                input, loadout, &variants, &world, &combat, &risks, &selected,
            );
            *source_rows
                .entry(input.source["set"].as_str().unwrap_or("unknown").to_owned())
                .or_default() += 1;
            *loadout_rows.entry(loadout.id.to_owned()).or_default() += 1;
            records.push(record);
        }
    }

    let model = detect_model(&records)?;
    let guide_coverage = guide_coverage(&inputs);
    let corpus_signature = input_signature(&records)?;
    let artifact = json!({
        "schema": "survivable-p1c-corpus-v1",
        "model": model,
        "model_detection": "RouteAssessment Debug contains top-level bound: P1-C probability; otherwise approved P1-B worst replay",
        "product": {
            "worktree": worktree.display().to_string(),
            "commit": product_commit,
            "test_source": worktree.join("crates/host-play/tests/survivable_corpus.rs").display().to_string(),
            "feature_flags": "ordinary host-play integration-test features; no model-selection product flag",
        },
        "selected_data": {
            "client_revision": selected.revision(),
            "combat_facts_version": selected.npc_names().map(|rows| rows.combat_facts_version),
        },
        "selected_navigation": {
            "path": pack_path.display().to_string(),
            "pack_version": "reference-v17",
        },
        "sources": {
            "w4": w4_source,
            "path_schema": path_schema_source,
            "query_file": query_source,
            "guides": source_roots,
            "design_r2": source_file("design_r2", &design_r2_path)?,
            "design_r4": source_file("design_r4", &design_r4_path)?,
        },
        "retained_layout_sizes_bytes": {
            "script::combat::risk::RouteAssessment": std::mem::size_of::<script::combat::risk::RouteAssessment>(),
            "host_play::admission::RiskInput": std::mem::size_of::<RiskInput>(),
            "api::condition::BotCondition": std::mem::size_of::<api::condition::BotCondition>(),
        },
        "retained_layout_target": {
            "architecture": std::env::consts::ARCH,
            "operating_system": std::env::consts::OS,
            "pointer_width_bits": usize::BITS,
        },
        "risk_policy": { "policy": "Inherit", "enforce": true, "runtime_settings_written": false },
        "loadouts": loadouts.iter().map(loadout_json).collect::<Vec<_>>(),
        "corpus": {
            "input_count": inputs.len(),
            "record_count": records.len(),
            "rows_by_source": source_rows,
            "rows_by_loadout": loadout_rows,
            "guide_coverage": guide_coverage,
            "underspecified_steps_retained": records.iter().filter(|record| {
                record["route_status"].as_str().is_some_and(|status| status.starts_with("underspecified"))
            }).count(),
            "unreachable_rows_retained": records.iter().filter(|record| {
                record["route_status"].as_str() == Some("unreachable")
            }).count(),
            "fight_queries_retained_without_route_claim": records.iter().filter(|record| {
                record["route_status"].as_str() == Some("non_route_fight_query_retained")
            }).count(),
        },
        "input_signature_sha256": corpus_signature,
        "input_signature_basis": "materialized inputs and source content identity; content-addressed source paths remain in records but do not distinguish private clones",
        "records": records,
    });
    let (file_name, model) = match model.as_str() {
        "p1b_worst" => ("p1b-worst.json", "p1b_worst"),
        "p1c_probability" => ("p1c-probability.json", "p1c_probability"),
        other => return Err(format!("unknown model label {other}")),
    };
    write_json(&output.join(file_name), &artifact)?;
    if model == "p1c_probability" {
        let old_path = output.join("p1b-worst.json");
        let Some(old_path) = api::test_support::local_asset_path(
            "",
            &old_path.to_string_lossy(),
            "approved P1-B survivability corpus",
        ) else {
            return Ok(());
        };
        let old: Value = serde_json::from_slice(
            &fs::read(&old_path)
                .map_err(|e| format!("read approved P1-B corpus {}: {e}", old_path.display()))?,
        )
        .map_err(|e| format!("parse approved P1-B corpus: {e}"))?;
        if old["model"] != "p1b_worst" {
            return Err("comparison input is not the approved P1-B worst-model export".into());
        }
        let diff = diff_artifacts(&old, &artifact)?;
        write_json(&output.join("diff.json"), &diff)?;
    }
    println!(
        "survivability corpus model={model} records={} source_rows={source_rows:?} output={}",
        artifact["records"].as_array().map_or(0, Vec::len),
        output.display()
    );
    Ok(())
}

fn env_path(name: &str, fallback: &str) -> PathBuf {
    std::env::var_os(name).map_or_else(|| PathBuf::from(fallback), PathBuf::from)
}

fn git_head(root: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|e| format!("git rev-parse {}: {e}", root.display()))?;
    if !output.status.success() {
        return Err(format!("git rev-parse {} failed", root.display()));
    }
    String::from_utf8(output.stdout)
        .map(|text| text.trim().to_owned())
        .map_err(|e| format!("git head is not UTF-8: {e}"))
}

fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 15)]));
    }
    out
}

fn source_file(label: &str, path: &Path) -> Result<Value, String> {
    let bytes = fs::read(path).map_err(|e| format!("read {label} {}: {e}", path.display()))?;
    Ok(json!({
        "label": label,
        "path": path.display().to_string(),
        "sha256": sha256(&bytes),
        "bytes": bytes.len(),
    }))
}

fn load_guide_roots(guide_25_root: PathBuf, guide_67_root: PathBuf) -> Result<Value, String> {
    let roots = [
        ("guide_25", guide_25_root, GUIDE_25_PIN),
        ("guide_67", guide_67_root, GUIDE_67_PIN_PREFIX),
    ];
    let mut result = Map::new();
    for (name, root, expected_pin) in roots {
        let actual = git_head(&root)?;
        if !actual.starts_with(expected_pin) {
            return Err(format!(
                "{name} root is not pinned input {expected_pin}: {}",
                actual
            ));
        }
        let directory = root.join("crates/script/paths/289");
        let index_path = directory.join("index.json");
        let index_bytes = fs::read(&index_path)
            .map_err(|e| format!("read guide index {}: {e}", index_path.display()))?;
        let index: Value = serde_json::from_slice(&index_bytes)
            .map_err(|e| format!("parse guide index {}: {e}", index_path.display()))?;
        let paths = index["paths"]
            .as_array()
            .ok_or_else(|| format!("{} has no paths array", index_path.display()))?;
        let expected_files = if name == "guide_25" { 25 } else { 67 };
        let file_entries = paths
            .iter()
            .filter(|entry| entry["file"].is_string())
            .count();
        let expected_entries = expected_files + 1;
        let unavailable_entries = paths
            .iter()
            .filter(|entry| entry["unavailable"].is_string())
            .count();
        let malformed_entries = paths
            .iter()
            .filter(|entry| entry["file"].is_string() == entry["unavailable"].is_string())
            .count();
        if file_entries != expected_files
            || paths.len() != expected_entries
            || unavailable_entries != 1
            || malformed_entries != 0
        {
            return Err(format!(
                "{name} expected {expected_files} files and one explicitly unavailable indexed entry, found {} entries, {file_entries} files, {unavailable_entries} unavailable, {malformed_entries} malformed",
                paths.len()
            ));
        }
        let mut files = Vec::new();
        for entry in paths {
            let Some(file) = entry["file"].as_str() else {
                continue;
            };
            let file_path = directory.join(file);
            let bytes = fs::read(&file_path)
                .map_err(|e| format!("read guide {}: {e}", file_path.display()))?;
            files.push(json!({
                "id": entry["id"],
                "path": file_path.display().to_string(),
                "sha256": sha256(&bytes),
                "bytes": bytes.len(),
                "status": entry.get("status").cloned().unwrap_or(Value::Null),
            }));
        }
        result.insert(
            name.to_owned(),
            json!({
                "root": root.display().to_string(),
                "commit": actual,
                "expected_commit": expected_pin,
                "index": {
                    "path": index_path.display().to_string(),
                    "sha256": sha256(&index_bytes),
                    "entry_count": paths.len(),
                    "unavailable_entries": unavailable_entries,
                },
                "guide_file_count": files.len(),
                "files": files,
            }),
        );
    }
    Ok(Value::Object(result))
}

fn tile(x: i32, z: i32, level: i32) -> WorldTile {
    WorldTile { x, z, level }
}

fn tile_json(tile: WorldTile) -> Value {
    json!([tile.x, tile.z, tile.level])
}

fn parse_tile(value: &Value) -> Option<WorldTile> {
    let row = value.as_array()?;
    if row.len() != 3 {
        return None;
    }
    Some(tile(
        i32::try_from(row[0].as_i64()?).ok()?,
        i32::try_from(row[1].as_i64()?).ok()?,
        i32::try_from(row[2].as_i64()?).ok()?,
    ))
}

fn rich_289_state() -> WorldState {
    WorldState {
        map_members: true,
        combat_level: Some(126),
        quests: HashSet::from([
            "Prince Ali Rescue".into(),
            "Rune Mysteries".into(),
            "Lost City".into(),
            "Shilo Village".into(),
            "Tree Gnome Village".into(),
            "The Grand Tree".into(),
        ]),
        inv: HashMap::from([
            (995, 10_000),
            (554, 1_000),
            (555, 1_000),
            (556, 1_000),
            (557, 1_000),
            (558, 1_000),
            (561, 1_000),
            (563, 1_000),
        ]),
        stats: (0..21).map(|skill| (skill, 99)).collect(),
        ..WorldState::empty()
    }
}

fn rich_289_state_json() -> Value {
    json!({
        "authored_by": "crates/nav-planner/examples/perf_corpus.rs::rich_289_state",
        "map_members": true,
        "combat_level": 126,
        "quests": ["Prince Ali Rescue", "Rune Mysteries", "Lost City", "Shilo Village", "Tree Gnome Village", "The Grand Tree"],
        "inventory": [[995,10000],[554,1000],[555,1000],[556,1000],[557,1000],[558,1000],[561,1000],[563,1000]],
        "stats": (0..21).map(|skill| json!([skill,99])).collect::<Vec<_>>(),
        "inventory_ready": false,
        "equipment_ready": false,
    })
}

fn empty_state_json() -> Value {
    json!({
        "authored_by": "WorldState::empty() in crates/nav-planner/examples/perf_corpus.rs",
        "map_members": false,
        "combat_level": null,
        "quests": [],
        "inventory": [],
        "stats": [],
        "varps": [],
        "worn": [],
        "inventory_ready": false,
        "equipment_ready": false,
    })
}

fn route_options_json(options: FindOptions, zone_names: &[String]) -> Value {
    json!({
        "allow_teleports": options.allow_teleports,
        "allow_wilderness": options.allow_wilderness,
        "allow_bank_fetch": options.allow_bank_fetch,
        "prefer_target_order": options.prefer_target_order,
        "avoid_zone_names": zone_names,
        "zone_mask_is_all": options.zones.is_all(),
    })
}

fn w4_inputs(world: &NavWorld, source: &Value) -> Result<Vec<CorpusInput>, String> {
    let empty = WorldState::empty();
    let rich = rich_289_state();
    let legacy = FindOptions {
        zones: ZoneExempt::all(),
        ..FindOptions::default()
    };
    let source_json = |name: &str| {
        json!({
            "set": "w4",
            "case": name,
            "source": source,
            "case_source": "crates/nav-planner/examples/perf_corpus.rs",
        })
    };
    let mut inputs = Vec::new();
    let reachable_from = tile(3222, 3218, 0);
    let reachable_to = tile(2965, 3379, 0);
    inputs.push(w4_many_input(W4ManyInput {
        world,
        id: "w4/reachable_lumbridge_falador",
        source: source_json("reachable_lumbridge_falador"),
        from: reachable_from,
        targets: vec![reachable_to],
        state: empty.clone(),
        state_json: empty_state_json(),
        frozen_options: legacy,
        route_options: FindOptions::default(),
        route_source: "unmodified W4 all-zone benchmark route; assessment grants remain empty",
    }));

    let bank_targets: Vec<_> = world
        .named_bank_facts()
        .ok_or("named bank facts unavailable after selected pack binding")?
        .banks()
        .iter()
        .filter(|bank| bank.routable)
        .map(|bank| bank.tile)
        .collect();
    if bank_targets.is_empty() {
        return Err("selected pack has no routable named bank stands".into());
    }
    inputs.push(w4_many_input(W4ManyInput {
        world,
        id: "w4/bank_pick_all_stands",
        source: source_json("bank_pick_all_stands"),
        from: tile(3016, 9840, 0),
        targets: bank_targets,
        state: rich.clone(),
        state_json: rich_289_state_json(),
        frozen_options: legacy,
        route_options: FindOptions::default(),
        route_source:
            "unmodified W4 all-zone FindMany baseline; grants remain empty for assessment",
    }));

    let center = tile(2553, 3406, 0);
    let mut radius_targets = Vec::new();
    for x in center.x - 10..=center.x + 10 {
        for z in center.z - 10..=center.z + 10 {
            let candidate = tile(x, z, 0);
            if world.collision.standable(candidate) {
                radius_targets.push(candidate);
            }
        }
    }
    let radius_start = tile(2615, 3332, 0);
    let radius_search = find_first_with(
        &world.collision,
        &world.graph,
        radius_start,
        &radius_targets,
        legacy,
        &rich,
    );
    let (radius_route, radius_search_error) = match radius_search.route() {
        Ok(route) => (Some(route.clone()), None),
        Err(error) => (None, Some(format!("{error:?}"))),
    };
    let radius_target = radius_route.as_ref().map(|route| route.dest);
    let radius_candidate = radius_route.map(|route| FrozenRoute {
        id: "selected-all-zone-first-route".into(),
        target: route.dest,
        route: Some(route),
        search_error: None,
        route_source: "W4 find_first_with all-zone result".into(),
    });
    inputs.push(CorpusInput {
        id: "w4/radius_walk_candidates".into(),
        source: source_json("radius_walk_candidates"),
        kind: "w4_radius_find_first".into(),
        raw: json!({ "from": tile_json(radius_start), "center": tile_json(center), "radius": 10, "standable_targets": radius_targets.iter().map(|t| tile_json(*t)).collect::<Vec<_>>() }),
        from: Some(radius_start),
        targets: radius_targets,
        state: rich.clone(),
        state_json: rich_289_state_json(),
        route_options: FindOptions::default(),
        grant_names: Vec::new(),
        frozen: radius_candidate.into_iter().collect(),
        witness_target: radius_target,
        status_without_route: None,
        provenance: json!({
            "frozen_route_source": "W4 FindFirst with ZoneExempt::all",
            "frozen_route_authority": "benchmark geometry only; no admission grant is carried into assessment",
            "e2e_search": "strict FindFirst with FindOptions::default, then at most one named witness retry",
            "w4_options": route_options_json(legacy, &[]),
            "frozen_search_error": radius_search_error,
        }),
    });

    let teleport_from = tile(3222, 3218, 0);
    let teleport_to = tile(2817, 3443, 0);
    let teleport_options = FindOptions {
        allow_teleports: true,
        ..legacy
    };
    inputs.push(w4_many_input(W4ManyInput {
        world,
        id: "w4/teleports_on_real_state",
        source: source_json("teleports_on_real_state"),
        from: teleport_from,
        targets: vec![teleport_to],
        state: rich.clone(),
        state_json: rich_289_state_json(),
        frozen_options: teleport_options,
        route_options: FindOptions {
            allow_teleports: true,
            ..FindOptions::default()
        },
        route_source: "unmodified W4 all-zone FindMany route; assessment grants remain empty",
    }));
    inputs.push(w4_many_input(W4ManyInput {
        world,
        id: "w4/unreachable_fail_closed",
        source: source_json("unreachable_fail_closed"),
        from: teleport_from,
        targets: vec![teleport_to],
        state: empty,
        state_json: empty_state_json(),
        frozen_options: legacy,
        route_options: FindOptions::default(),
        route_source: "unmodified W4 all-zone FindMany control; strict end-to-end search must preserve NoPath",
    }));
    Ok(inputs)
}

struct W4ManyInput<'a> {
    world: &'a NavWorld,
    id: &'a str,
    source: Value,
    from: WorldTile,
    targets: Vec<WorldTile>,
    state: WorldState,
    state_json: Value,
    frozen_options: FindOptions,
    route_options: FindOptions,
    route_source: &'a str,
}

fn w4_many_input(params: W4ManyInput<'_>) -> CorpusInput {
    let W4ManyInput {
        world,
        id,
        source,
        from,
        targets,
        state,
        state_json,
        frozen_options,
        route_options,
        route_source,
    } = params;
    let search = find_many_with(
        &world.collision,
        &world.graph,
        from,
        &targets,
        frozen_options,
        &state,
    );
    let frozen = targets
        .iter()
        .enumerate()
        .map(|(index, target)| match search.route(index) {
            Ok(route) => FrozenRoute {
                id: format!("target-{index}-{}-{}-{}", target.x, target.z, target.level),
                target: *target,
                route: Some(route.clone()),
                search_error: None,
                route_source: route_source.to_owned(),
            },
            Err(error) => FrozenRoute {
                id: format!("target-{index}-{}-{}-{}", target.x, target.z, target.level),
                target: *target,
                route: None,
                search_error: Some(format!("{error:?}")),
                route_source: route_source.to_owned(),
            },
        })
        .collect::<Vec<_>>();
    let witness_target = frozen
        .iter()
        .find(|row| row.route.is_some())
        .map(|row| row.target);
    CorpusInput {
        id: id.to_owned(),
        source,
        kind: "w4_find_many".into(),
        raw: json!({
            "from": tile_json(from),
            "targets": targets.iter().map(|target| tile_json(*target)).collect::<Vec<_>>(),
            "w4_case_result": frozen.iter().map(|row| json!({ "target": tile_json(row.target), "error": row.search_error })).collect::<Vec<_>>(),
        }),
        from: Some(from),
        targets,
        state,
        state_json,
        route_options,
        grant_names: Vec::new(),
        frozen,
        witness_target,
        status_without_route: None,
        provenance: json!({
            "frozen_route_source": route_source,
            "frozen_route_options": route_options_json(frozen_options, &[]),
            "assessment_grants": [],
            "e2e_search": "strict W4 FindFirst route selection then at most one named witness retry",
        }),
    }
}

fn loadouts(selected: &api::game_data::SelectedGameData) -> Result<Vec<Loadout>, String> {
    let profiles = [
        Loadout {
            id: "base40_rune_melee_no_food",
            attack: 40,
            strength: 40,
            defence: 40,
            hp: 40,
            prayer: 0,
            aliases: &[
                "rune_full_helm",
                "rune_chainbody",
                "rune_platelegs",
                "rune_kiteshield",
                "rune_scimitar",
            ],
            lobster_count: 0,
            map_members: None,
            hp_variants: &[],
        },
        Loadout {
            id: "consent_def1_hp10_unworn_no_food",
            attack: 1,
            strength: 1,
            defence: 1,
            hp: 10,
            prayer: 0,
            aliases: &[],
            lobster_count: 0,
            map_members: None,
            hp_variants: &[],
        },
        Loadout {
            id: "def20_hp20_iron_with_two_lobsters_old_replay_only",
            attack: 40,
            strength: 40,
            defence: 20,
            hp: 20,
            prayer: 0,
            aliases: &[
                "iron_full_helm",
                "iron_platebody",
                "iron_platelegs",
                "iron_kiteshield",
                "iron_scimitar",
            ],
            lobster_count: 2,
            map_members: None,
            hp_variants: &[],
        },
    ];
    let mut out = profiles.to_vec();
    for profile in &out {
        for alias in profile.aliases {
            let item = selected.item_by_alias(alias).ok_or_else(|| {
                format!("selected R289 data has no required loadout item {alias}")
            })?;
            if selected.item_combat_bonus(item.id).is_none() {
                return Err(format!(
                    "selected R289 data has no combat bonus for {alias}"
                ));
            }
        }
    }
    // P1-C's accepted WWM high-HP control is the same base-40 rune kit at
    // HP 56. It is a second observation variant, not a fourth equipment set.
    out[0].hp_variants = &[("hp56_design_control", 56)];
    Ok(out)
}

fn loadout_bonus(
    selected: &api::game_data::SelectedGameData,
    profile: &Loadout,
) -> Result<[i16; 4], String> {
    let mut sum = [0i16; 4];
    for alias in profile.aliases {
        let item = selected
            .item_by_alias(alias)
            .ok_or_else(|| format!("missing selected item {alias}"))?;
        let bonus = selected
            .item_combat_bonus(item.id)
            .ok_or_else(|| format!("missing selected combat bonus {alias}"))?;
        for (total, addition) in sum.iter_mut().zip([
            bonus.stabdefence,
            bonus.slashdefence,
            bonus.crushdefence,
            bonus.rangedefence,
        ]) {
            *total = total
                .checked_add(addition)
                .ok_or_else(|| format!("loadout defence bonus overflow for {alias}"))?;
        }
    }
    Ok(sum)
}

fn combat_level(profile: &Loadout) -> Option<u8> {
    // The worked-example source states no prayer and fixes Attack/Strength at
    // 40. Preserve the ordinary combat-level formula instead of guessing a
    // combat level from a label; all inputs are recorded beside the result.
    let base =
        (u16::from(profile.defence) + u16::from(profile.hp) + u16::from(profile.prayer / 2)) / 4;
    let melee = u16::from(profile.attack) + u16::from(profile.strength);
    let melee_part = (u32::from(melee) * 13) / 40;
    u8::try_from(u32::from(base) + melee_part).ok()
}

fn loadout_json(profile: &Loadout) -> Value {
    json!({
        "id": profile.id,
        "stats": {
            "attack": profile.attack,
            "strength": profile.strength,
            "defence": profile.defence,
            "hitpoints": profile.hp,
            "prayer": profile.prayer,
            "combat_level": combat_level(profile),
            "combat_level_basis": "floor((Defence + Hitpoints + floor(Prayer/2))/4 + max(melee, ranged, magic)*13/40); Attack/Strength/Prayer from P1.11 worked example",
        },
        "worn_aliases": profile.aliases,
        "food": if profile.lobster_count == 0 { json!([]) } else { json!([{ "alias": "lobster", "count": profile.lobster_count, "old_replay_credit_only": true }]) },
        "new_model_food_credit": "none",
        "map_members": profile.map_members,
        "poison_basis": "P1.11 base-40 live cell is Clear; all other inputs remain Unknown because their sources do not state poison",
        "hp_variants": profile.hp_variants.iter().map(|(id,hp)| json!({"id":id,"hp":hp})).collect::<Vec<_>>(),
    })
}

fn profile_variants(profile: &Loadout, input_id: &str) -> Vec<(String, u8)> {
    let mut result = vec![("primary".to_owned(), profile.hp)];
    if input_id == "live/white_wolf_mountain" && profile.id == "base40_rune_melee_no_food" {
        result.push(("hp56_design_control".to_owned(), 56));
    }
    result
}

fn poison_for(input: &CorpusInput, profile: &Loadout) -> PoisonState {
    if input.kind == "live_cell_static_route_replay" && profile.id == "base40_rune_melee_no_food" {
        PoisonState::Clear
    } else {
        PoisonState::Unknown { since: 0 }
    }
}

fn make_risk_input(
    profile: &Loadout,
    hp: u8,
    position: Option<WorldTile>,
    map_members: bool,
    poison: PoisonState,
    worn_defence: [i16; 4],
) -> RiskInput {
    let mut input = RiskInput::default();
    if let Some(position) = position {
        input.pos = position;
    }
    input.poison = poison;
    input.tick = 0;
    input.hp = hp;
    input.hp_max = hp;
    input.prayer = profile.prayer;
    input.prayer_base = profile.prayer;
    input.combat = combat_level(profile);
    input.defence = profile.defence;
    input.worn_defence = worn_defence;
    input.style_def = 0;
    input.active_protect = Style::Unknown;
    input.food_len = u8::from(profile.lobster_count > 0);
    if profile.lobster_count > 0 {
        input.food_ids[0] = 379;
        input.food_counts[0] = profile.lobster_count;
    }
    input.free_slots = 28u8.saturating_sub(profile.lobster_count);
    input.set_map_members(map_members);
    input.set_run(false);
    input.set_other_prayers_on(false);
    input.set_off_debt(false);
    input.set_worn_shield(false);
    input.set_more_food(false);
    input.set_missing_facts(false);
    input.set_unattributed(false);
    input.set_input_locked(false);
    input.set_overflow(false);
    input
}

fn query_inputs(
    world: &NavWorld,
    query_path: &Path,
    query_bytes: &[u8],
    query_source: &Value,
) -> Result<Vec<CorpusInput>, String> {
    let mut inputs = Vec::new();
    let mut line_number = 0usize;
    for line in query_bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        line_number += 1;
        let text = std::str::from_utf8(line)
            .map_err(|e| format!("query line {line_number} is not UTF-8: {e}"))?;
        let value: Value = serde_json::from_str(text)
            .map_err(|e| format!("parse calculator query line {line_number}: {e}"))?;
        let mode = value["mode"].as_str().unwrap_or("unknown");
        let id = format!("seed-calc/query-{line_number:02}");
        let source = json!({
            "set": "seed_calc_queries",
            "query_file": query_source,
            "line": line_number,
            "raw_line_sha256": sha256(line),
            "mode": mode,
        });
        if mode != "crossing" {
            inputs.push(CorpusInput {
                id,
                source,
                kind: "seed_calc_fight_query".into(),
                raw: value.clone(),
                from: None,
                targets: Vec::new(),
                state: WorldState::empty(),
                state_json: json!({ "authored": "none; fight query is not a route-state input" }),
                route_options: FindOptions::default(),
                grant_names: Vec::new(),
                frozen: Vec::new(),
                witness_target: None,
                status_without_route: Some("non_route_fight_query_retained".into()),
                provenance: json!({
                    "facts_path": value["facts"],
                    "route_assessment": "not_applicable",
                    "fight_solver": "not invoked by P1-C route admission exporter; no fight result is claimed",
                    "raw_input_preserved": true,
                }),
            });
            continue;
        }
        let from = value.get("from").and_then(parse_tile);
        let to = value.get("to").and_then(parse_tile);
        let (Some(from), Some(to)) = (from, to) else {
            inputs.push(CorpusInput {
                id,
                source,
                kind: "seed_calc_crossing_query".into(),
                raw: value,
                from: None,
                targets: Vec::new(),
                state: WorldState::empty(),
                state_json: json!({ "status": "underspecified_missing_authored_endpoints" }),
                route_options: FindOptions::default(),
                grant_names: Vec::new(),
                frozen: Vec::new(),
                witness_target: None,
                status_without_route: Some("underspecified_missing_authored_endpoints".into()),
                provenance: json!({"query_file": query_path.display().to_string(), "line":line_number}),
            });
            continue;
        };
        let names = value["zones"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let (keys, unresolved) = resolve_grants(world, &names);
        let (options, grant_error) = if unresolved.is_empty() {
            match ZoneExempt::named(&keys) {
                Ok(zones) => (
                    FindOptions {
                        zones,
                        ..FindOptions::default()
                    },
                    None,
                ),
                Err(error) => (
                    FindOptions::default(),
                    Some(format!("too many named grants: {error:?}")),
                ),
            }
        } else {
            (
                FindOptions::default(),
                Some(format!("unresolved named grant identities: {unresolved:?}")),
            )
        };
        let state = query_world_state(&value["route_state"]);
        let state_json = if value.get("route_state").is_some() {
            value["route_state"].clone()
        } else {
            empty_state_json()
        };
        let frozen = if grant_error.is_some() {
            Vec::new()
        } else {
            let search = find_with_avoid(
                &world.collision,
                &world.graph,
                from,
                to,
                options,
                &state,
                &[],
            );
            match search {
                Ok(route) => vec![FrozenRoute {
                    id: "query-route".into(),
                    target: to,
                    route: Some(route),
                    search_error: None,
                    route_source:
                        "seed-calc crossing query exact from/to with authored named zone list"
                            .into(),
                }],
                Err(error) => vec![FrozenRoute {
                    id: "query-route".into(),
                    target: to,
                    route: None,
                    search_error: Some(format!("{error:?}")),
                    route_source:
                        "seed-calc crossing query exact from/to with authored named zone list"
                            .into(),
                }],
            }
        };
        inputs.push(CorpusInput {
            id,
            source,
            kind: "seed_calc_crossing_query".into(),
            raw: value.clone(),
            from: Some(from),
            targets: vec![to],
            state,
            state_json,
            route_options: options,
            grant_names: names,
            frozen,
            witness_target: Some(to),
            status_without_route: grant_error
                .as_ref()
                .map(|_| "invalid_named_grants".to_owned()),
            provenance: json!({
                "query_file": query_path.display().to_string(),
                "line": line_number,
                "frozen_route_source": "same declared query route and named zones for both models",
                "declared_grants_unresolved": unresolved,
                "grant_resolution_error": grant_error.as_deref(),
            }),
        });
    }
    if line_number != 12 {
        return Err(format!(
            "queries.jsonl expected 12 input lines, found {line_number}"
        ));
    }
    Ok(inputs)
}

fn query_world_state(value: &Value) -> WorldState {
    let mut state = WorldState::empty();
    for (key, row) in value["skills"].as_object().into_iter().flatten() {
        if let (Ok(skill), Some(level)) = (key.parse::<i32>(), row.as_i64()) {
            state.stats.insert(skill, level as i32);
        }
    }
    for (key, row) in value["varps"].as_object().into_iter().flatten() {
        if let (Ok(varp), Some(value)) = (key.parse::<i32>(), row.as_i64()) {
            state.varps.insert(varp, value as i32);
        }
    }
    if let Some(quests) = value["completed_quests"].as_array() {
        state
            .quests
            .extend(quests.iter().filter_map(Value::as_str).map(str::to_owned));
    }
    if let Some(inventory) = value["inventory"].as_object() {
        for (key, count) in inventory {
            if let (Ok(item), Some(count)) = (key.parse::<i32>(), count.as_i64()) {
                state.inv.insert(item, count as i32);
            }
        }
    }
    state
}

fn resolve_grants(world: &NavWorld, names: &[String]) -> (Vec<ZoneKey>, Vec<String>) {
    let mut keys = Vec::new();
    let mut unresolved = Vec::new();
    let mut seen = HashSet::with_capacity(names.len());
    for name in names {
        if !seen.insert(name.as_str()) {
            continue;
        }
        match world
            .graph
            .zones
            .as_ref()
            .and_then(|table| table.resolve(name))
        {
            Some(key) => keys.push(key),
            None => unresolved.push(name.clone()),
        }
    }
    (keys, unresolved)
}

fn live_inputs(world: &NavWorld, design_source: &Value) -> Result<Vec<CorpusInput>, String> {
    let mut state = WorldState::empty().with_map_members(true);
    state.combat_level = Some(46);
    let state_json = json!({
        "basis": "P1.11 worked example base-40 account; members world for member-area live cells",
        "map_members": true,
        "combat_level": 46,
        "quests": [], "inventory": [], "stats": [], "varps": [],
    });
    let cases = [
        (
            "live/rock_crabs_to_seers_bank",
            "rock_crabs_to_seers_bank",
            tile(2704, 3726, 0),
            tile(2725, 3492, 0),
        ),
        (
            "live/observatory_moss_giants",
            "observatory_moss_giants",
            tile(2438, 3185, 0),
            tile(2552, 3574, 0),
        ),
        (
            "live/white_wolf_mountain",
            "white_wolf_mountain",
            tile(2893, 3429, 0),
            tile(2809, 3436, 0),
        ),
    ];
    let all = FindOptions {
        zones: ZoneExempt::all(),
        ..FindOptions::default()
    };
    let mut inputs = Vec::new();
    for (id, label, from, to) in cases {
        let route = find_with_avoid(&world.collision, &world.graph, from, to, all, &state, &[]);
        let (route, search_error) = match route {
            Ok(route) => (Some(route), None),
            Err(error) => (None, Some(format!("{error:?}"))),
        };
        inputs.push(CorpusInput {
            id: id.into(),
            source: json!({
                "set": "p1c_live_cells",
                "cell": label,
                "design": design_source,
                "design_section": "design-r4.md §P1.12 P1-C / §10.4",
            }),
            kind: "live_cell_static_route_replay".into(),
            raw: json!({ "from": tile_json(from), "to": tile_json(to), "account_profiles": "three loadouts; WWM base40 kit also has an HP56 variant" }),
            from: Some(from),
            targets: vec![to],
            state: state.clone(),
            state_json: state_json.clone(),
            route_options: FindOptions::default(),
            grant_names: Vec::new(),
            frozen: vec![FrozenRoute {
                id: label.into(), target: to, route, search_error,
                route_source: "same endpoint route found with ZoneExempt::all for frozen geometry; admission grants remain empty".into(),
            }],
            witness_target: Some(to),
            status_without_route: None,
            provenance: json!({
                "run_type": "offline selected-pack route assessment input, not a live account execution",
                "route_state_source": "members-world P1.11 design fixture; no account settings or live target is changed",
                "frozen_route_options": route_options_json(all, &[]),
                "strict_end_to_end": "default FindOptions and one named witness retry",
                "design_expected_not_asserted": match label {
                    "rock_crabs_to_seers_bank" | "observatory_moss_giants" => "P1-C live acceptance expects admit at base40",
                    "white_wolf_mountain" => "P1-C live acceptance expects refuse at HP40 and a separate HP56 base40 kit run to admit",
                    _ => "",
                },
            }),
        });
    }
    Ok(inputs)
}

fn path_schema_probes(
    world: &NavWorld,
    roots: &Value,
    path_schema_source: &Value,
) -> Result<Vec<CorpusInput>, String> {
    // Exact route probes and starts from the existing ignored real-navigation
    // path_schema test. These origins are test-authored inputs, not inferred
    // player positions. Source guide rows remain underspecified separately.
    let probes: [(&str, &str, &str, WorldTile, WorldTile, i32); 6] = [
        (
            "druid",
            "walk-to-prison-door",
            "druid.json",
            tile(2884, 9797, 0),
            tile(2888, 9831, 0),
            20,
        ),
        (
            "junglepotion",
            "walk-to-snake-weed",
            "junglepotion.json",
            tile(2809, 3086, 0),
            tile(2761, 3015, 0),
            3,
        ),
        (
            "junglepotion",
            "walk-to-rogues-purse",
            "junglepotion.json",
            tile(2830, 9521, 0),
            tile(2850, 9477, 0),
            3,
        ),
        (
            "junglepotion",
            "walk-back-to-trufitus",
            "junglepotion.json",
            tile(2761, 3015, 0),
            tile(2809, 3086, 0),
            3,
        ),
        (
            "junglepotion",
            "walk-to-cave-exit",
            "junglepotion.json",
            tile(2850, 9477, 0),
            tile(2830, 9521, 0),
            3,
        ),
        (
            "druid",
            "exit-dungeon-walk-to-ladder-for-missing-meat",
            "druid.json",
            tile(2892, 9831, 0),
            tile(2884, 9798, 0),
            20,
        ),
    ];
    let mut inputs = Vec::new();
    for (guide_id, step_id, file, from, to, combat_level) in probes {
        let root_key = if guide_id == "druid" || guide_id == "junglepotion" {
            if roots.get("guide_67").is_some_and(|root| {
                root["files"].as_array().is_some_and(|rows| {
                    rows.iter().any(|row| {
                        row["path"]
                            .as_str()
                            .is_some_and(|path| path.ends_with(file))
                    })
                })
            }) {
                "guide_67"
            } else {
                "guide_25"
            }
        } else {
            "guide_25"
        };
        let guide_root = PathBuf::from(roots[root_key]["root"].as_str().unwrap_or_default());
        let guide_path = guide_root.join("crates/script/paths/289").join(file);
        let guide: Value =
            serde_json::from_slice(&fs::read(&guide_path).map_err(|e| {
                format!("read path-schema probe guide {}: {e}", guide_path.display())
            })?)
            .map_err(|e| {
                format!(
                    "parse path-schema probe guide {}: {e}",
                    guide_path.display()
                )
            })?;
        let step = find_guide_step(&guide, step_id).ok_or_else(|| {
            format!(
                "path-schema probe step {guide_id}/{step_id} missing from {}",
                guide_path.display()
            )
        })?;
        let names = step
            .pointer("/args/cross")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let (keys, unresolved) = resolve_grants(world, &names);
        if !unresolved.is_empty() {
            return Err(format!("path-schema step {guide_id}/{step_id} has unresolved grant identities: {unresolved:?}"));
        }
        let named = ZoneExempt::named(&keys)
            .map_err(|e| format!("path-schema grant count {guide_id}/{step_id}: {e:?}"))?;
        let options = FindOptions {
            zones: named,
            ..FindOptions::default()
        };
        let mut state = WorldState::empty().with_map_members(true);
        state.combat_level = Some(combat_level);
        let state_json = json!({
            "basis": "exact state from path_schema.rs::danger_cross_grants_route_low_combat_legs",
            "map_members": true,
            "combat_level": combat_level,
            "quests": [], "inventory": [], "stats": [], "varps": [],
        });
        let route = find_with_avoid(
            &world.collision,
            &world.graph,
            from,
            to,
            options,
            &state,
            &[],
        );
        let (route, search_error) = match route {
            Ok(route) => (Some(route), None),
            Err(error) => (None, Some(format!("{error:?}"))),
        };
        inputs.push(CorpusInput {
            id: format!("path_schema/{guide_id}/{step_id}"),
            source: json!({
                "set": "path_schema_real_navigation",
                "guide_root": root_key,
                "guide": guide_id,
                "step": step_id,
                "guide_path": guide_path.display().to_string(),
                "guide_sha256": sha256(&fs::read(&guide_path).map_err(|e| format!("read {}: {e}", guide_path.display()))?),
                "test_source": path_schema_source,
                "test_function": "danger_cross_grants_route_low_combat_legs",
                "source_root_commit": roots[root_key]["commit"],
            }),
            kind: "path_schema_real_navigation_probe".into(),
            raw: json!({ "from":tile_json(from), "to":tile_json(to), "combat_level":combat_level, "guide_step":step, "stage_grants":names }),
            from: Some(from),
            targets: vec![to],
            state,
            state_json,
            route_options: options,
            grant_names: names,
            frozen: vec![FrozenRoute {
                id: step_id.into(), target: to, route, search_error,
                route_source: "path_schema.rs exact real-pack named-grant FindWith probe".into(),
            }],
            witness_target: Some(to),
            status_without_route: None,
            provenance: json!({
                "authored_start": tile_json(from),
                "start_authority": "existing path_schema.rs real-navigation assertion; not a guide's dynamic player position",
                "stage_grants": step.pointer("/args/cross").cloned().unwrap_or(Value::Null),
                "initial_route_options": "exact named grants carried by the path_schema walk",
            }),
        });
    }
    Ok(inputs)
}

fn guide_inputs(
    world: &NavWorld,
    roots: &Value,
    selected: &api::game_data::SelectedGameData,
) -> Result<Vec<CorpusInput>, String> {
    let mut out = Vec::new();
    let identities = selected
        .quest_identity()
        .ok_or_else(|| "selected seed quest identities are unavailable".to_owned())?;
    for set in ["guide_25", "guide_67"] {
        let root = PathBuf::from(roots[set]["root"].as_str().unwrap_or_default());
        let index_path = root.join("crates/script/paths/289/index.json");
        let index: Value = serde_json::from_slice(
            &fs::read(&index_path).map_err(|e| format!("read {}: {e}", index_path.display()))?,
        )
        .map_err(|e| format!("parse {}: {e}", index_path.display()))?;
        for entry in index["paths"]
            .as_array()
            .ok_or_else(|| format!("{} has no paths", index_path.display()))?
        {
            let id = entry["id"].as_str().unwrap_or("unknown").to_owned();
            let Some(file) = entry["file"].as_str() else {
                out.push(guide_placeholder(set, &id, entry.clone()));
                continue;
            };
            let guide_path = root.join("crates/script/paths/289").join(file);
            let bytes = fs::read(&guide_path)
                .map_err(|e| format!("read guide {}: {e}", guide_path.display()))?;
            let document: Value = serde_json::from_slice(&bytes)
                .map_err(|e| format!("parse guide {}: {e}", guide_path.display()))?;
            let source = json!({
                "set": set,
                "guide_id": id,
                "file": file,
                "path": guide_path.display().to_string(),
                "sha256": sha256(&bytes),
                "root_commit": roots[set]["commit"],
                "index_status": entry.get("status").cloned().unwrap_or(Value::Null),
            });
            let steps = collect_guide_steps(&document);
            let mut fixtures = HashMap::<usize, GuideQualificationFixture>::new();
            let mut previous_sequence = None;
            let mut previous_target = None;
            for GuideStepRef {
                role_index,
                role,
                container,
                sequence_index,
                stage,
                step_index,
                step,
            } in steps
            {
                let kind = step["kind"].as_str().unwrap_or("unknown");
                if kind != "walk" && kind != "combat" && kind != "fight" {
                    continue;
                }
                let step_id = step["id"].as_str().unwrap_or("unnamed");
                let args = &step["args"];
                let target = args
                    .get("tile")
                    .and_then(parse_tile)
                    .or_else(|| args.pointer("/stand/tile").and_then(parse_tile));
                let authored_from = step
                    .get("from")
                    .and_then(parse_tile)
                    .or_else(|| args.get("from").and_then(parse_tile));
                let fixture = match fixtures.entry(role_index) {
                    std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                    std::collections::hash_map::Entry::Vacant(entry) => entry.insert(
                        GuideQualificationFixture::load(&id, &role, selected, identities)?,
                    ),
                };
                let sequence = (role_index, sequence_index);
                let (from, start_rule) = if previous_sequence != Some(sequence) {
                    (Some(fixture.start), fixture.first_start_rule)
                } else if previous_target.is_some() {
                    (previous_target, "preceding_authored_destination")
                } else {
                    (None, "no_usable_preceding_destination")
                };
                previous_sequence = Some(sequence);
                previous_target = target;
                let arg_grants = args.get("cross").cloned().unwrap_or(Value::Null);
                let stage_grants = container.get("cross").cloned().unwrap_or(Value::Null);
                let names = combine_grant_names(&arg_grants, &stage_grants);
                let (keys, unresolved) = resolve_grants(world, &names);
                let (options, grant_error) = if unresolved.is_empty() {
                    match ZoneExempt::named(&keys) {
                        Ok(zones) => (
                            FindOptions {
                                zones,
                                ..FindOptions::default()
                            },
                            None,
                        ),
                        Err(error) => (
                            FindOptions::default(),
                            Some(format!("too many named grants: {error:?}")),
                        ),
                    }
                } else {
                    (
                        FindOptions::default(),
                        Some(format!("unresolved named grant identities: {unresolved:?}")),
                    )
                };
                let state = fixture.state.clone();
                let route_result = if grant_error.is_none() {
                    match (from, target) {
                        (Some(from), Some(target)) => Some(find_with_avoid(
                            &world.collision,
                            &world.graph,
                            from,
                            target,
                            options,
                            &state,
                            &[],
                        )),
                        _ => None,
                    }
                } else {
                    None
                };
                let (frozen, status) = if grant_error.is_some() {
                    (Vec::new(), Some("invalid_named_grants".to_owned()))
                } else {
                    match route_result {
                        Some(Ok(route)) => (vec![FrozenRoute {
                            id: step_id.to_owned(),
                            target: target.expect("route result requires an authored target"),
                            route: Some(route),
                            search_error: None,
                            route_source: "qualification fixture start and seed state; authored destination and named cross list".into(),
                        }], None),
                        Some(Err(error)) => (vec![FrozenRoute {
                            id: step_id.to_owned(),
                            target: target.expect("route result requires an authored target"),
                            route: None,
                            search_error: Some(format!("{error:?}")),
                            route_source: "qualification fixture start and seed state; authored destination and named cross list".into(),
                        }], None),
                        None => (Vec::new(), Some(if from.is_none() {
                            "qualification_missing_preceding_destination".to_owned()
                        } else {
                            "underspecified_missing_authored_destination".to_owned()
                        })),
                    }
                };
                let container_id = sequence_index
                    .map_or_else(|| "prelude".to_owned(), |index| format!("sequence-{index}"));
                let record_id = format!("{set}/{id}/role-{role_index}/{container_id}/{stage}/step-{step_index}/{step_id}");
                let raw = json!({
                    "guide_id":id,
                    "role":role,
                    "role_index":role_index,
                    "container":container,
                    "stage":stage,
                    "step_index":step_index,
                    "step":step,
                    "target":target.map(tile_json),
                    "authored_from":authored_from.map(tile_json),
                    "qualification_from":from.map(tile_json),
                    "stage_required":container.get("required").cloned().unwrap_or(Value::Null),
                    "sequence_cross":stage_grants,
                    "step_cross":arg_grants,
                    "combined_stage_grant_names":names,
                    "unresolved_grants":unresolved,
                    "grant_resolution_error":grant_error.as_deref(),
                });
                out.push(CorpusInput {
                    id: record_id,
                    source: source.clone(),
                    kind: format!("guide_{kind}_step"),
                    raw,
                    from,
                    targets: target.into_iter().collect(),
                    state: state.clone(),
                    state_json: fixture.state_json.clone(),
                    route_options: options,
                    grant_names: names,
                    frozen,
                    witness_target: target,
                    status_without_route: status,
                    provenance: json!({
                        "source_coverage": "every indexed walk/combat/fight step is preserved",
                        "navigation": "Main-authorized qualification fixture, not an authored start or dynamic quest progress",
                        "qualification_fixture": {
                            "start_rule":start_rule,
                            "start":from.map(tile_json),
                            "initial_start":tile_json(fixture.start),
                            "seed_source":fixture.source,
                            "role":role,
                            "sequence":container_id,
                            "stage":stage,
                        },
                        "walk_target_or_fight_stand": target.map(tile_json),
                        "grants_are_metadata_and_exact_named_route_options_only": true,
                        "grant_resolution_error": grant_error.as_deref(),
                    }),
                });
            }
        }
    }
    Ok(out)
}

fn guide_placeholder(set: &str, id: &str, entry: Value) -> CorpusInput {
    CorpusInput {
        id: format!("{set}/{id}/unavailable"),
        source: json!({"set":set,"guide_id":id,"status":"index entry has no JSON file"}),
        kind: "guide_unavailable_index_entry".into(),
        raw: json!({ "index_entry":entry }),
        from: None,
        targets: Vec::new(),
        state: WorldState::empty(),
        state_json: json!({"status":"no guide JSON source"}),
        route_options: FindOptions::default(),
        grant_names: Vec::new(),
        frozen: Vec::new(),
        witness_target: None,
        status_without_route: Some("unavailable_guide_source".into()),
        provenance: json!({"must_preserve":true,"route_claim":"none"}),
    }
}

struct GuideStepRef {
    role_index: usize,
    role: String,
    container: Value,
    sequence_index: Option<usize>,
    stage: String,
    step_index: usize,
    step: Value,
}

fn collect_guide_steps(document: &Value) -> Vec<GuideStepRef> {
    let mut out = Vec::new();
    let Some(roles) = document["roles"].as_array() else {
        return out;
    };
    for (role_index, role) in roles.iter().enumerate() {
        let role_name = role["role"].as_str().unwrap_or("unknown").to_owned();
        if let Some(prelude) = role["prelude"].as_array() {
            for (step_index, step) in prelude.iter().enumerate() {
                if is_route_step(step) {
                    out.push(GuideStepRef {
                        role_index,
                        role: role_name.clone(),
                        container: role.clone(),
                        sequence_index: None,
                        stage: "prelude".into(),
                        step_index,
                        step: step.clone(),
                    });
                }
            }
        }
        if let Some(sequences) = role["sequences"].as_array() {
            for (sequence_index, sequence) in sequences.iter().enumerate() {
                let stage = sequence["stage"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| sequence_index.to_string());
                if let Some(steps) = sequence["steps"].as_array() {
                    for (step_index, step) in steps.iter().enumerate() {
                        if is_route_step(step) {
                            out.push(GuideStepRef {
                                role_index,
                                role: role_name.clone(),
                                container: sequence.clone(),
                                sequence_index: Some(sequence_index),
                                stage: stage.clone(),
                                step_index,
                                step: step.clone(),
                            });
                        }
                    }
                }
            }
        }
    }
    out
}

fn is_route_step(step: &Value) -> bool {
    matches!(step["kind"].as_str(), Some("walk" | "combat" | "fight"))
}

fn find_guide_step(document: &Value, id: &str) -> Option<Value> {
    collect_guide_steps(document)
        .into_iter()
        .find(|row| row.step["id"].as_str() == Some(id))
        .map(|row| row.step)
}

fn combine_grant_names(args_cross: &Value, stage_cross: &Value) -> Vec<String> {
    let mut names = Vec::new();
    for value in [args_cross, stage_cross] {
        if let Some(rows) = value.as_array() {
            names.extend(rows.iter().filter_map(Value::as_str).map(str::to_owned));
        } else if let Some(name) = value.as_str() {
            names.push(name.to_owned());
        }
    }
    names
}

fn write_qualification_fixture(output: &Path, inputs: &[CorpusInput]) -> Result<(), String> {
    let mut counts = BTreeMap::<String, usize>::new();
    let mut coordinate_counts = BTreeMap::<String, usize>::new();
    let mut fallback_guides = HashSet::new();
    let mut no_start = Vec::new();
    let mut no_destination = Vec::new();
    let mut steps = Vec::new();
    for input in inputs {
        let fixture = &input.provenance["qualification_fixture"];
        let Some(rule) = fixture["start_rule"].as_str() else {
            continue;
        };
        *counts.entry(rule.to_owned()).or_default() += 1;
        if input.from.is_some() && !input.targets.is_empty() {
            *coordinate_counts.entry(rule.to_owned()).or_default() += 1;
        }
        if fixture["seed_source"].is_null() {
            fallback_guides.insert(format!(
                "{}/{}",
                input.source["set"].as_str().unwrap_or("unknown"),
                input.source["guide_id"].as_str().unwrap_or("unknown"),
            ));
        }
        let step = json!({
            "input_id":input.id,"source":input.source,"fixture":fixture,
            "authored_start":input.raw["authored_from"],
            "authored_destination":input.targets.iter().copied().map(tile_json).collect::<Vec<_>>(),
        });
        if input.from.is_none() {
            no_start.push(step.clone());
        }
        if input.targets.is_empty() {
            no_destination.push(step.clone());
        }
        steps.push(step);
    }
    let mut fallback_guides: Vec<_> = fallback_guides.into_iter().collect();
    fallback_guides.sort_unstable();
    write_json(
        &output.join("guide-qualification-starts.json"),
        &json!({
            "fixture":"guide_seed_qualification_v1",
            "authority":"Main 2026-10-30: each sequence starts at its QUEST-TERRA seed, or Lumbridge only without a seed; later spatial steps start at the preceding authored destination",
            "navigation_state":"seed state remains constant; authored guide facts are not changed",
            "lumbridge_fallback_authority":"W4 reachable_lumbridge_falador from [3222,3218,0]",
            "start_rule_step_counts":counts,
            "usable_coordinate_pairs_by_start_rule":coordinate_counts,
            "guides_without_seed":fallback_guides,
            "steps_without_usable_start":no_start,
            "steps_without_authored_destination":no_destination,
            "steps":steps,
        }),
    )
}

fn export_record(
    input: &CorpusInput,
    profile: &Loadout,
    variants: &[(String, u8)],
    world: &NavWorld,
    combat: &CombatTables,
    risks: &RiskTables,
    selected: &api::game_data::SelectedGameData,
) -> Value {
    let worn = loadout_bonus(selected, profile)
        .expect("loadout bonuses were validated before corpus export");
    let map_members = profile.map_members.unwrap_or(input.state.map_members);
    let poison = poison_for(input, profile);
    let mut frozen_rows = Vec::new();
    for frozen in &input.frozen {
        let assessments = frozen
            .route
            .as_ref()
            .map(|route| {
                variants
                    .iter()
                    .map(|(id, hp)| {
                        let risk_input =
                            make_risk_input(profile, *hp, input.from, map_members, poison, worn);
                        let admission =
                            make_admission(risk_input, grants_for(world, &input.grant_names));
                        assessment_json(route, world, combat, risks, &admission).map(
                            |assessment| json!({ "variant":id,"hp":hp,"assessment":assessment }),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let route_json = frozen.route.as_ref().map(canonical_route);
        let route_hash = route_json
            .as_ref()
            .and_then(|route| serde_json::to_vec(route).ok())
            .map(|bytes| sha256(&bytes));
        frozen_rows.push(json!({
            "candidate_id": frozen.id,
            "target": tile_json(frozen.target),
            "route_source": frozen.route_source,
            "search_error": frozen.search_error,
            "route_sha256": route_hash,
            "route": route_json,
            "assessments": assessments,
        }));
    }

    let end_to_end = if input.status_without_route.is_none() {
        if let Some(from) = input.from {
            // The W4 and guide states are preserved byte-for-byte. Do not
            // synthesize a combat level from the loadout.
            let search = end_to_end_search(
                input,
                from,
                world,
                &input.state,
                variants,
                profile,
                map_members,
                worn,
                combat,
                risks,
            );
            json!(search)
        } else {
            json!({
                "status": "underspecified_missing_authored_start",
                "strict_search": null,
                "witness": null,
                "route": null,
                "route_sha256": null,
                "assessments": [],
            })
        }
    } else {
        json!({
            "status": input.status_without_route.as_deref(),
            "strict_search": null,
            "witness": null,
            "route": null,
            "route_sha256": null,
            "assessments": [],
        })
    };
    let route_status = input.status_without_route.clone().unwrap_or_else(|| {
        if input.frozen.iter().any(|row| row.route.is_some()) || end_to_end["route"].is_object() {
            "assessed".into()
        } else if input.from.is_none() {
            "underspecified_missing_authored_start".into()
        } else if input.targets.is_empty() {
            "underspecified_missing_authored_destination".into()
        } else {
            "unreachable".into()
        }
    });
    json!({
        "record_id": format!("{}/{}",input.id,profile.id),
        "source": input.source,
        "input_kind": input.kind,
        "input": input.raw,
        "loadout": loadout_json(profile),
        "risk_input": variants.iter().map(|(id,hp)| {
            let risk_input=make_risk_input(profile,*hp,input.from,map_members,poison,worn);
            json!({
                "variant":id,
                "hp":hp,
                "hp_max":hp,
                "position": input.from.map(tile_json),
                "defence":risk_input.defence,
                "worn_defence_stab_slash_crush_ranged":risk_input.worn_defence,
                "combat_level":risk_input.combat,
                "map_members":risk_input.map_members(),
                "poison":format!("{:?}",risk_input.poison),
                "food":risk_input.food_iter().map(|(id,count)| json!({"item_id":id,"count":count})).collect::<Vec<_>>(),
                "run":risk_input.run(),
                "assumptions": "no live attackers, prayer 0, no protection, no boost, no input lock; see pinned P1.11 profile metadata",
            })
        }).collect::<Vec<_>>(),
        "navigation_state": input.state_json,
        "navigation": {
            "start": input.from.map(tile_json),
            "targets": input.targets.iter().map(|target| tile_json(*target)).collect::<Vec<_>>(),
            "route_options": route_options_json(input.route_options, &input.grant_names),
            "assessment_grants": input.grant_names,
            "policy": "Inherit",
            "enforce": true,
            "frozen_routes": frozen_rows,
            "end_to_end": end_to_end,
        },
        "route_status": route_status,
        "provenance": input.provenance,
    })
}

fn make_admission(input: RiskInput, grants: ZoneExempt) -> Admission {
    let options = FindOptions {
        zones: grants,
        ..FindOptions::default()
    };
    let mut admission = Admission::manual(options, input, 0, WalkGlobals::default());
    admission.policy = RiskPolicy::Inherit;
    admission.enforce = true;
    admission.grants = grants;
    admission
}

fn grants_for(world: &NavWorld, names: &[String]) -> ZoneExempt {
    let (keys, unresolved) = resolve_grants(world, names);
    if unresolved.is_empty() {
        ZoneExempt::named(&keys).unwrap_or(ZoneExempt::NONE)
    } else {
        ZoneExempt::NONE
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "one corpus row needs the route, world, profile and both risk tables; a bundle struct would only rename them"
)]
fn end_to_end_search(
    input: &CorpusInput,
    from: WorldTile,
    world: &NavWorld,
    state: &WorldState,
    variants: &[(String, u8)],
    profile: &Loadout,
    map_members: bool,
    worn: [i16; 4],
    combat: &CombatTables,
    risks: &RiskTables,
) -> Value {
    let targets = input.targets.as_slice();
    if targets.is_empty() {
        return json!({"status":"underspecified_no_targets","route":null,"route_sha256":null,"assessments":[]});
    }
    let strict = find_first_with(
        &world.collision,
        &world.graph,
        from,
        targets,
        input.route_options,
        state,
    );
    let mut witness_json = Value::Null;
    let route = match strict.route() {
        Ok(route) => Some(route.clone()),
        Err(RouteError::NoPath) => {
            if let Some(witness_target) = input.witness_target {
                let blocked = find_blocking_zones(
                    &world.collision,
                    &world.graph,
                    from,
                    witness_target,
                    input.route_options,
                    state,
                    &[],
                );
                match blocked {
                    Some(keys) => {
                        let names = world
                            .graph
                            .zones
                            .as_ref()
                            .map(|table| {
                                keys.iter().map(|key| table.name(*key)).collect::<Vec<_>>()
                            })
                            .unwrap_or_default();
                        witness_json = json!({"blocked_zone_names":names,"admissible":false,"retry_attempts":0,"reason":"the product diagnoses the blocking zones but never retries by zone name; NoPath is preserved"});
                        None
                    }
                    None => {
                        witness_json = json!({"blocked_zone_names":[],"admissible":false,"retry_attempts":0,"reason":"no blocking-zone witness; preserve NoPath"});
                        None
                    }
                }
            } else {
                witness_json = json!({"admissible":false,"retry_attempts":0,"reason":"no authored witness target"});
                None
            }
        }
        Err(error) => {
            witness_json = json!({"admissible":false,"retry_attempts":0,"reason":format!("strict search error: {error:?}")});
            None
        }
    };
    let route_json = route.as_ref().map(canonical_route);
    let route_hash = route_json
        .as_ref()
        .and_then(|value| serde_json::to_vec(value).ok())
        .map(|bytes| sha256(&bytes));
    let assessments = route
        .as_ref()
        .map(|route| {
            variants
                .iter()
                .map(|(id, hp)| {
                    let risk_input = make_risk_input(
                        profile,
                        *hp,
                        Some(from),
                        map_members,
                        poison_for(input, profile),
                        worn,
                    );
                    let admission =
                        make_admission(risk_input, grants_for(world, &input.grant_names));
                    assessment_json(route, world, combat, risks, &admission)
                        .map(|result| json!({"variant":id,"hp":hp,"assessment":result}))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json!({
        "status": if route.is_some() { "routed" } else { "unreachable" },
        "strict_search": strict.route().map(canonical_route).map_err(|error| format!("{error:?}")),
        "strict_targets": targets.iter().map(|target| tile_json(*target)).collect::<Vec<_>>(),
        "witness": witness_json,
        "route_sha256": route_hash,
        "route": route_json,
        "assessments": assessments,
    })
}

fn assessment_json(
    route: &Route,
    world: &NavWorld,
    combat: &CombatTables,
    risks: &RiskTables,
    admission: &Admission,
) -> Option<Value> {
    let assessment = admission::assess(route, world, admission);
    let permitted = admission::permits(route, world, admission, &assessment);
    let debug = format!("{assessment:#?}");
    let refusal = (!permitted).then(|| refusal_from_debug(&debug)).flatten();
    let route_view = nav_planner::producer::route_view(world, route).ok()?;
    let path = script::combat::risk::RoutePath::new(route_view).ok();
    let zone_view = nav_planner::producer::zones_view(world).ok().flatten();
    let mut engaged = Vec::new();
    for row in assessment.plan.intervals.iter() {
        let key = zone_view.and_then(|table| row.key(table).ok());
        let zone_name = key.and_then(|key| world.graph.zones.as_ref().map(|table| table.name(key)));
        let npc = combat.npc(row.ident());
        let kind = risks.npc(row.ident());
        let (start_tick, end_tick) = match path {
            Some(path) => (
                path.point(row.a).map(|point| point.tick),
                path.interval_end_tick(row).ok(),
            ),
            None => (None, None),
        };
        let duration = start_tick
            .zip(end_tick)
            .and_then(|(start, end)| end.checked_sub(start));
        let attack_count = duration.and_then(|ticks| {
            (row.rate > 0 && ticks >= 0)
                .then(|| (ticks + i32::from(row.rate) - 1) / i32::from(row.rate))
        });
        let axis =
            npc.and_then(|npc| defence_axis(npc.damagetype.as_deref(), kind.map(|row| row.style)));
        let hit_chance = kind.and_then(|kind| {
            let axis = axis?;
            if kind.unknown_for().is_some() {
                return None;
            }
            let defence = risk::defence_rolls(
                admission.input.defence,
                admission.input.style_def,
                admission.input.worn_defence,
            )
            .ok()?[axis];
            Some(risk::odds::hit_chance(kind.attack_roll, defence))
        });
        let spawn = row.spawn.tile();
        engaged.push(json!({
            "zone": zone_name,
            "zone_key": key.map(|key| format!("{key:?}")),
            "npc_id": row.ident(),
            "npc_config": npc.map(|npc| npc.config.as_str()),
            "npc_display": npc.and_then(|npc| npc.display.as_deref()),
            "spawn": tile_json(spawn),
            "unknown_kind": row.unknown_kind().map(|unknown| format!("{unknown:?}")),
            "interval_indices": {"a":row.a,"b":row.b,"e":row.e,"f":row.f},
            "kind": {"style":format!("{:?}",row.style),"max_hit":row.max_hit,"rate_ticks":row.rate,"reach":row.reach,"attack_roll":kind.map(|kind|kind.attack_roll),"unknown":row.unknown()},
            "h": hit_chance,
            "h_axis": axis.map(|index| ["stab","slash","crush","ranged"][index]),
            "schedule": {
                "start_tick":start_tick,
                "end_tick_exclusive":end_tick,
                "exposure_ticks":duration,
                "rate_ticks":row.rate,
                "attack_windows":attack_count,
                "schedule_basis":"ceil((interval_end_tick - start_tick) / rate), independently per engaged zone",
            },
            "ap":row.ap(),
            "hazard":row.hazard(),
            "escaping":row.escaping(),
        }));
    }
    let crossings = assessment
        .crossings
        .iter()
        .map(|crossing| {
            let zone_name = world
                .graph
                .zones
                .as_ref()
                .map(|table| table.name(crossing.key));
            let zone_key = format!("{:?}", crossing.key);
            let rows = engaged
                .iter()
                .filter(|row| row["zone_key"].as_str() == Some(zone_key.as_str()))
                .collect::<Vec<_>>();
            let attack_count = rows
                .iter()
                .filter_map(|row| row["schedule"]["attack_windows"].as_i64())
                .sum::<i64>();
            json!({
                "zone":zone_name,
                "key":zone_key,
                "first":crossing.first,
                "last":crossing.last,
                "ticks":crossing.ticks,
                "worst":crossing.worst,
                "volley":crossing.volley,
                "max_hit":crossing.max_hit,
                "rate":crossing.rate,
                "style":format!("{:?}",crossing.style),
                "floor_deep":crossing.floor_deep,
                "bites":crossing.bites,
                "single":crossing.single,
                "protect_credited":crossing.protect_credited,
                "unknown":crossing.unknown,
                "engaged_intervals":rows,
                "scheduled_attack_windows_sum":attack_count,
            })
        })
        .collect::<Vec<_>>();
    Some(json!({
        "verdict":format!("{:?}",assessment.verdict),
        "permitted":permitted,
        "refusal":refusal,
        "reason":assessment.reason.as_ref(),
        "plan_complete":assessment.plan.complete,
        "plan_interval_count":assessment.plan.intervals.len(),
        "plan_crossing_count":assessment.plan.crossings.len(),
        "crossings":crossings,
        "engaged_kinds":engaged,
        "bound":debug_number(&debug,"bound"),
        "fire":debug_number(&debug,"fire"),
        "estimate":debug_field(&debug,"estimate"),
        "mitigation":debug_field(&debug,"mitigation"),
        "supplies":format!("{:?}",assessment.supplies),
        "grants":debug_field(&debug,"grants"),
        "poison_test":debug_field(&debug,"poison_test"),
        "assessment_debug_sha256":sha256(debug.as_bytes()),
        "assessment_debug":debug,
        "risk_policy":"Inherit",
        "enforce":true,
        "assessment_grants": debug_field(&debug,"grants"),
    }))
}

fn defence_axis(damagetype: Option<&str>, style: Option<Style>) -> Option<usize> {
    let damage = damagetype?.to_ascii_lowercase();
    match damage.as_str() {
        "stab" => Some(0),
        "slash" => Some(1),
        "crush" => Some(2),
        "ranged" | "range" | "arrow" | "bolt" | "thrown" => Some(3),
        "magic" | "dragonfire" | "mixed" => None,
        _ if style == Some(Style::Ranged) => Some(3),
        _ => None,
    }
}
fn refusal_from_debug(debug: &str) -> Option<String> {
    let explicit = debug_field(debug, "refusal");
    if let Some(value) = explicit.as_str() {
        return Some(value.to_owned());
    }
    let verdict = debug_field(debug, "verdict");
    let verdict = verdict.as_str()?;
    Some(if verdict == "Survivable" {
        "Unsurvivable".to_owned()
    } else {
        verdict.to_owned()
    })
}

fn debug_field(debug: &str, field: &str) -> Value {
    let prefix = format!("    {field}:");
    let mut lines = debug.lines();
    while let Some(line) = lines.next() {
        let Some(value) = line.strip_prefix(&prefix) else {
            continue;
        };
        let mut result = value.trim().to_owned();
        let mut depth = 0i32;
        let mut in_string = false;
        let mut escaped = false;
        debug_delimiters(&result, &mut depth, &mut in_string, &mut escaped);
        if depth == 0 {
            return Value::String(result.trim_end_matches(',').to_owned());
        }
        for line in lines {
            result.push('\n');
            let line = line.trim();
            result.push_str(line);
            debug_delimiters(line, &mut depth, &mut in_string, &mut escaped);
            if depth == 0 {
                return Value::String(result.trim_end_matches(',').to_owned());
            }
        }
        return json!({"debug":result,"complete":false});
    }
    Value::Null
}

fn debug_delimiters(text: &str, depth: &mut i32, in_string: &mut bool, escaped: &mut bool) {
    for character in text.chars() {
        if *in_string {
            if *escaped {
                *escaped = false;
            } else if character == '\\' {
                *escaped = true;
            } else if character == '"' {
                *in_string = false;
            }
            continue;
        }
        match character {
            '"' => *in_string = true,
            '{' | '(' | '[' => *depth += 1,
            '}' | ')' | ']' => *depth -= 1,
            _ => {}
        }
    }
}

fn debug_number(debug: &str, field: &str) -> Value {
    let raw = debug_field(debug, field);
    let Some(text) = raw.as_str() else {
        return Value::Null;
    };
    let token = text.trim_matches(|c: char| {
        !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+' || c == 'e' || c == 'E')
    });
    token
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite())
        .map_or(Value::Null, |number| json!(number))
}

fn canonical_route(route: &Route) -> Value {
    let legs = route.legs.iter().map(|leg| match leg {
        Leg::Walk { tiles } => json!({"kind":"walk","tiles":tiles.iter().map(|tile| tile_json(*tile)).collect::<Vec<_>>()}),
        Leg::Transport { edge } => json!({
            "kind":"transport",
            "transport":{
                "kind":format!("{:?}",edge.kind),
                "at":tile_json(edge.at),
                "to":tile_json(edge.to),
                "takeoff":edge.takeoff.map(tile_json),
                "player_delta":edge.player_delta.map(tile_json),
                "loc_id":edge.loc_id,
                "option":edge.option,
                "ticks":edge.ticks,
                "dir":edge.dir.map(|dir|format!("{dir:?}")),
                "open_loc_id":edge.open_loc_id,
                "skill_req":edge.skill_req,
                "item_req":edge.item_req,
                "consumed_req":edge.consumed_req,
                "item_returns":edge.item_returns,
                "quest_req":edge.quest_req,
                "varp_req":edge.varp_req,
                "worn_req":edge.worn_req,
                "worn_all_req":edge.worn_all_req,
                "forbidden_held":edge.forbidden_held,
                "forbidden_worn":edge.forbidden_worn,
            }
        }),
    }).collect::<Vec<_>>();
    json!({"from":route.legs.first().and_then(|leg| match leg {Leg::Walk{tiles}=>tiles.first().copied(),Leg::Transport{edge}=>Some(edge.at)}).map(tile_json),"dest":tile_json(route.dest),"ticks":route.ticks,"legs":legs})
}

fn detect_model(records: &[Value]) -> Result<String, String> {
    let found = records
        .iter()
        .flat_map(|record| {
            record["navigation"]["frozen_routes"]
                .as_array()
                .into_iter()
                .flatten()
        })
        .flat_map(|row| row["assessments"].as_array().into_iter().flatten())
        .find_map(|variant| variant["assessment"]["assessment_debug"].as_str());
    let debug = found
        .ok_or("no frozen route was assessable; cannot determine model from assessment Debug")?;
    if debug
        .lines()
        .any(|line| line.trim_start().starts_with("bound:"))
    {
        Ok("p1c_probability".into())
    } else {
        Ok("p1b_worst".into())
    }
}

fn guide_coverage(inputs: &[CorpusInput]) -> Value {
    let mut sets: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for input in inputs.iter().filter(|input| {
        input.source["set"]
            .as_str()
            .is_some_and(|set| set.starts_with("guide_"))
    }) {
        let set = input.source["set"].as_str().unwrap_or("unknown").to_owned();
        let counts = sets.entry(set).or_default();
        *counts.entry("indexed_records".into()).or_default() += 1;
        match input.kind.as_str() {
            "guide_walk_step" => *counts.entry("walk_steps".into()).or_default() += 1,
            "guide_combat_step" | "guide_fight_step" => {
                *counts.entry("fight_steps".into()).or_default() += 1
            }
            "guide_unavailable_index_entry" => {
                *counts
                    .entry("unavailable_index_entries".into())
                    .or_default() += 1
            }
            _ => *counts.entry("other_route_steps".into()).or_default() += 1,
        }
        if input.from.is_none() && input.kind.contains("step") {
            *counts
                .entry("underspecified_missing_start".into())
                .or_default() += 1;
        }
    }
    json!(sets)
}

fn source_content_identity(value: &Value) -> Value {
    match value {
        Value::Object(fields) => {
            let content_addressed = fields.get("sha256").is_some_and(Value::is_string);
            Value::Object(
                fields
                    .iter()
                    .filter(|(key, _)| !content_addressed || key.as_str() != "path")
                    .map(|(key, value)| (key.clone(), source_content_identity(value)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(source_content_identity).collect()),
        value => value.clone(),
    }
}

#[test]
fn corpus_input_identity_uses_source_content_not_clone_location() {
    let old = json!({
        "record_id": "source-identity-probe",
        "source": {
            "file": {"path": "/old-clone/input.json", "sha256": "abc", "bytes": 12},
            "unhashed_locator": {"path": "/authored/path"}
        },
        "input": {"from": [1, 2, 0], "targets": [[3, 4, 0]]}
    });
    let signature = input_signature(std::slice::from_ref(&old)).unwrap();
    let mut relocated = old.clone();
    relocated["source"]["file"]["path"] = json!("/new-clone/input.json");
    assert_eq!(input_signature(&[relocated]).unwrap(), signature);
    let mut changed_content = old.clone();
    changed_content["source"]["file"]["sha256"] = json!("def");
    assert_ne!(input_signature(&[changed_content]).unwrap(), signature);
    let mut changed_input = old.clone();
    changed_input["input"]["targets"] = json!([[5, 6, 0]]);
    assert_ne!(input_signature(&[changed_input]).unwrap(), signature);
    let mut changed_unhashed_path = old;
    changed_unhashed_path["source"]["unhashed_locator"]["path"] = json!("/different/path");
    assert_ne!(
        input_signature(&[changed_unhashed_path]).unwrap(),
        signature
    );
}

fn canonicalize_input(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            fields.sort_keys();
            fields.values_mut().for_each(canonicalize_input);
        }
        Value::Array(items) => items.iter_mut().for_each(canonicalize_input),
        _ => {}
    }
}

#[test]
fn corpus_input_identity_ignores_json_object_order() {
    let first: Value = serde_json::from_str(
        r#"{"record_id":"order-probe","source":{},"navigation":{},"navigation_state":{"stats":{"20":40,"0":40}}}"#,
    )
    .unwrap();
    let reordered: Value = serde_json::from_str(
        r#"{"navigation_state":{"stats":{"0":40,"20":40}},"navigation":{},"source":{},"record_id":"order-probe"}"#,
    )
    .unwrap();
    assert_eq!(
        input_signature(&[first]).unwrap(),
        input_signature(&[reordered]).unwrap()
    );
}

fn input_signature(records: &[Value]) -> Result<String, String> {
    let mut signature = Value::Array(
        records
            .iter()
            .map(|record| {
                let frozen = record["navigation"]["frozen_routes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|row| {
                        json!({
                            "candidate_id":row["candidate_id"],
                            "target":row["target"],
                            "route_sha256":row["route_sha256"],
                            "route_source":row["route_source"],
                        })
                    })
                    .collect::<Vec<_>>();
                json!({
                    "record_id":record["record_id"],
                    "source":source_content_identity(&record["source"]),
                    "input_kind":record["input_kind"],
                    "input":record["input"],
                    "loadout":record["loadout"],
                    "risk_input":record["risk_input"],
                    "navigation_state":record["navigation_state"],
                    "navigation_start":record["navigation"]["start"],
                    "navigation_targets":record["navigation"]["targets"],
                    "route_options":record["navigation"]["route_options"],
                    "assessment_grants":record["navigation"]["assessment_grants"],
                    "frozen":frozen,
                })
            })
            .collect::<Vec<_>>(),
    );
    canonicalize_input(&mut signature);
    let bytes = serde_json::to_vec(&signature)
        .map_err(|e| format!("serialize corpus input signature: {e}"))?;
    Ok(sha256(&bytes))
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|e| format!("serialize {}: {e}", path.display()))?;
    fs::write(path, bytes).map_err(|e| format!("write {}: {e}", path.display()))
}

fn diff_artifacts(old: &Value, new: &Value) -> Result<Value, String> {
    if old["retained_layout_target"] != new["retained_layout_target"] {
        return Err("old/new retained-layout measurements use different targets".into());
    }
    let layout_deltas = retained_layout_deltas(old, new)?;
    let old_rows = old["records"]
        .as_array()
        .ok_or("old corpus has no records")?;
    let new_rows = new["records"]
        .as_array()
        .ok_or("new corpus has no records")?;
    let old_signature = input_signature(old_rows)?;
    let new_signature = input_signature(new_rows)?;
    if new_signature != old_signature {
        return Err(format!(
            "old/new corpus inputs differ: old {old_signature}, new {new_signature}",
        ));
    }
    let old_by_id = old_rows
        .iter()
        .filter_map(|row| row["record_id"].as_str().map(|id| (id.to_owned(), row)))
        .collect::<HashMap<_, _>>();
    let new_by_id = new_rows
        .iter()
        .filter_map(|row| row["record_id"].as_str().map(|id| (id.to_owned(), row)))
        .collect::<HashMap<_, _>>();
    let mut diff_rows = Vec::new();
    for (record_id, old_record) in &old_by_id {
        let Some(new_record) = new_by_id.get(record_id) else {
            return Err(format!("new corpus omitted input row {record_id}"));
        };
        let old_candidates = old_record["navigation"]["frozen_routes"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let new_candidates = new_record["navigation"]["frozen_routes"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let old_candidate_map = old_candidates
            .iter()
            .filter_map(|row| row["candidate_id"].as_str().map(|id| (id.to_owned(), row)))
            .collect::<HashMap<_, _>>();
        let new_candidate_map = new_candidates
            .iter()
            .filter_map(|row| row["candidate_id"].as_str().map(|id| (id.to_owned(), row)))
            .collect::<HashMap<_, _>>();
        for (candidate_id, old_candidate) in &old_candidate_map {
            let Some(new_candidate) = new_candidate_map.get(candidate_id) else {
                return Err(format!(
                    "new corpus omitted frozen route {record_id}/{candidate_id}"
                ));
            };
            let old_variants = variant_map(&old_candidate["assessments"]);
            let new_variants = variant_map(&new_candidate["assessments"]);
            for (variant_id, old_variant) in &old_variants {
                let Some(new_variant) = new_variants.get(variant_id) else {
                    return Err(format!("new corpus omitted assessment variant {record_id}/{candidate_id}/{variant_id}"));
                };
                let old_assessment = &old_variant["assessment"];
                let new_assessment = &new_variant["assessment"];
                let mut classes = Vec::new();
                let old_permitted = old_assessment["permitted"].as_bool();
                let new_permitted = new_assessment["permitted"].as_bool();
                if old_permitted == Some(false) && new_permitted == Some(true) {
                    classes.push("gained");
                }
                if old_permitted == Some(true) && new_permitted == Some(false) {
                    classes.push("lost");
                }
                if old_assessment["supplies"] != new_assessment["supplies"] {
                    classes.push("changed_food");
                }
                if old_candidate["route_sha256"] != new_candidate["route_sha256"] {
                    classes.push("changed_route");
                }
                let mut other_changes = Vec::new();
                if old_assessment["refusal"] != new_assessment["refusal"] {
                    other_changes.push("refusal");
                }
                if old_assessment["mitigation"] != new_assessment["mitigation"] {
                    other_changes.push("mitigation");
                }
                if old_assessment["bound"] != new_assessment["bound"] {
                    other_changes.push("bound");
                }
                if old_assessment["poison_test"] != new_assessment["poison_test"] {
                    other_changes.push("poison_test");
                }
                if old_assessment["verdict"] != new_assessment["verdict"] && classes.is_empty() {
                    other_changes.push("verdict");
                }
                if !classes.is_empty() || !other_changes.is_empty() {
                    diff_rows.push(json!({
                        "record_id":record_id,
                        "candidate_id":candidate_id,
                        "variant":variant_id,
                        "primary_classes":classes,
                        "other_changes":other_changes,
                        "source":new_record["source"],
                        "input":new_record["input"],
                        "guide_stage_grants":new_record["input"]["combined_stage_grant_names"],
                        "old":old_assessment,
                        "new":new_assessment,
                    }));
                }
            }
        }
        let old_e2e = &old_record["navigation"]["end_to_end"];
        let new_e2e = &new_record["navigation"]["end_to_end"];
        if old_e2e["route_sha256"] != new_e2e["route_sha256"] {
            diff_rows.push(json!({
                "record_id":record_id,
                "candidate_id":"end_to_end",
                "primary_classes":["changed_route"],
                "other_changes":[],
                "source":new_record["source"],
                "input":new_record["input"],
                "guide_stage_grants":new_record["input"]["combined_stage_grant_names"],
                "old":old_e2e,
                "new":new_e2e,
                "reason": {
                    "old_witness": old_e2e["witness"]["reason"],
                    "new_witness": new_e2e["witness"]["reason"],
                },
            }));
        }
    }
    if old_by_id.len() != new_by_id.len() {
        return Err(format!(
            "old/new corpus row counts differ: {} vs {}",
            old_by_id.len(),
            new_by_id.len()
        ));
    }
    let mut classes: BTreeMap<String, usize> = BTreeMap::new();
    for row in &diff_rows {
        if let Some(primary) = row["primary_classes"].as_array() {
            for class in primary.iter().filter_map(Value::as_str) {
                *classes.entry(class.to_owned()).or_default() += 1;
            }
        }
    }
    Ok(json!({
        "schema":"survivable-p1c-diff-v1",
        "old_model":old["model"],
        "new_model":new["model"],
        "input_signature_sha256":new["input_signature_sha256"],
        "input_signature_basis":"materialized inputs and source content identity; content-addressed source paths remain in records but do not distinguish private clones",
        "old_artifact_input_signature_sha256":old["input_signature_sha256"],
        "retained_layout_target":new["retained_layout_target"],
        "retained_layout_deltas_bytes":layout_deltas,
        "row_counts":{
            "old":old_rows.len(),
            "new":new_rows.len(),
            "changed":diff_rows.len(),
            "by_primary_class":classes,
        },
        "classes":["gained","lost","changed_food","changed_route"],
        "lost_admission_policy":"ruling required; never force a pass; preserve guide/stage/zone provenance on each lost row",
        "records":diff_rows,
    }))
}
fn retained_layout_deltas(old: &Value, new: &Value) -> Result<Value, String> {
    let old_sizes = old["retained_layout_sizes_bytes"]
        .as_object()
        .ok_or("old corpus has no retained-layout sizes")?;
    let new_sizes = new["retained_layout_sizes_bytes"]
        .as_object()
        .ok_or("new corpus has no retained-layout sizes")?;
    let type_names = [
        "script::combat::risk::RouteAssessment",
        "host_play::admission::RiskInput",
        "api::condition::BotCondition",
    ];
    let mut deltas = Map::new();
    for type_name in type_names {
        let old_size = old_sizes
            .get(type_name)
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("old retained-layout size missing for {type_name}"))?;
        let new_size = new_sizes
            .get(type_name)
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("new retained-layout size missing for {type_name}"))?;
        let old_signed = i64::try_from(old_size)
            .map_err(|_| format!("old retained-layout size too large for {type_name}"))?;
        let new_signed = i64::try_from(new_size)
            .map_err(|_| format!("new retained-layout size too large for {type_name}"))?;
        let delta = new_signed
            .checked_sub(old_signed)
            .ok_or_else(|| format!("retained-layout delta overflow for {type_name}"))?;
        deltas.insert(
            type_name.to_owned(),
            json!({
                "old_bytes": old_size,
                "new_bytes": new_size,
                "delta_bytes": delta,
            }),
        );
    }
    Ok(Value::Object(deltas))
}

fn variant_map(value: &Value) -> HashMap<String, &Value> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["variant"].as_str().map(|id| (id.to_owned(), row)))
        .collect()
}
