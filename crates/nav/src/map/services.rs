//! Nav-bake producer for the data-only `274bot.navpois` sidecar.
//!
//! NPC coordinates come from jm2 `==== NPC ====` rows (already game-plane).
//! Server-only bank capability follows rs2 handlers that can reach `@openbank`
//! and explicit `loc_change` / `open_chest` transitions. Place labels come
//! from `maps/labels.txt`. This is bounded static evidence, not a script
//! interpreter and not a frozen town roster.

use super::formats::{
    Coverage, CoverageIssue, CoverageLevel, CoverageReason, ServiceIdentity, ServicePois,
    NAVPOIS_VERSION,
};
use super::identity::Digest;
use super::poi::{
    classify_definition, Capability, CapabilityEvidence, Definition, DisplayAnchor, Eligibility,
    EntityKind, Footprint, OperationSlot, PoiKey, PoiKind, PoiRecord, ServiceTrigger, SourceSpace,
};
use super::{Rows, Text};
use client::config::{LocType, NpcType};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Baker sources whose bytes join the navpois generator identity. Pack/flags
/// bytes do not depend on these files.
pub const POIS_GENERATOR_SOURCES: [&str; 2] = ["src/map/services.rs", "src/map/poi.rs"];
pub const POIS_GENERATOR_ID: &str = "navpois-1";
pub const POIS_POLICY_ALGORITHM: &str = "openbank-reach-1";

