//! Client-only POI derivation, isolated from terrain policy so catalogue-only
//! classifier changes do not invalidate already-baked imagery.

use super::client_cache::{load_config, CacheReader};
use super::formats::{
    ClientPois, Coverage, CoverageIssue, CoverageLevel, CoverageReason, MAX_POIS,
};
use super::identity::{CatalogueIdentity, CATALOGUE_SCHEMA};
use super::poi::{
    classify_definition, CapabilityEvidence, Definition, DisplayAnchor, EntityKind, Footprint,
    PoiKey, PoiKind, PoiRecord, SourceSpace,
};
use super::producer::ClientMapInput;
use super::{MapError, Rows, Text};
use client::config::{LocType, NpcType};
use client::io::ClientRevision;
use client::map_cache::MAP_SQUARE_SIZE;
use std::collections::{BTreeSet, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};

#[derive(Debug, Clone)]
struct LocDefinition {
    name: String,
    operations: [Option<String>; 5],
    width: u8,
    length: u8,
    active: bool,
    mapfunction: Option<u16>,
}

impl LocDefinition {
    fn footprint(&self, rotation: u8) -> Result<Footprint, MapError> {
        Footprint {
            width: self.width,
            length: self.length,
        }
        .rotated(rotation)
    }
}

/// World-map Key row for rare-tree groves (`WORLDMAP_KEY_NAMES[34]`). Marker
/// locs carry only this mapfunction; the per-placement tree name comes from a
/// nearby woodcutting loc placement already visited by the generator.
const RARE_TREES_SYMBOL: u16 = 34;

/// World-map Key row for quest starts (`WORLDMAP_KEY_NAMES[6]`). The
/// content-derived relation is optional; unmatched markers remain generic.
const QUEST_STARTS_SYMBOL: u16 = 6;
const MAX_QUEST_START_MATCH_DIST2: f64 = 8.0 * 8.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuestStartTargetKind {
    Npc,
    Loc,
}

#[derive(Debug, Clone)]
struct QuestStartEntry {
    display: String,
    kind: QuestStartTargetKind,
    id: i32,
}

#[derive(Debug, Clone, Copy)]
struct QuestStartPlacement {
    id: i32,
    x: i32,
    z: i32,
    plane: u8,
}

#[derive(Debug, Clone, Default)]
struct QuestStartCatalog {
    entries: Vec<QuestStartEntry>,
    npc_placements: Vec<QuestStartPlacement>,
    loc_ids: HashSet<i32>,
}

fn quest_start_catalog(input: ClientMapInput<'_>) -> Option<QuestStartCatalog> {
    let revision = match input.revision {
        274 => ClientRevision::R274,
        289 => ClientRevision::R289,
        _ => return None,
    };
    let data = api::game_data::for_revision(revision).ok()?;
    let content_id = input.content.to_string();
    if data.content_id() != Some(content_id.as_str()) {
        return None;
    }
    let starts = data.quest_starts()?;
    let identities = data.quest_identity()?;
    let entries = starts
        .rows
        .iter()
        .filter_map(|row| {
            let display = identities
                .rows
                .iter()
                .find(|identity| identity.id == row.quest)?
                .display
                .clone();
            let kind = match row.target.kind.as_str() {
                "npc" => QuestStartTargetKind::Npc,
                "loc" => QuestStartTargetKind::Loc,
                _ => return None,
            };
            Some(QuestStartEntry {
                display,
                kind,
                id: row.target.id,
            })
        })
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return None;
    }
    let npc_ids = entries
        .iter()
        .filter_map(|entry| (entry.kind == QuestStartTargetKind::Npc).then_some(entry.id))
        .collect::<HashSet<_>>();
    let npc_placements = data
        .npc_placements()
        .into_iter()
        .flat_map(|facts| facts.rows.iter())
        .filter(|placement| npc_ids.contains(&placement.npc_id))
        .map(|placement| QuestStartPlacement {
            id: placement.npc_id,
            x: placement.x,
            z: placement.z,
            plane: u8::try_from(placement.plane).unwrap_or(u8::MAX),
        })
        .collect();
    let loc_ids = entries
        .iter()
        .filter_map(|entry| (entry.kind == QuestStartTargetKind::Loc).then_some(entry.id))
        .collect();
    Some(QuestStartCatalog {
        entries,
        npc_placements,
        loc_ids,
    })
}

fn quest_start_name(
    catalog: &QuestStartCatalog,
    marker: &PoiRecord,
    loc_placements: &[QuestStartPlacement],
) -> Option<Text> {
    quest_start_name_at(
        catalog,
        marker.effective_plane,
        marker.display.x,
        marker.display.z,
        loc_placements,
    )
}

