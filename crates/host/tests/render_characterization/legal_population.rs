use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const ENGINE_COMMIT: &str = "cc359656b4acd216ca452495874b6beba9a0ac75";
pub const CONTENT_COMMIT: &str = "92649430fcbc83538d8c4367ecb96cee1a67a944";

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileRecord {
    path: String,
    bytes: u64,
    sha256: String,
}

impl FileRecord {
    fn json(&self) -> Value {
        json!({
            "path": self.path,
            "bytes": self.bytes,
            "sha256": self.sha256,
        })
    }
}

#[derive(Debug)]
pub struct VerifiedEngineInputs {
    engine_root: PathBuf,
    manifest_path: PathBuf,
    manifest_sha256: String,
    engine_commit: String,
    content_commit: String,
    manifest_artifact_count: usize,
    total_engine_bytes: u64,
    files: BTreeMap<String, FileRecord>,
    unbound_manifest_artifacts: Vec<Value>,
}

impl VerifiedEngineInputs {
    fn binding_json(&self) -> Value {
        let source_file_count = self
            .files
            .keys()
            .filter(|path| path.starts_with("src/"))
            .count();
        let pack_file_count = self
            .files
            .keys()
            .filter(|path| path.starts_with("data/pack/"))
            .count();
        let config_file_count = self
            .files
            .keys()
            .filter(|path| path.starts_with("data/config/"))
            .count();
        json!({
            "manifest_path": self.manifest_path.display().to_string(),
            "manifest_sha256": self.manifest_sha256,
            "manifest_artifact_count": self.manifest_artifact_count,
            "engine_commit": self.engine_commit,
            "content_commit": self.content_commit,
            "verified_engine_file_count": self.files.len(),
            "verified_engine_total_bytes": self.total_engine_bytes,
            "verified_source_file_count": source_file_count,
            "verified_pack_file_count": pack_file_count,
            "verified_config_file_count": config_file_count,
            "verified_engine_files": self.files.values().map(FileRecord::json).collect::<Vec<_>>(),
            "manifest_artifacts_outside_engine_root": self.unbound_manifest_artifacts,
            "all_staged_engine_files_match_manifest": true,
        })
    }

    fn read_verified_source(&self, relative: &str) -> Result<Vec<u8>, String> {
        let record = self.files.get(relative).ok_or_else(|| {
            format!("required engine source is absent from the manifest: {relative}")
        })?;
        let bytes = fs::read(self.engine_root.join(relative))
            .map_err(|error| format!("cannot read engine source {relative}: {error}"))?;
        let digest = Sha256::digest(&bytes);
        let sha256 = format!("{digest:x}");
        let length = u64::try_from(bytes.len())
            .map_err(|_| format!("engine source length does not fit u64: {relative}"))?;
        if length != record.bytes || sha256 != record.sha256 {
            return Err(format!(
                "engine source changed after manifest verification: {relative}"
            ));
        }
        Ok(bytes)
    }

    fn source_text(&self, relative: &str) -> Result<String, String> {
        String::from_utf8(self.read_verified_source(relative)?)
            .map_err(|error| format!("engine source is not UTF-8 ({relative}): {error}"))
    }

    fn marker_lines(&self, relative: &str, marker: &str) -> Result<Vec<usize>, String> {
        let source = self.source_text(relative)?;
        let lines = source
            .lines()
            .enumerate()
            .filter_map(|(index, line)| line.contains(marker).then_some(index + 1))
            .collect::<Vec<_>>();
        if lines.is_empty() {
            return Err(format!(
                "engine marker {marker:?} is absent from {relative}"
            ));
        }
        Ok(lines)
    }
}

fn safe_relative_path(path: &str) -> Result<String, String> {
    let path = Path::new(path);
    if path.is_absolute() {
        return Err("runtime manifest contains an absolute engine-relative path".to_string());
    }
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => components.push(
                value
                    .to_str()
                    .ok_or_else(|| "runtime manifest path is not UTF-8".to_string())?,
            ),
            _ => {
                return Err(
                    "runtime manifest contains a non-normal engine-relative path".to_string(),
                )
            }
        }
    }
    if components.is_empty() {
        return Err("runtime manifest contains an empty engine-relative path".to_string());
    }
    Ok(components.join("/"))
}

fn engine_relative_path(path: &str) -> Option<&str> {
    path.split_once("/engine/").map(|(_, relative)| relative)
}

fn collect_files(root: &Path) -> Result<BTreeSet<String>, String> {
    fn visit(root: &Path, current: &Path, output: &mut BTreeSet<String>) -> Result<(), String> {
        let entries = fs::read_dir(current).map_err(|error| {
            format!(
                "cannot enumerate staged engine path {}: {error}",
                current.display()
            )
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!(
                    "cannot read staged engine directory {}: {error}",
                    current.display()
                )
            })?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|error| {
                format!(
                    "cannot inspect staged engine path {}: {error}",
                    path.display()
                )
            })?;
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "staged engine input is a symlink: {}",
                    path.display()
                ));
            }
            if metadata.is_dir() {
                visit(root, &path, output)?;
                continue;
            }
            if !metadata.is_file() {
                return Err(format!(
                    "staged engine input is not a regular file: {}",
                    path.display()
                ));
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|error| {
                    format!(
                        "cannot relativize staged engine path {}: {error}",
                        path.display()
                    )
                })?
                .to_str()
                .ok_or_else(|| format!("staged engine path is not UTF-8: {}", path.display()))?
                .replace('\\', "/");
            let relative = safe_relative_path(&relative)?;
            if !output.insert(relative.clone()) {
                return Err(format!("duplicate staged engine path: {relative}"));
            }
        }
        Ok(())
    }

    let metadata = fs::symlink_metadata(root).map_err(|error| {
        format!(
            "cannot inspect staged engine root {}: {error}",
            root.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "staged engine root is not a real directory: {}",
            root.display()
        ));
    }
    let mut files = BTreeSet::new();
    visit(root, root, &mut files)?;
    Ok(files)
}

fn hash_file(path: &Path) -> Result<(u64, String), String> {
    let mut file = File::open(path).map_err(|error| {
        format!(
            "cannot open staged engine input {}: {error}",
            path.display()
        )
    })?;
    let mut digest = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|error| {
            format!(
                "cannot read staged engine input {}: {error}",
                path.display()
            )
        })?;
        if count == 0 {
            break;
        }
        bytes = bytes
            .checked_add(u64::try_from(count).map_err(|_| "file read length does not fit u64")?)
            .ok_or_else(|| format!("staged engine byte count overflowed: {}", path.display()))?;
        digest.update(&buffer[..count]);
    }
    Ok((bytes, format!("{:x}", digest.finalize())))
}

pub fn verify_staged_engine(
    engine_root: &Path,
    manifest_path: &Path,
) -> Result<VerifiedEngineInputs, String> {
    let manifest_bytes = fs::read(manifest_path).map_err(|error| {
        format!(
            "cannot read runtime input manifest {}: {error}",
            manifest_path.display()
        )
    })?;
    let manifest_sha256 = format!("{:x}", Sha256::digest(&manifest_bytes));
    let manifest: Value = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("cannot parse runtime input manifest: {error}"))?;
    let engine_commit = manifest
        .get("engine_commit")
        .and_then(Value::as_str)
        .ok_or_else(|| "runtime input manifest has no engine_commit".to_string())?;
    if engine_commit != ENGINE_COMMIT {
        return Err(format!(
            "runtime input manifest engine commit is {engine_commit}, expected {ENGINE_COMMIT}"
        ));
    }
    let content_commit = manifest
        .get("content_commit")
        .and_then(Value::as_str)
        .ok_or_else(|| "runtime input manifest has no content_commit".to_string())?;
    if content_commit != CONTENT_COMMIT {
        return Err(format!(
            "runtime input manifest content commit is {content_commit}, expected {CONTENT_COMMIT}"
        ));
    }
    let artifacts = manifest
        .get("artifacts")
        .and_then(Value::as_array)
        .ok_or_else(|| "runtime input manifest has no artifacts array".to_string())?;
    let mut expected = BTreeMap::<String, (u64, String)>::new();
    let mut unbound_manifest_artifacts = Vec::new();
    for artifact in artifacts {
        let path = artifact
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| "runtime input artifact has no path".to_string())?;
        let bytes = artifact
            .get("bytes")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("runtime input artifact has invalid byte size: {path}"))?;
        let sha256 = artifact
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("runtime input artifact has no SHA-256: {path}"))?;
        let Some(relative) = engine_relative_path(path) else {
            unbound_manifest_artifacts.push(json!({
                "path": path,
                "bytes": bytes,
                "sha256": sha256,
                "status": "outside_staged_engine_root_not_consumed_by_legal_census",
            }));
            continue;
        };
        let relative = safe_relative_path(relative)?;
        if !(relative.starts_with("src/") || relative.starts_with("data/")) {
            return Err(format!(
                "manifest engine artifact is outside src/ and data/: {relative}"
            ));
        }
        if expected
            .insert(relative.clone(), (bytes, sha256.to_string()))
            .is_some()
        {
            return Err(format!(
                "runtime input manifest repeats engine artifact: {relative}"
            ));
        }
    }
    if !expected.keys().any(|path| path.starts_with("src/")) {
        return Err("runtime input manifest has no staged engine sources".to_string());
    }
    if !expected.keys().any(|path| path.starts_with("data/pack/")) {
        return Err("runtime input manifest has no staged engine pack files".to_string());
    }

    let actual = collect_files(engine_root)?;
    for path in expected.keys() {
        if !actual.contains(path) {
            return Err(format!(
                "manifest engine input is missing from stage: {path}"
            ));
        }
    }
    for path in &actual {
        if !expected.contains_key(path) {
            return Err(format!(
                "staged engine input is absent from manifest: {path}"
            ));
        }
    }

    let mut files = BTreeMap::new();
    let mut total_engine_bytes = 0u64;
    for (relative, (expected_bytes, expected_sha256)) in expected {
        let (bytes, sha256) = hash_file(&engine_root.join(&relative))?;
        if bytes != expected_bytes {
            return Err(format!(
                "staged engine input size mismatch for {relative}: manifest={expected_bytes}, stage={bytes}"
            ));
        }
        if sha256 != expected_sha256 {
            return Err(format!("staged engine input SHA-256 mismatch: {relative}"));
        }
        total_engine_bytes = total_engine_bytes
            .checked_add(bytes)
            .ok_or_else(|| "verified staged engine total byte count overflowed".to_string())?;
        files.insert(
            relative.clone(),
            FileRecord {
                path: relative,
                bytes,
                sha256,
            },
        );
    }

    Ok(VerifiedEngineInputs {
        engine_root: engine_root.to_path_buf(),
        manifest_path: manifest_path.to_path_buf(),
        manifest_sha256,
        engine_commit: engine_commit.to_string(),
        content_commit: content_commit.to_string(),
        manifest_artifact_count: artifacts.len(),
        total_engine_bytes,
        files,
        unbound_manifest_artifacts,
    })
}

fn numeric_after(source: &str, marker: &str) -> Result<usize, String> {
    let line = source
        .lines()
        .find(|line| line.contains(marker))
        .ok_or_else(|| format!("missing source declaration {marker:?}"))?;
    let (_, remainder) = line
        .split_once(marker)
        .ok_or_else(|| format!("invalid source declaration {marker:?}"))?;
    let digits = remainder
        .trim_start()
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '_')
        .filter(|&character| character != '_')
        .collect::<String>();
    if digits.is_empty() {
        return Err(format!(
            "source declaration has no integer value: {marker:?}"
        ));
    }
    digits
        .parse::<usize>()
        .map_err(|error| format!("invalid integer in source declaration {marker:?}: {error}"))
}