pub fn pois_generator_identity(sources: &[(&str, &str)]) -> String {
    let mut digest = Sha256::new();
    digest.update(POIS_GENERATOR_ID.as_bytes());
    digest.update([0]);
    digest.update(POIS_POLICY_ALGORITHM.as_bytes());
    for (label, text) in sources {
        digest.update([0]);
        digest.update(label.as_bytes());
        digest.update([0]);
        digest.update(text.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

pub fn pois_generator_identity_from_crate() -> Result<String, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut sources = Vec::new();
    for relative in POIS_GENERATOR_SOURCES {
        let text = std::fs::read_to_string(root.join(relative))
            .map_err(|e| format!("navpois generator source {relative}: {e}"))?;
        sources.push((relative, text));
    }
    let refs: Vec<(&str, &str)> = sources
        .iter()
        .map(|(label, text)| (*label, text.as_str()))
        .collect();
    Ok(pois_generator_identity(&refs))
}

pub fn pois_policy_digest() -> Digest {
    let mut h = Sha256::new();
    h.update(b"274bot.map.navpois-policy\0");
    h.update(1u16.to_be_bytes());
    h.update((POIS_POLICY_ALGORITHM.len() as u16).to_be_bytes());
    h.update(POIS_POLICY_ALGORITHM.as_bytes());
    Digest(h.finalize().into())
}

pub struct ProduceRequest<'a> {
    pub revision: u16,
    pub content_root: &'a Path,
    pub npcs: &'a [NpcType],
    pub locs: &'a [LocType],
    pub content_id: Digest,
    pub nav_sha256: Digest,
    pub source_sha256: Digest,
    pub generator_sha256: Digest,
}

pub struct ProducedPois {
    pub bytes: Vec<u8>,
    pub coverage: Coverage,
}

pub fn produce_navpois(request: &ProduceRequest<'_>) -> Result<ProducedPois, String> {
    let policy = pois_policy_digest();
    let identity = ServiceIdentity {
        revision: request.revision,
        content: request.content_id,
        nav_sha256: request.nav_sha256,
        source_sha256: request.source_sha256,
        generator_sha256: request.generator_sha256,
        policy,
    };
    let document = build_document(request, identity)?;
    let bytes = document
        .encode_navpois()
        .map_err(|e| format!("navpois encode: {e}"))?;
    Ok(ProducedPois {
        coverage: document.coverage,
        bytes,
    })
}

fn build_document(
    request: &ProduceRequest<'_>,
    identity: ServiceIdentity,
) -> Result<ServicePois, String> {
    let npc_ids = pack_ids(request.content_root, "npc.pack");
    let loc_ids = pack_ids(request.content_root, "loc.pack");
    let tree = load_script_tree(request.content_root)?;
    let mut npc_cfg = tree.npc_cfg;
    let mut loc_cfg = tree.loc_cfg;
    bind_pack_ids(&mut npc_cfg, &npc_ids);
    bind_pack_ids(&mut loc_cfg, &loc_ids);
    let analysis = analyse_bank_handlers(&tree.files, &npc_ids, &loc_ids, &npc_cfg, &loc_cfg);
    drop(tree.files);

    let mut interesting_npcs: HashSet<i32> = HashSet::new();
    for npc in request.npcs {
        if npc.id < 0 {
            continue;
        }
        if classifies_as_service(
            request.revision,
            EntityKind::Npc,
            npc.name.as_str(),
            &npc.op,
        ) {
            interesting_npcs.insert(npc.id);
        }
    }
    interesting_npcs.extend(analysis.npc_ids.iter().copied());

    let mut interesting_locs: HashSet<i32> = HashSet::new();
    for id in &analysis.loc_ids {
        if !client_proves_bank(request.revision, entity_loc(request.locs, *id)) {
            interesting_locs.insert(*id);
        }
    }
    for (from, to) in &analysis.transitions {
        if analysis.loc_ids.contains(to) || loc_has_bank_op(request.locs, *to) {
            interesting_locs.insert(*from);
        }
    }

    let (map_count, npc_hits, loc_hits) =
        scan_maps(request.content_root, &interesting_npcs, &interesting_locs)?;

    let mut unresolved = Vec::new();
    for (reference, ids) in &analysis.unresolved {
        push_issue(
            &mut unresolved,
            CoverageReason::UnresolvedHandler,
            reference,
            ids.len() as u32,
        );
    }
    for id in &analysis.npc_ids {
        if npc_hits.get(id).is_none_or(|hits| hits.is_empty()) {
            let name = npc_ids
                .iter()
                .find_map(|(n, packed)| (*packed == *id).then_some(n.as_str()))
                .unwrap_or("npc");
            push_issue(
                &mut unresolved,
                CoverageReason::MissingNpcPlacements,
                &format!("npc {id} ({name})"),
                1,
            );
        }
    }

    let mut records = Vec::new();
    emit_npc_records(request, &analysis, &npc_cfg, &npc_hits, &mut records)?;
    emit_loc_records(request, &analysis, &loc_cfg, &loc_hits, &mut records)?;
    let labels_level = emit_label_records(request.content_root, &mut records, &mut unresolved)?;

    records.sort_by_key(|record: &PoiRecord| record.key);
    let npc_level = if map_count == 0 {
        CoverageLevel::Unavailable
    } else if npc_hits.is_empty() {
        CoverageLevel::Limited
    } else {
        CoverageLevel::Complete
    };
    let bank_level = if analysis.npc_ids.is_empty() && analysis.loc_ids.is_empty() {
        CoverageLevel::Unavailable
    } else if unresolved
        .iter()
        .any(|(reason, _, _)| *reason != CoverageReason::MissingLabels)
    {
        CoverageLevel::Limited
    } else {
        CoverageLevel::Complete
    };
    let coverage = Coverage {
        npc_placements: npc_level,
        bank_services: bank_level,
        place_labels: labels_level,
        unresolved: Rows::new(unresolved_rows(unresolved))
            .map_err(|e| format!("navpois coverage: {e}"))?,
    };
    let document = ServicePois {
        schema: NAVPOIS_VERSION,
        identity,
        coverage,
        records: Rows::new(records).map_err(|e| format!("navpois records: {e}"))?,
    };
    document
        .validate(identity)
        .map_err(|e| format!("navpois validate: {e}"))?;
    Ok(document)
}

fn classifies_as_service(
    revision: u16,
    entity: EntityKind,
    name: &str,
    ops: &[Option<String>],
) -> bool {
    let mut keep = false;
    classify_entity(revision, entity, name, ops, false, None, |kind, _| {
        keep |= matches!(kind, PoiKind::Bank | PoiKind::Shop | PoiKind::Altar);
    });
    keep
}

fn client_proves_bank(revision: u16, loc: Option<&LocType>) -> bool {
    let Some(loc) = loc else {
        return false;
    };
    let mut bank = false;
    classify_entity(
        revision,
        EntityKind::Loc,
        loc.name.as_str(),
        &loc.op,
        loc.active,
        mapfunction(loc),
        |kind, evidence| {
            bank |= kind == PoiKind::Bank
                && matches!(
                    evidence,
                    CapabilityEvidence::ClientOperation {
                        capability: Capability::Bank,
                        ..
                    } | CapabilityEvidence::ActiveQuickBooth
                );
        },
    );
    bank
}

fn loc_has_bank_op(locs: &[LocType], id: i32) -> bool {
    let Some(loc) = entity_loc(locs, id) else {
        return false;
    };
    loc.op
        .iter()
        .flatten()
        .any(|op| op.eq_ignore_ascii_case("Bank"))
}

fn classify_entity(
    revision: u16,
    entity: EntityKind,
    name: &str,
    ops: &[Option<String>],
    active: bool,
    mapfunction: Option<u16>,
    emit: impl FnMut(PoiKind, CapabilityEvidence),
) {
    let mut operations = [None; 5];
    for (index, op) in ops.iter().take(5).enumerate() {
        operations[index] = op.as_deref().filter(|value| !value.is_empty());
    }
    classify_definition(
        revision,
        &Definition {
            entity,
            name,
            operations,
            active,
            mapfunction,
        },
        emit,
    );
}

fn emit_npc_records(
    request: &ProduceRequest<'_>,
    analysis: &BankAnalysis,
    npc_cfg: &HashMap<String, NamedConfig>,
    hits: &BTreeMap<i32, Vec<NpcHit>>,
    records: &mut Vec<PoiRecord>,
) -> Result<(), String> {
    for (id, placements) in hits {
        let npc = entity_npc(request.npcs, *id);
        let name = npc
            .map(|row| row.name.as_str())
            .filter(|name| !name.is_empty())
            .or_else(|| {
                npc_cfg.values().find_map(|cfg| {
                    (cfg.packed_id == Some(*id) && !cfg.name.is_empty())
                        .then_some(cfg.name.as_str())
                })
            })
            .unwrap_or("NPC");
        let ops = npc.map(|row| row.op.as_slice()).unwrap_or(&[]);
        let size = npc
            .map(|row| row.size)
            .filter(|size| *size > 0)
            .unwrap_or(1)
            .clamp(1, 64) as u8;
        let mut classified = Vec::new();
        classify_entity(
            request.revision,
            EntityKind::Npc,
            name,
            ops,
            true,
            None,
            |kind, evidence| classified.push((kind, evidence)),
        );
        for hit in placements {
            let mut evidence = classified.clone();
            if let Some(services) = analysis.npc_services.get(id) {
                for service in services {
                    evidence.push((
                        PoiKind::Bank,
                        source_service(service, analysis.file_digest.get(&service.file)),
                    ));
                }
            }
            evidence.sort_by_key(|(kind, _)| kind_rank(*kind));
            evidence.dedup_by(|a, b| evidence_eq(&a.1, &b.1));
            let Some(kind) = evidence
                .iter()
                .map(|(kind, _)| *kind)
                .find(|kind| matches!(kind, PoiKind::Bank | PoiKind::Shop | PoiKind::Altar))
            else {
                continue;
            };
            let kind = if evidence.iter().any(|(k, _)| *k == PoiKind::Bank) {
                PoiKind::Bank
            } else {
                kind
            };
            records.push(poi_record(
                PoiKey {
                    entity: EntityKind::Npc,
                    id: *id as u32,
                    x: hit.x,
                    z: hit.z,
                    source: SourceSpace::ServerGame { plane: hit.plane },
                    shape: 0,
                    rotation: 0,
                },
                name,
                kind,
                Footprint {
                    width: size,
                    length: size,
                },
                evidence,
            )?);
        }
    }
    Ok(())
}

fn emit_loc_records(
    request: &ProduceRequest<'_>,
    analysis: &BankAnalysis,
    loc_cfg: &HashMap<String, NamedConfig>,
    hits: &BTreeMap<i32, Vec<LocHit>>,
    records: &mut Vec<PoiRecord>,
) -> Result<(), String> {
    for (id, placements) in hits {
        let loc = entity_loc(request.locs, *id);
        let name = loc
            .map(|row| row.name.as_str())
            .filter(|name| !name.is_empty())
            .or_else(|| {
                loc_cfg
                    .iter()
                    .find_map(|(_, cfg)| (cfg.packed_id == Some(*id)).then_some(cfg.name.as_str()))
            })
            .filter(|name| !name.is_empty())
            .unwrap_or("Loc");
        let ops = loc.map(|row| row.op.as_slice()).unwrap_or(&[]);
        let active = loc.map(|row| row.active).unwrap_or(false);
        let width = loc
            .map(|row| row.width)
            .filter(|value| *value > 0)
            .unwrap_or(1)
            .clamp(1, 64) as u8;
        let length = loc
            .map(|row| row.length)
            .filter(|value| *value > 0)
            .unwrap_or(1)
            .clamp(1, 64) as u8;
        for hit in placements {
            let mut evidence = Vec::new();
            classify_entity(
                request.revision,
                EntityKind::Loc,
                name,
                ops,
                active,
                loc.and_then(mapfunction),
                |kind, ev| evidence.push((kind, ev)),
            );
            if let Some(services) = analysis.loc_services.get(id) {
                for service in services {
                    evidence.push((
                        PoiKind::Bank,
                        source_service(service, analysis.file_digest.get(&service.file)),
                    ));
                }
            }
            if let Some(targets) = analysis.transitions_from.get(id) {
                for target in targets {
                    if analysis.loc_ids.contains(target) || loc_has_bank_op(request.locs, *target) {
                        let digest = analysis
                            .transition_files
                            .get(&(*id, *target))
                            .and_then(|file| analysis.file_digest.get(file));
                        let reference = analysis
                            .transition_files
                            .get(&(*id, *target))
                            .cloned()
                            .unwrap_or_else(|| "loc_change".into());
                        evidence.push((
                            PoiKind::Bank,
                            CapabilityEvidence::SourceService {
                                capability: Capability::Bank,
                                trigger: ServiceTrigger::LocVariant {
                                    definition: *target as u32,
                                },
                                eligibility: Eligibility::Conditional,
                                reference: text(&reference)?,
                                source_sha256: digest.copied().unwrap_or(Digest([0; 32])),
                            },
                        ));
                    }
                }
            }
            evidence.sort_by_key(|(kind, _)| kind_rank(*kind));
            evidence.dedup_by(|a, b| evidence_eq(&a.1, &b.1));
            if !evidence.iter().any(|(kind, _)| *kind == PoiKind::Bank) {
                continue;
            }
            let footprint = Footprint { width, length }
                .rotated(hit.rotation)
                .map_err(|e| format!("navpois loc footprint: {e}"))?;
            records.push(poi_record(
                PoiKey {
                    entity: EntityKind::Loc,
                    id: *id as u32,
                    x: hit.x,
                    z: hit.z,
                    source: SourceSpace::ClientVisual {
                        plane: hit.plane,
                        link_below: hit.link_below,
                    },
                    shape: hit.shape,
                    rotation: hit.rotation,
                },
                name,
                PoiKind::Bank,
                footprint,
                evidence,
            )?);
        }
    }
    Ok(())
}

fn emit_label_records(
    content_root: &Path,
    records: &mut Vec<PoiRecord>,
    unresolved: &mut Vec<(CoverageReason, String, u32)>,
) -> Result<CoverageLevel, String> {
    let path = content_root.join("maps/labels.txt");
    let Ok(bytes) = std::fs::read(&path) else {
        push_issue(
            unresolved,
            CoverageReason::MissingLabels,
            "maps/labels.txt",
            1,
        );
        return Ok(CoverageLevel::Unavailable);
    };
    let digest = Digest::of(&bytes);
    let text_body = String::from_utf8_lossy(&bytes);
    let mut count = 0u32;
    for (index, raw) in text_body.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(row) = parse_label_line(line) else {
            push_issue(
                unresolved,
                CoverageReason::MissingLabels,
                &format!("maps/labels.txt:{}", index + 1),
                1,
            );
            continue;
        };
        count += 1;
        records.push(poi_record(
            PoiKey {
                entity: EntityKind::Label,
                id: (index as u32).saturating_add(1),
                x: row.x,
                z: row.z,
                source: SourceSpace::ServerGame { plane: 0 },
                shape: 0,
                rotation: 0,
            },
            &row.name,
            PoiKind::Label {
                priority: row.priority,
            },
            Footprint {
                width: 1,
                length: 1,
            },
            vec![(
                PoiKind::Label {
                    priority: row.priority,
                },
                CapabilityEvidence::SourceLabel {
                    reference: text("maps/labels.txt")?,
                    source_sha256: digest,
                },
            )],
        )?);
    }
    Ok(if count == 0 {
        CoverageLevel::Limited
    } else {
        CoverageLevel::Complete
    })
}

