//! Immutable generated game facts selected by the bound client revision.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};

use client::io::ClientRevision;
use serde::Deserialize;

const SCHEMA_VERSION: u16 = 4;
const REVISION_274: &[u8] = include_bytes!("../data/game-data/274.json");
const REVISION_289: &[u8] = include_bytes!("../data/game-data/289.json");

static DATA_274: OnceLock<Result<Arc<SelectedGameData>, String>> = OnceLock::new();
static DATA_289: OnceLock<Result<Arc<SelectedGameData>, String>> = OnceLock::new();

#[derive(Debug, Deserialize)]
struct CacheIdentity {
    cache_id: String,
    /// Decoded (`274DCI01`) identity of the same pinned cache. Absent in
    /// generated data written before decoded identity existed; runtime
    /// profiles assert it instead of a packed equivalence list.
    #[serde(default)]
    content_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Provenance {
    cache_identity: CacheIdentity,
    inputs: Vec<SourceInput>,
    content_inputs: Vec<SourceInput>,
    decoder_sources: Vec<SourceInput>,
}

/// Actual generator input retained for local-server compatibility checks.
#[derive(Debug, Deserialize)]
pub struct SourceInput {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

/// One selected-cache object row. Aliases come from generated server data;
/// no runtime normalization or alias invention is applied.
#[derive(Debug, Deserialize)]
pub struct GameItem {
    pub alias: Option<String>,
    pub id: i32,
    pub name: Option<String>,
    pub cost: i32,
    pub stackable: bool,
    pub members: bool,
    pub certificate_link: i32,
    pub certificate_template: i32,
    pub wear_position: i32,
    pub wear_position_2: i32,
    pub wear_position_3: i32,
    /// The engine's decoded trade flag: `tradeable=no`, a nonzero
    /// `dummyitem`, or the note of an untradeable item clears it.
    pub tradeable: bool,
    /// A pile-size model another obj names in `countobj`: it repeats the
    /// base item's name and is not a separate item.
    pub stack_variant: bool,
}

#[derive(Debug, Deserialize)]
struct NamedFact {
    name: String,
}

#[derive(Debug, Deserialize)]
struct StatHeal {
    stat: String,
    base: i32,
    percent: i32,
}

#[derive(Debug, Deserialize)]
struct ConsumptionFact {
    item: NamedFact,
    stat_heal: Vec<StatHeal>,
    qualification: String,
}

impl ConsumptionFact {
    fn fixed_hp_heal(&self) -> Option<i32> {
        let heal = self.stat_heal.as_slice();
        (self.qualification == "fixed_hp_heal"
            && heal.len() == 1
            && heal[0].stat == "hitpoints"
            && heal[0].percent == 0
            && heal[0].base > 0)
            .then(|| heal[0].base)
    }
}

fn ascii_fold_hash(value: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in value.bytes() {
        hash ^= u64::from(byte.to_ascii_lowercase());
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

#[derive(Debug, Deserialize)]
struct PickpocketFact {
    npcs: Vec<NamedFact>,
    level: i32,
}

/// Frozen staff_spells grid base: component id 1830 + ssb.
/// Inventory-target casts use posted magic-tab buttons, not this base.
/// Autocast uses the selected-cache `AutocastControls.spell_grid_base` when present.
pub const STAFF_SPELLS_COM0: i32 = 1830;

/// Packed combat-tab autocast controls from the selected interface/varp archives.
#[derive(Debug, Deserialize, Clone, Copy)]
pub struct AutocastControls {
    pub staff_tab_root: i32,
    pub spell_panel_root: i32,
    pub choose_com: i32,
    pub toggle_com: i32,
    pub spell_grid_base: i32,
    pub magic_varp: i32,
    pub selected_value: i32,
    pub armed_value: i32,
}

impl AutocastControls {
    pub fn available(&self) -> bool {
        self.staff_tab_root >= 0
            && self.spell_panel_root >= 0
            && self.choose_com >= 0
            && self.toggle_com >= 0
            && self.spell_grid_base >= 0
            && self.magic_varp >= 0
    }
}

/// Packed Duel Arena modal roots and partner/waiting/accept controls.
#[derive(Debug, Deserialize, Clone, Copy)]
pub struct DuelControls {
    pub select_modal: i32,
    pub confirm_modal: i32,
    pub win_modal: i32,
    pub select_accept: i32,
    pub confirm_accept: i32,
    pub select_partner: i32,
    pub select_status: i32,
    pub confirm_status: i32,
}

impl DuelControls {
    pub fn available(&self) -> bool {
        self.select_modal >= 0
            && self.confirm_modal >= 0
            && self.win_modal >= 0
            && self.select_accept >= 0
            && self.confirm_accept >= 0
            && self.select_partner >= 0
            && self.select_status >= 0
            && self.confirm_status >= 0
    }
}

/// Packed combat-tab spec bar for one weapon interface root.
#[derive(Debug, Deserialize, Clone)]
pub struct SpecialBar {
    pub root: String,
    pub root_id: i32,
    pub bar: i32,
}

/// Weapon that carries both `specwep` and a positive `sa_energy` cost.
#[derive(Debug, Deserialize, Clone)]
pub struct SpecialWeapon {
    pub alias: String,
    pub id: i32,
    pub name: String,
    pub cost: i32,
}

/// Packed non-target spellbook teleport from selected cache metadata.
/// Distinguished from target combat/enchant spells by a `tele_coord` row.
#[derive(Debug, Deserialize, Clone)]
pub struct TeleportSpell {
    pub name: String,
    pub source_row: String,
    pub spell: String,
    pub component_id: i32,
    pub members: bool,
    pub level: i32,
    pub runes: Vec<SpellRune>,
    pub experience: i32,
    pub tele_coord: String,
    pub x: i32,
    pub z: i32,
    pub plane: i32,
}

impl TeleportSpell {
    pub fn available(&self) -> bool {
        self.component_id >= 0 && self.level > 0 && !self.name.is_empty()
    }
}

/// Packed special-attack varps, bars and weapon costs from the selected cache.
#[derive(Debug, Deserialize, Clone)]
pub struct SpecialControls {
    pub energy_varp: i32,
    pub armed_varp: i32,
    pub armed_value: i32,
    pub max_energy: i32,
    pub arm_confirm_ticks: i32,
    pub bars: Vec<SpecialBar>,
    pub weapons: Vec<SpecialWeapon>,
}

impl SpecialControls {
    pub fn available(&self) -> bool {
        self.energy_varp >= 0
            && self.armed_varp >= 0
            && self.max_energy > 0
            && self.arm_confirm_ticks > 0
            && !self.bars.is_empty()
            && !self.weapons.is_empty()
    }

    /// Energy cost for a display name, or `None` when the weapon has no special.
    pub fn cost(&self, weapon_name: &str) -> Option<i32> {
        let wanted = weapon_name.trim();
        self.weapons
            .iter()
            .find(|weapon| weapon.name.eq_ignore_ascii_case(wanted))
            .map(|weapon| weapon.cost)
    }

    /// Spec-bar component for a posted combat-tab root, or -1 when absent.
    pub fn bar_for_root(&self, combat_tab_root: i32) -> i32 {
        self.bars
            .iter()
            .find(|bar| bar.root_id == combat_tab_root)
            .map(|bar| bar.bar)
            .unwrap_or(-1)
    }
}

/// One generated per-cast rune cost.
#[derive(Debug, Deserialize, Clone)]
pub struct SpellRune {
    pub alias: String,
    pub id: i32,
    pub name: String,
    pub count: i32,
}

/// Named autocast combat spell from the selected cache.
#[derive(Debug, Deserialize)]
pub struct SpellFact {
    pub name: String,
    pub ssb: i32,
    pub level: i32,
    pub continue_by_autocast: bool,
    pub runes: Vec<SpellRune>,
}

/// Rune provided by a staff while wielded.
#[derive(Debug, Deserialize, Clone)]
pub struct StaffRune {
    pub alias: String,
    pub id: i32,
    pub name: String,
}

/// Staff that substitutes one or more runes.
#[derive(Debug, Deserialize)]
pub struct StaffFact {
    pub alias: String,
    pub id: i32,
    pub name: String,
    pub runes: Vec<StaffRune>,
}

/// One generated drop-table item, preserving alias/id evidence.
#[derive(Debug, Deserialize, Clone)]
pub struct DropItem {
    pub alias: String,
    pub id: i32,
    pub name: String,
}

/// One selected-revision monster drop row for the four enabled combat cards.
#[derive(Debug, Deserialize, Clone)]
pub struct DropTable {
    pub npc_alias: String,
    pub npc_id: i32,
    pub name: String,
    pub source_block: String,
    pub items: Vec<DropItem>,
    pub display_names: Vec<String>,
}

/// One identified/unidentified herb pair from selected herblore content.
#[derive(Debug, Deserialize, Clone)]
pub struct HerbFact {
    pub key: String,
    pub name: String,
    pub id: i32,
    #[serde(rename = "unidId")]
    pub unid_id: i32,
    pub level: i32,
}

/// Remaining rune cost after staff substitution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemainingRuneCost {
    pub rune: String,
    pub count: i32,
}

/// One prayer row from selected dbrow/interface/varp joins.
#[derive(Debug, Deserialize, Clone)]
pub struct PrayerFact {
    pub name: String,
    pub level: i32,
    pub source_row: String,
    pub prayer_constant: String,
    pub button_com: i32,
    pub com_alias: String,
    pub varp: i32,
    pub varp_alias: String,
}

/// One pickaxe row from `[pickaxeshop]` stock joined to ObjType base cost.
#[derive(Debug, Deserialize, Clone)]
pub struct PickaxeShopFact {
    pub alias: String,
    pub id: i32,
    pub name: String,
    pub base_cost: i32,
    pub shop_baseline_qty: i32,
    pub cost_source: String,
}

/// Mapsquare predicate for the enclosed Rune Essence mine (m45_75).
#[derive(Debug, Deserialize, Clone)]
pub struct EssenceRegionFact {
    pub mapsquare_mx: i32,
    pub mapsquare_mz: i32,
    pub predicate: String,
    pub source_map: String,
    pub mapsquare_from_filename: bool,
    pub mine_portal_loc_alias: String,
    pub mine_portal_loc_id: i32,
    pub mine_portal_loc_placements: i32,
    pub example_inside: EssenceExampleTile,
}

#[derive(Debug, Deserialize, Clone)]
pub struct EssenceExampleTile {
    pub x: i32,
    pub z: i32,
    pub plane: i32,
}

/// Curated shop-approach tiles copied from frozen reference tactics, not jm2 facts.
#[derive(Debug, Deserialize, Clone)]
pub struct CuratedVendorTactics {
    pub label: String,
    pub authority: String,
    pub keeper: String,
    pub stand: TileFact,
    pub bank_stand: TileFact,
    pub hop_from: TileFact,
    pub hop_loc: String,
    pub hop_action: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TileFact {
    pub x: i32,
    pub z: i32,
    pub plane: i32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AuburyTravelNote {
    pub note: String,
    pub npc_alias: String,
    pub npc_id: i32,
    pub npc_name: String,
    pub already_packed: bool,
    pub return_anchor_constant: String,
    pub return_anchor_coord: String,
    pub return_anchor_source: String,
    pub return_anchor_role: String,
}

/// Nurmof pickaxe shop + essence region facts (no acquire routing).
#[derive(Debug, Deserialize, Clone)]
pub struct NurmofEssenceFacts {
    pub npc_alias: String,
    pub npc_id: i32,
    pub npc_name: String,
    pub shop_inv: String,
    pub shop_inv_source: String,
    pub pickaxes: Vec<PickaxeShopFact>,
    pub essence_region: EssenceRegionFact,
    pub aubury_travel: AuburyTravelNote,
    pub curated_vendor_tactics: CuratedVendorTactics,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FlourItemFact {
    pub alias: String,
    pub id: i32,
    pub name: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FlourLocFact {
    pub alias: String,
    pub id: i32,
    pub name: String,
    pub loc_config_source: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ProvenanceTile {
    pub x: i32,
    pub z: i32,
    pub plane: i32,
    #[serde(default)]
    pub role: Option<String>,
    pub provenance: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub authority: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub mapsquare: Option<String>,
    #[serde(default)]
    pub local: Option<FlourLocalTile>,
    #[serde(default)]
    pub loc_shape: Option<i32>,
    #[serde(default)]
    pub loc_angle: Option<i32>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FlourLocalTile {
    pub lx: i32,
    pub lz: i32,
}

/// Six Murder Mystery flour pins (quest name, pots, barrel, object + approach + bank tiles).
#[derive(Debug, Deserialize, Clone)]
pub struct FlourSixFacts {
    pub quest_name: String,
    pub quest_name_source: String,
    pub pot: FlourItemFact,
    pub pot_flour: FlourItemFact,
    pub flour_barrel: FlourLocFact,
    pub flour_barrel_object_tile: ProvenanceTile,
    pub flour_barrel_approach_tile: ProvenanceTile,
    pub bank_tile: ProvenanceTile,
}

#[derive(Debug, Deserialize, Clone)]
pub struct EquipmentInputPin {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    #[serde(default)]
    pub commit: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct EquipmentNameCandidate {
    pub alias: String,
    pub id: i32,
    pub name: String,
    pub wear_position: i32,
    pub note: String,
}

/// One frozen equipment.ts display name joined to selected item facts.
#[derive(Debug, Deserialize, Clone)]
pub struct EquipmentNameEntry {
    pub requested_name: String,
    pub disposition: String,
    #[serde(default)]
    pub selected_name: Option<String>,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub id: Option<i32>,
    #[serde(default)]
    pub wear_position: Option<i32>,
    #[serde(default)]
    pub disambiguation: Option<String>,
    #[serde(default)]
    pub absent_class: Option<String>,
    #[serde(default)]
    pub candidates: Option<Vec<EquipmentNameCandidate>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct EquipmentBoltJoinLimitation {
    pub generic_display_name: String,
    pub generic_is_substitute: bool,
    pub note: String,
}

/// Exact display-name join limitation: a miss is not a global content claim.
#[derive(Debug, Deserialize, Clone)]
pub struct EquipmentExactNameJoin {
    pub matching: String,
    pub no_exact_selected_match_means: String,
    pub bolts: EquipmentBoltJoinLimitation,
}

/// Curated rs2b0t combat equipment name families with selected-revision joins.
#[derive(Debug, Deserialize, Clone)]
pub struct EquipmentNamesFacts {
    pub curated_input: EquipmentInputPin,
    pub equipment_source: EquipmentInputPin,
    #[serde(default)]
    pub equipment_evidence: Option<EquipmentInputPin>,
    #[serde(default)]
    pub exact_name_join: Option<EquipmentExactNameJoin>,
    pub bows: Vec<EquipmentNameEntry>,
    pub crossbows: Vec<EquipmentNameEntry>,
    pub darts: Vec<EquipmentNameEntry>,
    pub arrows: Vec<EquipmentNameEntry>,
    pub bolts: Vec<EquipmentNameEntry>,
    pub melee_weapons: Vec<EquipmentNameEntry>,
    pub staffs: Vec<EquipmentNameEntry>,
}

impl EquipmentNamesFacts {
    pub fn family(&self, family: &str) -> Option<&[EquipmentNameEntry]> {
        match family.trim().to_ascii_lowercase().as_str() {
            "bows" => Some(&self.bows),
            "crossbows" => Some(&self.crossbows),
            "darts" => Some(&self.darts),
            "arrows" => Some(&self.arrows),
            "bolts" => Some(&self.bolts),
            "melee_weapons" | "melee" => Some(&self.melee_weapons),
            "staffs" | "staves" => Some(&self.staffs),
            _ => None,
        }
    }

    pub fn entry(&self, family: &str, requested_name: &str) -> Option<&EquipmentNameEntry> {
        let wanted = requested_name.trim();
        self.family(family)?
            .iter()
            .find(|row| row.requested_name.eq_ignore_ascii_case(wanted))
    }
}

/// One packed loc or depleted-stage id joined from this pin. Alias is the pack key, not a display name.
#[derive(Debug, Deserialize, Clone)]
pub struct GatherLocId {
    pub alias: String,
    pub id: i32,
}

/// Item produced by a loc-resource row. Absent when the table names no output.
#[derive(Debug, Deserialize, Clone)]
pub struct GatherOutput {
    pub alias: String,
    pub id: i32,
}

/// Wood or mining identity. Not a fishing method. Resource key is the wood key or `ore_name`, never the loc display name.
#[derive(Debug, Deserialize, Clone)]
pub struct GatherLocResource {
    pub table: String,
    pub resource_key: String,
    pub loc_ids: Vec<GatherLocId>,
    pub empty_ids: Vec<GatherLocId>,
    pub output: Option<GatherOutput>,
    pub level: i32,
    pub qualification: String,
    pub partial_sides: Vec<String>,
    pub missing_transform: Vec<String>,
    #[serde(default)]
    pub publication: Option<String>,
}

/// Fishing method identity: category plus posted ops. Not a spawn tile.
#[derive(Debug, Deserialize, Clone)]
pub struct GatherFishingMethod {
    pub category: String,
    pub primary_op: String,
    pub pair_op: Option<String>,
    pub level: Option<i32>,
    pub output: Option<GatherOutput>,
    pub qualification: String,
    pub partial_sides: Vec<String>,
}

/// Coverage for conditional woods and revision-absent locs. Not a copied id.
#[derive(Debug, Deserialize, Clone)]
pub struct GatherCoverageRecord {
    pub class: String,
    #[serde(default)]
    pub table: Option<String>,
    #[serde(default)]
    pub resource_key: Option<String>,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub on_revision: Option<i32>,
    #[serde(default)]
    pub other_pin_id: Option<i32>,
    #[serde(default)]
    pub copied: Option<bool>,
    pub reason: String,
}

/// One gather family. Absence is `None`, not an empty list.
#[derive(Debug, Deserialize, Clone)]
pub struct GatherMethodsFacts {
    pub woods: Vec<GatherLocResource>,
    pub mining: Vec<GatherLocResource>,
    pub fishing: Vec<GatherFishingMethod>,
    #[serde(default)]
    pub coverage: Vec<GatherCoverageRecord>,
}

/// One world LOC placement of a published resource loc id. `x`/`z` are world
/// coordinates; no local coords, shape, or angle is stored.
#[derive(Debug, Deserialize, Clone)]
pub struct GatherPlacement {
    pub loc_id: i32,
    pub x: i32,
    pub z: i32,
    pub plane: i32,
}

/// A gather family with no selected published set. Unknown is not empty rows.
#[derive(Debug, Deserialize, Clone)]
pub struct GatherPlacementCoverage {
    pub class: String,
    pub family: String,
    pub reason: String,
}

/// Published-resource world placements. Absence is `None`, not an empty list.
/// `coverage` records what this family does not publish, so a present family
/// that omits it is a decode error and so is an empty row list.
#[derive(Debug, Deserialize, Clone)]
pub struct GatherPlacementsFacts {
    pub rows: Vec<GatherPlacement>,
    pub coverage: Vec<GatherPlacementCoverage>,
}

/// Script alias from a selected handler. Not a pack-joined display name.
/// `quantity` is the `inv_del` count; a use-site check has none.
#[derive(Debug, Deserialize, Clone)]
pub struct QuestItemAlias {
    pub alias: String,
    pub quantity: Option<i32>,
    pub kind: String,
}

/// Mid-quest stat gate. Not a complete start set.
#[derive(Debug, Deserialize, Clone)]
pub struct QuestSkillGate {
    pub skill: String,
    pub level: i32,
}

/// Requirements sit on the identity row. Qualification stays `partial`.
/// Empty frozen mustHave is not complete and is not unknown-as-satisfied.
#[derive(Debug, Deserialize, Clone)]
pub struct QuestRequirements {
    pub qualification: String,
    pub skills: Vec<QuestSkillGate>,
    pub items: Vec<QuestItemAlias>,
    pub empty_must_have: bool,
    pub unknown_as_satisfied: bool,
}

/// One seed join. `id` is the seed id. `complete` is one constant, not a range.
/// `unknown_sides` stays empty. Enum index is not a field.
#[derive(Debug, Deserialize, Clone)]
pub struct QuestIdentityRow {
    pub id: String,
    pub component: String,
    pub display: String,
    pub varp: String,
    pub varp_id: i32,
    pub complete: i32,
    pub quest_points: i32,
    pub unknown_sides: Vec<String>,
    pub requirements: QuestRequirements,
}

/// Revision coverage. Not an identity row and not a copied id.
#[derive(Debug, Deserialize, Clone)]
pub struct QuestCoverageRecord {
    pub class: String,
    pub alias: String,
    pub on_revision: i32,
    pub other_pin_id: i32,
    pub copied: bool,
    pub reason: String,
}

/// Six-seed quest identity. Absence is `None`, not an empty list.
/// A coverage record is a sibling, not a row.
#[derive(Debug, Deserialize, Clone)]
pub struct QuestIdentityFacts {
    pub rows: Vec<QuestIdentityRow>,
    #[serde(default)]
    pub coverage: Vec<QuestCoverageRecord>,
}

/// One raw `param=` line on a selected trail obj block. Repeated keys stay a list,
/// in file order. Values are never coerced to a boolean or a number.
#[derive(Debug, Deserialize, Clone)]
pub struct TrailParam {
    pub key: String,
    pub value: String,
}

/// One trail membership row: an enum alias joined to `pack/obj.pack`, or a casket
/// alias those rows name. `params` is part of the row shape, not a defaulted list.
/// `access` is present only on the one bounded inclusion (packed 3554).
#[derive(Debug, Deserialize, Clone)]
pub struct TrailMembershipRow {
    pub alias: String,
    pub id: i32,
    pub role: String,
    pub params: Vec<TrailParam>,
    #[serde(default)]
    pub access: Option<String>,
}

/// One selected challenge answer. The raw param string, not a coerced number.
#[derive(Debug, Deserialize, Clone)]
pub struct TrailChallengeAnswer {
    pub alias: String,
    pub id: i32,
    pub answer: String,
}

/// Trail inventory and its selected challenge answers. Absence is `None`, not an
/// empty list. `challenge_answers` has no serde default: a present family that
/// omits the sibling list is a decode error, and so is an empty list.
#[derive(Debug, Deserialize, Clone)]
pub struct TrailFacts {
    pub rows: Vec<TrailMembershipRow>,
    pub challenge_answers: Vec<TrailChallengeAnswer>,
}

/// One jm2 `==== NPC ====` spawn in world coordinates. `plane` is the scene
/// plane, never the `level` field of a coordinate triple.
#[derive(Debug, Deserialize, Clone)]
pub struct TalkKeySpawn {
    pub x: i32,
    pub z: i32,
    pub plane: i32,
}

/// The named NPC one talk step (or one type keeper) speaks to.
#[derive(Debug, Deserialize, Clone)]
pub struct TalkKeyNpcRef {
    pub alias: String,
    pub id: i32,
    pub name: String,
}

/// One keeper matcher, discriminated on `kind`. `type` is one packed npc id and
/// carries the `.npc` display name; `category` and `name` match many npcs and
/// never carry an alias or an id. `decode` refuses any other combination.
#[derive(Debug, Deserialize, Clone)]
pub struct TalkKeyKeeper {
    pub kind: String,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub id: Option<i32>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
}

/// One opnpc1 talk step: the membership clue that anchors it and the NPC it names.
/// `spawn` is omitted when the jm2 spawn is not unique; a present-but-null spawn
/// is a decode error, not an absent one.
#[derive(Debug, Deserialize, Clone)]
pub struct TalkKeyTalkRow {
    pub alias: String,
    pub id: i32,
    pub npc: TalkKeyNpcRef,
    #[serde(default, deserialize_with = "deserialize_present_spawn")]
    pub spawn: Option<TalkKeySpawn>,
}

/// One key-keeper step: the membership clue, its key object, and the matcher.
#[derive(Debug, Deserialize, Clone)]
pub struct TalkKeyKeyRow {
    pub alias: String,
    pub id: i32,
    pub key_alias: String,
    pub key_id: i32,
    pub keeper: TalkKeyKeeper,
    #[serde(default, deserialize_with = "deserialize_present_spawn")]
    pub spawn: Option<TalkKeySpawn>,
}

/// Why one step publishes no spawn. A coverage record is a sibling, not a row.
#[derive(Debug, Deserialize, Clone)]
pub struct TalkKeyCoverageRecord {
    pub class: String,
    pub family: String,
    pub alias: String,
    pub reason: String,
}

/// Selected talk steps and key keepers. Absence is `None`, not an empty list.
/// `talk`, `keys`, and `coverage` have no serde default: a present family that
/// omits one is a decode error, and so is an empty list. A step whose spawn is
/// not unique keeps its row and records coverage instead of dropping the row.
#[derive(Debug, Deserialize, Clone)]
pub struct TalkKeyFacts {
    pub talk: Vec<TalkKeyTalkRow>,
    pub keys: Vec<TalkKeyKeyRow>,
    pub coverage: Vec<TalkKeyCoverageRecord>,
}

/// One jm2 `==== NPC ====` spawn in world coordinates for a published giver.
/// `plane` is the scene plane, never the `level` field of a coordinate triple.
#[derive(Debug, Deserialize, Clone)]
pub struct TrioGiverSpawn {
    pub x: i32,
    pub z: i32,
    pub plane: i32,
}

/// One selected coordinate-tool giver: the packed NPC identity itself, plus the
/// world tile it stands on. `spawn` is omitted when the jm2 spawn is not unique;
/// a present-but-null spawn is a decode error, not an absent one. The row is the
/// identity, never a clue alias wrapping a nested npc.
#[derive(Debug, Deserialize, Clone)]
pub struct TrioGiverRow {
    pub alias: String,
    pub id: i32,
    pub name: String,
    #[serde(default, deserialize_with = "deserialize_present_trio_giver_spawn")]
    pub spawn: Option<TrioGiverSpawn>,
}

/// Why one giver publishes no spawn. A coverage record is a sibling, not a row.
#[derive(Debug, Deserialize, Clone)]
pub struct TrioGiverCoverageRecord {
    pub class: String,
    pub family: String,
    pub alias: String,
    pub reason: String,
}

/// Selected givers and their unknown-spawn coverage. Absence is `None`, not an
/// empty list. Neither `rows` nor `coverage` has a serde default: a present
/// family that omits either is a decode error. Empty `coverage` is allowed only
/// when every row published a unique spawn, and then it is required.
#[derive(Debug, Deserialize, Clone)]
pub struct TrioGiverFacts {
    pub rows: Vec<TrioGiverRow>,
    pub coverage: Vec<TrioGiverCoverageRecord>,
}

/// A row either publishes a spawn or omits the key. `null` is neither, so it is
/// refused instead of silently decoding as an absent spawn.
fn deserialize_present_spawn<'de, D>(deserializer: D) -> Result<Option<TalkKeySpawn>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    present_spawn(deserializer, "talk_key")
}

/// The same rule for the sibling giver family, under its own name.
fn deserialize_present_trio_giver_spawn<'de, D>(
    deserializer: D,
) -> Result<Option<TrioGiverSpawn>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    present_spawn(deserializer, "trio_givers")
}

fn present_spawn<'de, D, T>(deserializer: D, family: &str) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    if value.is_null() {
        return Err(serde::de::Error::custom(format!(
            "{family} spawn must be omitted, not null"
        )));
    }
    T::deserialize(value)
        .map(Some)
        .map_err(serde::de::Error::custom)
}

/// Generated immutable facts for one client/cache revision.
#[derive(Debug, Deserialize)]
pub struct SelectedGameData {
    schema_version: u16,
    revision: i32,
    provenance: Provenance,
    items: Vec<GameItem>,
    consumption: Vec<ConsumptionFact>,
    pickpocket: Vec<PickpocketFact>,
    #[serde(default)]
    spells: Vec<SpellFact>,
    #[serde(default)]
    staves: Vec<StaffFact>,
    #[serde(default)]
    autocast: Option<AutocastControls>,
    #[serde(default)]
    duel: Option<DuelControls>,
    #[serde(default)]
    special: Option<SpecialControls>,
    #[serde(default)]
    teleports: Vec<TeleportSpell>,
    #[serde(default)]
    herbs: Vec<HerbFact>,
    #[serde(default)]
    herb_level_default: Option<i32>,
    #[serde(default)]
    drop_tables: Vec<DropTable>,
    #[serde(default)]
    prayers: Vec<PrayerFact>,
    #[serde(default)]
    nurmof_essence: Option<NurmofEssenceFacts>,
    #[serde(default)]
    flour_six: Option<FlourSixFacts>,
    #[serde(default)]
    equipment_names: Option<EquipmentNamesFacts>,
    #[serde(default)]
    gather_methods: Option<GatherMethodsFacts>,
    #[serde(default)]
    gather_placements: Option<GatherPlacementsFacts>,
    #[serde(default)]
    quest_identity: Option<QuestIdentityFacts>,
    #[serde(default)]
    trails: Option<TrailFacts>,
    #[serde(default)]
    talk_key: Option<TalkKeyFacts>,
    #[serde(default)]
    trio_givers: Option<TrioGiverFacts>,
    #[serde(default)]
    bank_placements: Option<crate::named_banks::BankPlacementFacts>,
    #[serde(default)]
    cook_surfaces: Option<crate::cook_locations::CookSurfaceFacts>,
    #[serde(skip)]
    item_id_index: Vec<Option<usize>>,
    #[serde(skip)]
    fixed_food_heals_index: Vec<(String, i32)>,
    #[serde(skip)]
    fixed_food_heal_index: HashMap<u64, Vec<usize>>,
}

impl SelectedGameData {
    fn decode(bytes: &[u8], expected_revision: ClientRevision) -> Result<Arc<Self>, String> {
        let mut data: Self = serde_json::from_slice(bytes)
            .map_err(|error| format!("generated game data decode: {error}"))?;
        if data.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "generated game data schema mismatch: expected {SCHEMA_VERSION}, got {}",
                data.schema_version
            ));
        }
        if let Some(facts) = &data.bank_placements {
            if facts
                .rows
                .iter()
                .any(|row| row.width < 1 || row.length < 1 || !(0..4).contains(&row.level))
            {
                return Err("bank_placements invalid access footprint".to_string());
            }
        }
        if data
            .cook_surfaces
            .as_ref()
            .is_some_and(|facts| facts.rows.is_empty())
        {
            return Err("cook_surfaces present with no placed surfaces".to_string());
        }
        if let Some(facts) = &data.gather_methods {
            if facts.woods.is_empty() && facts.mining.is_empty() && facts.fishing.is_empty() {
                return Err("gather_methods present with no extracted rows".to_string());
            }
        }
        if let Some(facts) = &data.gather_placements {
            if facts.rows.is_empty() {
                return Err("gather_placements present with no world rows".to_string());
            }
            if facts.coverage.is_empty() {
                return Err("gather_placements present with no coverage".to_string());
            }
        }
        if let Some(facts) = &data.quest_identity {
            if facts.rows.is_empty() {
                return Err("quest_identity present with no identity rows".to_string());
            }
            if facts
                .rows
                .iter()
                .any(|row| row.requirements.qualification != "partial")
            {
                return Err("quest_identity requirements must stay partial".to_string());
            }
            if facts
                .rows
                .iter()
                .any(|row| row.requirements.unknown_as_satisfied)
            {
                return Err("quest_identity empty mustHave is not unknown-satisfied".to_string());
            }
            if facts.rows.iter().any(|row| !row.unknown_sides.is_empty()) {
                return Err("quest_identity unknown_sides must stay empty".to_string());
            }
            if facts.rows.iter().any(|row| {
                row.requirements.empty_must_have
                    && (!row.requirements.items.is_empty() || !row.requirements.skills.is_empty())
            }) {
                return Err(
                    "quest_identity empty mustHave cannot carry items or skills".to_string()
                );
            }
        }
        if let Some(facts) = &data.trails {
            if facts.rows.is_empty() {
                return Err("trails present with no membership rows".to_string());
            }
            if facts.challenge_answers.is_empty() {
                return Err("trails present with no selected challenge answers".to_string());
            }
            if facts
                .rows
                .iter()
                .any(|row| row.role != "clue" && row.role != "casket")
            {
                return Err("trails row role must be clue or casket".to_string());
            }
            if facts.rows.iter().any(|row| {
                row.access
                    .as_deref()
                    .is_some_and(|access| access != "constrained")
            }) {
                return Err("trails access is only the constrained membership row".to_string());
            }
            if facts.rows.iter().filter(|row| row.access.is_some()).count() > 1 {
                return Err("trails access must stay on one bounded row".to_string());
            }
        }
        if let Some(facts) = &data.talk_key {
            if facts.talk.is_empty() {
                return Err("talk_key present with no talk steps".to_string());
            }
            if facts.keys.is_empty() {
                return Err("talk_key present with no key keepers".to_string());
            }
            if facts.coverage.is_empty() {
                return Err("talk_key present with no coverage".to_string());
            }
            for row in &facts.keys {
                let keeper = &row.keeper;
                match keeper.kind.as_str() {
                    "type" => {
                        if keeper.alias.is_none()
                            || keeper.id.is_none()
                            || keeper.category.is_some()
                        {
                            return Err(
                                "talk_key type keeper must join one packed npc id".to_string()
                            );
                        }
                    }
                    "category" => {
                        if keeper.category.is_none()
                            || keeper.alias.is_some()
                            || keeper.id.is_some()
                            || keeper.name.is_some()
                        {
                            return Err(
                                "talk_key category keeper must be the bare category".to_string()
                            );
                        }
                    }
                    "name" => {
                        if keeper.name.is_none()
                            || keeper.alias.is_some()
                            || keeper.id.is_some()
                            || keeper.category.is_some()
                        {
                            return Err("talk_key name keeper must be the bare name".to_string());
                        }
                    }
                    other => {
                        return Err(format!("talk_key keeper kind {other} is not a keeper"));
                    }
                }
            }
        }
        if let Some(facts) = &data.trio_givers {
            if facts.rows.is_empty() {
                return Err("trio_givers present with no givers".to_string());
            }
            if facts.coverage.iter().any(|record| {
                record.class != "unknown"
                    || record.family != "trio_givers"
                    || record.reason.is_empty()
            }) {
                return Err(
                    "trio_givers coverage must be a named unknown with a reason".to_string()
                );
            }
            if facts.coverage.iter().any(|record| {
                !facts
                    .rows
                    .iter()
                    .any(|row| row.alias == record.alias && row.spawn.is_none())
            }) || facts.rows.iter().any(|row| {
                row.spawn.is_none()
                    && !facts
                        .coverage
                        .iter()
                        .any(|record| record.alias == row.alias)
            }) {
                return Err(
                    "trio_givers coverage must be exactly the givers without a unique spawn"
                        .to_string(),
                );
            }
        }
        if data.revision != expected_revision.as_i32() {
            return Err(format!(
                "generated game data revision mismatch: expected {}, got {}",
                expected_revision.as_i32(),
                data.revision
            ));
        }
        data.build_indexes();
        Ok(Arc::new(data))
    }

    fn build_indexes(&mut self) {
        let max_id = self
            .items
            .iter()
            .filter_map(|item| usize::try_from(item.id).ok())
            .max();
        self.item_id_index = max_id.map_or_else(Vec::new, |max| vec![None; max + 1]);
        for (index, item) in self.items.iter().enumerate() {
            let Ok(id) = usize::try_from(item.id) else {
                continue;
            };
            if self.item_id_index[id].is_none() {
                self.item_id_index[id] = Some(index);
            }
        }

        self.fixed_food_heals_index.clear();
        for (index, fact) in self.consumption.iter().enumerate() {
            let name = fact.item.name.as_str();
            if self.consumption[..index]
                .iter()
                .any(|prior| prior.item.name.eq_ignore_ascii_case(name))
            {
                continue;
            }
            let mut matching = self
                .consumption
                .iter()
                .filter(|other| other.item.name.eq_ignore_ascii_case(name));
            let Some(heal) = matching.next().and_then(ConsumptionFact::fixed_hp_heal) else {
                continue;
            };
            if matching.all(|other| other.fixed_hp_heal() == Some(heal)) {
                self.fixed_food_heals_index.push((name.to_string(), heal));
            }
        }
        self.fixed_food_heal_index.clear();
        for (index, (name, _)) in self.fixed_food_heals_index.iter().enumerate() {
            self.fixed_food_heal_index
                .entry(ascii_fold_hash(name))
                .or_default()
                .push(index);
        }
    }

    pub fn revision(&self) -> i32 {
        self.revision
    }

    pub fn cache_id(&self) -> &str {
        &self.provenance.cache_identity.cache_id
    }

    /// Decoded (`274DCI01`) content id pinned by the generator, when present.
    pub fn content_id(&self) -> Option<&str> {
        self.provenance.cache_identity.content_id.as_deref()
    }

    pub fn source_inputs(&self) -> impl Iterator<Item = (bool, &SourceInput)> {
        self.provenance
            .inputs
            .iter()
            .chain(&self.provenance.decoder_sources)
            .map(|input| (false, input))
            .chain(
                self.provenance
                    .content_inputs
                    .iter()
                    .map(|input| (true, input)),
            )
    }

    pub fn items(&self) -> &[GameItem] {
        &self.items
    }

    pub fn item_by_alias(&self, alias: &str) -> Option<&GameItem> {
        self.items
            .iter()
            .find(|item| item.alias.as_deref() == Some(alias))
    }

    /// Generated consumption rows that are qualified as a fixed hitpoint heal.
    pub fn fixed_food_heals(&self) -> impl Iterator<Item = (&str, i32)> {
        self.fixed_food_heals_index
            .iter()
            .map(|(name, heal)| (name.as_str(), *heal))
    }

    pub fn fixed_food_heal(&self, name: &str) -> Option<i32> {
        self.fixed_food_heal_index
            .get(&ascii_fold_hash(name))?
            .iter()
            .find_map(|index| {
                let (known, heal) = &self.fixed_food_heals_index[*index];
                known.eq_ignore_ascii_case(name).then_some(*heal)
            })
    }

    /// Required level for a generated pickpocket NPC display name.
    pub fn required_thieving(&self, name: &str) -> Option<i32> {
        self.pickpocket.iter().find_map(|fact| {
            fact.npcs
                .iter()
                .any(|npc| npc.name.eq_ignore_ascii_case(name))
                .then_some(fact.level)
        })
    }

    pub fn spells(&self) -> &[SpellFact] {
        &self.spells
    }

    pub fn staves(&self) -> &[StaffFact] {
        &self.staves
    }

    /// Packed choose/grid/toggle identities from the selected cache.
    pub fn autocast_controls(&self) -> Option<&AutocastControls> {
        self.autocast
            .as_ref()
            .filter(|controls| controls.available())
    }

    pub fn duel_controls(&self) -> Option<&DuelControls> {
        self.duel.as_ref().filter(|controls| controls.available())
    }

    pub fn special_controls(&self) -> Option<&SpecialControls> {
        self.special
            .as_ref()
            .filter(|controls| controls.available())
    }

    pub fn special_cost(&self, weapon_name: &str) -> Option<i32> {
        self.special_controls()?.cost(weapon_name)
    }

    pub fn special_bar(&self, combat_tab_root: i32) -> i32 {
        self.special_controls()
            .map(|controls| controls.bar_for_root(combat_tab_root))
            .unwrap_or(-1)
    }

    pub fn teleports(&self) -> &[TeleportSpell] {
        &self.teleports
    }

    pub fn herbs(&self) -> &[HerbFact] {
        &self.herbs
    }

    pub fn drop_tables(&self) -> &[DropTable] {
        &self.drop_tables
    }

    pub fn drop_table(&self, name: &str) -> Option<&DropTable> {
        let wanted = name.trim();
        self.drop_tables
            .iter()
            .find(|row| row.name.eq_ignore_ascii_case(wanted))
    }

    pub fn prayers(&self) -> &[PrayerFact] {
        &self.prayers
    }

    pub fn prayer_by_name(&self, name: &str) -> Option<&PrayerFact> {
        let wanted = name.trim();
        self.prayers
            .iter()
            .find(|row| row.name.eq_ignore_ascii_case(wanted))
    }

    pub fn nurmof_essence(&self) -> Option<&NurmofEssenceFacts> {
        self.nurmof_essence.as_ref()
    }

    pub fn flour_six(&self) -> Option<&FlourSixFacts> {
        self.flour_six.as_ref()
    }

    pub fn equipment_names(&self) -> Option<&EquipmentNamesFacts> {
        self.equipment_names.as_ref()
    }

    /// Gather methods and loc-resource ids. `None` is family absence, not an empty extract.
    pub fn gather_methods(&self) -> Option<&GatherMethodsFacts> {
        self.gather_methods.as_ref()
    }

    /// Published-wood world placements. `None` is family absence, not an empty extract.
    pub fn gather_placements(&self) -> Option<&GatherPlacementsFacts> {
        self.gather_placements.as_ref()
    }

    /// Six-seed quest identity. `None` is family absence, not an empty extract.
    pub fn quest_identity(&self) -> Option<&QuestIdentityFacts> {
        self.quest_identity.as_ref()
    }

    /// Trail inventory and challenge answers. `None` is family absence, not an empty extract.
    pub fn trails(&self) -> Option<&TrailFacts> {
        self.trails.as_ref()
    }

    /// Talk steps and key keepers. `None` is family absence, not an empty extract.
    pub fn talk_key(&self) -> Option<&TalkKeyFacts> {
        self.talk_key.as_ref()
    }

    pub fn trio_givers(&self) -> Option<&TrioGiverFacts> {
        self.trio_givers.as_ref()
    }

    /// Resolved equipment family row by frozen display name, when present.
    pub fn equipment_name(
        &self,
        family: &str,
        requested_name: &str,
    ) -> Option<&EquipmentNameEntry> {
        self.equipment_names()?.entry(family, requested_name)
    }

    /// Mapsquare predicate from generated facts. `None` when `nurmof_essence` is absent.
    pub fn in_essence_mine(&self, x: i32, z: i32) -> Option<bool> {
        let region = &self.nurmof_essence.as_ref()?.essence_region;
        Some((x >> 6) == region.mapsquare_mx && (z >> 6) == region.mapsquare_mz)
    }

    pub fn herb_level_default(&self) -> Option<i32> {
        self.herb_level_default
    }

    pub fn bank_placements(&self) -> Option<&crate::named_banks::BankPlacementFacts> {
        self.bank_placements.as_ref()
    }

    /// Every cook surface placed in the selected map pack, in the frozen
    /// generator's order (level, x, z); empty when not generated.
    pub fn cook_surfaces(&self) -> &[crate::cook_locations::CookSurface] {
        self.cook_surfaces
            .as_ref()
            .map_or(&[], |facts| facts.rows.as_slice())
    }

    pub fn herb_by_key(&self, key: &str) -> Option<&HerbFact> {
        let wanted = key.trim().to_ascii_lowercase();
        self.herbs
            .iter()
            .find(|herb| herb.key.eq_ignore_ascii_case(&wanted))
    }

    pub fn teleport(&self, name: &str) -> Option<&TeleportSpell> {
        let wanted = teleport_key(name);
        if wanted.is_empty() {
            return None;
        }
        self.teleports.iter().find(|spell| {
            spell.available()
                && (teleport_key(&spell.name) == wanted
                    || spell.spell.eq_ignore_ascii_case(&wanted)
                    || spell
                        .spell
                        .strip_suffix("_teleport")
                        .is_some_and(|dest| dest.eq_ignore_ascii_case(&wanted)))
        })
    }

    pub fn spell(&self, name: &str) -> Option<&SpellFact> {
        self.spells
            .iter()
            .find(|spell| spell.name.eq_ignore_ascii_case(name.trim()))
    }

    /// Staff display name → rune display names it provides.
    pub fn staff_runes_table(&self) -> Vec<(&str, Vec<&str>)> {
        self.staves
            .iter()
            .map(|staff| {
                (
                    staff.name.as_str(),
                    staff.runes.iter().map(|rune| rune.name.as_str()).collect(),
                )
            })
            .collect()
    }

    fn provided_runes(&self, wielded: &[impl AsRef<str>]) -> HashSet<String> {
        let mut provided = HashSet::new();
        for item in wielded {
            let wanted = item.as_ref().trim();
            if let Some(staff) = self
                .staves
                .iter()
                .find(|staff| staff.name.eq_ignore_ascii_case(wanted))
            {
                for rune in &staff.runes {
                    provided.insert(rune.name.to_ascii_lowercase());
                }
            }
        }
        provided
    }

    /// Remaining per-cast rune costs after staff substitution.
    /// `None` is an unknown spell; `Some([])` means no remaining cost.
    pub fn runes_per_cast(
        &self,
        spell_name: &str,
        wielded: &[impl AsRef<str>],
    ) -> Option<Vec<RemainingRuneCost>> {
        let spell = self.spell(spell_name)?;
        let provided = self.provided_runes(wielded);
        Some(
            spell
                .runes
                .iter()
                .filter(|rune| !provided.contains(&rune.name.to_ascii_lowercase()))
                .map(|rune| RemainingRuneCost {
                    rune: rune.name.clone(),
                    count: rune.count,
                })
                .collect(),
        )
    }

    /// Staff-spell grid component for a known autocast spell, otherwise -1.
    /// Posted selected-cache `spell_grid_base` wins over the frozen 1830 audit.
    pub fn spell_button_com(&self, spell_name: &str) -> i32 {
        let base = self
            .autocast
            .as_ref()
            .map(|controls| controls.spell_grid_base)
            .unwrap_or(STAFF_SPELLS_COM0);
        self.spell(spell_name)
            .map(|spell| base + spell.ssb)
            .unwrap_or(-1)
    }

    pub fn item_by_id(&self, id: i32) -> Option<&GameItem> {
        let index = self
            .item_id_index
            .get(usize::try_from(id).ok()?)
            .copied()
            .flatten()?;
        self.items.get(index)
    }

    /// Bounded slot search over generated wearpos facts. Empty query still
    /// returns only a capped page so the editor never serializes the table.
    pub fn search_slot_items(&self, slot: &str, query: &str, limit: usize) -> Vec<ItemSearchHit> {
        let Some(pos) = wearpos_for_loadout_slot(slot) else {
            return Vec::new();
        };
        search_items(
            self.items.iter().filter(|item| {
                !item.is_certificate() && item.wear_position == pos && item.name.is_some()
            }),
            query,
            limit,
        )
    }

    /// Bounded name search for supply rows. Requires a query so the full
    /// object table is not copied into the UI.
    pub fn search_named_items(&self, query: &str, limit: usize) -> Vec<ItemSearchHit> {
        if query.trim().is_empty() {
            return Vec::new();
        }
        search_items(
            self.items
                .iter()
                .filter(|item| !item.is_certificate() && item.name.is_some()),
            query,
            limit,
        )
    }
}

/// One filtered item row for native editors. `alias` and `id` distinguish
/// equal display names without guessing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemSearchHit {
    pub name: String,
    pub alias: String,
    pub id: i32,
}

impl GameItem {
    /// Primary loadout slot from generated `wear_position`, or none for
    /// appearance-only wearpos values (arms/head/jaw) and unequippable rows.
    pub fn loadout_slot(&self) -> Option<&'static str> {
        loadout_slot_for_wearpos(self.wear_position)
    }

