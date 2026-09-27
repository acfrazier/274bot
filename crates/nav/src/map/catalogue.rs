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
use client::map_cache::MAP_SQUARE_SIZE;
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
                if matches!(kind, PoiKind::MapSymbol { .. }) {
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