struct LabelRow {
    name: String,
    x: i32,
    z: i32,
    priority: u8,
}

fn parse_label_line(line: &str) -> Option<LabelRow> {
    let line = line.strip_prefix('=')?;
    let mut parts = line.rsplitn(4, ',');
    let priority = parts.next()?.trim();
    let z = parts.next()?.trim();
    let x = parts.next()?.trim();
    let name = parts.next()?.trim();
    Some(LabelRow {
        name: name.to_string(),
        x: x.parse().ok()?,
        z: z.parse().ok()?,
        priority: priority.parse().ok()?,
    })
}

fn poi_record(
    key: PoiKey,
    name: &str,
    kind: PoiKind,
    footprint: Footprint,
    evidence: Vec<(PoiKind, CapabilityEvidence)>,
) -> Result<PoiRecord, String> {
    let plane = key
        .source
        .game_plane()
        .map_err(|e| format!("navpois plane: {e}"))?
        .ok_or_else(|| "navpois placement dropped by LINK_BELOW".to_string())?;
    let evidence = Rows::new(
        evidence
            .into_iter()
            .map(|(_, evidence)| evidence)
            .take(8)
            .collect(),
    )
    .map_err(|e| format!("navpois evidence: {e}"))?;
    let record = PoiRecord {
        key,
        name: text(name)?,
        kind,
        effective_plane: plane,
        footprint,
        display: DisplayAnchor {
            x: f64::from(key.x) + 0.5,
            z: f64::from(key.z) + 0.5,
            plane,
        },
        evidence,
        walk_target: None,
    };
    record
        .validate()
        .map_err(|e| format!("navpois record: {e}"))?;
    Ok(record)
}