fn quest_start_name_at(
    catalog: &QuestStartCatalog,
    plane: u8,
    centre_x: f64,
    centre_z: f64,
    loc_placements: &[QuestStartPlacement],
) -> Option<Text> {
    let mut names = BTreeSet::new();
    for entry in &catalog.entries {
        let matched = match entry.kind {
            QuestStartTargetKind::Npc => catalog.npc_placements.iter().any(|placement| {
                placement.id == entry.id
                    && placement.plane == plane
                    && nearby(placement.x, placement.z, centre_x, centre_z)
            }),
            QuestStartTargetKind::Loc => loc_placements.iter().any(|placement| {
                placement.id == entry.id
                    && placement.plane == plane
                    && nearby(placement.x, placement.z, centre_x, centre_z)
            }),
        };
        if matched {
            names.insert(entry.display.clone());
        }
    }
    if names.is_empty() {
        return None;
    }
    let joined = names.into_iter().collect::<Vec<_>>().join(", ");
    Text::new(&joined).ok()
}

fn nearby(x: i32, z: i32, centre_x: f64, centre_z: f64) -> bool {
    let dx = f64::from(x) + 0.5 - centre_x;
    let dz = f64::from(z) + 0.5 - centre_z;
    dx * dx + dz * dz <= MAX_QUEST_START_MATCH_DIST2
}

/// A placed tree the generator already visited: non-empty client-cache name
/// with a Chop-down operation. Content-derived (client cache ops), never a
/// handwritten id table.
fn is_woodcut_tree(definition: &LocDefinition) -> bool {
    !definition.name.is_empty()
        && definition
            .operations
            .iter()
            .flatten()
            .any(|op| op.trim().eq_ignore_ascii_case("Chop down"))
}

/// One visited tree placement for Rare-Trees renaming: world origin, rotated
/// footprint and game plane plus the loc definition id for its name.
struct TreePlacement {
    x: i32,
    z: i32,
    width: u8,
    length: u8,
    plane: u8,
    id: u32,
}

/// Maximum centre distance (tiles) from a Rare-Trees marker to its tree.
/// Same-tile markers match at ~0-1; grove-centre markers within a few tiles.
/// Beyond this the marker stays an honest generic label.
const MAX_TREE_MATCH_DIST2: f64 = 8.0 * 8.0;