    pub fn is_two_handed(&self) -> bool {
        self.wear_position == WEARPOS_RIGHTHAND && self.wear_position_2 == WEARPOS_LEFTHAND
    }

    pub fn is_certificate(&self) -> bool {
        self.certificate_template != -1
    }
}

/// Content `wearpos` ids from the selected ObjType decoder (`getWearPosId`).
pub const WEARPOS_HAT: i32 = 0;
pub const WEARPOS_BACK: i32 = 1;
pub const WEARPOS_FRONT: i32 = 2;
pub const WEARPOS_RIGHTHAND: i32 = 3;
pub const WEARPOS_TORSO: i32 = 4;
pub const WEARPOS_LEFTHAND: i32 = 5;
pub const WEARPOS_LEGS: i32 = 7;
pub const WEARPOS_HANDS: i32 = 9;
pub const WEARPOS_FEET: i32 = 10;
pub const WEARPOS_RING: i32 = 12;
pub const WEARPOS_QUIVER: i32 = 13;

/// Map a generated wearpos onto the loadout slot names used by callers.
/// Appearance slots 6/8/11 (arms/head/jaw) stay unmapped.
pub fn loadout_slot_for_wearpos(pos: i32) -> Option<&'static str> {
    match pos {
        WEARPOS_HAT => Some("hat"),
        WEARPOS_BACK => Some("back"),
        WEARPOS_FRONT => Some("front"),
        WEARPOS_RIGHTHAND => Some("righthand"),
        WEARPOS_TORSO => Some("torso"),
        WEARPOS_LEFTHAND => Some("lefthand"),
        WEARPOS_LEGS => Some("legs"),
        WEARPOS_HANDS => Some("hands"),
        WEARPOS_FEET => Some("feet"),
        WEARPOS_RING => Some("ring"),
        WEARPOS_QUIVER => Some("quiver"),
        _ => None,
    }
}