fn player_slot_capacity(world: &str) -> Result<(usize, usize, usize), String> {
    let method = world
        .split_once("getNextPlayerSlot(): number {")
        .map(|(_, body)| body)
        .ok_or_else(|| "World.getNextPlayerSlot is absent".to_string())?;
    let line = method
        .lines()
        .find(|line| line.contains("for (let i = "))
        .ok_or_else(|| "World.getNextPlayerSlot has no slot scan".to_string())?
        .trim();
    let range = line
        .strip_prefix("for (let i = ")
        .and_then(|tail| tail.split_once("; i < "))
        .ok_or_else(|| "World.getNextPlayerSlot slot loop changed".to_string())?;
    let start = range
        .0
        .trim()
        .parse::<usize>()
        .map_err(|error| format!("invalid first player slot: {error}"))?;
    let (end_text, _) = range
        .1
        .split_once("; i++")
        .ok_or_else(|| "World.getNextPlayerSlot loop has no increment".to_string())?;
    let end = end_text
        .trim()
        .parse::<usize>()
        .map_err(|error| format!("invalid exclusive player slot bound: {error}"))?;
    let count = end
        .checked_sub(start)
        .ok_or_else(|| "World.getNextPlayerSlot range is reversed".to_string())?;
    Ok((start, end, count))
}

fn require_markers(source: &str, path: &str, markers: &[&str]) -> Result<(), String> {
    for marker in markers {
        if !source.contains(marker) {
            return Err(format!(
                "required source marker {marker:?} is absent from {path}"
            ));
        }
    }
    Ok(())
}

struct EmitterSpec {
    name: &'static str,
    path: &'static str,
    markers: &'static [&'static str],
    effect: &'static str,
    population_bound: &'static str,
}

const EMITTERS: &[EmitterSpec] = &[
    EmitterSpec {
        name: "LOC_ADD",
        path: "src/engine/script/handlers/LocOps.ts",
        markers: &["[ScriptOpcode.LOC_ADD]:", "World.addLoc(created, duration);"],
        effect: "Adds or changes a dynamic location in a zone.",
        population_bound: "At most one client node per supported level, tile, and mapped location layer; see location_overlay_capacity.",
    },
    EmitterSpec {
        name: "NPC_ADD",
        path: "src/engine/script/handlers/NpcOps.ts",
        markers: &["[ScriptOpcode.NPC_ADD]:", "World.addNpc(npc, duration);"],
        effect: "Adds a dynamic NPC to the world NPC list.",
        population_bound: "The shared world NPC slot limit bounds all static and dynamic NPCs.",
    },
    EmitterSpec {
        name: "SPOTANIM_NPC",
        path: "src/engine/script/handlers/NpcOps.ts",
        markers: &["[ScriptOpcode.SPOTANIM_NPC]:", "state.activeNpc.spotanim("],
        effect: "Attaches a spot animation to the active NPC.",
        population_bound: "One client spotanim field per actor; the per-view actor upper bounds simultaneous actor-attached spot animations.",
    },
    EmitterSpec {
        name: "SPOTANIM_PL",
        path: "src/engine/script/handlers/PlayerOps.ts",
        markers: &["[ScriptOpcode.SPOTANIM_PL]:", "state.activePlayer.spotanim("],
        effect: "Attaches a spot animation to the active player.",
        population_bound: "One client spotanim field per actor; the per-view actor upper bounds simultaneous actor-attached spot animations.",
    },
    EmitterSpec {
        name: "SPOTANIM_MAP",
        path: "src/engine/script/handlers/ServerOps.ts",
        markers: &["[ScriptOpcode.SPOTANIM_MAP]:", "World.animMap("],
        effect: "Queues a map spot animation in a server zone.",
        population_bound: "Compiled instruction occurrences are counted; the client list has no cap and per-cycle emission remains unqualified.",
    },
    EmitterSpec {
        name: "PROJANIM_PL",
        path: "src/engine/script/handlers/ServerOps.ts",
        markers: &["[ScriptOpcode.PROJANIM_PL]:", "World.mapProjAnim("],
        effect: "Queues a map projectile targeted at a player.",
        population_bound: "The client list has no admission cap; the complete pinned compiled script pack is checked separately for an emitter opcode.",
    },
    EmitterSpec {
        name: "PROJANIM_NPC",
        path: "src/engine/script/handlers/ServerOps.ts",
        markers: &["[ScriptOpcode.PROJANIM_NPC]:", "World.mapProjAnim("],
        effect: "Queues a map projectile targeted at an NPC.",
        population_bound: "The client list has no admission cap; the complete pinned compiled script pack is checked separately for an emitter opcode.",
    },
    EmitterSpec {
        name: "PROJANIM_MAP",
        path: "src/engine/script/handlers/ServerOps.ts",
        markers: &["[ScriptOpcode.PROJANIM_MAP]:", "World.mapProjAnim("],
        effect: "Queues a map projectile between map coordinates.",
        population_bound: "The client list has no admission cap; the complete pinned compiled script pack is checked separately for an emitter opcode.",
    },
    EmitterSpec {
        name: "MAP_LOCADDUNSAFE",
        path: "src/engine/script/handlers/ServerOps.ts",
        markers: &["[ScriptOpcode.MAP_LOCADDUNSAFE]:", "getAllLocsUnsafe()"],
        effect: "Queues dynamic map-location updates around the requested zone.",
        population_bound: "Bounded by the client current-level coalescing key over supported levels, tiles, and mapped location layers.",
    },
];

fn inventory_emitters_from_sources(
    sources: &BTreeMap<&'static str, String>,
) -> Result<Vec<Value>, String> {
    let mut inventory = Vec::with_capacity(EMITTERS.len());
    for spec in EMITTERS {
        let source = sources
            .get(spec.path)
            .ok_or_else(|| format!("emitter source is missing: {}", spec.path))?;
        let markers = spec
            .markers
            .iter()
            .map(|marker| {
                let lines = source
                    .lines()
                    .enumerate()
                    .filter_map(|(index, line)| line.contains(marker).then_some(index + 1))
                    .collect::<Vec<_>>();
                if lines.is_empty() {
                    return Err(format!(
                        "required emitter marker {marker:?} is absent from {}",
                        spec.path
                    ));
                }
                Ok(json!({ "marker": marker, "lines": lines }))
            })
            .collect::<Result<Vec<_>, String>>()?;
        inventory.push(json!({
            "emitter": spec.name,
            "source": spec.path,
            "markers": markers,
            "effect": spec.effect,
            "population_bound": spec.population_bound,
        }));
    }
    Ok(inventory)
}

fn emitter_inventory(inputs: &VerifiedEngineInputs) -> Result<Vec<Value>, String> {
    let mut sources = BTreeMap::new();
    for spec in EMITTERS {
        if let std::collections::btree_map::Entry::Vacant(entry) = sources.entry(spec.path) {
            entry.insert(inputs.source_text(spec.path)?);
        }
    }
    inventory_emitters_from_sources(&sources)
}

fn client_source_text<'a>(
    sources: &'a BTreeMap<&'static str, Vec<u8>>,
    relative: &'static str,
) -> Result<&'a str, String> {
    let bytes = sources
        .get(relative)
        .ok_or_else(|| format!("required client authority source is missing: {relative}"))?;
    std::str::from_utf8(bytes)
        .map_err(|error| format!("client authority source is not UTF-8 ({relative}): {error}"))
}

fn first_call_arguments(source: &str, call: &str) -> Result<Vec<String>, String> {
    let mut arguments = Vec::new();
    for (start, _) in source.match_indices(call) {
        let tail = &source[start + call.len()..];
        let end = tail
            .find(',')
            .or_else(|| tail.find(')'))
            .ok_or_else(|| format!("call has no first argument terminator: {call}"))?;
        arguments.push(tail[..end].trim().to_string());
    }
    Ok(arguments)
}

fn verify_location_change_guards(source: &str) -> Result<(), String> {
    use syn::visit::Visit;

    let syntax = syn::parse_file(source)
        .map_err(|error| format!("cannot parse client location-change authority: {error}"))?;
    let expected_guard: syn::Expr = syn::parse_str(
        "shape < LOC_SHAPE_TO_LAYER.len() as i32 && x >= 0 && z >= 0 && x < BuildArea::SIZE && z < BuildArea::SIZE",
    )
    .expect("location-change guard is valid Rust");
    struct Calls<'a> {
        expected_guard: &'a syn::Expr,
        guarded: bool,
        total: usize,
        unguarded: usize,
    }
    impl<'ast> Visit<'ast> for Calls<'_> {
        fn visit_expr_if(&mut self, node: &'ast syn::ExprIf) {
            self.visit_expr(&node.cond);
            let previous = self.guarded;
            self.guarded |= node.cond.as_ref() == self.expected_guard;
            self.visit_block(&node.then_branch);
            self.guarded = previous;
            if let Some((_, branch)) = &node.else_branch {
                self.visit_expr(branch);
            }
        }

        fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
            if node.method == "loc_change_create" {
                self.total += 1;
                self.unguarded += usize::from(!self.guarded);
            }
            syn::visit::visit_expr_method_call(self, node);
        }
    }
    let mut calls = Calls {
        expected_guard: &expected_guard,
        guarded: false,
        total: 0,
        unguarded: 0,
    };
    calls.visit_file(&syntax);
    if calls.total != 3 || calls.unguarded != 0 {
        return Err(format!(
            "client location-change guards cover {} of {} calls; expected three guarded calls",
            calls.total - calls.unguarded,
            calls.total
        ));
    }
    Ok(())
}