fn source_service(service: &HandlerService, digest: Option<&Digest>) -> CapabilityEvidence {
    CapabilityEvidence::SourceService {
        capability: Capability::Bank,
        trigger: service.trigger,
        eligibility: service.eligibility,
        reference: Text::new(&service.reference).unwrap_or_else(|_| Text::new("rs2").unwrap()),
        source_sha256: digest.copied().unwrap_or(Digest([0; 32])),
    }
}

fn text(value: &str) -> Result<Text, String> {
    let value = if value.is_empty() { "unknown" } else { value };
    let value = if value.len() > Text::MAX_BYTES {
        &value[..Text::MAX_BYTES]
    } else {
        value
    };
    Text::new(value).map_err(|e| format!("navpois text: {e}"))
}

fn kind_rank(kind: PoiKind) -> u8 {
    match kind {
        PoiKind::Bank => 0,
        PoiKind::Shop => 1,
        PoiKind::Altar => 2,
        PoiKind::Label { .. } => 3,
        PoiKind::MapSymbol { .. } => 4,
        PoiKind::Transport => 5,
        PoiKind::Teleport => 6,
    }
}

fn evidence_eq(a: &CapabilityEvidence, b: &CapabilityEvidence) -> bool {
    a == b
}

fn mapfunction(loc: &LocType) -> Option<u16> {
    (loc.mapfunction >= 0).then_some(loc.mapfunction as u16)
}

fn entity_npc(npcs: &[NpcType], id: i32) -> Option<&NpcType> {
    npcs.get(id as usize)
        .filter(|npc| npc.id == id)
        .or_else(|| npcs.iter().find(|npc| npc.id == id))
}

fn entity_loc(locs: &[LocType], id: i32) -> Option<&LocType> {
    locs.get(id as usize)
        .filter(|loc| loc.id == id)
        .or_else(|| locs.iter().find(|loc| loc.id == id))
}

fn push_issue(
    issues: &mut Vec<(CoverageReason, String, u32)>,
    reason: CoverageReason,
    reference: &str,
    count: u32,
) {
    if let Some(row) = issues
        .iter_mut()
        .find(|(existing, path, _)| *existing == reason && path == reference)
    {
        row.2 += count;
        return;
    }
    issues.push((reason, reference.to_string(), count));
}

fn unresolved_rows(mut issues: Vec<(CoverageReason, String, u32)>) -> Vec<CoverageIssue> {
    issues.sort_by(|a, b| a.1.cmp(&b.1));
    if issues.len() > 128 {
        let extra = issues.len() as u32 - 127;
        issues.truncate(127);
        issues.push((
            CoverageReason::UnresolvedHandler,
            format!("{extra} additional unresolved handlers"),
            extra,
        ));
    }
    issues
        .into_iter()
        .filter_map(|(reason, reference, count)| {
            Some(CoverageIssue {
                reason,
                reference: Text::new(&reference).ok()?,
                count,
            })
        })
        .collect()
}

fn pack_ids(content_root: &Path, file: &str) -> HashMap<String, i32> {
    let Ok(text) = std::fs::read_to_string(content_root.join("pack").join(file)) else {
        return HashMap::new();
    };
    let mut out = HashMap::new();
    for line in text.lines() {
        let Some((id, name)) = line.split_once('=') else {
            continue;
        };
        let Ok(id) = id.trim().parse::<i32>() else {
            continue;
        };
        let name = name.trim();
        if id >= 0 && !name.is_empty() {
            out.insert(name.to_string(), id);
        }
    }
    out
}

fn bind_pack_ids(cfgs: &mut HashMap<String, NamedConfig>, ids: &HashMap<String, i32>) {
    for (alias, cfg) in cfgs {
        cfg.packed_id = ids.get(alias).copied();
    }
}

struct NamedConfig {
    name: String,
    ops: [Option<String>; 5],
    category: Option<String>,
    packed_id: Option<i32>,
}

fn ingest_named_config(text: &str, out: &mut HashMap<String, NamedConfig>) {
    let mut header: Option<String> = None;
    let mut cur = NamedConfig {
        name: String::new(),
        ops: std::array::from_fn(|_| None),
        category: None,
        packed_id: None,
    };
    let flush = |header: &mut Option<String>,
                 cur: &mut NamedConfig,
                 out: &mut HashMap<String, NamedConfig>| {
        if let Some(name) = header.take() {
            out.entry(name).or_insert(std::mem::replace(
                cur,
                NamedConfig {
                    name: String::new(),
                    ops: std::array::from_fn(|_| None),
                    category: None,
                    packed_id: None,
                },
            ));
        }
    };
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(name) = named_header(line) {
            flush(&mut header, &mut cur, out);
            header = Some(name.to_string());
            continue;
        }
        if header.is_none() {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "name" => cur.name = value.to_string(),
            "category" => cur.category = Some(value.to_string()),
            key if key.starts_with("op") && key.len() == 3 => {
                if let Some(slot) = key.as_bytes().get(2).and_then(|b| {
                    let n = b.wrapping_sub(b'1');
                    (n < 5).then_some(n as usize)
                }) {
                    cur.ops[slot] = Some(value.to_string());
                }
            }
            _ => {}
        }
    }
    flush(&mut header, &mut cur, out);
}

