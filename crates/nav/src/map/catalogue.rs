//! Client-only POI derivation, isolated from terrain policy so catalogue-only
//! classifier changes do not invalidate already-baked imagery.

use super::client_cache::{
    load_config, CacheReader, CacheReaderCursor, GroundCell, LandSquare,
    MAX_JAG_ARCHIVE_RETAINED_BYTES, MAX_JAG_MEMBER_PEAK_BYTES, MAX_JAG_READ_PEAK_BYTES,
    MAX_JAG_UNPACKED_BYTES, MAX_MAP_INDEX_ROWS,
};
use super::formats::{
    ClientPois, Coverage, CoverageIssue, CoverageLevel, CoverageReason, MAX_POIS,
};
use super::identity::{CatalogueIdentity, CATALOGUE_SCHEMA};
use super::poi::{
    classify_definition, CapabilityEvidence, Definition, DisplayAnchor, EntityKind, Footprint,
    PoiKey, PoiKind, PoiRecord, SourceSpace,
};
use super::producer::{
    charge_product, charge_sum, make_work_charge, ClientMapInput, OwnedClientMapInput, ProducerStep,
};
use super::records::{
    RecordOffset, MAX_MAP_LOC_PLACEMENTS_PER_RECORD, MAX_MAP_RECORDS, MAX_MAP_RECORD_BYTES,
};
use super::{MapError, Rows, Text};
use api::work::{Checkpoint, StepContext, WorkCharge, WorkPhase};
use client::config::{LocType, NpcType};
use client::io::ClientRevision;
use client::map_cache::MAP_SQUARE_SIZE;
use client::map_cache::{LocPlacement, MapIndexEntry, MAP_PLANES};
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

// Source facts from api/data/game-data/{274,289}.json plus manifest.json:
// uncompressed file bytes, JSON object/array-element counts, decoded UTF-8
// string bytes, item-index extent, and the catalogue rows copied by this job.
#[derive(Clone, Copy)]
struct GameDataSourceBounds {
    json_bytes: usize,
    json_objects: usize,
    array_items: usize,
    string_bytes: usize,
    item_index_len: usize,
    consumption_rows: usize,
    quest_start_rows: usize,
    npc_placement_rows: usize,
}

const GAME_DATA_274_BOUNDS: GameDataSourceBounds = GameDataSourceBounds {
    json_bytes: 5_335_952,
    json_objects: 24_350,
    array_items: 27_561,
    string_bytes: 708_849,
    item_index_len: 3_894,
    consumption_rows: 262,
    quest_start_rows: 43,
    npc_placement_rows: 7_322,
};

const GAME_DATA_289_BOUNDS: GameDataSourceBounds = GameDataSourceBounds {
    json_bytes: 5_799_600,
    json_objects: 26_233,
    array_items: 29_943,
    string_bytes: 762_571,
    item_index_len: 4_089,
    consumption_rows: 262,
    quest_start_rows: 43,
    npc_placement_rows: 8_141,
};

const GAME_DATA_MANIFEST_BYTES: usize = 708_799;
const GAME_DATA_READ_BUFFER_BYTES: usize = 64 * 1024;
// GzDecoder::new uses BufReader::new, whose current standard default is 8 KiB.
const GZIP_READ_BUFFER_BYTES: usize = 8 * 1024;
// miniz_oxide 0.8.9's boxed InflateState has a 32 KiB LZ dictionary, three
// 10-bit lookup/576-entry Huffman tables, and fixed code-size/header state.
const GZIP_INFLATE_STATE_BOUND_BYTES: usize = 64 * 1024;
// Every heap-stored object row in SelectedGameData is covered by this compiled
// maximum; inline child objects are already part of their containing row or
// the separately charged root. Update this list when the source schema adds
// an allocated row type. Fourfold capacity headroom is applied below.
const GAME_DATA_TYPED_OBJECT_ROW_BYTES: usize = {
    use api::game_data as data;
    use std::mem::{align_of, size_of};

    let rows = [
        size_of::<data::SourceInput>(),
        size_of::<data::GameItem>(),
        size_of::<data::NamedFact>(),
        size_of::<data::StatChange>(),
        size_of::<data::StatHeal>(),
        size_of::<data::ConsumptionFact>(),
        // Private PickpocketFact: one Vec<NamedFact>, one i32, and padding.
        size_of::<Vec<data::NamedFact>>() + size_of::<i32>() + align_of::<Vec<data::NamedFact>>(),
        size_of::<data::SpecialBar>(),
        size_of::<data::SpecialWeapon>(),
        size_of::<data::TeleportSpell>(),
        size_of::<data::SpellRune>(),
        size_of::<data::SpellFact>(),
        size_of::<data::StaffRune>(),
        size_of::<data::StaffFact>(),
        size_of::<data::DropItem>(),
        size_of::<data::DropTable>(),
        size_of::<data::HerbFact>(),
        size_of::<data::PrayerFact>(),
        size_of::<data::PickaxeShopFact>(),
        size_of::<data::EquipmentNameCandidate>(),
        size_of::<data::EquipmentNameEntry>(),
        size_of::<data::QuestItemAlias>(),
        size_of::<data::QuestSkillGate>(),
        size_of::<data::QuestIdentityRow>(),
        size_of::<data::QuestCoverageRecord>(),
        size_of::<data::QuestStartRow>(),
        size_of::<data::StyleSequenceFact>(),
        size_of::<data::StyleSpotanimFact>(),
        size_of::<data::CombatTabFact>(),
        size_of::<data::MeleeModeFact>(),
        size_of::<data::RangedWeaponFact>(),
        size_of::<data::RangedAmmoFact>(),
        size_of::<data::RangedModeFact>(),
        size_of::<data::WeaponStyleFact>(),
        size_of::<data::NpcNameRow>(),
        size_of::<data::LocNameRow>(),
        size_of::<data::NpcPlacementRow>(),
        size_of::<data::KaramjaSpawn>(),
        size_of::<data::TrailParam>(),
        size_of::<data::TrailMembershipRow>(),
        size_of::<data::TrailChallengeAnswer>(),
        size_of::<data::TalkKeyTalkRow>(),
        size_of::<data::TalkKeyKeyRow>(),
        size_of::<data::TalkKeyCoverageRecord>(),
        size_of::<data::TrioGiverRow>(),
        size_of::<data::TrioGiverCoverageRecord>(),
        size_of::<data::GatherResourceOption>(),
        size_of::<data::GatherSiteOption>(),
        size_of::<data::GatherSiteRequirement>(),
        size_of::<data::GatherSiteKey>(),
        size_of::<api::named_banks::BankPlacement>(),
        size_of::<api::cook_locations::CookSurface>(),
        size_of::<api::gather_methods::MiningHazard>(),
    ];
    let mut maximum = 0;
    let mut index = 0;
    while index < rows.len() {
        if rows[index] > maximum {
            maximum = rows[index];
        }
        index += 1;
    }
    maximum
};
const VECTOR_CAPACITY_HEADROOM: usize = 4;