fn verify_client_authorities(
    sources: &BTreeMap<&'static str, Vec<u8>>,
    location_shape_layers: &[i32],
) -> Result<usize, String> {
    let client_build = client_source_text(
        sources,
        "vendor/fr-client-rust/crates/client/src/client/client.rs",
    )?;
    let client_draw = client_source_text(
        sources,
        "vendor/fr-client-rust/crates/client/src/render/draw.rs",
    )?;
    let client_loc_shape = client_source_text(
        sources,
        "vendor/fr-client-rust/crates/client/src/dash3d/loc_shape.rs",
    )?;
    let client_map_spot = client_source_text(
        sources,
        "vendor/fr-client-rust/crates/client/src/dash3d/map_spot_anim.rs",
    )?;
    let client_link_list = client_source_text(
        sources,
        "vendor/fr-client-rust/crates/client/src/datastruct/link_list.rs",
    )?;
    let client_arena = client_source_text(
        sources,
        "vendor/fr-client-rust/crates/client/src/datastruct/linkable.rs",
    )?;
    let client_zone = client_source_text(
        sources,
        "vendor/fr-client-rust/crates/client/src/client/zone_289.rs",
    )?;
    let client_build_area = client_source_text(
        sources,
        "vendor/fr-client-rust/crates/client/src/dash3d/collision_map.rs",
    )?;
    let client_actor = client_source_text(
        sources,
        "vendor/fr-client-rust/crates/client/src/client/actor_289.rs",
    )?;
    let client_entity = client_source_text(
        sources,
        "vendor/fr-client-rust/crates/client/src/dash3d/client_entity.rs",
    )?;

    require_markers(
        client_draw,
        "vendor/fr-client-rust/crates/client/src/render/draw.rs",
        &[
            "let count = if add_self { 1 } else { client.player_count };",
            "client.local_player.as_mut()",
            "client.loop_cycle > proj.t2",
            "spot.anim_complete",
        ],
    )?;
    require_markers(
        client_build,
        "vendor/fr-client-rust/crates/client/src/client/client.rs",
        &[
            "pub loc_changes: LinkList<LocChange>",
            "pub projectiles: LinkList<ClientProj>",
            "pub spotanims: LinkList<MapSpotAnim>",
            "self.projectiles.push(proj);",
            "self.spotanims.push(MapSpotAnim::new(",
            "if loc.level == self.minusedlevel && loc.x == x && loc.z == z && loc.layer == layer",
            "self.loc_changes.push(loc);",
            "self.minusedlevel = buf.gbit(2);",
            "fn loc_change_create(",
        ],
    )?;
    let build_calls = first_call_arguments(client_build, "self.loc_change_create(")?;
    if build_calls.len() != 3
        || build_calls
            .iter()
            .any(|argument| argument != "self.minusedlevel")
    {
        return Err(format!(
            "client.rs location-change callers do not all use minusedlevel: {build_calls:?}"
        ));
    }
    verify_location_change_guards(client_build)?;
    require_markers(
        client_zone,
        "vendor/fr-client-rust/crates/client/src/client/zone_289.rs",
        &[
            "let level = c.minusedlevel;",
            "fn tile(x: i32, z: i32) -> bool {",
            "(0..BuildArea::SIZE).contains(&x) && (0..BuildArea::SIZE).contains(&z)",
            "assert!(tile(x, z), \"invalid merge tile\");",
            "if tile(x, z) {",
        ],
    )?;
    let zone_calls = first_call_arguments(client_zone, "c.loc_change_create(")?;
    if zone_calls.len() != 2 || zone_calls[0] != "level" || zone_calls[1] != "c.minusedlevel" {
        return Err(format!(
            "zone_289 location-change callers do not use the current level: {zone_calls:?}"
        ));
    }
    require_markers(
        client_build_area,
        "vendor/fr-client-rust/crates/client/src/dash3d/collision_map.rs",
        &[
            "pub const LEVELS: i32 = 4;",
            "pub const SIZE: i32 = 13 << 3;",
        ],
    )?;
    require_markers(
        client_actor,
        "vendor/fr-client-rust/crates/client/src/client/actor_289.rs",
        &["frame.plane = Some(bit(p, 2));", "c.minusedlevel = plane;"],
    )?;
    require_markers(
        client_entity,
        "vendor/fr-client-rust/crates/client/src/dash3d/client_entity.rs",
        &[
            "pub spotanim_id: i32,",
            "pub spotanim_frame: i32,",
            "spotanim_id: -1,",
            "spotanim_frame: 0,",
        ],
    )?;
    require_markers(
        client_loc_shape,
        "vendor/fr-client-rust/crates/client/src/dash3d/loc_shape.rs",
        &["pub const LOC_SHAPE_TO_LAYER: [i32; 23] = ["],
    )?;
    require_markers(
        client_map_spot,
        "vendor/fr-client-rust/crates/client/src/dash3d/map_spot_anim.rs",
        &[
            "let Some(seq) = cache.spot(self.spotanim as usize).seq else {",
            "return;",
            "self.anim_complete = true;",
        ],
    )?;
    require_markers(
        client_link_list,
        "vendor/fr-client-rust/crates/client/src/datastruct/link_list.rs",
        &[
            "pub struct LinkList<T: LinkableTrait>",
            "pub fn push(&mut self, node: T)",
            "self.arena.alloc(node)",
        ],
    )?;
    require_markers(
        client_arena,
        "vendor/fr-client-rust/crates/client/src/datastruct/linkable.rs",
        &[
            "pub struct Arena<T: LinkableTrait>",
            "self.nodes.push(Some(node));",
        ],
    )?;
    let unique_location_layers = location_shape_layers
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let location_layers = unique_location_layers.len();
    if unique_location_layers.iter().any(|layer| *layer < 0) {
        return Err("client location-shape mapping contains a negative layer".to_string());
    }
    if location_layers == 0 {
        return Err("client location-shape mapping has no layers".to_string());
    }
    Ok(location_layers)
}
fn verify_client_dynamic_sprite_capacity(
    sources: &BTreeMap<&'static str, Vec<u8>>,
) -> Result<Value, String> {
    const CORE_WORLD: &str = "vendor/fr-client-rust/crates/client/src/core/world.rs";
    const RENDER_WORLD: &str = "vendor/fr-client-rust/crates/client/src/render/world.rs";
    const DRAW: &str = "vendor/fr-client-rust/crates/client/src/render/draw.rs";

    let core_world = client_source_text(sources, CORE_WORLD)?;
    let render_world = client_source_text(sources, RENDER_WORLD)?;
    let draw = client_source_text(sources, DRAW)?;
    require_markers(
        core_world,
        CORE_WORLD,
        &[
            "pub const MAX_DYNAMIC_SPRITES: usize =",
            "pub fn add_dynamic(",
            "pub fn add_dynamic2(",
            "self.dynamic_sprites = vec![None; MAX_DYNAMIC_SPRITES];",
            "self.dynamic_sprites[self.dynamic_count as usize] = Some(index);",
            "self.dynamic_count += 1;",
        ],
    )?;
    require_markers(
        render_world,
        RENDER_WORLD,
        &[
            "let Some(budget) = self.head_misc.as_ref() else {",
            "if world.dynamic_count as usize >= crate::core::world::MAX_DYNAMIC_SPRITES {",
            "return Err(RenderLimit::new(",
            "world.dynamic_sprites.resize(crate::core::world::MAX_DYNAMIC_SPRITES, None);",
        ],
    )?;
    let mut events = Vec::new();
    for (needle, event) in [
        (
            "self.world.prepare_dynamic_storage(&mut client.world)?;",
            "actor_preflight",
        ),
        (
            "self.world.prepare_dynamic_storage(world)?;",
            "transient_preflight",
        ),
        (".add_dynamic(", "dynamic_add"),
        (".add_dynamic2(", "dynamic_add2"),
    ] {
        events.extend(
            draw.match_indices(needle)
                .map(|(offset, _)| (offset, event)),
        );
    }
    events.sort_unstable_by_key(|(offset, _)| *offset);
    let event_order = events.iter().map(|(_, event)| *event).collect::<Vec<_>>();
    let expected_event_order = [
        "actor_preflight",
        "dynamic_add",
        "dynamic_add2",
        "actor_preflight",
        "dynamic_add",
        "transient_preflight",
        "dynamic_add",
        "transient_preflight",
        "dynamic_add",
    ];
    if event_order != expected_event_order.to_vec() {
        return Err(format!(
            "dynamic draw admissions no longer match the preflighted player/player-location/NPC/projectile/map-spot sequence: {event_order:?}"
        ));
    }
    let mut indexed_client_source_count = 0;
    for (path, bytes) in sources {
        if !path.starts_with("vendor/fr-client-rust/crates/client/src/") || *path == DRAW {
            continue;
        }
        indexed_client_source_count += 1;
        let source = std::str::from_utf8(bytes)
            .map_err(|error| format!("client source {path} is not UTF-8: {error}"))?;
        let production_source = source.split("#[cfg(test)]").next().unwrap_or(source);
        if production_source.contains(".add_dynamic(")
            || production_source.contains(".add_dynamic2(")
        {
            return Err(format!(
                "dynamic sprite insertion outside render/draw.rs lacks a counted admission path: {path}"
            ));
        }
    }
    let preflight_count = events
        .iter()
        .filter(|(_, event)| event.ends_with("_preflight"))
        .count();
    let add_count = events
        .iter()
        .filter(|(_, event)| *event == "dynamic_add")
        .count();
    let add2_count = events
        .iter()
        .filter(|(_, event)| *event == "dynamic_add2")
        .count();
    let capacity = numeric_after(core_world, "pub const MAX_DYNAMIC_SPRITES: usize =")?;
    if capacity != client::core::world::MAX_DYNAMIC_SPRITES {
        return Err(format!(
            "source MAX_DYNAMIC_SPRITES {capacity} differs from the compiled client limit {}",
            client::core::world::MAX_DYNAMIC_SPRITES
        ));
    }
    Ok(json!({
        "max_dynamic_sprites_per_client_world": capacity,
        "all_dynamic_draw_adds_preflighted": true,
        "preflight_call_count": preflight_count,
        "draw_add_count": add_count,
        "draw_add2_count": add2_count,
        "indexed_client_source_entries_scanned_for_extra_insertions": indexed_client_source_count,
        "bound_scope": "successfully admitted dynamic sprite indices in a client World; RenderWorld.remove_sprites resets the count",
        "typed_overflow_behavior": "With an active HeadMisc budget, prepare_dynamic_storage returns RenderLimit at the fixed slot capacity. Without that budget the direct fixed-slot write is not a typed rejection; the census does not claim a safe over-capacity path.",
        "does_not_bound_client_transient_list_nodes": true,
        "source_files": [
            {"path": CORE_WORLD, "lines": [34, 542, 578, 590, 599, 612, 624, 982, 984, 986, 987]},
            {"path": RENDER_WORLD, "lines": [692, 694, 697, 698, 700, 701, 705, 714, 959, 972]},
            {"path": DRAW, "lines": [916, 930, 945, 1024, 1025, 1056, 1057, 1108, 1110]},
        ],
    }))
}

fn parse_script_opcode_values(source: &str) -> Result<BTreeMap<String, u16>, String> {
    let (_, body) = source
        .split_once("export const enum ScriptOpcode {")
        .ok_or_else(|| "ScriptOpcode enum declaration is absent".to_string())?;
    let mut values = BTreeMap::new();
    let mut current = None::<u32>;
    let mut closed = false;
    for line in body.lines() {
        let code = line.split("//").next().unwrap_or_default().trim();
        if code.starts_with('}') {
            closed = true;
            break;
        }
        let code = code.trim_end_matches(',').trim();
        if code.is_empty() {
            continue;
        }
        let (name, value) = if let Some((name, value)) = code.split_once('=') {
            let value = value
                .trim()
                .parse::<u32>()
                .map_err(|error| format!("invalid ScriptOpcode value for {name}: {error}"))?;
            (name.trim(), value)
        } else {
            let value = current
                .and_then(|value| value.checked_add(1))
                .ok_or_else(|| format!("ScriptOpcode sequence is invalid at {code:?}"))?;
            (code, value)
        };
        if name.is_empty()
            || !name.chars().all(|character| {
                character.is_ascii_uppercase() || character == '_' || character.is_ascii_digit()
            })
        {
            return Err(format!("invalid ScriptOpcode member {name:?}"));
        }
        let value = u16::try_from(value)
            .map_err(|_| format!("ScriptOpcode value does not fit u16: {name}={value}"))?;
        if values.insert(name.to_string(), value).is_some() {
            return Err(format!("duplicate ScriptOpcode member: {name}"));
        }
        current = Some(u32::from(value));
    }
    if !closed {
        return Err("ScriptOpcode enum has no closing brace".to_string());
    }
    Ok(values)
}

fn script_opcode_member_lines(
    source: &str,
    names: &[&str],
) -> Result<BTreeMap<String, usize>, String> {
    let (prefix, body) = source
        .split_once("export const enum ScriptOpcode {")
        .ok_or_else(|| "ScriptOpcode enum declaration is absent".to_string())?;
    let first_line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let mut lines = BTreeMap::new();
    let mut closed = false;
    for (offset, line) in body.lines().enumerate() {
        let code = line
            .split("//")
            .next()
            .unwrap_or_default()
            .trim()
            .trim_end_matches(',')
            .trim();
        if code.starts_with('}') {
            closed = true;
            break;
        }
        let name = code.split_once('=').map_or(code, |(name, _)| name.trim());
        if names.contains(&name) {
            let line_number = first_line + offset;
            if lines.insert(name.to_string(), line_number).is_some() {
                return Err(format!("duplicate ScriptOpcode source member: {name}"));
            }
        }
    }
    if !closed {
        return Err("ScriptOpcode enum has no closing brace".to_string());
    }
    if lines.len() != names.len() {
        let missing = names
            .iter()
            .filter(|name| !lines.contains_key(**name))
            .copied()
            .collect::<Vec<_>>();
        return Err(format!(
            "ScriptOpcode source members are missing: {missing:?}"
        ));
    }
    Ok(lines)
}