fn load_loc_definitions(jag: &client::io::JagFile) -> Result<Vec<LocDefinition>, MapError> {
    let decoded_locs = catch_unwind(AssertUnwindSafe(|| LocType::unpack(jag)))
        .map_err(|_| MapError::Invalid("location definitions"))?;
    if decoded_locs.is_empty() {
        return Err(MapError::Invalid("missing location definitions"));
    }
    let mut locs = Vec::with_capacity(decoded_locs.len());
    for loc in decoded_locs {
        let width = u8::try_from(loc.width).map_err(|_| MapError::Invalid("location width"))?;
        let length = u8::try_from(loc.length).map_err(|_| MapError::Invalid("location length"))?;
        Footprint { width, length }.validate()?;
        locs.push(LocDefinition {
            name: loc.name,
            operations: std::array::from_fn(|index| loc.op.get(index).cloned().flatten()),
            width,
            length,
            active: loc.active,
            mapfunction: optional_u16(loc.mapfunction, "mapfunction id")?,
        });
    }
    Ok(locs)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CatalogueStats {
    pub map_squares: u32,
    pub loc_placements: u64,
    pub retained_records: u32,
    pub mapfunction_records: u32,
    pub physical_service_records: u32,
    pub npc_definitions: u32,
    pub npc_service_definitions: u32,
    pub max_record_bytes: u32,
}

pub(super) fn derive_client_pois(
    input: ClientMapInput<'_>,
    identity: CatalogueIdentity,
) -> Result<(ClientPois, CatalogueStats), MapError> {
    if identity.revision != input.revision || identity.content != input.content {
        return Err(MapError::Identity);
    }
    let config = load_config(input)?;
    let locs = load_loc_definitions(&config)?;
    let decoded_npcs = catch_unwind(AssertUnwindSafe(|| NpcType::unpack(&config)))
        .map_err(|_| MapError::Invalid("NPC definitions"))?;
    let npc_definitions = decoded_npcs.len() as u32;
    let npc_service_definitions = decoded_npcs
        .iter()
        .filter(|npc| {
            let definition = Definition {
                entity: EntityKind::Npc,
                name: &npc.name,
                operations: std::array::from_fn(|index| {
                    npc.op.get(index).and_then(|operation| operation.as_deref())
                }),
                active: true,
                mapfunction: None,
            };
            let mut service = false;
            classify_definition(input.revision, &definition, |_, _| service = true);
            service
        })
        .count() as u32;
    let quest_starts = quest_start_catalog(input);

    let mut reader = CacheReader::open(input)?;
    let entries = reader.entries.clone();
    let mut records = Vec::new();
    let mut stats = CatalogueStats {
        map_squares: entries.len() as u32,
        npc_definitions,
        npc_service_definitions,
        ..CatalogueStats::default()
    };
    let mut unknown_mapfunctions = 0u32;
    let mut woodcut_trees: Vec<TreePlacement> = Vec::new();
    let mut quest_start_loc_placements: Vec<QuestStartPlacement> = Vec::new();

    for entry in entries {
        let land = reader.read_land(entry)?;
        let mut placement_error = None;
        reader.visit_locs(entry, |placement| {
            stats.loc_placements += 1;
            if placement_error.is_some() {
                return;
            }
            let Some(definition) = locs.get(placement.id as usize) else {
                placement_error = Some(MapError::Invalid("location definition id"));
                return;
            };
            let link_below = land.link_below(placement.x, placement.z);
            let source = SourceSpace::ClientVisual {
                plane: placement.plane,
                link_below,
            };
            let effective_plane = match source.game_plane() {
                Ok(Some(plane)) => plane,
                Ok(None) => return,
                Err(error) => {
                    placement_error = Some(error);
                    return;
                }
            };
            let footprint = match definition.footprint(placement.rotation) {
                Ok(footprint) => footprint,
                Err(error) => {
                    placement_error = Some(error);
                    return;
                }
            };
            let world_x =
                i32::from(entry.square_x) * i32::from(MAP_SQUARE_SIZE) + i32::from(placement.x);
            let world_z =
                i32::from(entry.square_z) * i32::from(MAP_SQUARE_SIZE) + i32::from(placement.z);
            if let Some(catalog) = &quest_starts {
                if let Ok(id) = i32::try_from(placement.id) {
                    if catalog.loc_ids.contains(&id) {
                        quest_start_loc_placements.push(QuestStartPlacement {
                            id,
                            x: world_x,
                            z: world_z,
                            plane: effective_plane,
                        });
                    }
                }
            }
            // Remember every visited woodcutting tree for Rare-Trees renaming
            // below. Uses only the generator's own placements + client-cache
            // ops/names, never a handwritten coordinate table.
            if is_woodcut_tree(definition) {
                woodcut_trees.push(TreePlacement {
                    x: world_x,
                    z: world_z,
                    width: footprint.width,
                    length: footprint.length,
                    plane: effective_plane,
                    id: placement.id,
                });
            }
            let name = match poi_name(definition) {
                Ok(Some(name)) => name,
                // Nameless and off the Key legend: no picker label exists.
                Ok(None) => return,
                Err(error) => {
                    placement_error = Some(error);
                    return;
                }
            };
            let mut physical_kind = None;
            let mut physical_evidence = Vec::new();
            let mut annotation = None;
            classify_definition(input.revision, &borrowed(definition), |kind, evidence| {
                if matches!(evidence, CapabilityEvidence::MapFunction { .. }) {
                    annotation = Some((kind, evidence));
                } else {
                    if physical_kind
                        .is_none_or(|current| poi_priority(kind) < poi_priority(current))
                    {
                        physical_kind = Some(kind);
                    }
                    physical_evidence.push(evidence);
                }
            });
            let make_record =
                |entity, kind, evidence: Vec<CapabilityEvidence>| -> Result<PoiRecord, MapError> {
                    Ok(PoiRecord {
                        key: PoiKey {
                            entity,
                            id: placement.id,
                            x: world_x,
                            z: world_z,
                            source,
                            shape: placement.shape,
                            rotation: placement.rotation,
                        },
                        name: name.clone(),
                        kind,
                        effective_plane,
                        footprint,
                        display: DisplayAnchor {
                            x: f64::from(world_x) + f64::from(footprint.width) / 2.0,
                            z: f64::from(world_z) + f64::from(footprint.length) / 2.0,
                            plane: effective_plane,
                        },
                        evidence: Rows::new(evidence)?,
                        walk_target: None,
                    })
                };
            if let Some(kind) = physical_kind {
                if records.len() == MAX_POIS {
                    placement_error = Some(MapError::Limit("POI count"));
                    return;
                }
                match make_record(EntityKind::Loc, kind, physical_evidence) {
                    Ok(record) => records.push(record),
                    Err(error) => {
                        placement_error = Some(error);
                        return;
                    }
                }
            }
            if let Some((kind, evidence)) = annotation {
                if records.len() == MAX_POIS {
                    placement_error = Some(MapError::Limit("POI count"));
                    return;
                }
                // Rare-Trees markers resolve to per-placement tree names below;
                // only still-generic ones count as unknown there.
                if matches!(kind, PoiKind::MapSymbol { symbol } if symbol != RARE_TREES_SYMBOL) {
                    unknown_mapfunctions += 1;
                }
                let evidence = match Rows::new(vec![evidence]) {
                    Ok(evidence) => evidence,
                    Err(error) => {
                        placement_error = Some(error);
                        return;
                    }
                };
                records.push(PoiRecord {
                    key: PoiKey {
                        entity: EntityKind::MapFunction,
                        id: placement.id,
                        x: world_x,
                        z: world_z,
                        source,
                        shape: placement.shape,
                        rotation: placement.rotation,
                    },
                    name,
                    kind,
                    effective_plane,
                    footprint,
                    display: DisplayAnchor {
                        x: f64::from(world_x) + f64::from(footprint.width) / 2.0,
                        z: f64::from(world_z) + f64::from(footprint.length) / 2.0,
                        plane: effective_plane,
                    },
                    evidence,
                    walk_target: None,
                });
                stats.mapfunction_records += 1;
            }
        })?;
        if let Some(error) = placement_error {
            return Err(error);
        }
    }
    // Replace generic Rare-Trees annotation names with the nearest visited
    // woodcutting tree's client-cache loc name on the same plane. Markers and
    // trees share the generator's own placements (same tile for per-tree
    // markers, a few tiles for grove-centre markers); no handwritten table.
    // Already-specific MapSymbol 34 records (tree locs carrying their own
    // mapfunction) keep their names and never count as unknown.
    for record in records.iter_mut() {
        let PoiKind::MapSymbol { symbol } = record.kind else {
            continue;
        };
        if symbol != RARE_TREES_SYMBOL {
            continue;
        }
        if record.name.as_str() != "Rare Trees" {
            continue;
        }
        let mut best: Option<(f64, u32)> = None;
        for tree in &woodcut_trees {
            if tree.plane != record.effective_plane {
                continue;
            }
            let tree_cx = f64::from(tree.x) + f64::from(tree.width) / 2.0;
            let tree_cz = f64::from(tree.z) + f64::from(tree.length) / 2.0;
            let dx = tree_cx - record.display.x;
            let dz = tree_cz - record.display.z;
            let dist2 = dx * dx + dz * dz;
            if dist2 > MAX_TREE_MATCH_DIST2 {
                continue;
            }
            if best.is_none_or(|(best_dist2, _)| dist2 < best_dist2) {
                best = Some((dist2, tree.id));
            }
        }
        let Some((_, tree_id)) = best else {
            unknown_mapfunctions += 1;
            continue;
        };
        let Some(tree_def) = locs.get(tree_id as usize) else {
            unknown_mapfunctions += 1;
            continue;
        };
        match Text::new(&tree_def.name) {
            Ok(name) => record.name = name,
            Err(_) => unknown_mapfunctions += 1,
        }
    }
    if let Some(catalog) = &quest_starts {
        for record in records.iter_mut() {
            if !matches!(
                record.kind,
                PoiKind::MapSymbol {
                    symbol: QUEST_STARTS_SYMBOL
                }
            ) || record.name.as_str() != "Quest Start"
            {
                continue;
            }
            if let Some(name) = quest_start_name(catalog, record, &quest_start_loc_placements) {
                record.name = name;
                unknown_mapfunctions = unknown_mapfunctions.saturating_sub(1);
            }
        }
    }

    // Physical and annotation records use different entity tags, so this also
    // catches a genuinely duplicated source placement instead of merging by name.
    records.sort_unstable_by_key(|record| record.key);
    stats.retained_records = records.len() as u32;
    stats.physical_service_records = records
        .iter()
        .filter(|record| record.key.entity == EntityKind::Loc)
        .count() as u32;
    stats.max_record_bytes = reader.max_buffer_bytes();

    let mut unresolved = vec![
        CoverageIssue {
            reason: CoverageReason::MissingNpcPlacements,
            reference: Text::new("npc.dat contains definitions but no static world placements")?,
            count: npc_service_definitions,
        },
        CoverageIssue {
            reason: CoverageReason::MissingLabels,
            reference: Text::new("ordinary client cache has no world place-label placements")?,
            count: 1,
        },
    ];
    if unknown_mapfunctions != 0 {
        unresolved.push(CoverageIssue {
            reason: CoverageReason::UnknownMapFunction,
            reference: Text::new("unmapped client mapfunction categories remain generic")?,
            count: unknown_mapfunctions,
        });
    }
    let document = ClientPois {
        schema: CATALOGUE_SCHEMA,
        identity,
        coverage: Coverage {
            npc_placements: CoverageLevel::Unavailable,
            bank_services: CoverageLevel::Limited,
            place_labels: CoverageLevel::Unavailable,
            unresolved: Rows::new(unresolved)?,
        },
        records: Rows::new(records)?,
    };
    document.validate(identity)?;
    Ok((document, stats))
}

fn borrowed(definition: &LocDefinition) -> Definition<'_> {
    Definition {
        entity: EntityKind::Loc,
        name: &definition.name,
        operations: std::array::from_fn(|index| definition.operations[index].as_deref()),
        active: definition.active,
        mapfunction: definition.mapfunction,
    }
}