fn named_header(line: &str) -> Option<&str> {
    line.strip_prefix('[')?.strip_suffix(']')
}

struct ScriptFile {
    rel: String,
    digest: Digest,
    restricted: bool,
    blocks: Vec<ScriptBlock>,
}

struct ScriptBlock {
    kind: String,
    name: String,
    params: Vec<String>,
    labels: Vec<String>,
    proc_calls: Vec<(String, Vec<String>)>,
    loc_changes: Vec<String>,
    mentions_openbank: bool,
    has_dialogue: bool,
    has_members: bool,
}

struct ScriptTree {
    npc_cfg: HashMap<String, NamedConfig>,
    loc_cfg: HashMap<String, NamedConfig>,
    files: Vec<ScriptFile>,
}

fn load_script_tree(content_root: &Path) -> Result<ScriptTree, String> {
    let mut npc_cfg = HashMap::new();
    let mut loc_cfg = HashMap::new();
    let mut files = Vec::new();
    visit_script_tree(
        content_root,
        &content_root.join("scripts"),
        &mut |rel, path| {
            let ext = path.extension().and_then(|s| s.to_str());
            match ext {
                Some("npc") => {
                    if let Ok(text) = std::fs::read_to_string(&path) {
                        ingest_named_config(&text, &mut npc_cfg);
                    }
                }
                Some("loc") => {
                    if let Ok(text) = std::fs::read_to_string(&path) {
                        ingest_named_config(&text, &mut loc_cfg);
                    }
                }
                Some("rs2") => {
                    if let Ok(text) = std::fs::read_to_string(&path) {
                        files.push(ScriptFile {
                            digest: Digest::of(text.as_bytes()),
                            restricted: rel.contains("/tutorial/"),
                            blocks: parse_rs2_blocks(&text),
                            rel,
                        });
                    }
                }
                _ => {}
            }
        },
    );
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(ScriptTree {
        npc_cfg,
        loc_cfg,
        files,
    })
}

fn parse_rs2_blocks(text: &str) -> Vec<ScriptBlock> {
    let mut blocks = Vec::new();
    let mut header: Option<(String, String, Vec<String>)> = None;
    let mut body = String::new();
    let flush = |header: &mut Option<(String, String, Vec<String>)>,
                 body: &mut String,
                 blocks: &mut Vec<ScriptBlock>| {
        if let Some((kind, name, params)) = header.take() {
            blocks.push(compact_block(kind, name, params, &std::mem::take(body)));
        }
    };
    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if let Some((kind, name)) = parse_rs2_header(line) {
            flush(&mut header, &mut body, &mut blocks);
            let rest = line
                .find(']')
                .map(|end| line[end + 1..].trim())
                .unwrap_or("");
            let (params, rest) = split_signature(rest);
            header = Some((kind, name, params));
            if !rest.is_empty() {
                body.push_str(rest);
                body.push('\n');
            }
            continue;
        }
        if header.is_some() {
            body.push_str(line);
            body.push('\n');
        }
    }
    flush(&mut header, &mut body, &mut blocks);
    blocks
}

fn compact_block(kind: String, name: String, params: Vec<String>, body: &str) -> ScriptBlock {
    let mut labels = at_names(body);
    labels.extend(multi_labels(body));
    ScriptBlock {
        kind,
        name,
        params,
        labels,
        proc_calls: extract_proc_calls(body),
        loc_changes: all_call_args(body, "loc_change")
            .into_iter()
            .filter_map(|args| args.into_iter().next())
            .collect(),
        mentions_openbank: body_mentions_openbank(body),
        has_dialogue: body_has_dialogue(body),
        has_members: body_has_members(body),
    }
}

fn split_signature(rest: &str) -> (Vec<String>, &str) {
    let rest = rest.trim_start();
    if !rest.starts_with('(') {
        return (Vec::new(), rest);
    }
    let mut depth = 0i32;
    for (index, ch) in rest.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    let params = rest[1..index]
                        .split(',')
                        .filter_map(|part| {
                            part.split_whitespace()
                                .find(|token| token.starts_with('$'))
                                .map(str::to_string)
                        })
                        .collect();
                    return (params, rest[index + 1..].trim_start());
                }
            }
            _ => {}
        }
    }
    (Vec::new(), rest)
}

fn parse_rs2_header(line: &str) -> Option<(String, String)> {
    let start = line.strip_prefix('[')?;
    let end = start.find(']')?;
    let inner = &start[..end];
    let mut parts = inner.splitn(2, ',');
    let kind = parts.next()?.trim().to_string();
    let name = parts.next().unwrap_or("").trim().to_string();
    Some((kind, name))
}

fn strip_comment(line: &str) -> &str {
    match line.find("//") {
        Some(index) => &line[..index],
        None => line,
    }
}

struct BankAnalysis {
    npc_ids: BTreeSet<i32>,
    loc_ids: BTreeSet<i32>,
    npc_services: BTreeMap<i32, Vec<HandlerService>>,
    loc_services: BTreeMap<i32, Vec<HandlerService>>,
    transitions: Vec<(i32, i32)>,
    transitions_from: BTreeMap<i32, BTreeSet<i32>>,
    transition_files: BTreeMap<(i32, i32), String>,
    unresolved: BTreeMap<String, Vec<String>>,
    file_digest: HashMap<String, Digest>,
}

#[derive(Clone)]
struct HandlerService {
    file: String,
    reference: String,
    trigger: ServiceTrigger,
    eligibility: Eligibility,
}