fn game_data_source_bounds(revision: u16) -> Result<GameDataSourceBounds, MapError> {
    match revision {
        274 => Ok(GAME_DATA_274_BOUNDS),
        289 => Ok(GAME_DATA_289_BOUNDS),
        _ => Err(MapError::Invalid("unsupported game data revision")),
    }
}

fn game_data_retained_owner_bound(source: GameDataSourceBounds) -> Result<usize, MapError> {
    let typed_row_capacity =
        charge_product(VECTOR_CAPACITY_HEADROOM, GAME_DATA_TYPED_OBJECT_ROW_BYTES)?;
    let object_rows = charge_product(source.json_objects, typed_row_capacity)?;
    let root_object = std::mem::size_of::<api::game_data::SelectedGameData>();
    let array_slots = charge_product(
        charge_product(VECTOR_CAPACITY_HEADROOM, source.array_items)?,
        std::mem::size_of::<String>(),
    )?;
    let parsed_strings = charge_product(VECTOR_CAPACITY_HEADROOM, source.json_bytes)?;
    let item_index = charge_product(source.item_index_len, std::mem::size_of::<Option<usize>>())?;
    let food_rows = charge_product(
        2 * source.consumption_rows,
        std::mem::size_of::<(i32, String, i32)>(),
    )?;
    let food_hash_entries = charge_product(
        2 * source.consumption_rows,
        std::mem::size_of::<(u64, Vec<usize>)>() + 3 * std::mem::size_of::<usize>(),
    )?;
    let food_hash_values =
        charge_product(2 * source.consumption_rows, std::mem::size_of::<usize>())?;
    let selected_pin = charge_sum([
        std::mem::size_of::<api::selected::SelectedPin>(),
        2 * std::mem::size_of::<usize>(),
        4 * 2 * std::mem::size_of::<usize>(),
        GAME_DATA_MANIFEST_BYTES,
    ])?;
    charge_sum([
        object_rows,
        root_object,
        array_slots,
        parsed_strings,
        charge_product(2, source.string_bytes)?,
        item_index,
        food_rows,
        food_hash_entries,
        food_hash_values,
        selected_pin,
        2 * std::mem::size_of::<usize>(),
    ])
}

fn game_data_initializer_peak_bound(source: GameDataSourceBounds) -> Result<usize, MapError> {
    charge_sum([
        game_data_retained_owner_bound(source)?,
        GAME_DATA_READ_BUFFER_BYTES,
        GZIP_READ_BUFFER_BYTES,
        GZIP_INFLATE_STATE_BOUND_BYTES,
        GAME_DATA_MANIFEST_BYTES,
        source.json_bytes,
    ])
}

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MapMarkerKind {
    RareTrees,
    QuestStart,
}

#[derive(Debug, Clone, Copy)]
struct MapMarkerMatch {
    record: usize,
    kind: MapMarkerKind,
    rare_tree_best: Option<(f64, u32)>,
}

#[derive(Debug, Clone, Default)]
struct QuestStartCatalog {
    entries: Vec<QuestStartEntry>,
    npc_placements: Vec<QuestStartPlacement>,
}

fn quest_start_catalog(input: ClientMapInput<'_>) -> Option<QuestStartCatalog> {
    let revision = match input.revision {
        274 => ClientRevision::R274,
        289 => ClientRevision::R289,
        _ => return None,
    };
    // Main approved game_data static owner classification 10-15; this OnceLock remains process-global.
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
    Some(QuestStartCatalog {
        entries,
        npc_placements,
    })
}