/// Classic worldmap Key legend names; index = client mapfunction sprite id.
/// Verbatim `WORLDMAP_KEY_NAMES` from the frozen rs2b0t pin
/// (`src/client/mapview/worldmapKeyNames.ts`), the list its world map Key and
/// WalkTo picker (`src/bot/panel/mapPickerTheme.ts` `keyNameToTypeId`) share.
const WORLDMAP_KEY_NAMES: [&str; 49] = [
    "General Store",
    "Sword Shop",
    "Magic Shop",
    "Axe Shop",
    "Helmet Shop",
    "Bank",
    "Quest Start",
    "Amulet Shop",
    "Mining Site",
    "Furnace",
    "Anvil",
    "Combat Training",
    "Dungeon",
    "Staff Shop",
    "Platebody Shop",
    "Platelegs Shop",
    "Scimitar Shop",
    "Archery Shop",
    "Shield Shop",
    "Altar",
    "Herbalist",
    "Jewelery",
    "Gem Shop",
    "Crafting Shop",
    "Candle Shop",
    "Fishing Shop",
    "Fishing Spot",
    "Clothes Shop",
    "Apothecary",
    "Silk Trader",
    "Kebab Seller",
    "Pub/Bar",
    "Mace Shop",
    "Tannery",
    "Rare Trees",
    "Spinning Wheel",
    "Food Shop",
    "Cookery Shop",
    "???",
    "Water Source",
    "Cooking Range",
    "Skirt Shop",
    "Potters Wheel",
    "Windmill",
    "Mining Shop",
    "Chainmail Shop",
    "Silver Shop",
    "Fur Trader",
    "Spice Shop",
];