fn analyse_bank_handlers(
    scripts: &[ScriptFile],
    npc_ids: &HashMap<String, i32>,
    loc_ids: &HashMap<String, i32>,
    npc_cfg: &HashMap<String, NamedConfig>,
    loc_cfg: &HashMap<String, NamedConfig>,
) -> BankAnalysis {
    let mut blocks: HashMap<(&str, &str), Vec<(&ScriptFile, &ScriptBlock)>> = HashMap::new();
    for file in scripts {
        for block in &file.blocks {
            blocks
                .entry((block.kind.as_str(), block.name.as_str()))
                .or_default()
                .push((file, block));
        }
    }
    let mut analysis = BankAnalysis {
        npc_ids: BTreeSet::new(),
        loc_ids: BTreeSet::new(),
        npc_services: BTreeMap::new(),
        loc_services: BTreeMap::new(),
        transitions: Vec::new(),
        transitions_from: BTreeMap::new(),
        transition_files: BTreeMap::new(),
        unresolved: BTreeMap::new(),
        file_digest: HashMap::new(),
    };
    let mut openbank_memo = HashMap::new();
    for file in scripts {
        analysis.file_digest.insert(file.rel.clone(), file.digest);
        for block in &file.blocks {
            if let Some(slot) = op_slot(&block.kind) {
                if reaches_openbank(block, &blocks, &mut openbank_memo) {
                    let eligibility = if file.restricted {
                        Eligibility::Restricted
                    } else if block.has_dialogue || block.has_members {
                        Eligibility::Conditional
                    } else {
                        Eligibility::Unknown
                    };
                    let trigger = if eligibility == Eligibility::Conditional && block.has_dialogue {
                        ServiceTrigger::Dialogue
                    } else {
                        ServiceTrigger::Operation {
                            slot: OperationSlot::new(slot)
                                .unwrap_or(OperationSlot::new(1).unwrap()),
                        }
                    };
                    let service = HandlerService {
                        file: file.rel.clone(),
                        reference: format!("{}:{},{}", file.rel, block.kind, block.name),
                        trigger,
                        eligibility,
                    };
                    record_handler(
                        &block.kind,
                        &block.name,
                        service,
                        npc_ids,
                        loc_ids,
                        npc_cfg,
                        loc_cfg,
                        &mut analysis,
                    );
                }
            }
            record_transitions(file, block, loc_ids, &blocks, &mut analysis);
        }
    }
    analysis
}

#[allow(clippy::too_many_arguments)]
fn record_handler(
    kind: &str,
    name: &str,
    service: HandlerService,
    npc_ids: &HashMap<String, i32>,
    loc_ids: &HashMap<String, i32>,
    npc_cfg: &HashMap<String, NamedConfig>,
    loc_cfg: &HashMap<String, NamedConfig>,
    analysis: &mut BankAnalysis,
) {
    let npc = kind.contains("npc");
    let ids = if npc { npc_ids } else { loc_ids };
    let cfgs = if npc { npc_cfg } else { loc_cfg };
    let resolved = resolve_targets(name, ids, cfgs);
    if resolved.is_empty() {
        analysis
            .unresolved
            .entry(service.reference.clone())
            .or_default()
            .push(name.to_string());
        return;
    }
    for id in resolved {
        if npc {
            analysis.npc_ids.insert(id);
            analysis
                .npc_services
                .entry(id)
                .or_default()
                .push(service.clone());
        } else {
            analysis.loc_ids.insert(id);
            analysis
                .loc_services
                .entry(id)
                .or_default()
                .push(service.clone());
        }
    }
}

fn resolve_targets(
    name: &str,
    ids: &HashMap<String, i32>,
    cfgs: &HashMap<String, NamedConfig>,
) -> Vec<i32> {
    if let Some(category) = name.strip_prefix('_') {
        return cfgs
            .iter()
            .filter(|(_, cfg)| cfg.category.as_deref() == Some(category))
            .filter_map(|(alias, _)| ids.get(alias).copied())
            .collect();
    }
    if let Some(id) = ids.get(name).copied() {
        return vec![id];
    }
    if let Some(id) = name
        .strip_prefix("loc_")
        .or_else(|| name.strip_prefix("npc_"))
        .and_then(|id| id.parse().ok())
    {
        if ids.values().any(|packed| *packed == id) {
            return vec![id];
        }
    }
    Vec::new()
}

fn record_transitions<'a>(
    file: &'a ScriptFile,
    block: &'a ScriptBlock,
    loc_ids: &HashMap<String, i32>,
    blocks: &HashMap<(&'a str, &'a str), Vec<(&'a ScriptFile, &'a ScriptBlock)>>,
    analysis: &mut BankAnalysis,
) {
    if !block.kind.starts_with("oploc") && !block.kind.starts_with("aploc") {
        return;
    }
    let Some(from) = loc_ids.get(&block.name).copied().or_else(|| {
        block
            .name
            .strip_prefix("loc_")
            .and_then(|id| id.parse().ok())
            .filter(|id| loc_ids.values().any(|packed| packed == id))
    }) else {
        return;
    };
    let mut seen = HashSet::new();
    let targets = loc_change_targets(block, &HashMap::new(), blocks, true, &mut seen);
    for target in targets {
        let Some(to) = resolve_loc_name(&target, loc_ids) else {
            continue;
        };
        analysis.transitions.push((from, to));
        analysis
            .transitions_from
            .entry(from)
            .or_default()
            .insert(to);
        analysis
            .transition_files
            .entry((from, to))
            .or_insert_with(|| file.rel.clone());
    }
}

fn loc_change_targets<'a>(
    block: &'a ScriptBlock,
    bindings: &HashMap<String, String>,
    blocks: &HashMap<(&'a str, &'a str), Vec<(&'a ScriptFile, &'a ScriptBlock)>>,
    follow_procs: bool,
    seen: &mut HashSet<(&'a str, &'a str)>,
) -> Vec<String> {
    if !seen.insert((block.kind.as_str(), block.name.as_str())) {
        return Vec::new();
    }
    let mut targets = Vec::new();
    for raw in &block.loc_changes {
        targets.push(substitute(raw, bindings));
    }
    for label in &block.labels {
        if let Some(next) = blocks.get(&("label", label.as_str())) {
            for (_, nested) in next {
                targets.extend(loc_change_targets(nested, bindings, blocks, false, seen));
            }
        }
    }
    if follow_procs {
        for (proc, args) in &block.proc_calls {
            let Some(next) = blocks.get(&("proc", proc.as_str())) else {
                continue;
            };
            for (_, nested) in next {
                let bound: Vec<String> = args.iter().map(|arg| substitute(arg, bindings)).collect();
                let nested_bindings = bind_params(&nested.params, &bound);
                targets.extend(loc_change_targets(
                    nested,
                    &nested_bindings,
                    blocks,
                    false,
                    seen,
                ));
            }
        }
    }
    targets
}