fn take_u16(bytes: &[u8], cursor: &mut usize, label: &str) -> Result<u16, String> {
    let end = cursor
        .checked_add(2)
        .ok_or_else(|| format!("{label}: offset overflow"))?;
    let value = bytes
        .get(*cursor..end)
        .ok_or_else(|| format!("{label}: truncated u16 at byte {}", *cursor))?;
    *cursor = end;
    Ok(u16::from_be_bytes([value[0], value[1]]))
}

fn take_u32(bytes: &[u8], cursor: &mut usize, label: &str) -> Result<u32, String> {
    let end = cursor
        .checked_add(4)
        .ok_or_else(|| format!("{label}: offset overflow"))?;
    let value = bytes
        .get(*cursor..end)
        .ok_or_else(|| format!("{label}: truncated u32 at byte {}", *cursor))?;
    *cursor = end;
    Ok(u32::from_be_bytes([value[0], value[1], value[2], value[3]]))
}

fn take_c_string<'a>(bytes: &'a [u8], cursor: &mut usize, label: &str) -> Result<&'a [u8], String> {
    let tail = bytes
        .get(*cursor..)
        .ok_or_else(|| format!("{label}: string offset is outside the record"))?;
    let length = tail
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| format!("{label}: unterminated string"))?;
    let end = cursor
        .checked_add(length + 1)
        .ok_or_else(|| format!("{label}: string end overflow"))?;
    let value = &bytes[*cursor..end - 1];
    *cursor = end;
    Ok(value)
}

fn script_string(bytes: &[u8]) -> String {
    bytes.iter().copied().map(char::from).collect()
}

#[derive(Default)]
struct DecodedScript {
    id: usize,
    name: String,
    source_file: String,
    emitter_counts: BTreeMap<String, usize>,
}

fn decode_script_record(
    id: usize,
    bytes: &[u8],
    push_constant_string: u16,
    small_operand_opcodes: &BTreeSet<u16>,
    emitter_names: &BTreeMap<u16, &'static str>,
) -> Result<DecodedScript, String> {
    if bytes.len() < 16 {
        return Err(format!("script {id} is shorter than the minimum 16 bytes"));
    }
    let trailer_length = usize::from(u16::from_be_bytes([
        bytes[bytes.len() - 2],
        bytes[bytes.len() - 1],
    ]));
    let trailer_position = bytes
        .len()
        .checked_sub(trailer_length + 14)
        .ok_or_else(|| format!("script {id} has an invalid trailer length"))?;
    if trailer_position >= bytes.len() {
        return Err(format!("script {id} trailer starts outside its record"));
    }
    let mut trailer_cursor = trailer_position;
    let _declared_instruction_count = take_u32(bytes, &mut trailer_cursor, "script trailer")?;
    for _ in 0..4 {
        let _ = take_u16(bytes, &mut trailer_cursor, "script trailer locals")?;
    }
    let switch_count = *bytes
        .get(trailer_cursor)
        .ok_or_else(|| format!("script {id} has a truncated switch count"))?;
    trailer_cursor += 1;
    for _ in 0..switch_count {
        let entries = usize::from(take_u16(bytes, &mut trailer_cursor, "script switch table")?);
        let table_bytes = entries
            .checked_mul(8)
            .ok_or_else(|| format!("script {id} switch table length overflowed"))?;
        trailer_cursor = trailer_cursor
            .checked_add(table_bytes)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| format!("script {id} has a truncated switch table"))?;
    }
    if trailer_cursor != bytes.len() - 2 {
        return Err(format!(
            "script {id} trailer does not end before its length field"
        ));
    }

    let mut cursor = 0usize;
    let name = script_string(take_c_string(bytes, &mut cursor, "script name")?);
    let source_file = script_string(take_c_string(bytes, &mut cursor, "script source path")?);
    let _lookup_key = take_u32(bytes, &mut cursor, "script lookup key")?;
    let parameter_count = usize::from(
        *bytes
            .get(cursor)
            .ok_or_else(|| format!("script {id} has no parameter count"))?,
    );
    cursor = cursor
        .checked_add(1 + parameter_count)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| format!("script {id} has a truncated parameter list"))?;
    let line_count = usize::from(take_u16(bytes, &mut cursor, "script line table")?);
    let line_bytes = line_count
        .checked_mul(8)
        .ok_or_else(|| format!("script {id} line table length overflowed"))?;
    cursor = cursor
        .checked_add(line_bytes)
        .filter(|end| *end <= trailer_position)
        .ok_or_else(|| format!("script {id} line table overlaps its opcode trailer"))?;

    let mut emitter_counts = BTreeMap::<String, usize>::new();
    while trailer_position > cursor {
        let opcode = take_u16(bytes, &mut cursor, "script opcode")?;
        if let Some(name) = emitter_names.get(&opcode) {
            let count = emitter_counts.entry((*name).to_string()).or_default();
            *count = count
                .checked_add(1)
                .ok_or_else(|| format!("script {id} opcode count overflowed"))?;
        }
        if opcode == push_constant_string {
            let _ = take_c_string(bytes, &mut cursor, "script string operand")?;
        } else {
            let operand_bytes = if opcode > 100 || small_operand_opcodes.contains(&opcode) {
                1
            } else {
                4
            };
            cursor = cursor
                .checked_add(operand_bytes)
                .filter(|end| *end <= bytes.len())
                .ok_or_else(|| format!("script {id} opcode {opcode} has a truncated operand"))?;
        }
    }
    Ok(DecodedScript {
        id,
        name,
        source_file,
        emitter_counts,
    })
}

const SCRIPT_EMITTER_NAMES: &[&str] = &[
    "LOC_ADD",
    "NPC_ADD",
    "SPOTANIM_NPC",
    "SPOTANIM_PL",
    "SPOTANIM_MAP",
    "PROJANIM_PL",
    "PROJANIM_NPC",
    "PROJANIM_MAP",
    "MAP_LOCADDUNSAFE",
];