/// The Key legend name rs2b0t shows for a mapfunction. Jagex's `???` row (38)
/// is shown as "Minigames" (rs2b0t `src/bot/runtime/Settings.ts`
/// `keyIconTypes.optionLabels`). Ids past the legend have no name there.
fn mapfunction_key_name(symbol: u16) -> Option<&'static str> {
    match *WORLDMAP_KEY_NAMES.get(usize::from(symbol))? {
        "???" => Some("Minigames"),
        name => Some(name),
    }
}

/// The loc's own name, else its mapfunction's Key legend name: invisible
/// marker locs carry only a mapfunction. `None` when neither exists; rs2b0t
/// labels no such place, so it is not a POI.
fn poi_name(definition: &LocDefinition) -> Result<Option<Text>, MapError> {
    if !definition.name.is_empty() {
        return Text::new(&definition.name).map(Some);
    }
    definition
        .mapfunction
        .and_then(mapfunction_key_name)
        .map(Text::new)
        .transpose()
}

fn poi_priority(kind: PoiKind) -> u8 {
    match kind {
        PoiKind::Bank => 0,
        PoiKind::Shop => 1,
        PoiKind::Altar => 2,
        PoiKind::MapSymbol { .. } => 3,
        PoiKind::Label { .. } => 4,
        PoiKind::Transport => 5,
        PoiKind::Teleport => 6,
    }
}

fn optional_u16(value: i32, what: &'static str) -> Result<Option<u16>, MapError> {
    if value == -1 {
        Ok(None)
    } else {
        u16::try_from(value)
            .map(Some)
            .map_err(|_| MapError::Invalid(what))
    }
}

#[cfg(test)]
#[path = "catalogue_tests.rs"]
mod tests;