fn bind_params(params: &[String], args: &[String]) -> HashMap<String, String> {
    params
        .iter()
        .zip(args.iter())
        .map(|(param, arg)| (param.clone(), arg.clone()))
        .collect()
}

fn substitute(arg: &str, bindings: &HashMap<String, String>) -> String {
    let mut out = arg.trim().to_string();
    for (param, value) in bindings {
        if out == *param {
            return value.clone();
        }
        out = out.replace(param, value);
    }
    out
}

fn all_call_args(body: &str, func: &str) -> Vec<Vec<String>> {
    let needle = format!("{func}(");
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(pos) = rest.find(&needle) {
        if let Some(args) = parse_args(&rest[pos + needle.len()..]) {
            out.push(args);
        }
        rest = &rest[pos + needle.len()..];
    }
    out
}

fn resolve_loc_name(name: &str, loc_ids: &HashMap<String, i32>) -> Option<i32> {
    let name = name.trim();
    if let Some(id) = loc_ids.get(name) {
        return Some(*id);
    }
    name.strip_prefix("loc_")
        .and_then(|id| id.parse().ok())
        .filter(|id| loc_ids.values().any(|packed| packed == id))
}

fn reaches_openbank<'a>(
    start: &'a ScriptBlock,
    blocks: &HashMap<(&'a str, &'a str), Vec<(&'a ScriptFile, &'a ScriptBlock)>>,
    memo: &mut HashMap<(&'a str, &'a str), bool>,
) -> bool {
    reaches_openbank_inner(start, blocks, memo, &mut HashSet::new())
}

fn reaches_openbank_inner<'a>(
    block: &'a ScriptBlock,
    blocks: &HashMap<(&'a str, &'a str), Vec<(&'a ScriptFile, &'a ScriptBlock)>>,
    memo: &mut HashMap<(&'a str, &'a str), bool>,
    visiting: &mut HashSet<(&'a str, &'a str)>,
) -> bool {
    let key = (block.kind.as_str(), block.name.as_str());
    if let Some(hit) = memo.get(&key) {
        return *hit;
    }
    if !visiting.insert(key) {
        return false;
    }
    let hit = (block.kind == "label" && block.name == "openbank")
        || block.mentions_openbank
        || block.labels.iter().any(|label| {
            blocks
                .get(&("label", label.as_str()))
                .into_iter()
                .flatten()
                .any(|(_, nested)| reaches_openbank_inner(nested, blocks, memo, visiting))
        });
    visiting.remove(&key);
    memo.insert(key, hit);
    hit
}

fn body_mentions_openbank(body: &str) -> bool {
    body.contains("@openbank") || body.contains("~openbank(")
}

fn body_has_dialogue(body: &str) -> bool {
    body.contains("~p_choice")
        || body.contains("@multi")
        || body.contains("~chatnpc")
        || body.contains("~chatplayer")
        || body.contains("~chatnpcrange")
}

fn body_has_members(body: &str) -> bool {
    body.contains("map_members")
}

fn op_slot(kind: &str) -> Option<u8> {
    let rest = kind
        .strip_prefix("opnpc")
        .or_else(|| kind.strip_prefix("apnpc"))
        .or_else(|| kind.strip_prefix("oploc"))
        .or_else(|| kind.strip_prefix("aploc"))?;
    rest.parse().ok().filter(|slot| (1..=5).contains(slot))
}

fn at_names(body: &str) -> Vec<String> {
    let mut names = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'@' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            if end > start {
                names.push(body[start..end].to_string());
            }
            i = end;
        } else {
            i += 1;
        }
    }
    names
}

fn extract_proc_calls(body: &str) -> Vec<(String, Vec<String>)> {
    let mut calls = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'~' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            if end > start && end < bytes.len() && bytes[end] == b'(' {
                let name = body[start..end].to_string();
                let args = parse_args(&body[end + 1..]).unwrap_or_default();
                calls.push((name, args));
            }
            i = end;
        } else {
            i += 1;
        }
    }
    calls
}

fn multi_labels(body: &str) -> Vec<String> {
    let mut labels = Vec::new();
    for n in 2..=5 {
        let name = format!("@multi{n}");
        if let Some(args) = first_call_args(body, &name) {
            for (index, arg) in args.iter().enumerate() {
                if index % 2 == 1 {
                    let ident = arg.trim();
                    if ident.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                        && ident.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                    {
                        labels.push(ident.to_string());
                    }
                }
            }
        }
    }
    labels
}

fn first_call_args(body: &str, func: &str) -> Option<Vec<String>> {
    let needle = format!("{func}(");
    let pos = body.find(&needle)?;
    parse_args(&body[pos + needle.len()..])
}