#[cfg(test)]
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
                quest_start_target_matches(entry, *placement, plane, centre_x, centre_z)
            }),
            QuestStartTargetKind::Loc => loc_placements.iter().any(|placement| {
                quest_start_target_matches(entry, *placement, plane, centre_x, centre_z)
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

fn quest_start_target_matches(
    quest: &QuestStartEntry,
    placement: QuestStartPlacement,
    marker_plane: u8,
    centre_x: f64,
    centre_z: f64,
) -> bool {
    quest.id == placement.id
        && placement.plane == marker_plane
        && nearby(placement.x, placement.z, centre_x, centre_z)
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

fn map_marker_kind(record: &PoiRecord) -> Option<MapMarkerKind> {
    if matches!(
        record.kind,
        PoiKind::MapSymbol {
            symbol: RARE_TREES_SYMBOL
        }
    ) && record.name.as_str() == "Rare Trees"
    {
        Some(MapMarkerKind::RareTrees)
    } else if matches!(
        record.kind,
        PoiKind::MapSymbol {
            symbol: QUEST_STARTS_SYMBOL
        }
    ) && record.name.as_str() == "Quest Start"
    {
        Some(MapMarkerKind::QuestStart)
    } else {
        None
    }
}

fn rare_tree_candidate(
    mut best: Option<(f64, u32)>,
    marker: &PoiRecord,
    candidate: LocPlacement,
    map_entry: MapIndexEntry,
    link_below: bool,
    locs: &[LocDefinition],
) -> Result<Option<(f64, u32)>, MapError> {
    let definition = locs
        .get(candidate.id as usize)
        .ok_or(MapError::Invalid("location definition id"))?;
    if is_woodcut_tree(definition) {
        let source = SourceSpace::ClientVisual {
            plane: candidate.plane,
            link_below,
        };
        if let Some(plane) = source.game_plane()? {
            if plane == marker.effective_plane {
                let footprint = definition.footprint(candidate.rotation)?;
                let world_x = i32::from(map_entry.square_x) * i32::from(MAP_SQUARE_SIZE)
                    + i32::from(candidate.x);
                let world_z = i32::from(map_entry.square_z) * i32::from(MAP_SQUARE_SIZE)
                    + i32::from(candidate.z);
                let tree_cx = f64::from(world_x) + f64::from(footprint.width) / 2.0;
                let tree_cz = f64::from(world_z) + f64::from(footprint.length) / 2.0;
                let dx = tree_cx - marker.display.x;
                let dz = tree_cz - marker.display.z;
                let dist2 = dx * dx + dz * dz;
                if dist2 <= MAX_TREE_MATCH_DIST2
                    && best.is_none_or(|(best_dist2, _)| dist2 < best_dist2)
                {
                    best = Some((dist2, candidate.id));
                }
            }
        }
    }
    Ok(best)
}

/// Reject only squares outside every possible candidate centre and the
/// matching radius. Rare-tree footprints can span the full rotated u8 range.
fn marker_may_match_square(marker: &PoiRecord, kind: MapMarkerKind, entry: MapIndexEntry) -> bool {
    let west = f64::from(entry.square_x) * f64::from(MAP_SQUARE_SIZE);
    let south = f64::from(entry.square_z) * f64::from(MAP_SQUARE_SIZE);
    let (centre_extent, radius) = match kind {
        MapMarkerKind::RareTrees => (f64::from(u8::MAX) / 2.0, MAX_TREE_MATCH_DIST2.sqrt()),
        MapMarkerKind::QuestStart => (0.5, MAX_QUEST_START_MATCH_DIST2.sqrt()),
    };
    let maximum = f64::from(MAP_SQUARE_SIZE) - 1.0 + centre_extent + radius;
    marker.display.x >= west - radius
        && marker.display.x <= west + maximum
        && marker.display.z >= south - radius
        && marker.display.z <= south + maximum
}

fn quest_loc_candidate_matches(
    quest: &QuestStartEntry,
    marker: &PoiRecord,
    candidate: LocPlacement,
    map_entry: MapIndexEntry,
    link_below: bool,
) -> Result<bool, MapError> {
    let candidate_id =
        i32::try_from(candidate.id).map_err(|_| MapError::Invalid("quest-start location id"))?;
    if candidate_id != quest.id {
        return Ok(false);
    }
    let source = SourceSpace::ClientVisual {
        plane: candidate.plane,
        link_below,
    };
    let Some(plane) = source.game_plane()? else {
        return Ok(false);
    };
    let world_x =
        i32::from(map_entry.square_x) * i32::from(MAP_SQUARE_SIZE) + i32::from(candidate.x);
    let world_z =
        i32::from(map_entry.square_z) * i32::from(MAP_SQUARE_SIZE) + i32::from(candidate.z);
    Ok(quest_start_target_matches(
        quest,
        QuestStartPlacement {
            id: candidate_id,
            x: world_x,
            z: world_z,
            plane,
        },
        marker.effective_plane,
        marker.display.x,
        marker.display.z,
    ))
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogueStage {
    Preparing,
    Indexing,
    Scanning,
    Matching,
    Finalizing,
    Complete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogueProgress {
    pub stage: CatalogueStage,
    pub units_completed: u64,
    pub map_squares_completed: u32,
    pub map_squares_total: Option<u32>,
    pub loc_placements_processed: u64,
    pub poi_records_ready: u32,
    pub poi_records_total: Option<u32>,
}

#[derive(Debug)]
pub struct CatalogueResult {
    pub pois: ClientPois,
    pub stats: CatalogueStats,
}

enum CataloguePhase {
    Identity,
    Config,
    LocDefinitions,
    NpcDefinitions,
    QuestStarts,
    ReaderInit,
    Reader,
    Land,
    LocPlacements,
    Placement,
    MatchInit {
        record: usize,
    },
    MatchLand,
    MatchLocs,
    MatchPlacements,
    FinalizeMatches {
        marker: usize,
        entry: usize,
        target: usize,
        names: BTreeSet<String>,
    },
    SortRecords,
    Stats,
    Document,
    Complete,
}

enum CatalogueAction {
    Transition,
    Unit,
    Completed(CatalogueResult),
}

// Cheap placement work shares one admission unit. Every operation still
// checks the live deadline and cancellation before it starts.
const PLACEMENT_OPERATIONS_PER_UNIT: u32 = 64;

// The real-content census found 14,844 candidates in 274 and 15,059 in
// 289. Round twice the larger count up to a power of two: 32,768 slots,
// or 117.6% headroom. Overflow uses the unchanged second scan, not refusal.
// See TRACK-T5/T5W/map-bake/candidate-census-r2/command.log.
const MAX_RETAINED_CANDIDATES: usize = 32_768;

#[derive(Clone, Copy)]
struct RetainedCandidate {
    placement: LocPlacement,
    link_below: bool,
}

/// Resumable client catalogue producer. Native calls remain invocation-local.
/// Placement and matching operations share bounded work units, with a live
/// checkpoint before every operation.
pub struct CatalogueCursor {
    input: OwnedClientMapInput,
    phase: CataloguePhase,
    identity: Option<CatalogueIdentity>,
    config: Option<client::io::JagFile>,
    locs: Vec<LocDefinition>,
    quest_starts: Option<QuestStartCatalog>,
    reader_cursor: Option<CacheReaderCursor>,
    reader: Option<CacheReader>,
    entry_index: usize,
    land: Option<LandSquare>,
    placements: Vec<LocPlacement>,
    placement_index: usize,
    // None means retention overflowed. No candidate allocation grows.
    retained_candidates: Option<Vec<RetainedCandidate>>,
    retained_square_ends: Vec<usize>,
    matching_squares_completed: u32,
    marker_matches: Vec<MapMarkerMatch>,
    quest_loc_matches: Vec<u8>,
    // Sorted by loc id so each candidate checks only matching quest rows.
    quest_loc_targets: Vec<(i32, usize)>,
    quest_start_entries: usize,
    matching_square: usize,
    matching_placement: usize,
    matching_marker: usize,
    matching_quest_entry: usize,
    matching_quest_target_end: usize,
    needs_map_match: bool,
    squares_completed: u32,
    records: Vec<PoiRecord>,
    unknown_mapfunctions: u32,
    stats: CatalogueStats,
    units_completed: u64,
    result: Option<CatalogueResult>,
    completed: bool,
}

impl CatalogueCursor {
    pub fn new(input: ClientMapInput<'_>) -> Result<Self, MapError> {
        Ok(Self {
            input: OwnedClientMapInput::copy(input)?,
            phase: CataloguePhase::Identity,
            identity: None,
            config: None,
            locs: Vec::new(),
            quest_starts: None,
            reader_cursor: None,
            reader: None,
            entry_index: 0,
            land: None,
            placements: Vec::new(),
            placement_index: 0,
            retained_candidates: Some(Vec::with_capacity(MAX_RETAINED_CANDIDATES)),
            retained_square_ends: Vec::new(),
            matching_squares_completed: 0,
            marker_matches: Vec::new(),
            quest_loc_matches: Vec::new(),
            quest_loc_targets: Vec::new(),
            quest_start_entries: 0,
            matching_square: 0,
            matching_placement: 0,
            matching_marker: 0,
            matching_quest_entry: 0,
            matching_quest_target_end: 0,
            needs_map_match: false,
            squares_completed: 0,
            records: Vec::new(),
            unknown_mapfunctions: 0,
            stats: CatalogueStats::default(),
            units_completed: 0,
            result: None,
            completed: false,
        })
    }

    /// Pure upper bound for the cursor charge. It does not inspect the
    /// process-global OnceLock: every job reserves the cold initializer peak.
    pub fn work_charge(input: ClientMapInput<'_>) -> Result<WorkCharge, MapError> {
        let input = ClientMapInput::new(
            input.revision,
            input.content,
            input.jag_dir,
            input.snapshot_dir,
        )?;
        let source = game_data_source_bounds(input.revision)?;
        let max_definitions = u16::MAX as usize;
        let game_data_owner = game_data_retained_owner_bound(source)?;
        let game_data_peak = game_data_initializer_peak_bound(source)?;
        let game_data_scratch = game_data_peak
            .checked_sub(game_data_owner)
            .ok_or(MapError::Limit("producer work charge"))?;

        let input_path_bytes = charge_sum([
            charge_product(
                2,
                charge_sum([
                    input.jag_dir.as_os_str().len(),
                    input.snapshot_dir.as_os_str().len(),
                ])?,
            )?,
            charge_sum([input.jag_dir.as_os_str().len(), "versionlist".len()])?,
            charge_sum([input.snapshot_dir.as_os_str().len(), "maps.bin".len()])?,
        ])?;
        let loc_definition_bytes = charge_sum([
            charge_product(max_definitions, std::mem::size_of::<LocDefinition>())?,
            charge_product(2, MAX_JAG_UNPACKED_BYTES as usize)?,
        ])?;
        let quest_catalog_bytes = charge_sum([
            charge_product(
                2 * source.quest_start_rows,
                std::mem::size_of::<QuestStartEntry>(),
            )?,
            charge_product(
                2 * source.npc_placement_rows,
                std::mem::size_of::<QuestStartPlacement>(),
            )?,
            charge_product(2, source.string_bytes)?,
        ])?;
        let marker_capacity = charge_product(VECTOR_CAPACITY_HEADROOM, MAX_POIS)?;
        let quest_match_capacity = charge_product(
            VECTOR_CAPACITY_HEADROOM,
            charge_product(MAX_POIS, source.quest_start_rows)?,
        )?;
        let quest_target_index_capacity =
            charge_product(VECTOR_CAPACITY_HEADROOM, source.quest_start_rows)?;
        let marker_matching_bytes = charge_sum([
            charge_product(marker_capacity, std::mem::size_of::<MapMarkerMatch>())?,
            charge_product(quest_match_capacity, std::mem::size_of::<u8>())?,
            charge_product(
                quest_target_index_capacity,
                std::mem::size_of::<(i32, usize)>(),
            )?,
        ])?;
        let retained_candidate_bytes = charge_sum([
            charge_product(
                MAX_RETAINED_CANDIDATES,
                std::mem::size_of::<RetainedCandidate>(),
            )?,
            charge_product(MAX_MAP_INDEX_ROWS, std::mem::size_of::<usize>())?,
        ])?;
        let cache_reader_bytes = charge_sum([
            charge_product(MAX_MAP_INDEX_ROWS, std::mem::size_of::<MapIndexEntry>())?,
            charge_product(MAX_MAP_RECORDS, std::mem::size_of::<RecordOffset>())?,
            MAX_MAP_RECORD_BYTES,
            MAX_JAG_ARCHIVE_RETAINED_BYTES as usize,
        ])?;
        let placement_capacity = charge_product(
            2 * MAX_MAP_LOC_PLACEMENTS_PER_RECORD,
            std::mem::size_of::<LocPlacement>(),
        )?;
        let land_capacity = charge_product(
            usize::from(MAP_PLANES) * usize::from(MAP_SQUARE_SIZE) * usize::from(MAP_SQUARE_SIZE),
            std::mem::size_of::<GroundCell>(),
        )?;
        let input_bytes = charge_sum([
            std::mem::size_of::<Self>(),
            input_path_bytes,
            loc_definition_bytes,
            quest_catalog_bytes,
            marker_matching_bytes,
            retained_candidate_bytes,
            cache_reader_bytes,
            charge_product(2, placement_capacity)?,
            charge_product(2, land_capacity)?,
            game_data_owner,
        ])?;

        let definition_rows = charge_product(
            max_definitions,
            std::mem::size_of::<LocType>().max(std::mem::size_of::<NpcType>()),
        )?;
        let jag_open_scratch = (MAX_JAG_READ_PEAK_BYTES - MAX_JAG_ARCHIVE_RETAINED_BYTES) as usize;
        let jag_member_scratch = charge_sum([
            (MAX_JAG_MEMBER_PEAK_BYTES - MAX_JAG_ARCHIVE_RETAINED_BYTES) as usize,
            definition_rows,
        ])?;
        let quest_matching_scratch = charge_sum([
            charge_product(
                2 * source.quest_start_rows,
                std::mem::size_of::<String>() + 3 * std::mem::size_of::<usize>(),
            )?,
            charge_product(2, source.string_bytes)?,
        ])?;
        let placement_scratch = charge_sum([placement_capacity, land_capacity, Text::MAX_BYTES])?;
        let scratch_bytes = [
            game_data_scratch,
            jag_open_scratch,
            jag_member_scratch,
            quest_matching_scratch,
            placement_scratch,
        ]
        .into_iter()
        .max()
        .ok_or(MapError::Limit("producer work charge"))?;

        let poi_rows = charge_product(2 * MAX_POIS, std::mem::size_of::<PoiRecord>())?;
        let evidence_rows =
            charge_product(2 * MAX_POIS * 8, std::mem::size_of::<CapabilityEvidence>())?;
        let record_text = charge_product(MAX_POIS, 2 * 9 * Text::MAX_BYTES)?;
        let unresolved = charge_product(
            128,
            std::mem::size_of::<CoverageIssue>() + 2 * Text::MAX_BYTES,
        )?;
        let result_bytes = charge_sum([
            std::mem::size_of::<CatalogueResult>(),
            poi_rows,
            evidence_rows,
            record_text,
            unresolved,
        ])?;
        make_work_charge(input_bytes, scratch_bytes, result_bytes)
    }

    pub fn progress(&self) -> CatalogueProgress {
        let stage = match &self.phase {
            CataloguePhase::Identity
            | CataloguePhase::Config
            | CataloguePhase::LocDefinitions
            | CataloguePhase::NpcDefinitions
            | CataloguePhase::QuestStarts => CatalogueStage::Preparing,
            CataloguePhase::ReaderInit | CataloguePhase::Reader => CatalogueStage::Indexing,
            CataloguePhase::Land | CataloguePhase::LocPlacements | CataloguePhase::Placement => {
                CatalogueStage::Scanning
            }
            CataloguePhase::MatchInit { .. }
            | CataloguePhase::MatchLand
            | CataloguePhase::MatchLocs
            | CataloguePhase::MatchPlacements
            | CataloguePhase::FinalizeMatches { .. } => CatalogueStage::Matching,
            CataloguePhase::SortRecords | CataloguePhase::Stats | CataloguePhase::Document => {
                CatalogueStage::Finalizing
            }
            CataloguePhase::Complete => CatalogueStage::Complete,
        };
        let map_squares_total = self
            .reader
            .as_ref()
            .map(|reader| reader.entries.len() as u32);
        let map_squares_completed = if stage == CatalogueStage::Matching {
            self.matching_squares_completed
        } else {
            self.squares_completed
        };
        let poi_records_total =
            matches!(stage, CatalogueStage::Matching | CatalogueStage::Finalizing)
                .then_some(self.records.len() as u32);
        CatalogueProgress {
            stage,
            units_completed: self.units_completed,
            map_squares_completed,
            map_squares_total,
            loc_placements_processed: self.stats.loc_placements,
            poi_records_ready: self.records.len() as u32,
            poi_records_total,
        }
    }

    pub fn step<P: WorkPhase>(
        &mut self,
        context: &mut StepContext<'_, P>,
    ) -> Result<ProducerStep<CatalogueResult>, MapError> {
        if self.completed {
            return Err(MapError::Invalid("catalogue cursor complete"));
        }
        let mut placement_operations = 0;
        loop {
            match context.checkpoint() {
                Checkpoint::Continue => {}
                Checkpoint::Yield => return Ok(ProducerStep::Yielded),
                Checkpoint::Cancel(_) => return Ok(ProducerStep::Cancelled),
            }
            let batch_placement = matches!(
                self.phase,
                CataloguePhase::Placement | CataloguePhase::MatchPlacements
            );
            match self.advance()? {
                CatalogueAction::Transition => {}
                CatalogueAction::Unit => {
                    placement_operations += 1;
                    if batch_placement && placement_operations < PLACEMENT_OPERATIONS_PER_UNIT {
                        continue;
                    }
                    placement_operations = 0;
                    self.units_completed = self.units_completed.saturating_add(1);
                    match context.consume_unit() {
                        Checkpoint::Continue => {}
                        Checkpoint::Yield => return Ok(ProducerStep::Yielded),
                        Checkpoint::Cancel(_) => return Ok(ProducerStep::Cancelled),
                    }
                }
                CatalogueAction::Completed(result) => match context.checkpoint() {
                    Checkpoint::Continue => {
                        self.completed = true;
                        return Ok(ProducerStep::Completed(result));
                    }
                    Checkpoint::Yield => {
                        self.result = Some(result);
                        return Ok(ProducerStep::Yielded);
                    }
                    Checkpoint::Cancel(_) => return Ok(ProducerStep::Cancelled),
                },
            }
        }
    }

    fn advance(&mut self) -> Result<CatalogueAction, MapError> {
        let phase = std::mem::replace(&mut self.phase, CataloguePhase::Complete);
        match phase {
            CataloguePhase::Identity => {
                let input = self.input.view();
                self.identity = Some(super::producer::catalogue_identity(input));
                self.phase = CataloguePhase::Config;
                Ok(CatalogueAction::Unit)
            }
            CataloguePhase::Config => {
                self.config = Some(load_config(self.input.view())?);
                self.phase = CataloguePhase::LocDefinitions;
                Ok(CatalogueAction::Unit)
            }
            CataloguePhase::LocDefinitions => {
                let config = self
                    .config
                    .as_ref()
                    .ok_or(MapError::Invalid("missing catalogue config"))?;
                self.locs = load_loc_definitions(config)?;
                self.phase = CataloguePhase::NpcDefinitions;
                Ok(CatalogueAction::Unit)
            }
            CataloguePhase::NpcDefinitions => {
                let config = self
                    .config
                    .as_ref()
                    .ok_or(MapError::Invalid("missing catalogue config"))?;
                let decoded_npcs = catch_unwind(AssertUnwindSafe(|| NpcType::unpack(config)))
                    .map_err(|_| MapError::Invalid("NPC definitions"))?;
                self.stats.npc_definitions = decoded_npcs.len() as u32;
                self.stats.npc_service_definitions = decoded_npcs
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
                        classify_definition(self.input.revision(), &definition, |_, _| {
                            service = true
                        });
                        service
                    })
                    .count() as u32;
                self.config = None;
                self.phase = CataloguePhase::QuestStarts;
                Ok(CatalogueAction::Unit)
            }
            CataloguePhase::QuestStarts => {
                self.quest_starts = quest_start_catalog(self.input.view());
                if let Some(catalog) = &self.quest_starts {
                    self.quest_loc_targets
                        .extend(
                            catalog
                                .entries
                                .iter()
                                .enumerate()
                                .filter_map(|(index, entry)| {
                                    (entry.kind == QuestStartTargetKind::Loc)
                                        .then_some((entry.id, index))
                                }),
                        );
                    self.quest_loc_targets.sort_unstable();
                }
                self.phase = CataloguePhase::ReaderInit;
                Ok(CatalogueAction::Unit)
            }
            CataloguePhase::ReaderInit => {
                self.reader_cursor = Some(CacheReaderCursor::new(self.input.view()));
                self.phase = CataloguePhase::Reader;
                Ok(CatalogueAction::Transition)
            }
            CataloguePhase::Reader => {
                let cursor = self
                    .reader_cursor
                    .as_mut()
                    .ok_or(MapError::Invalid("missing cache reader cursor"))?;
                if let Some(reader) = cursor.advance()? {
                    self.stats.map_squares = reader.entries.len() as u32;
                    self.retained_square_ends = Vec::with_capacity(reader.entries.len());
                    self.reader = Some(reader);
                    self.reader_cursor = None;
                    self.phase = CataloguePhase::Land;
                } else {
                    self.phase = CataloguePhase::Reader;
                }
                Ok(CatalogueAction::Unit)
            }
            CataloguePhase::Land => {
                let reader = self
                    .reader
                    .as_mut()
                    .ok_or(MapError::Invalid("missing cache reader"))?;
                let Some(entry) = reader.entries.get(self.entry_index).copied() else {
                    self.phase = CataloguePhase::MatchInit { record: 0 };
                    return Ok(CatalogueAction::Transition);
                };
                self.land = Some(reader.read_land(entry)?);
                self.phase = CataloguePhase::LocPlacements;
                Ok(CatalogueAction::Unit)
            }
            CataloguePhase::LocPlacements => {
                let entry = self
                    .reader
                    .as_ref()
                    .and_then(|reader| reader.entries.get(self.entry_index).copied())
                    .ok_or(MapError::Invalid("missing catalogue map square"))?;
                let reader = self
                    .reader
                    .as_mut()
                    .ok_or(MapError::Invalid("missing cache reader"))?;
                self.placements = reader.read_locs(entry)?;
                self.placement_index = 0;
                self.phase = CataloguePhase::Placement;
                Ok(CatalogueAction::Unit)
            }
            CataloguePhase::Placement => {
                let Some(placement) = self.placements.get(self.placement_index).copied() else {
                    if let Some(candidates) = &self.retained_candidates {
                        self.retained_square_ends.push(candidates.len());
                    }
                    self.squares_completed = self.squares_completed.saturating_add(1);
                    self.entry_index += 1;
                    self.land = None;
                    self.placements.clear();
                    self.placement_index = 0;
                    self.phase = CataloguePhase::Land;
                    return Ok(CatalogueAction::Transition);
                };
                let entry = self
                    .reader
                    .as_ref()
                    .and_then(|reader| reader.entries.get(self.entry_index).copied())
                    .ok_or(MapError::Invalid("missing catalogue map square"))?;
                let link_below = self
                    .land
                    .as_ref()
                    .ok_or(MapError::Invalid("missing catalogue land square"))?
                    .link_below(placement.x, placement.z);
                self.placement_index += 1;
                self.stats.loc_placements += 1;
                self.phase = CataloguePhase::Placement;
                self.retain_candidate(placement, link_below)?;
                self.process_placement(placement, entry, link_below)?;
                Ok(CatalogueAction::Unit)
            }
            CataloguePhase::MatchInit { record } => {
                if record < self.records.len() {
                    if let Some(kind) = map_marker_kind(&self.records[record]) {
                        let needs_map_match = match kind {
                            MapMarkerKind::RareTrees => true,
                            MapMarkerKind::QuestStart => {
                                self.quest_starts.as_ref().is_some_and(|catalog| {
                                    catalog
                                        .entries
                                        .iter()
                                        .any(|entry| entry.kind == QuestStartTargetKind::Loc)
                                })
                            }
                        };
                        self.needs_map_match |= needs_map_match;
                        self.marker_matches.push(MapMarkerMatch {
                            record,
                            kind,
                            rare_tree_best: None,
                        });
                    }
                    self.phase = CataloguePhase::MatchInit { record: record + 1 };
                    return Ok(CatalogueAction::Unit);
                }
                self.quest_start_entries = self
                    .quest_starts
                    .as_ref()
                    .map_or(0, |catalog| catalog.entries.len());
                self.matching_quest_entry = 0;
                self.matching_quest_target_end = 0;
                let match_count = self
                    .marker_matches
                    .len()
                    .checked_mul(self.quest_start_entries)
                    .ok_or(MapError::Limit("quest-start match count"))?;
                self.quest_loc_matches.resize(match_count, 0);
                self.phase = if self.needs_map_match {
                    CataloguePhase::MatchLand
                } else {
                    CataloguePhase::FinalizeMatches {
                        marker: 0,
                        entry: 0,
                        target: 0,
                        names: BTreeSet::new(),
                    }
                };
                Ok(CatalogueAction::Unit)
            }
            CataloguePhase::MatchLand => {
                let reader = self
                    .reader
                    .as_mut()
                    .ok_or(MapError::Invalid("missing cache reader"))?;
                let Some(entry) = reader.entries.get(self.matching_square).copied() else {
                    self.phase = CataloguePhase::FinalizeMatches {
                        marker: 0,
                        entry: 0,
                        target: 0,
                        names: BTreeSet::new(),
                    };
                    return Ok(CatalogueAction::Transition);
                };
                if self.retained_candidates.is_some() {
                    self.matching_placement = 0;
                    self.matching_marker = 0;
                    self.matching_quest_entry = 0;
                    self.matching_quest_target_end = 0;
                    self.phase = CataloguePhase::MatchPlacements;
                    return Ok(CatalogueAction::Transition);
                }
                self.land = Some(reader.read_land(entry)?);
                self.phase = CataloguePhase::MatchLocs;
                Ok(CatalogueAction::Unit)
            }
            CataloguePhase::MatchLocs => {
                let entry = self
                    .reader
                    .as_ref()
                    .and_then(|reader| reader.entries.get(self.matching_square).copied())
                    .ok_or(MapError::Invalid("missing catalogue map square"))?;
                let reader = self
                    .reader
                    .as_mut()
                    .ok_or(MapError::Invalid("missing cache reader"))?;
                self.placements = reader.read_locs(entry)?;
                self.matching_placement = 0;
                self.matching_marker = 0;
                self.matching_quest_entry = 0;
                self.matching_quest_target_end = 0;
                self.phase = CataloguePhase::MatchPlacements;
                Ok(CatalogueAction::Unit)
            }
            CataloguePhase::MatchPlacements => {
                let retained_start = if self.matching_square == 0 {
                    0
                } else {
                    self.retained_square_ends
                        .get(self.matching_square - 1)
                        .copied()
                        .unwrap_or(0)
                };
                let placement_count = if self.retained_candidates.is_some() {
                    self.retained_square_ends[self.matching_square] - retained_start
                } else {
                    self.placements.len()
                };
                if self.matching_marker >= self.marker_matches.len() || placement_count == 0 {
                    self.matching_squares_completed =
                        self.matching_squares_completed.saturating_add(1);
                    self.matching_square += 1;
                    self.matching_placement = 0;
                    self.matching_marker = 0;
                    self.matching_quest_entry = 0;
                    self.matching_quest_target_end = 0;
                    self.land = None;
                    self.placements.clear();
                    self.phase = CataloguePhase::MatchLand;
                    return Ok(CatalogueAction::Transition);
                }
                if self.matching_placement >= placement_count {
                    self.matching_marker += 1;
                    self.matching_placement = 0;
                    self.matching_quest_entry = 0;
                    self.matching_quest_target_end = 0;
                    self.phase = CataloguePhase::MatchPlacements;
                    return Ok(CatalogueAction::Transition);
                }
                let marker_match = self.marker_matches[self.matching_marker];
                let entry = self
                    .reader
                    .as_ref()
                    .and_then(|reader| reader.entries.get(self.matching_square).copied())
                    .ok_or(MapError::Invalid("missing catalogue map square"))?;
                if self.matching_placement == 0 {
                    let marker = self
                        .records
                        .get(marker_match.record)
                        .ok_or(MapError::Invalid("missing map marker"))?;
                    if !marker_may_match_square(marker, marker_match.kind, entry) {
                        self.matching_marker += 1;
                        self.phase = CataloguePhase::MatchPlacements;
                        return Ok(CatalogueAction::Unit);
                    }
                }
                // Each marker still visits squares and placements in source
                // order, so an equal-distance tree keeps the original winner.
                let RetainedCandidate {
                    placement: candidate,
                    link_below,
                } = if let Some(candidates) = &self.retained_candidates {
                    candidates[retained_start + self.matching_placement]
                } else {
                    let placement = self.placements[self.matching_placement];
                    let land = self
                        .land
                        .as_ref()
                        .ok_or(MapError::Invalid("missing catalogue land square"))?;
                    RetainedCandidate {
                        placement,
                        link_below: land.link_below(placement.x, placement.z),
                    }
                };
                match marker_match.kind {
                    MapMarkerKind::RareTrees => {
                        let marker = self
                            .records
                            .get(marker_match.record)
                            .ok_or(MapError::Invalid("missing rare-tree marker"))?;
                        self.marker_matches[self.matching_marker].rare_tree_best =
                            rare_tree_candidate(
                                marker_match.rare_tree_best,
                                marker,
                                candidate,
                                entry,
                                link_below,
                                &self.locs,
                            )?;
                        self.matching_placement += 1;
                        self.phase = CataloguePhase::MatchPlacements;
                        Ok(CatalogueAction::Unit)
                    }
                    MapMarkerKind::QuestStart => {
                        let target_id = i32::try_from(candidate.id)
                            .map_err(|_| MapError::Invalid("quest-start location id"))?;
                        if self.matching_quest_target_end == 0 {
                            let start = self
                                .quest_loc_targets
                                .partition_point(|(id, _)| *id < target_id);
                            let end = self
                                .quest_loc_targets
                                .partition_point(|(id, _)| *id <= target_id);
                            if start == end {
                                self.matching_placement += 1;
                                self.matching_quest_entry = 0;
                                self.matching_quest_target_end = 0;
                                self.phase = CataloguePhase::MatchPlacements;
                                return Ok(CatalogueAction::Unit);
                            }
                            self.matching_quest_entry = start;
                            self.matching_quest_target_end = end;
                        }
                        if self.matching_quest_entry >= self.matching_quest_target_end {
                            self.matching_placement += 1;
                            self.matching_quest_entry = 0;
                            self.matching_quest_target_end = 0;
                            self.phase = CataloguePhase::MatchPlacements;
                            return Ok(CatalogueAction::Transition);
                        }
                        let entry_index = self.quest_loc_targets[self.matching_quest_entry].1;
                        self.matching_quest_entry += 1;
                        let match_index =
                            self.matching_marker * self.quest_start_entries + entry_index;
                        if self.quest_loc_matches[match_index] == 0 {
                            let quest = self
                                .quest_starts
                                .as_ref()
                                .and_then(|catalog| catalog.entries.get(entry_index))
                                .ok_or(MapError::Invalid("missing quest-start entry"))?;
                            let marker = self
                                .records
                                .get(marker_match.record)
                                .ok_or(MapError::Invalid("missing quest-start marker"))?;
                            if quest_loc_candidate_matches(
                                quest, marker, candidate, entry, link_below,
                            )? {
                                self.quest_loc_matches[match_index] = 1;
                            }
                        }
                        self.phase = CataloguePhase::MatchPlacements;
                        Ok(CatalogueAction::Unit)
                    }
                }
            }
            CataloguePhase::FinalizeMatches {
                marker,
                entry,
                target,
                mut names,
            } => {
                if marker >= self.marker_matches.len() {
                    self.phase = CataloguePhase::SortRecords;
                    return Ok(CatalogueAction::Transition);
                }
                let marker_match = self.marker_matches[marker];
                match marker_match.kind {
                    MapMarkerKind::RareTrees => {
                        if let Some((_, tree_id)) = marker_match.rare_tree_best {
                            if let Some(tree_def) = self.locs.get(tree_id as usize) {
                                match Text::new(&tree_def.name) {
                                    Ok(name) => self.records[marker_match.record].name = name,
                                    Err(_) => {
                                        self.unknown_mapfunctions =
                                            self.unknown_mapfunctions.saturating_add(1);
                                    }
                                }
                            } else {
                                self.unknown_mapfunctions =
                                    self.unknown_mapfunctions.saturating_add(1);
                            }
                        } else {
                            self.unknown_mapfunctions = self.unknown_mapfunctions.saturating_add(1);
                        }
                        self.phase = CataloguePhase::FinalizeMatches {
                            marker: marker + 1,
                            entry: 0,
                            target: 0,
                            names: BTreeSet::new(),
                        };
                        Ok(CatalogueAction::Unit)
                    }
                    MapMarkerKind::QuestStart => {
                        let Some(catalog) = self.quest_starts.as_ref() else {
                            self.phase = CataloguePhase::FinalizeMatches {
                                marker: marker + 1,
                                entry: 0,
                                target: 0,
                                names: BTreeSet::new(),
                            };
                            return Ok(CatalogueAction::Unit);
                        };
                        if entry >= catalog.entries.len() {
                            if !names.is_empty() {
                                let joined = names.into_iter().collect::<Vec<_>>().join(", ");
                                if let Ok(name) = Text::new(&joined) {
                                    self.records[marker_match.record].name = name;
                                    self.unknown_mapfunctions =
                                        self.unknown_mapfunctions.saturating_sub(1);
                                }
                            }
                            self.phase = CataloguePhase::FinalizeMatches {
                                marker: marker + 1,
                                entry: 0,
                                target: 0,
                                names: BTreeSet::new(),
                            };
                            return Ok(CatalogueAction::Unit);
                        }
                        let quest = &catalog.entries[entry];
                        match quest.kind {
                            QuestStartTargetKind::Npc => {
                                if target >= catalog.npc_placements.len() {
                                    self.phase = CataloguePhase::FinalizeMatches {
                                        marker,
                                        entry: entry + 1,
                                        target: 0,
                                        names,
                                    };
                                    return Ok(CatalogueAction::Unit);
                                }
                                let placement = catalog.npc_placements[target];
                                let matched = placement.id == quest.id
                                    && placement.plane
                                        == self.records[marker_match.record].effective_plane
                                    && nearby(
                                        placement.x,
                                        placement.z,
                                        self.records[marker_match.record].display.x,
                                        self.records[marker_match.record].display.z,
                                    );
                                if matched {
                                    names.insert(quest.display.clone());
                                    self.phase = CataloguePhase::FinalizeMatches {
                                        marker,
                                        entry: entry + 1,
                                        target: 0,
                                        names,
                                    };
                                } else {
                                    self.phase = CataloguePhase::FinalizeMatches {
                                        marker,
                                        entry,
                                        target: target + 1,
                                        names,
                                    };
                                }
                                Ok(CatalogueAction::Unit)
                            }
                            QuestStartTargetKind::Loc => {
                                let match_index = marker
                                    .checked_mul(self.quest_start_entries)
                                    .and_then(|base| base.checked_add(entry))
                                    .ok_or(MapError::Limit("quest-start match count"))?;
                                if self
                                    .quest_loc_matches
                                    .get(match_index)
                                    .is_some_and(|matched| *matched != 0)
                                {
                                    names.insert(quest.display.clone());
                                }
                                self.phase = CataloguePhase::FinalizeMatches {
                                    marker,
                                    entry: entry + 1,
                                    target: 0,
                                    names,
                                };
                                Ok(CatalogueAction::Unit)
                            }
                        }
                    }
                }
            }
            CataloguePhase::SortRecords => {
                // Physical and annotation records use different entity tags, so
                // duplicates remain separate even when their labels match.
                self.records.sort_unstable_by_key(|record| record.key);
                self.phase = CataloguePhase::Stats;
                Ok(CatalogueAction::Unit)
            }
            CataloguePhase::Stats => {
                self.stats.retained_records = self.records.len() as u32;
                self.stats.physical_service_records = self
                    .records
                    .iter()
                    .filter(|record| record.key.entity == EntityKind::Loc)
                    .count() as u32;
                self.stats.max_record_bytes = self
                    .reader
                    .as_ref()
                    .ok_or(MapError::Invalid("missing cache reader"))?
                    .max_buffer_bytes();
                self.phase = CataloguePhase::Document;
                Ok(CatalogueAction::Unit)
            }
            CataloguePhase::Document => {
                let identity = self
                    .identity
                    .ok_or(MapError::Invalid("missing catalogue identity"))?;
                let mut unresolved = vec![
                    CoverageIssue {
                        reason: CoverageReason::MissingNpcPlacements,
                        reference: Text::new(
                            "npc.dat contains definitions but no static world placements",
                        )?,
                        count: self.stats.npc_service_definitions,
                    },
                    CoverageIssue {
                        reason: CoverageReason::MissingLabels,
                        reference: Text::new(
                            "ordinary client cache has no world place-label placements",
                        )?,
                        count: 1,
                    },
                ];
                if self.unknown_mapfunctions != 0 {
                    unresolved.push(CoverageIssue {
                        reason: CoverageReason::UnknownMapFunction,
                        reference: Text::new(
                            "unmapped client mapfunction categories remain generic",
                        )?,
                        count: self.unknown_mapfunctions,
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
                    records: Rows::new(std::mem::take(&mut self.records))?,
                };
                document.validate(identity)?;
                self.result = Some(CatalogueResult {
                    pois: document,
                    stats: self.stats,
                });
                self.phase = CataloguePhase::Complete;
                Ok(CatalogueAction::Unit)
            }
            CataloguePhase::Complete => self
                .result
                .take()
                .map(CatalogueAction::Completed)
                .ok_or(MapError::Invalid("missing completed catalogue")),
        }
    }

    fn retain_candidate(
        &mut self,
        placement: LocPlacement,
        link_below: bool,
    ) -> Result<(), MapError> {
        let Some(candidates) = &mut self.retained_candidates else {
            return Ok(());
        };
        let definition = self
            .locs
            .get(placement.id as usize)
            .ok_or(MapError::Invalid("location definition id"))?;
        let quest_id = i32::try_from(placement.id)
            .map_err(|_| MapError::Invalid("quest-start location id"))?;
        let quest = self
            .quest_loc_targets
            .binary_search_by_key(&quest_id, |(id, _)| *id)
            .is_ok();
        if is_woodcut_tree(definition) || quest {
            if candidates.len() == candidates.capacity() {
                // Release the complete retained prefix before the fallback scan.
                self.retained_candidates = None;
                self.retained_square_ends = Vec::new();
            } else {
                candidates.push(RetainedCandidate {
                    placement,
                    link_below,
                });
            }
        }
        Ok(())
    }

    fn process_placement(
        &mut self,
        placement: LocPlacement,
        entry: MapIndexEntry,
        link_below: bool,
    ) -> Result<(), MapError> {
        let Some(definition) = self.locs.get(placement.id as usize) else {
            return Err(MapError::Invalid("location definition id"));
        };
        let source = SourceSpace::ClientVisual {
            plane: placement.plane,
            link_below,
        };
        let effective_plane = match source.game_plane()? {
            Some(plane) => plane,
            None => return Ok(()),
        };
        let footprint = definition.footprint(placement.rotation)?;
        let world_x =
            i32::from(entry.square_x) * i32::from(MAP_SQUARE_SIZE) + i32::from(placement.x);
        let world_z =
            i32::from(entry.square_z) * i32::from(MAP_SQUARE_SIZE) + i32::from(placement.z);
        let name = match poi_name(definition)? {
            Some(name) => name,
            None => return Ok(()),
        };
        let mut physical_kind = None;
        let mut physical_evidence = Vec::new();
        let mut annotation = None;
        classify_definition(
            self.input.revision(),
            &borrowed(definition),
            |kind, evidence| {
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
            },
        );
        let make_record = |entity, kind, evidence| -> Result<PoiRecord, MapError> {
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
            if self.records.len() == MAX_POIS {
                return Err(MapError::Limit("POI count"));
            }
            self.records
                .push(make_record(EntityKind::Loc, kind, physical_evidence)?);
        }
        if let Some((kind, evidence)) = annotation {
            if self.records.len() == MAX_POIS {
                return Err(MapError::Limit("POI count"));
            }
            if matches!(kind, PoiKind::MapSymbol { symbol } if symbol != RARE_TREES_SYMBOL) {
                self.unknown_mapfunctions += 1;
            }
            self.records.push(PoiRecord {
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
                evidence: Rows::new(vec![evidence])?,
                walk_target: None,
            });
            self.stats.mapfunction_records += 1;
        }
        Ok(())
    }
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