pub fn wearpos_for_loadout_slot(slot: &str) -> Option<i32> {
    match slot {
        "hat" => Some(WEARPOS_HAT),
        "back" => Some(WEARPOS_BACK),
        "front" => Some(WEARPOS_FRONT),
        "righthand" => Some(WEARPOS_RIGHTHAND),
        "torso" => Some(WEARPOS_TORSO),
        "lefthand" => Some(WEARPOS_LEFTHAND),
        "legs" => Some(WEARPOS_LEGS),
        "hands" => Some(WEARPOS_HANDS),
        "feet" => Some(WEARPOS_FEET),
        "ring" => Some(WEARPOS_RING),
        "quiver" => Some(WEARPOS_QUIVER),
        _ => None,
    }
}

fn teleport_key(value: &str) -> String {
    let stripped = strip_color_tags(value);
    let joined = stripped.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut lower = joined.trim().to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("cast ") {
        lower = rest.trim().to_string();
    }
    if let Some(rest) = lower.strip_suffix(" teleport") {
        lower = rest.trim().to_string();
    }
    lower
}

fn strip_color_tags(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::with_capacity(value.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' {
            if let Some(rel) = chars[i + 1..].iter().position(|&c| c == '@') {
                i += rel + 2;
                out.push(' ');
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn search_items<'a, I>(items: I, query: &str, limit: usize) -> Vec<ItemSearchHit>
where
    I: Iterator<Item = &'a GameItem>,
{
    let q = query.trim().to_ascii_lowercase();
    items
        .filter(|item| {
            if q.is_empty() {
                return true;
            }
            let name = item.name.as_deref().unwrap_or("");
            name.to_ascii_lowercase().contains(&q)
                || item
                    .alias
                    .as_deref()
                    .is_some_and(|alias| alias.to_ascii_lowercase().contains(&q))
                || item.id.to_string() == q
        })
        .take(limit)
        .map(|item| ItemSearchHit {
            name: item.name.clone().unwrap_or_default(),
            alias: item.alias.clone().unwrap_or_default(),
            id: item.id,
        })
        .collect()
}

/// Load one revision exactly once and share it for the process lifetime.
pub fn for_revision(revision: ClientRevision) -> Result<Arc<SelectedGameData>, String> {
    let (cell, bytes) = match revision {
        ClientRevision::R274 => (&DATA_274, REVISION_274),
        ClientRevision::R289 => (&DATA_289, REVISION_289),
    };
    match cell.get_or_init(|| SelectedGameData::decode(bytes, revision)) {
        Ok(data) => Ok(Arc::clone(data)),
        Err(error) => Err(error.clone()),
    }
}

fn accepts_cache_id(data: &SelectedGameData, cache_id: &str) -> bool {
    if data.cache_id() == cache_id {
        return true;
    }
    data.content_id() == Some(cache_id)
}

/// Load the revision static and require the immutable profile's selected cache
/// identity: either the generator's pinned packed transfer id or the decoded
/// (`274DCI01`) content id proven in the generated provenance. Other caches
/// remain honestly metadata-free.
pub fn for_profile(
    revision: ClientRevision,
    cache_id: &str,
) -> Result<Arc<SelectedGameData>, String> {
    let data = for_revision(revision)?;
    if !accepts_cache_id(&data, cache_id) {
        return Err(format!(
            "generated game data cache identity mismatch: expected {}, got {cache_id}",
            data.cache_id()
        ));
    }
    Ok(data)
}

/// Return selected facts only when this profile uses the generated asset's
/// audited cache identity, or an audited fact-equivalent identity for the
/// same revision. Other caches remain honestly metadata-free.
pub fn for_optional_profile(
    revision: ClientRevision,
    cache_id: &str,
) -> Result<Option<Arc<SelectedGameData>>, String> {
    let data = for_revision(revision)?;
    Ok(accepts_cache_id(&data, cache_id).then_some(data))
}
#[cfg(test)]
#[path = "game_data_tests.rs"]
mod tests;