fn parse_args(after_open: &str) -> Option<Vec<String>> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    let mut in_str = false;
    for ch in after_open.chars() {
        match ch {
            '"' if !in_str => in_str = true,
            '"' if in_str => in_str = false,
            '(' if !in_str => {
                depth += 1;
                cur.push(ch);
            }
            ')' if !in_str && depth == 0 => {
                let arg = cur.trim();
                if !arg.is_empty() || !args.is_empty() {
                    args.push(arg.to_string());
                }
                return Some(args);
            }
            ')' if !in_str => {
                depth -= 1;
                cur.push(ch);
            }
            ',' if !in_str && depth == 0 => {
                args.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    None
}

struct NpcHit {
    x: i32,
    z: i32,
    plane: u8,
}

struct LocHit {
    x: i32,
    z: i32,
    plane: u8,
    shape: u8,
    rotation: u8,
    link_below: bool,
}

#[allow(clippy::type_complexity)]
fn scan_maps(
    content_root: &Path,
    interesting_npcs: &HashSet<i32>,
    interesting_locs: &HashSet<i32>,
) -> Result<
    (
        usize,
        BTreeMap<i32, Vec<NpcHit>>,
        BTreeMap<i32, Vec<LocHit>>,
    ),
    String,
> {
    let dir = content_root.join("maps");
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) => return Err(format!("maps {}: {e}", dir.display())),
    };
    let mut npc_hits: BTreeMap<i32, Vec<NpcHit>> = BTreeMap::new();
    let mut loc_hits: BTreeMap<i32, Vec<LocHit>> = BTreeMap::new();
    let mut map_count = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some((mx, mz)) = mapsquare_coords(name) else {
            continue;
        };
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("map {}: {e}", path.display()))?;
        map_count += 1;
        collect_hits_from_map(
            mx,
            mz,
            &text,
            interesting_npcs,
            interesting_locs,
            &mut npc_hits,
            &mut loc_hits,
        );
    }
    for hits in npc_hits.values_mut() {
        hits.sort_by_key(|hit| (hit.plane, hit.x, hit.z));
    }
    for hits in loc_hits.values_mut() {
        hits.sort_by_key(|hit| (hit.plane, hit.x, hit.z, hit.shape, hit.rotation));
    }
    Ok((map_count, npc_hits, loc_hits))
}

fn collect_hits_from_map(
    mx: i32,
    mz: i32,
    text: &str,
    interesting_npcs: &HashSet<i32>,
    interesting_locs: &HashSet<i32>,
    npc_hits: &mut BTreeMap<i32, Vec<NpcHit>>,
    loc_hits: &mut BTreeMap<i32, Vec<LocHit>>,
) {
    let mut section = None;
    let mut link_below = [0u64; 64];
    let mut pending_locs = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = section_name(line) {
            section = Some(name);
            continue;
        }
        match section {
            Some("MAP") => {
                if interesting_locs.is_empty() || line.as_bytes().first() != Some(&b'1') {
                    continue;
                }
                let Some((level, x, z, flags)) = crate::pack::parse_map_line(line) else {
                    continue;
                };
                if level == 1 && flags & 2 != 0 && x < 64 && z < 64 {
                    link_below[z] |= 1u64 << x;
                }
            }
            Some("NPC") => {
                let Some((plane, lx, lz, id)) = parse_npc_line(line) else {
                    continue;
                };
                if interesting_npcs.contains(&id) {
                    npc_hits.entry(id).or_default().push(NpcHit {
                        x: mx * 64 + lx,
                        z: mz * 64 + lz,
                        plane,
                    });
                }
            }
            Some("LOC") => {
                if interesting_locs.is_empty() {
                    continue;
                }
                let Some(id) = loc_line_id(line) else {
                    continue;
                };
                if !interesting_locs.contains(&id) {
                    continue;
                }
                let Some((plane, lx, lz, id, shape, rotation)) = parse_loc_line(line) else {
                    continue;
                };
                pending_locs.push((plane, lx, lz, id, shape, rotation));
            }
            _ => {}
        }
    }
    for (plane, lx, lz, id, shape, rotation) in pending_locs {
        let linked = (lx as usize) < 64
            && (lz as usize) < 64
            && link_below[lz as usize] & (1u64 << lx as usize) != 0;
        if crate::collision::game_plane(i32::from(plane), linked).is_none() {
            continue;
        }
        loc_hits.entry(id).or_default().push(LocHit {
            x: mx * 64 + lx,
            z: mz * 64 + lz,
            plane,
            shape,
            rotation,
            link_below: linked,
        });
    }
}

fn loc_line_id(line: &str) -> Option<i32> {
    line.split_once(':')?
        .1
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn mapsquare_coords(name: &str) -> Option<(i32, i32)> {
    let rest = name.strip_prefix('m')?.strip_suffix(".jm2")?;
    let (x, z) = rest.split_once('_')?;
    Some((x.parse().ok()?, z.parse().ok()?))
}

fn section_name(line: &str) -> Option<&str> {
    crate::pack::section(line)
}

fn parse_npc_line(line: &str) -> Option<(u8, i32, i32, i32)> {
    let (coords, data) = line.split_once(':')?;
    let mut c = coords.split_whitespace();
    let plane: i32 = c.next()?.parse().ok()?;
    let lx: i32 = c.next()?.parse().ok()?;
    let lz: i32 = c.next()?.parse().ok()?;
    if c.next().is_some()
        || !(0..=3).contains(&plane)
        || !(0..=63).contains(&lx)
        || !(0..=63).contains(&lz)
    {
        return None;
    }
    let mut d = data.split_whitespace();
    let id: i32 = d.next()?.parse().ok()?;
    if d.next().is_some() || id < 0 {
        return None;
    }
    Some((plane as u8, lx, lz, id))
}

fn parse_loc_line(line: &str) -> Option<(u8, i32, i32, i32, u8, u8)> {
    let loc = crate::pack::parse_loc_fields(line)?;
    if loc.level < 0
        || loc.level > 3
        || loc.shape < 0
        || loc.shape > 22
        || loc.angle < 0
        || loc.angle > 3
    {
        return None;
    }
    Some((
        loc.level as u8,
        loc.x as i32,
        loc.z as i32,
        loc.loc_id,
        loc.shape as u8,
        loc.angle as u8,
    ))
}

fn visit_script_tree(root: &Path, dir: &Path, cb: &mut impl FnMut(String, PathBuf)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == "_test" || name == "_unpack" || name == ".git" {
                continue;
            }
            visit_script_tree(root, &path, cb);
        } else if matches!(
            path.extension().and_then(|s| s.to_str()),
            Some("npc" | "loc" | "rs2")
        ) {
            cb(relative(root, &path), path);
        }
    }
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
#[path = "services_tests.rs"]
mod tests;