fn scan_script_pack(
    dat: &[u8],
    idx: &[u8],
    compiler_version: usize,
    opcode_values: &BTreeMap<String, u16>,
) -> Result<Value, String> {
    let label = "data/pack/server/script.dat";
    let mut data_cursor = 0usize;
    let entries = usize::try_from(take_u32(dat, &mut data_cursor, label)?)
        .map_err(|_| "script entry count does not fit usize".to_string())?;
    let version = usize::try_from(take_u32(dat, &mut data_cursor, label)?)
        .map_err(|_| "script compiler version does not fit usize".to_string())?;
    if version != compiler_version {
        return Err(format!(
            "compiled scripts use compiler version {version}, but ScriptProvider expects {compiler_version}"
        ));
    }
    let mut index_header_cursor = 0usize;
    let index_entries = usize::try_from(take_u32(
        idx,
        &mut index_header_cursor,
        "data/pack/server/script.idx",
    )?)
    .map_err(|_| "script index entry count does not fit usize".to_string())?;
    if index_entries != entries {
        return Err(format!(
            "script.dat has {entries} entries, but script.idx declares {index_entries}"
        ));
    }
    let expected_index_bytes = entries
        .checked_mul(4)
        .and_then(|size| size.checked_add(4))
        .ok_or_else(|| "script index size overflowed".to_string())?;
    if idx.len() != expected_index_bytes {
        return Err(format!(
            "script.idx has {} bytes, expected {expected_index_bytes}",
            idx.len()
        ));
    }

    let push_constant_string = *opcode_values
        .get("PUSH_CONSTANT_STRING")
        .ok_or_else(|| "ScriptOpcode.PUSH_CONSTANT_STRING is absent".to_string())?;
    let small_operand_opcodes = [
        "RETURN",
        "GOSUB",
        "JUMP",
        "POP_INT_DISCARD",
        "POP_STRING_DISCARD",
    ]
    .iter()
    .map(|name| {
        opcode_values
            .get(*name)
            .copied()
            .ok_or_else(|| format!("ScriptOpcode.{name} is absent"))
    })
    .collect::<Result<BTreeSet<_>, _>>()?;
    let emitter_opcodes = SCRIPT_EMITTER_NAMES
        .iter()
        .map(|name| {
            opcode_values
                .get(*name)
                .copied()
                .map(|opcode| (opcode, *name))
                .ok_or_else(|| format!("ScriptOpcode.{name} is absent"))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;

    let mut index_cursor = 4usize;
    let mut decoded_scripts = 0usize;
    let mut empty_slots = 0usize;
    let mut totals = SCRIPT_EMITTER_NAMES
        .iter()
        .map(|name| ((*name).to_string(), 0usize))
        .collect::<BTreeMap<_, _>>();
    let mut scripts_by_emitter = SCRIPT_EMITTER_NAMES
        .iter()
        .map(|name| ((*name).to_string(), Vec::<Value>::new()))
        .collect::<BTreeMap<_, _>>();
    for id in 0..entries {
        let size = usize::try_from(take_u32(idx, &mut index_cursor, "script index size")?)
            .map_err(|_| format!("script {id} size does not fit usize"))?;
        if size == 0 {
            empty_slots += 1;
            continue;
        }
        let end = data_cursor
            .checked_add(size)
            .filter(|end| *end <= dat.len())
            .ok_or_else(|| format!("script {id} extends beyond script.dat"))?;
        let script = decode_script_record(
            id,
            &dat[data_cursor..end],
            push_constant_string,
            &small_operand_opcodes,
            &emitter_opcodes,
        )?;
        data_cursor = end;
        decoded_scripts += 1;
        for (name, count) in script.emitter_counts {
            let total = totals
                .get_mut(&name)
                .ok_or_else(|| format!("unexpected compiled script emitter: {name}"))?;
            *total = total
                .checked_add(count)
                .ok_or_else(|| format!("compiled script opcode total overflowed: {name}"))?;
            scripts_by_emitter
                .get_mut(&name)
                .expect("emitter inventory is complete")
                .push(json!({
                    "script_id": script.id,
                    "script_name": &script.name,
                    "source_file": &script.source_file,
                    "instruction_occurrences": count,
                }));
        }
    }
    if data_cursor != dat.len() || index_cursor != idx.len() {
        return Err(format!(
            "script pack decode left bytes unconsumed: script.dat {data_cursor}/{}, script.idx {index_cursor}/{}",
            dat.len(),
            idx.len()
        ));
    }

    let opcode_inventory = SCRIPT_EMITTER_NAMES
        .iter()
        .map(|name| {
            json!({
                "name": name,
                "opcode": opcode_values.get(*name),
                "instruction_occurrences": totals.get(*name),
                "script_count": scripts_by_emitter.get(*name).map(Vec::len),
                "scripts": scripts_by_emitter.get(*name),
            })
        })
        .collect::<Vec<_>>();
    let projectile_opcode_count = ["PROJANIM_MAP", "PROJANIM_PL", "PROJANIM_NPC"]
        .iter()
        .map(|name| totals.get(*name).copied().unwrap_or_default())
        .try_fold(0usize, usize::checked_add)
        .ok_or_else(|| "projectile opcode count overflowed".to_string())?;
    let map_spot_animation_occurrences = totals
        .get("SPOTANIM_MAP")
        .copied()
        .ok_or_else(|| "compiled scripts have no SPOTANIM_MAP inventory".to_string())?;
    let map_spot_animation_scripts = scripts_by_emitter
        .get("SPOTANIM_MAP")
        .map(Vec::len)
        .ok_or_else(|| "compiled scripts have no SPOTANIM_MAP script inventory".to_string())?;
    Ok(json!({
        "compiler_version": version,
        "script_slots": entries,
        "decoded_script_count": decoded_scripts,
        "empty_script_slots": empty_slots,
        "all_nonempty_scripts_decoded": true,
        "opcode_inventory": opcode_inventory,
        "projectile_opcode_count": projectile_opcode_count,
        "map_spot_animation_instruction_occurrences": map_spot_animation_occurrences,
        "map_spot_animation_script_count": map_spot_animation_scripts,
        "map_spot_animation_operand_ids_resolved": false,
    }))
}

fn engine_source_marker_lines(
    inputs: &VerifiedEngineInputs,
    markers: &[&str],
) -> Result<BTreeMap<String, Vec<Value>>, String> {
    let mut result = markers
        .iter()
        .map(|marker| ((*marker).to_string(), Vec::new()))
        .collect::<BTreeMap<_, _>>();
    for path in inputs
        .files
        .keys()
        .filter(|path| path.starts_with("src/") && path.ends_with(".ts"))
    {
        let bytes = inputs.read_verified_source(path)?;
        let source = std::str::from_utf8(&bytes)
            .map_err(|error| format!("engine source is not UTF-8 ({path}): {error}"))?;
        for (line_number, line) in source.lines().enumerate() {
            for marker in markers {
                if line.contains(marker) {
                    result
                        .get_mut(*marker)
                        .expect("source marker inventory is initialized")
                        .push(json!({
                            "path": path,
                            "line": line_number + 1,
                        }));
                }
            }
        }
    }
    Ok(result)
}

fn require_marker_locations(
    inventory: &BTreeMap<String, Vec<Value>>,
    marker: &str,
    expected_path: &str,
    expected_count: usize,
) -> Result<(), String> {
    let locations = inventory
        .get(marker)
        .ok_or_else(|| format!("source marker inventory omits {marker:?}"))?;
    if locations.len() != expected_count {
        return Err(format!(
            "engine source marker {marker:?} occurs {} times, expected {expected_count}: {locations:?}",
            locations.len()
        ));
    }
    if locations
        .iter()
        .any(|location| location.get("path").and_then(Value::as_str) != Some(expected_path))
    {
        return Err(format!(
            "engine source marker {marker:?} appears outside {expected_path}: {locations:?}"
        ));
    }
    Ok(())
}

fn scan_script_content(inputs: &VerifiedEngineInputs) -> Result<Value, String> {
    const PROVIDER_PATH: &str = "src/engine/script/ScriptProvider.ts";
    const FILE_PATH: &str = "src/engine/script/ScriptFile.ts";
    const OPCODE_PATH: &str = "src/engine/script/ScriptOpcode.ts";
    const RUNNER_PATH: &str = "src/engine/script/ScriptRunner.ts";

    let provider = inputs.source_text(PROVIDER_PATH)?;
    let script_file = inputs.source_text(FILE_PATH)?;
    let opcode_source = inputs.source_text(OPCODE_PATH)?;
    let runner = inputs.source_text(RUNNER_PATH)?;
    require_markers(
        &provider,
        PROVIDER_PATH,
        &[
            "public static readonly COMPILER_VERSION = 27;",
            "const dat = Packet.load(`${dir}/server/script.dat`);",
            "const idx = Packet.load(`${dir}/server/script.idx`);",
            "const entries = dat.g4s();",
            "const version = dat.g4s();",
            "idx.pos += 4;",
            "const size = idx.g4s();",
            "const script = ScriptFile.decode(id, new Packet(data));",
        ],
    )?;
    require_markers(
        &script_file,
        FILE_PATH,
        &[
            "const trailerLen = stream.g2();",
            "const trailerPos = length - trailerLen - 12 - 2;",
            "function isLargeOperand(opcode: number): boolean {",
            "if (opcode > 100) {",
            "case ScriptOpcode.RETURN:",
            "case ScriptOpcode.POP_INT_DISCARD:",
            "case ScriptOpcode.POP_STRING_DISCARD:",
            "case ScriptOpcode.GOSUB:",
            "case ScriptOpcode.JUMP:",
            "while (trailerPos > stream.pos)",
            "const opcode = stream.g2();",
            "if (opcode === ScriptOpcode.PUSH_CONSTANT_STRING)",
            "return true;",
            "script.opcodes[instr++] = opcode;",
        ],
    )?;
    require_markers(
        &opcode_source,
        OPCODE_PATH,
        &[
            "export const enum ScriptOpcode {",
            "PUSH_CONSTANT_STRING,",
            "PROJANIM_MAP,",
            "SPOTANIM_MAP,",
            "PROJANIM_PL,",
            "SPOTANIM_PL,",
            "PROJANIM_NPC,",
            "SPOTANIM_NPC,",
            "LOC_ADD = 3000,",
            "NPC_ADD = 2500,",
            "MAP_LOCADDUNSAFE",
        ],
    )?;
    require_markers(
        &runner,
        RUNNER_PATH,
        &["if (state.opcount > 500_000)", "state.opcount++;"],
    )?;

    let compiler_version = numeric_after(&provider, "COMPILER_VERSION =")?;
    let opcode_values = parse_script_opcode_values(&opcode_source)?;
    let opcode_member_lines = script_opcode_member_lines(&opcode_source, SCRIPT_EMITTER_NAMES)?;
    let dat = inputs.read_verified_source("data/pack/server/script.dat")?;
    let idx = inputs.read_verified_source("data/pack/server/script.idx")?;
    let mut inventory = scan_script_pack(&dat, &idx, compiler_version, &opcode_values)?;
    let source_call_sites = engine_source_marker_lines(
        inputs,
        &[
            "World.animMap(",
            "World.mapProjAnim(",
            "new MapAnim(",
            "new MapProjAnim(",
        ],
    )?;
    require_marker_locations(
        &source_call_sites,
        "World.animMap(",
        "src/engine/script/handlers/ServerOps.ts",
        1,
    )?;
    require_marker_locations(
        &source_call_sites,
        "World.mapProjAnim(",
        "src/engine/script/handlers/ServerOps.ts",
        3,
    )?;
    require_marker_locations(
        &source_call_sites,
        "new MapAnim(",
        "src/engine/entity/zone/Zone.ts",
        1,
    )?;
    require_marker_locations(
        &source_call_sites,
        "new MapProjAnim(",
        "src/engine/entity/zone/Zone.ts",
        1,
    )?;

    let object = inventory
        .as_object_mut()
        .ok_or_else(|| "compiled script inventory is not an object".to_string())?;
    object.insert(
        "script_data_files".to_string(),
        json!([
            inputs
                .files
                .get("data/pack/server/script.dat")
                .map(FileRecord::json),
            inputs
                .files
                .get("data/pack/server/script.idx")
                .map(FileRecord::json),
        ]),
    );
    object.insert(
        "source_file_bindings".to_string(),
        json!([
            {
                "path": PROVIDER_PATH,
                "compiler_version_line": inputs.marker_lines(PROVIDER_PATH, "COMPILER_VERSION =")?,
                "script_cache_load_lines": inputs.marker_lines(PROVIDER_PATH, "Packet.load(`${dir}/server/script")?,
            },
            {
                "path": FILE_PATH,
                "decoder_lines": inputs.marker_lines(FILE_PATH, "const trailerPos =")?,
                "opcode_operand_rule_lines": inputs.marker_lines(FILE_PATH, "function isLargeOperand")?,
            },
            {
                "path": OPCODE_PATH,
                "enum_line": inputs.marker_lines(OPCODE_PATH, "export const enum ScriptOpcode")?,
            },
            {
                "path": RUNNER_PATH,
                "per_invocation_opcode_limit_lines": inputs.marker_lines(RUNNER_PATH, "state.opcount > 500_000")?,
            },
        ]),
    );
    object.insert(
        "opcode_enum_member_lines".to_string(),
        json!(opcode_member_lines),
    );
    object.insert(
        "source_emitter_call_sites".to_string(),
        json!(source_call_sites),
    );
    object.insert(
        "per_invocation_opcode_limit".to_string(),
        json!(500_000usize),
    );
    object.insert(
        "global_cycle_invocation_limit_derived".to_string(),
        json!(false),
    );
    object.insert(
        "global_cycle_invocation_limit_reason".to_string(),
        json!("the engine limits opcodes per invocation, but this census does not derive the total eligible script invocations or zone events in one cycle"),
    );
    object.insert(
        "map_spot_animation_per_cycle_emission_limit".to_string(),
        Value::Null,
    );
    object.insert(
        "map_spot_animation_max_retained_lifetime_cycles".to_string(),
        Value::Null,
    );
    object.insert(
        "map_spot_animation_lifetime_bound_derived".to_string(),
        json!(false),
    );
    object.insert(
        "map_spot_animation_lifetime_bound_reason".to_string(),
        json!("compiled SPOTANIM_MAP operands are not resolved to sequence IDs; missing sequence data can leave a client map-spot animation active"),
    );
    object.insert(
        "server_zone_event_count_per_cycle_cap".to_string(),
        Value::Null,
    );
    Ok(inventory)
}

pub fn checked_actor_count(
    nearby_players: usize,
    local_player: usize,
    nearby_npcs: usize,
) -> Result<usize, String> {
    nearby_players
        .checked_add(local_player)
        .and_then(|count| count.checked_add(nearby_npcs))
        .ok_or_else(|| "per-view actor population count overflowed".to_string())
}

pub fn checked_loc_slots(
    scene_tiles_per_axis: usize,
    planes: usize,
    layers_per_tile: usize,
) -> Result<usize, String> {
    scene_tiles_per_axis
        .checked_mul(scene_tiles_per_axis)
        .and_then(|cells| cells.checked_mul(planes))
        .and_then(|cells| cells.checked_mul(layers_per_tile))
        .ok_or_else(|| "conditional location overlay slot product overflowed".to_string())
}

fn map_spot_animation_population_is_bounded(
    instruction_occurrences: u64,
    operand_ids_resolved: bool,
    lifetime_bound_derived: bool,
    per_cycle_emission_limit: Option<u64>,
) -> bool {
    instruction_occurrences == 0
        || (operand_ids_resolved && lifetime_bound_derived && per_cycle_emission_limit.is_some())
}

fn all_dynamic_population_inputs_bounded(
    actor_bound: bool,
    location_change_bound: bool,
    projectile_bound: bool,
    map_spot_animation_bound: bool,
    zone_event_bound: bool,
) -> bool {
    actor_bound
        && location_change_bound
        && projectile_bound
        && map_spot_animation_bound
        && zone_event_bound
}
pub fn derive_population(
    inputs: &VerifiedEngineInputs,
    client_sources: &BTreeMap<&'static str, Vec<u8>>,
    scene_tiles_per_axis: usize,
    planes: usize,
    location_shape_layers: &[i32],
) -> Result<Value, String> {
    let build = inputs.source_text("src/network/rsbuf/build.ts")?;
    let info = inputs.source_text("src/network/rsbuf/info.ts")?;
    let world = inputs.source_text("src/engine/World.ts")?;
    let environment = inputs.source_text("src/util/Environment.ts")?;
    let world_config = inputs.source_text("src/util/WorldConfig.ts")?;
    let entity_list = inputs.source_text("src/engine/entity/EntityList.ts")?;
    let game_map = inputs.source_text("src/engine/GameMap.ts")?;
    let zone = inputs.source_text("src/engine/zone/Zone.ts")?;
    let npc_ops = inputs.source_text("src/engine/script/handlers/NpcOps.ts")?;

    require_markers(
        &build,
        "src/network/rsbuf/build.ts",
        &[
            "getNearbyPlayersZones(",
            "getNearbyNpcs(",
            "CoordGrid.withinDistanceSw(other.coord, CoordGrid.from(x, y, z), this.viewDistance)",
            "CoordGrid.withinDistanceSw(other.coord, CoordGrid.from(x, y, z), BuildArea.PREFERRED_VIEW_DISTANCE)",
        ],
    )?;
    require_markers(
        &info,
        "src/network/rsbuf/info.ts",
        &[
            "if (player.build.players.len() >= BuildArea.PREFERRED_PLAYERS)",
            "if (player.build.npcs.len() >= BuildArea.PREFERRED_NPCS)",
        ],
    )?;
    require_markers(
        &world,
        "src/engine/World.ts",
        &[
            "private static readonly NPCS: number = Environment.runtime.maxNpcs;",
            "this.players[slot] = player;",
            "this.npcs.set(npc.nid, npc);",
            "getNextNid(): number {",
            "return this.npcs.next();",
        ],
    )?;
    require_markers(
        &environment,
        "src/util/Environment.ts",
        &["runtime: {", "maxNpcs: 16383", "...config"],
    )?;
    require_markers(
        &world_config,
        "src/util/WorldConfig.ts",
        &[
            "for (const [key, defaultValue] of Object.entries(defaults))",
            "mergeConfig(createDefaultWorldConfig(), value)",
        ],
    )?;
    require_markers(
        &entity_list,
        "src/engine/entity/EntityList.ts",
        &[
            "if (!this.free.size)",
            "this.free.delete(index);",
            "export class NpcList extends EntityList<Npc>",
            "super(size, 0);",
        ],
    )?;
    require_markers(
        &game_map,
        "src/engine/GameMap.ts",
        &["private loadNpcs(packet: Packet", "World.addNpc(npc, -1);"],
    )?;
    require_markers(
        &zone,
        "src/engine/zone/Zone.ts",
        &[
            "private readonly players: DoublyLinkList<Player>",
            "private readonly npcs: DoublyLinkList<Npc>",
            "private readonly events: Set<ZoneEvent>",
            "this.events.add(new ZoneEvent(ZoneEventType.ENCLOSED, -1n, new MapAnim(",
            "this.events.add(new ZoneEvent(ZoneEventType.ENCLOSED, -1n, new MapProjAnim(",
            "private queueEvent(entity: NonPathingEntity, event: ZoneEvent): void",
            "this.events.add(event);",
            "exist.push(event);",
            "this.events.clear();",
            "this.entityEvents.clear();",
        ],
    )?;
    require_markers(
        &npc_ops,
        "src/engine/script/handlers/NpcOps.ts",
        &["[ScriptOpcode.NPC_ADD]:", "World.addNpc(npc, duration);"],
    )?;

    let location_layers = verify_client_authorities(client_sources, location_shape_layers)?;
    let renderer_dynamic_sprite_admission = verify_client_dynamic_sprite_capacity(client_sources)?;
    let nearby_players = numeric_after(&build, "static readonly PREFERRED_PLAYERS =")?;
    let nearby_npcs = numeric_after(&build, "static readonly PREFERRED_NPCS =")?;
    let view_distance = numeric_after(&build, "static readonly PREFERRED_VIEW_DISTANCE =")?;
    let view_actor_count = checked_actor_count(nearby_players, 1, nearby_npcs)?;
    let npc_limit = numeric_after(&environment, "maxNpcs:")?;
    let (first_player_slot, end_player_slot, login_player_slots) = player_slot_capacity(&world)?;
    let configured_world: Value =
        serde_json::from_slice(&inputs.read_verified_source("data/config/world.json")?)
            .map_err(|error| format!("cannot parse staged world config: {error}"))?;
    if configured_world.get("runtime").is_some() {
        return Err(
            "staged world config contains a runtime override not covered by the checked schema"
                .to_string(),
        );
    }
    if !world_config.contains("const defaults = createDefaultWorldConfig()")
        || !world_config.contains("function mergeConfig<")
    {
        return Err("WorldConfig default-merge behavior is not established".to_string());
    }

    let location_overlay_slots_per_level =
        checked_loc_slots(scene_tiles_per_axis, 1, location_layers)?;
    let location_overlay_slots = checked_loc_slots(scene_tiles_per_axis, planes, location_layers)?;
    let emitter_inventory = emitter_inventory(inputs)?;
    let script_content = scan_script_content(inputs)?;
    let projectile_opcode_count = script_content
        .get("projectile_opcode_count")
        .and_then(Value::as_u64)
        .ok_or_else(|| "compiled script inventory has no projectile opcode count".to_string())?;
    let map_spot_animation_instruction_occurrences = script_content
        .get("map_spot_animation_instruction_occurrences")
        .and_then(Value::as_u64)
        .ok_or_else(|| "compiled script inventory has no map-spot instruction count".to_string())?;
    let map_spot_animation_script_count = script_content
        .get("map_spot_animation_script_count")
        .and_then(Value::as_u64)
        .ok_or_else(|| "compiled script inventory has no map-spot script count".to_string())?;
    let script_bool = |name: &str| {
        script_content
            .get(name)
            .and_then(Value::as_bool)
            .ok_or_else(|| format!("compiled script inventory has no boolean {name:?}"))
    };
    let projectile_population_bound_established = projectile_opcode_count == 0;
    let map_spot_operand_ids_resolved = script_bool("map_spot_animation_operand_ids_resolved")?;
    let map_spot_lifetime_bound_derived = script_bool("map_spot_animation_lifetime_bound_derived")?;
    let map_spot_per_cycle_emission_limit = script_content
        .get("map_spot_animation_per_cycle_emission_limit")
        .and_then(Value::as_u64);
    let map_spot_animation_population_bound_established = map_spot_animation_population_is_bounded(
        map_spot_animation_instruction_occurrences,
        map_spot_operand_ids_resolved,
        map_spot_lifetime_bound_derived,
        map_spot_per_cycle_emission_limit,
    );
    let global_cycle_invocation_limit_derived =
        script_bool("global_cycle_invocation_limit_derived")?;
    let server_zone_event_count_per_cycle_cap = script_content
        .get("server_zone_event_count_per_cycle_cap")
        .and_then(Value::as_u64);
    let server_zone_event_population_bound_established =
        global_cycle_invocation_limit_derived || server_zone_event_count_per_cycle_cap.is_some();
    let actor_population_bound_established = true;
    let location_change_population_bound_established = true;
    let transient_population_bound_established = projectile_population_bound_established
        && map_spot_animation_population_bound_established
        && server_zone_event_population_bound_established;
    let dynamic_population_bound_established = all_dynamic_population_inputs_bounded(
        actor_population_bound_established,
        location_change_population_bound_established,
        projectile_population_bound_established,
        map_spot_animation_population_bound_established,
        server_zone_event_population_bound_established,
    );
    let engine_binding = inputs.binding_json();
    let maps_zip = inputs.files.get("data/pack/.cache/maps-server.zip");
    let maps_manifest = inputs.files.get("data/pack/.cache/maps.manifest.json");
    let content_source_dir = inputs
        .engine_root
        .parent()
        .map(|parent| parent.join("content/maps"));
    let content_source_dir_present = content_source_dir
        .as_ref()
        .is_some_and(|path| path.is_dir());

    Ok(json!({
        "engine_input_binding": engine_binding,
        "renderer_dynamic_sprite_admission": renderer_dynamic_sprite_admission,
        "actor_population": {
            "per_view": {
                "nearby_other_players_upper": nearby_players,
                "local_player_added_separately": 1,
                "nearby_npcs_upper": nearby_npcs,
                "total_actor_upper": view_actor_count,
                "preferred_view_distance_tiles": view_distance,
                "same_level_distance_filter_checked": true,
                "player_and_npc_add_guards_checked": true,
                "attached_spot_animation_slots_per_actor": 1,
                "attached_spot_animation_upper": view_actor_count,
                "attached_spot_animation_storage": "ClientEntity has one spotanim_id/frame state; player/NPC script opcodes set that field",
                "source_files": [
                    {"path":"src/network/rsbuf/build.ts","lines":[55,56,57,134,212,250,259]},
                    {"path":"src/network/rsbuf/info.ts","lines":[100,340]},
                    {"path":"vendor/fr-client-rust/crates/client/src/render/draw.rs","lines":[852,860]},
                    {"path":"vendor/fr-client-rust/crates/client/src/dash3d/client_entity.rs","lines":[47,48,49,50,51,107,108,109,110,111]},
                ],
            },
            "engine_wide": {
                "ordinary_login_slot_first": first_player_slot,
                "ordinary_login_slot_end_exclusive": end_player_slot,
                "ordinary_login_slot_capacity": login_player_slots,
                "npc_list_capacity": npc_limit,
                "npc_add_uses_fixed_capacity_list": true,
                "static_map_npc_spawns_share_npc_list": true,
                "script_npc_adds_share_npc_list": true,
                "per_zone_player_or_npc_count_cap": null,
                "per_zone_count_reason": "Zone uses linked lists. The global player and NPC limits bound total live actors.",
                "source_files": [
                    {"path":"src/engine/World.ts","lines":[117,118,1269,1272,1661,1668,1757,1758]},
                    {"path":"src/engine/entity/EntityList.ts","lines":[14,17,62,67,71,74,101,104]},
                    {"path":"src/util/Environment.ts","lines":[5,9]},
                    {"path":"src/util/WorldConfig.ts","lines":[151,160,178,186]},
                    {"path":"src/engine/GameMap.ts","lines":[135,141,153,154]},
                    {"path":"src/engine/script/handlers/NpcOps.ts","lines":[57,64,65]},
                    {"path":"src/engine/zone/Zone.ts","lines":[45,48,54,57]},
                ],
            },
            "spawn_table_count": null,
            "spawn_table_count_reason": "Static NPC spawn rows are carried by map content. The staged server map ZIP is hash-bound, but its rows are not decoded by this census; the engine NPC-list cap covers static and scripted additions.",
        },
        "location_overlay_capacity": {
            "conditional_slots_per_supported_level": location_overlay_slots_per_level,
            "maximum_accumulated_distinct_location_changes": location_overlay_slots,
            "loaded_scene_tiles_per_axis": scene_tiles_per_axis,
            "supported_levels": planes,
            "unique_loc_layers_per_tile": location_layers,
            "coalescing_key": ["level", "x", "z", "layer"],
            "all_five_production_call_sites_use_current_level": true,
            "client_location_change_calls": 3,
            "zone_location_change_calls": 2,
            "tile_and_shape_guards_checked": true,
            "list_has_no_intrinsic_count_cap": true,
            "arithmetic": format!("{scene_tiles_per_axis} * {scene_tiles_per_axis} * {planes} * {location_layers}"),
            "established_as_population_bound": true,
            "reason": "Every private client.rs call passes minusedlevel; zone_289's two call sites pass either a local copy of c.minusedlevel or c.minusedlevel. The two-bit plane state and BuildArea::LEVELS bound the key to four levels; producer paths enforce x/z in [0, 104) and valid shape-to-layer mapping. Coalescing replaces an existing node for each level/x/z/layer key, so the no-cap LinkList cannot exceed 104*104*4 distinct keys per level, or 173,056 across four levels.",
            "source_files": [
                {"path":"vendor/fr-client-rust/crates/client/src/client/client.rs","lines":[10007,10014,10042,10351,12427,12439,12441,12463]},
                {"path":"vendor/fr-client-rust/crates/client/src/client/zone_289.rs","lines":[319,339,494,516,517]},
                {"path":"vendor/fr-client-rust/crates/client/src/client/actor_289.rs","lines":[241,445]},
                {"path":"vendor/fr-client-rust/crates/client/src/dash3d/collision_map.rs","lines":[10,14,15]},
                {"path":"vendor/fr-client-rust/crates/client/src/dash3d/loc_shape.rs","lines":[35,58]},
                {"path":"vendor/fr-client-rust/crates/client/src/datastruct/link_list.rs","lines":[10,21,52,53]},
                {"path":"vendor/fr-client-rust/crates/client/src/datastruct/linkable.rs","lines":[86,100,105,106]},
            ],
        },
        "dynamic_emitters": emitter_inventory,
        "compiled_script_content": script_content,
        "transient_render_lists": {
            "projectiles": {
                "client_list": "Client.projectiles: LinkList<ClientProj>",
                "client_list_intrinsic_count_cap": null,
                "pinned_script_projectile_opcode_count": projectile_opcode_count,
                "pinned_compiled_content_producer_absent": projectile_opcode_count == 0,
                "projectile_handler_source_call_sites": script_content["source_emitter_call_sites"]["World.mapProjAnim("],
                "expiration": "Client list nodes expire by target cycle or level; the pinned complete script pack contains no projectile opcode, so these sources add no projectile nodes.",
                "source_files": [
                    {"path":"vendor/fr-client-rust/crates/client/src/client/client.rs","lines":[1104,10249]},
                    {"path":"vendor/fr-client-rust/crates/client/src/render/draw.rs","lines":[4567,4574,4616]},
                    {"path":"src/engine/script/ScriptOpcode.ts","lines":[61,163,256]},
                    {"path":"src/engine/script/ScriptProvider.ts","lines":[35,36,46,49,60,68]},
                    {"path":"src/engine/script/ScriptFile.ts","lines":[64,67,111,113,117,123]},
                    {"path":"src/engine/script/handlers/ServerOps.ts","lines":[172,183,186,200,203,210]},
                ],
            },
            "map_spot_animations": {
                "client_list": "Client.spotanims: LinkList<MapSpotAnim>",
                "client_list_intrinsic_count_cap": null,
                "pinned_content_instruction_occurrences": map_spot_animation_instruction_occurrences,
                "pinned_content_script_count": map_spot_animation_script_count,
                "operand_ids_resolved": map_spot_operand_ids_resolved,
                "per_cycle_emission_limit": map_spot_per_cycle_emission_limit,
                "maximum_retained_lifetime_cycles": script_content["map_spot_animation_max_retained_lifetime_cycles"],
                "lifetime_bound_derived": map_spot_lifetime_bound_derived,
                "expiration": "Removed after animation completion or a level change. A missing sequence makes update return without setting completion.",
                "source_files": [
                    {"path":"vendor/fr-client-rust/crates/client/src/client/client.rs","lines":[1105,10282]},
                    {"path":"vendor/fr-client-rust/crates/client/src/render/draw.rs","lines":[4621,4628,4634]},
                    {"path":"vendor/fr-client-rust/crates/client/src/dash3d/map_spot_anim.rs","lines":[42,45,49,56]},
                    {"path":"src/engine/script/ScriptOpcode.ts","lines":[61,63]},
                    {"path":"src/engine/script/ScriptProvider.ts","lines":[35,36,46,49,60,68]},
                    {"path":"src/engine/script/ScriptFile.ts","lines":[64,67,111,113,117,123]},
                    {"path":"src/engine/script/handlers/ServerOps.ts","lines":[85,91]},
                    {"path":"src/engine/script/ScriptRunner.ts","lines":[144,148]},
                ],
            },
            "server_zone_events": {
                "queue_type": "Set<ZoneEvent>",
                "cleared_each_cycle": true,
                "events_per_cycle_cap": server_zone_event_count_per_cycle_cap,
                "per_cycle_invocation_bound_derived": server_zone_event_population_bound_established,
                "source_files": [
                    {"path":"src/engine/zone/Zone.ts","lines":[56,57,221,223,224,397,398,401,402,549,551,553,554,557,576,577,580,583]},
                    {"path":"src/engine/World.ts","lines":[1135,1139,1143,1144,1145]},
                ],
            },
        },
        "static_map_content": {
            "server_maps_zip": maps_zip.map(FileRecord::json),
            "server_maps_manifest": maps_manifest.map(FileRecord::json),
            "sibling_content_maps_directory_present": content_source_dir_present,
            "source_map_to_server_pack_equivalence_established": false,
            "reason": "The staged data pack and cache-derived map rows are hash-bound separately. This census does not decode maps-server.zip and compare every entry with the client maps.bin record set.",
            "maximum_co_located_loaded_window_faces": null,
            "maximum_loaded_window_reason": "No source-equivalent map set is available for a legal 104x104 same-plane composition in this report.",
        },
        "qualification": {
            "engine_inputs_match_pinned_manifest": true,
            "renderer_admitted_dynamic_sprite_bound_established": true,
            "actor_population_bound_established": actor_population_bound_established,
            "location_change_population_bound_established": location_change_population_bound_established,
            "projectile_population_bound_established_for_pinned_compiled_content": projectile_population_bound_established,
            "map_spot_animation_population_bound_established": map_spot_animation_population_bound_established,
            "server_zone_event_population_bound_established": server_zone_event_population_bound_established,
            "transient_population_bound_established": transient_population_bound_established,
            "dynamic_population_bound_established": dynamic_population_bound_established,
            "unqualified_resources": ["Capture", "HeadModels", "MapSpotAnim", "Vertex"],
            "unqualified_content_inputs": ["map_spot_animation_emission_and_lifetime", "server_zone_events_per_cycle", "co_located_static_map_composition"],
            "selection_behavior": format!(
                "The client admits at most {} dynamic sprite indices in a client World. Keep workloads that require more unqualified; do not clamp or claim a transient-list count cap. Keep Capture, HeadModels, MapSpotAnim, and Vertex unqualified until map-spot lifetime/emission and co-located static-map inputs are resolved. When a legal content_upper_vertex_bytes is established, use B_vertex = checked_next_power_of_two(max(1048576, content_upper_vertex_bytes)) and reject any device whose max_storage_buffer_binding_size is below the full B_vertex.",
                renderer_dynamic_sprite_admission["max_dynamic_sprites_per_client_world"]
            ),
            "unresolved_inputs": [
                format!("The pinned pack has {map_spot_animation_instruction_occurrences} SPOTANIM_MAP instructions in {map_spot_animation_script_count} scripts, but operand IDs, sequence reachability, maximum retained lifetime, and eligible per-cycle invocations are not derived. A missing sequence can leave a client map-spot animation active."),
                "Zone stores events in a per-cycle Set with no count cap established here; a global per-cycle script-invocation bound is not derived.",
                "The staged server map ZIP is not directly reconciled with client maps.bin for co-located static scene faces.",
            ],
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

    struct Fixture {
        root: PathBuf,
        manifest: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "render-legal-population-{}-{id}",
                std::process::id()
            ));
            let manifest = root.with_extension("manifest.json");
            fs::create_dir_all(root.join("src/network/rsbuf")).unwrap();
            fs::create_dir_all(root.join("data/pack")).unwrap();
            let fixture = Self { root, manifest };
            fixture.write_file("src/network/rsbuf/build.ts", b"PREFERRED_PLAYERS=250\n");
            fixture.write_file("data/pack/maps-server.zip", b"complete map archive");
            fixture.write_manifest();
            fixture
        }

        fn write_file(&self, relative: &str, bytes: &[u8]) {
            let path = self.root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }

        fn write_manifest(&self) {
            let mut artifacts = Vec::new();
            for relative in ["src/network/rsbuf/build.ts", "data/pack/maps-server.zip"] {
                let bytes = fs::read(self.root.join(relative)).unwrap();
                artifacts.push(json!({
                    "path": format!("/fixture/engine/{relative}"),
                    "bytes": bytes.len(),
                    "sha256": format!("{:x}", Sha256::digest(bytes)),
                }));
            }
            fs::write(
                &self.manifest,
                serde_json::to_vec(&json!({
                    "engine_commit": ENGINE_COMMIT,
                    "content_commit": CONTENT_COMMIT,
                    "artifacts": artifacts,
                }))
                .unwrap(),
            )
            .unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
            let _ = fs::remove_file(&self.manifest);
        }
    }

    #[test]
    fn actor_count_adds_nearby_players_local_player_and_npcs() {
        assert_eq!(checked_actor_count(250, 1, 255).unwrap(), 506);
    }

    #[test]
    fn map_spot_population_requires_operand_emission_and_lifetime_bounds() {
        assert!(map_spot_animation_population_is_bounded(
            0, false, false, None
        ));
        assert!(!map_spot_animation_population_is_bounded(
            1,
            false,
            true,
            Some(1)
        ));
        assert!(!map_spot_animation_population_is_bounded(
            1,
            true,
            false,
            Some(1)
        ));
        assert!(!map_spot_animation_population_is_bounded(
            1, true, true, None
        ));
        assert!(map_spot_animation_population_is_bounded(
            1,
            true,
            true,
            Some(1)
        ));
    }

    #[test]
    fn dynamic_population_requires_every_collector_bound() {
        let all_bounded = [true; 5];
        assert!(all_dynamic_population_inputs_bounded(
            all_bounded[0],
            all_bounded[1],
            all_bounded[2],
            all_bounded[3],
            all_bounded[4],
        ));

        for missing in 0..all_bounded.len() {
            let mut bounds = all_bounded;
            bounds[missing] = false;
            assert!(!all_dynamic_population_inputs_bounded(
                bounds[0], bounds[1], bounds[2], bounds[3], bounds[4],
            ));
        }
    }

    #[test]
    fn client_dynamic_sprite_cap_is_source_checked_and_covers_draw_adds() {
        const CORE: &str = "vendor/fr-client-rust/crates/client/src/core/world.rs";
        const RENDER: &str = "vendor/fr-client-rust/crates/client/src/render/world.rs";
        const DRAW: &str = "vendor/fr-client-rust/crates/client/src/render/draw.rs";
        const EXTRA: &str = "vendor/fr-client-rust/crates/client/src/client/extra.rs";
        const VALID_DRAW: &[u8] = b"self.world.prepare_dynamic_storage(&mut client.world)?;\nclient.world.add_dynamic(\nclient.world.add_dynamic2(\nself.world.prepare_dynamic_storage(&mut client.world)?;\nclient.world.add_dynamic(\nself.world.prepare_dynamic_storage(world)?;\nworld.add_dynamic(\nself.world.prepare_dynamic_storage(world)?;\nworld.add_dynamic(";
        let mut sources = BTreeMap::new();
        sources.insert(
            CORE,
            b"pub const MAX_DYNAMIC_SPRITES: usize = 5000;\npub fn add_dynamic() {}\npub fn add_dynamic2() {}\nself.dynamic_sprites = vec![None; MAX_DYNAMIC_SPRITES];\nself.dynamic_sprites[self.dynamic_count as usize] = Some(index);\nself.dynamic_count += 1;"
                .to_vec(),
        );
        sources.insert(
            RENDER,
            b"let Some(budget) = self.head_misc.as_ref() else {\nreturn Ok(());\n};\nif world.dynamic_count as usize >= crate::core::world::MAX_DYNAMIC_SPRITES {\nreturn Err(RenderLimit::new(\nworld.dynamic_sprites.resize(crate::core::world::MAX_DYNAMIC_SPRITES, None);"
                .to_vec(),
        );
        sources.insert(DRAW, VALID_DRAW.to_vec());

        let report = verify_client_dynamic_sprite_capacity(&sources).unwrap();
        assert_eq!(
            report["max_dynamic_sprites_per_client_world"],
            client::core::world::MAX_DYNAMIC_SPRITES
        );
        assert_eq!(report["all_dynamic_draw_adds_preflighted"], true);
        assert_eq!(report["preflight_call_count"], 4);
        assert_eq!(report["draw_add_count"], 4);
        assert_eq!(report["draw_add2_count"], 1);
        assert_eq!(
            report["indexed_client_source_entries_scanned_for_extra_insertions"],
            2
        );
        assert_eq!(report["does_not_bound_client_transient_list_nodes"], true);

        sources.insert(
            DRAW,
            b"self.world.prepare_dynamic_storage(&mut client.world)?;\nclient.world.add_dynamic(\nself.world.prepare_dynamic_storage(&mut client.world)?;\nclient.world.add_dynamic(\nself.world.prepare_dynamic_storage(world)?;\nworld.add_dynamic("
                .to_vec(),
        );
        assert!(verify_client_dynamic_sprite_capacity(&sources).is_err());
        sources.insert(DRAW, VALID_DRAW.to_vec());
        sources.insert(EXTRA, b"world.add_dynamic(".to_vec());
        let error = verify_client_dynamic_sprite_capacity(&sources).unwrap_err();
        assert!(error.contains("outside render/draw.rs"));
    }

    #[test]
    fn location_overlay_slot_product_is_conditional_and_checked() {
        assert_eq!(checked_loc_slots(104, 1, 4).unwrap(), 43_264);
        assert_eq!(checked_loc_slots(104, 4, 4).unwrap(), 173_056);
        assert!(checked_loc_slots(usize::MAX, 4, 4).is_err());
    }

    #[test]
    fn actor_population_product_rejects_overflow() {
        assert!(checked_actor_count(usize::MAX, 1, 0).is_err());
    }

    #[test]
    fn staged_manifest_rejects_a_missing_source() {
        let fixture = Fixture::new();
        fs::remove_file(fixture.root.join("src/network/rsbuf/build.ts")).unwrap();
        let error = verify_staged_engine(&fixture.root, &fixture.manifest).unwrap_err();
        assert!(error.contains("manifest engine input is missing from stage"));
    }

    #[test]
    fn staged_manifest_rejects_a_mutated_source() {
        let fixture = Fixture::new();
        fixture.write_file("src/network/rsbuf/build.ts", b"PREFERRED_PLAYERS=251\n");
        let error = verify_staged_engine(&fixture.root, &fixture.manifest).unwrap_err();
        assert!(error.contains("size mismatch") || error.contains("SHA-256 mismatch"));
    }

    #[test]
    fn staged_manifest_rejects_a_truncated_pack() {
        let fixture = Fixture::new();
        fixture.write_file("data/pack/maps-server.zip", b"complete");
        let error = verify_staged_engine(&fixture.root, &fixture.manifest).unwrap_err();
        assert!(error.contains("data/pack/maps-server.zip"));
    }

    #[test]
    fn emitter_inventory_requires_every_declared_operation_marker() {
        let mut sources = BTreeMap::new();
        for spec in EMITTERS {
            let text = spec
                .markers
                .iter()
                .map(|marker| format!("{marker}\n"))
                .collect::<String>();
            sources
                .entry(spec.path)
                .or_insert_with(String::new)
                .push_str(&text);
        }
        let inventory = inventory_emitters_from_sources(&sources).unwrap();
        assert_eq!(inventory.len(), EMITTERS.len());
        sources.insert("src/engine/script/handlers/LocOps.ts", String::new());
        let error = inventory_emitters_from_sources(&sources).unwrap_err();
        assert!(error.contains("[ScriptOpcode.LOC_ADD]:"));
    }

    #[test]
    fn script_opcode_values_follow_explicit_and_sequential_enum_members() {
        let source = r#"
export const enum ScriptOpcode {
    PUSH_CONSTANT_INT = 0,
    PUSH_CONSTANT_STRING = 3,
    RETURN = 21,
    GOSUB,
    JUMP,
    POP_INT_DISCARD = 40,
    POP_STRING_DISCARD,
    LOC_ADD = 3000,
    NPC_ADD = 2500,
    SPOTANIM_NPC = 2548,
    SPOTANIM_PL = 2111,
    SPOTANIM_MAP = 1021,
    PROJANIM_PL = 2095,
    PROJANIM_NPC = 2547,
    PROJANIM_MAP = 1019,
    MAP_LOCADDUNSAFE = 1012,
}
"#;
        let values = parse_script_opcode_values(source).unwrap();
        assert_eq!(values["GOSUB"], 22);
        assert_eq!(values["JUMP"], 23);
        assert_eq!(values["POP_STRING_DISCARD"], 41);
        assert_eq!(values["SPOTANIM_MAP"], 1021);
        let member_lines = script_opcode_member_lines(source, &["GOSUB", "LOC_ADD"]).unwrap();
        assert_eq!(member_lines["GOSUB"], 6);
        assert_eq!(member_lines["LOC_ADD"], 10);
        let inline = "  export const enum ScriptOpcode {\nGOSUB,\n}";
        assert_eq!(
            script_opcode_member_lines(inline, &["GOSUB"]).unwrap()["GOSUB"],
            2
        );
    }

    #[test]
    fn compiled_script_decoder_counts_emitter_opcodes_and_empty_slots() {
        fn append_u16(bytes: &mut Vec<u8>, value: u16) {
            bytes.extend_from_slice(&value.to_be_bytes());
        }

        fn append_u32(bytes: &mut Vec<u8>, value: u32) {
            bytes.extend_from_slice(&value.to_be_bytes());
        }

        let opcode_source = r#"
export const enum ScriptOpcode {
    PUSH_CONSTANT_INT = 0,
    PUSH_CONSTANT_STRING = 3,
    RETURN = 21,
    GOSUB = 22,
    JUMP = 23,
    POP_INT_DISCARD = 40,
    POP_STRING_DISCARD = 41,
    LOC_ADD = 3000,
    NPC_ADD = 2500,
    SPOTANIM_NPC = 2548,
    SPOTANIM_PL = 2111,
    SPOTANIM_MAP = 1021,
    PROJANIM_PL = 2095,
    PROJANIM_NPC = 2547,
    PROJANIM_MAP = 1019,
    MAP_LOCADDUNSAFE = 1012,
}
"#;
        let opcodes = parse_script_opcode_values(opcode_source).unwrap();
        let instructions = vec![
            ("PUSH_CONSTANT_STRING", b"constant\0".to_vec()),
            ("PUSH_CONSTANT_INT", vec![0, 0, 0, 7]),
            ("RETURN", vec![0]),
            ("SPOTANIM_MAP", vec![1]),
            ("PROJANIM_MAP", vec![2]),
        ];
        let mut record = Vec::new();
        record.extend_from_slice(b"fixture\0source.rs2\0");
        append_u32(&mut record, 0);
        record.push(0);
        append_u16(&mut record, 0);
        for (name, operand) in &instructions {
            append_u16(&mut record, opcodes[*name]);
            record.extend_from_slice(operand);
        }
        append_u32(&mut record, u32::try_from(instructions.len()).unwrap());
        for _ in 0..4 {
            append_u16(&mut record, 0);
        }
        record.push(0);
        append_u16(&mut record, 1);

        let mut dat = Vec::new();
        append_u32(&mut dat, 2);
        append_u32(&mut dat, 27);
        dat.extend_from_slice(&record);
        let mut idx = Vec::new();
        append_u32(&mut idx, 2);
        append_u32(&mut idx, 0);
        append_u32(&mut idx, u32::try_from(record.len()).unwrap());

        let inventory = scan_script_pack(&dat, &idx, 27, &opcodes).unwrap();
        assert_eq!(inventory["script_slots"], 2);
        assert_eq!(inventory["decoded_script_count"], 1);
        assert_eq!(inventory["empty_script_slots"], 1);
        assert_eq!(inventory["map_spot_animation_instruction_occurrences"], 1);
        assert_eq!(inventory["map_spot_animation_script_count"], 1);
        assert_eq!(inventory["projectile_opcode_count"], 1);
    }

    #[test]
    fn client_authority_requires_current_level_coalescing_and_tile_bounds() {
        const BUILD: &str = "vendor/fr-client-rust/crates/client/src/client/client.rs";
        const DRAW: &str = "vendor/fr-client-rust/crates/client/src/render/draw.rs";
        const SHAPE: &str = "vendor/fr-client-rust/crates/client/src/dash3d/loc_shape.rs";
        const SPOT: &str = "vendor/fr-client-rust/crates/client/src/dash3d/map_spot_anim.rs";
        const LIST: &str = "vendor/fr-client-rust/crates/client/src/datastruct/link_list.rs";
        const ARENA: &str = "vendor/fr-client-rust/crates/client/src/datastruct/linkable.rs";
        const ZONE: &str = "vendor/fr-client-rust/crates/client/src/client/zone_289.rs";
        const BUILD_AREA: &str = "vendor/fr-client-rust/crates/client/src/dash3d/collision_map.rs";
        const ACTOR: &str = "vendor/fr-client-rust/crates/client/src/client/actor_289.rs";
        const ENTITY: &str = "vendor/fr-client-rust/crates/client/src/dash3d/client_entity.rs";
        let mut sources = BTreeMap::new();
        let guard = "if shape < LOC_SHAPE_TO_LAYER.len() as i32 && x >= 0 && z >= 0 && x < BuildArea::SIZE && z < BuildArea::SIZE";
        let mut build = String::from(
            "struct Client {\npub loc_changes: LinkList<LocChange>,\npub projectiles: LinkList<ClientProj>,\npub spotanims: LinkList<MapSpotAnim>,\n}\nimpl Client {\nfn loc_change_create(&mut self) {}\nfn zone_packet(&mut self) {\nself.projectiles.push(proj);\nself.spotanims.push(MapSpotAnim::new());\nself.minusedlevel = buf.gbit(2);\nif loc.level == self.minusedlevel && loc.x == x && loc.z == z && loc.layer == layer { self.loc_changes.push(loc); }\n",
        );
        for _ in 0..3 {
            build.push_str(guard);
            build.push_str(" { self.loc_change_create(self.minusedlevel, x, z, layer, id, shape, rotate, 0, -1); }\n");
        }
        build.push_str(guard);
        build.push_str(" { let unrelated_guard = true; }\n} }\n");
        sources.insert(BUILD, build.as_bytes().to_vec());
        sources.insert(
            DRAW,
            b"let count = if add_self { 1 } else { client.player_count };\nclient.local_player.as_mut()\nclient.loop_cycle > proj.t2\nspot.anim_complete"
                .to_vec(),
        );
        sources.insert(
            SHAPE,
            b"pub const LOC_SHAPE_TO_LAYER: [i32; 23] = [".to_vec(),
        );
        sources.insert(
            SPOT,
            b"let Some(seq) = cache.spot(self.spotanim as usize).seq else {\nreturn;\nself.anim_complete = true;"
                .to_vec(),
        );
        sources.insert(
            LIST,
            b"pub struct LinkList<T: LinkableTrait>\npub fn push(&mut self, node: T)\nself.arena.alloc(node)"
                .to_vec(),
        );
        sources.insert(
            ARENA,
            b"pub struct Arena<T: LinkableTrait>\nself.nodes.push(Some(node));".to_vec(),
        );
        sources.insert(
            ZONE,
            b"let level = c.minusedlevel;\nfn tile(x: i32, z: i32) -> bool {\n(0..BuildArea::SIZE).contains(&x) && (0..BuildArea::SIZE).contains(&z)\nassert!(tile(x, z), \"invalid merge tile\");\nif tile(x, z) {\nc.loc_change_create(level, x, z, layer, id, shape, angle, 0, -1);\nc.loc_change_create(c.minusedlevel, x, z, layer, id, shape, angle, 0, -1);"
                .to_vec(),
        );
        sources.insert(
            BUILD_AREA,
            b"pub const LEVELS: i32 = 4;\npub const SIZE: i32 = 13 << 3;".to_vec(),
        );
        sources.insert(
            ACTOR,
            b"frame.plane = Some(bit(p, 2));\nc.minusedlevel = plane;".to_vec(),
        );
        sources.insert(
            ENTITY,
            b"pub spotanim_id: i32,\npub spotanim_frame: i32,\nspotanim_id: -1,\nspotanim_frame: 0,"
                .to_vec(),
        );
        assert_eq!(
            verify_client_authorities(&sources, &[0, 1, 2, 3]).unwrap(),
            4
        );
        let bad_guard = build.replacen("&& x >= 0", "&& x >= 1", 1);
        sources.insert(BUILD, bad_guard.into_bytes());
        let error = verify_client_authorities(&sources, &[0, 1, 2, 3]).unwrap_err();
        assert!(error.contains("guards cover 2 of 3 calls"));
        sources.insert(
            BUILD,
            b"client loc changes have an unexplained count".to_vec(),
        );
        let error = verify_client_authorities(&sources, &[0, 1, 2, 3]).unwrap_err();
        assert!(error.contains("pub loc_changes: LinkList<LocChange>"));
    }
}
