//! Immutable generated game facts selected by the caller's client revision.

use std::collections::{HashMap, HashSet};
use std::io::{BufReader, Read};
use std::sync::Arc;
#[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
use std::sync::OnceLock;

use client::io::ClientRevision;
#[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
use flate2::read::GzDecoder;
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};

use crate::selected::{FactError, LOCAL_GAME_DATA_FAMILY};

const SCHEMA_VERSION: u16 = 5;
pub const NPC_COMBAT_FACTS_VERSION: u32 = 2;

/// An error while loading or validating selected game data.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GameDataError {
    /// The NPC combat-fact family predates this API's expected version.
    #[error("combat facts are missing or outdated; regenerate the selected game data")]
    CombatFacts,
    /// This build embeds no local selected game data.
    #[error("embedded game data is unavailable; generate local assets before building")]
    LocalDataUnavailable,
    /// A caller-supplied payload is internally inconsistent.
    #[error("{0}")]
    InvalidData(Arc<str>),
    /// A revision's selected-data JSON failed to decode.
    #[error("{message}")]
    Decode {
        /// The revision whose payload failed.
        revision: ClientRevision,
        /// The exact decoder message.
        message: Arc<str>,
    },
    /// A revision's prayer rows do not fit this API's prayer layout.
    #[error("{detail}")]
    PrayerLayout {
        /// The revision whose prayer rows failed validation.
        revision: ClientRevision,
        /// The number of prayer rows the payload carries.
        rows: usize,
        /// The number of rows this API supports.
        supported: usize,
        /// The exact mismatch, naming the row when one varp is out of sequence.
        detail: Arc<str>,
    },
}
#[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
const REVISION_274: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/game-data-274.json.gz"));
#[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
const REVISION_289: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/game-data-289.json.gz"));
#[cfg(all(
    feature = "embedded-game-data",
    not(no_local_game_data),
    not(no_local_game_data_377)
))]
const REVISION_377: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/game-data-377.json.gz"));

#[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
static DATA_274: OnceLock<Result<Arc<SelectedGameData>, GameDataError>> = OnceLock::new();
#[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
static DATA_289: OnceLock<Result<Arc<SelectedGameData>, GameDataError>> = OnceLock::new();
#[cfg(all(
    feature = "embedded-game-data",
    not(no_local_game_data),
    not(no_local_game_data_377)
))]
static DATA_377: OnceLock<Result<Arc<SelectedGameData>, GameDataError>> = OnceLock::new();

/// Whether this build embeds the local revision-274/289 game-data assets.
pub const fn local_game_data_available() -> bool {
    cfg!(all(feature = "embedded-game-data", not(no_local_game_data)))
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
    /// Whether the object has a male worn model (`manwear != -1`).
    pub worn_model: bool,
    /// Selected content category assigned to this item; absent when the object is uncategorized.
    #[serde(default)]
    pub category: Option<String>,
    /// The engine's decoded trade flag: `tradeable=no`, a nonzero
    /// `dummyitem`, or the note of an untradeable item clears it.
    pub tradeable: bool,
    /// A pile-size model another obj names in `countobj`: it repeats the
    /// base item's name and is not a separate item.
    pub stack_variant: bool,
    /// True when the obj carries `param=destroy_death,^true`.
    ///
    /// Content: `player/configs/death.param:1-3` (`type=int`, `default=0`);
    /// 19 rows carry it (18 in `quests/quest_viking/configs/viking.obj`, 1 in
    /// `quests/quest_desertrescue/configs/desertrescue.obj`); read by the death
    /// keep at `player/scripts/death.rs2:198,211`. Added for PvP V0
    /// (PVP-DESIGN-1026 §3.3), first consumed by §8.2 (V6). Defaults to
    /// `false` for generated data written before V0.
    #[serde(default)]
    pub destroy_death: bool,
}

/// One selected object referenced by a combat fact.
#[derive(Debug, Deserialize, Clone)]
pub struct NamedFact {
    pub alias: String,
    pub id: i32,
    pub name: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct StatChange {
    pub stat: String,
    pub base: i32,
    pub percent: i32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct StatHeal {
    pub stat: String,
    pub base: i32,
    pub percent: i32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ConsumptionFact {
    pub item: NamedFact,
    pub source_row: String,
    pub source_file: String,
    #[serde(default)]
    pub effect: String,
    /// Nullable eat-delay argument passed by the selected consume.rs2 handler.
    #[serde(default)]
    pub eat_delay_arg: Option<i32>,
    /// Nullable skill/action-delay argument passed by the selected consume.rs2 handler.
    #[serde(default)]
    pub skill_delay_arg: Option<i32>,
    /// Optional consume-message `p_delay` argument; absent source data means no delay call.
    #[serde(default)]
    pub message_delay: Option<i32>,
    #[serde(default)]
    pub stat_change: Vec<StatChange>,
    #[serde(default)]
    pub stat_heal: Vec<StatHeal>,
    #[serde(default)]
    pub heal_energy: Vec<i32>,
    pub qualification: String,
    #[serde(default)]
    pub next_stage: Option<String>,
    #[serde(default)]
    pub dose_family: Option<String>,
    #[serde(default)]
    pub dose_count: Option<u8>,
}

impl ConsumptionFact {
    /// Returns this row's fixed hitpoint heal when its canonical qualification is valid.
    pub fn fixed_hp_heal(&self) -> Option<i32> {
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

/// One selected pickpocket spawn target.
///
/// [`NamedFact`] is the shared item/object reference shape; this row also
/// carries the NPC config's own respawn window so a caller waits the selected
/// content's rate instead of a fixed constant.
#[derive(Debug, Deserialize)]
struct PickpocketNpc {
    id: i32,
    name: String,
    /// `NpcType.respawnrate` in game ticks. The generator writes the engine
    /// default 100 when the config declares none (`NpcType.ts:105`).
    respawn_ticks: u32,
}

#[derive(Debug, Deserialize)]
struct PickpocketFact {
    npcs: Vec<PickpocketNpc>,
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
    pub spell_text_component: i32,
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
            && self.spell_text_component >= 0
            && self.spell_panel_root >= 0
            && self.choose_com >= 0
            && self.toggle_com >= 0
            && self.spell_grid_base >= 0
            && self.magic_varp >= 0
    }
}

/// Packed Duel Arena modal roots, offer containers, rule toggle and controls.
///
/// `rule_buttons` holds the 14 rule-toggle components of
/// `minigames/game_duelarena/scripts/duel_arena_settings.rs2:1-85`, indexed
/// by the bit values of `minigames/game_duelarena/configs/duelarena.constant:1-14`
/// (`NoForfeit = 0` through `NoSpecs = 13`). Added for PvP V0
/// (PVP-DESIGN-1026 §3.3, §7.6).
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
    pub select_mine: i32,
    pub select_theirs: i32,
    pub confirm_mine: i32,
    pub confirm_theirs: i32,
    pub obstacles: i32,
    pub options_varp: i32,
    /// Rule-toggle component per duel-option bit (`rule_buttons[bit]`).
    /// Missing in generated data written before PvP V0; then `rule_button`
    /// reports `None`.
    #[serde(default)]
    pub rule_buttons: [i32; 14],
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
            && self.select_mine >= 0
            && self.select_theirs >= 0
            && self.confirm_mine >= 0
            && self.confirm_theirs >= 0
            && self.obstacles >= 0
            && self.options_varp >= 0
    }

    /// Rule-toggle component for one duel-option bit (0–13), or `None`
    /// when the bit is out of range or the generated data predates PvP V0.
    ///
    /// Bits follow `minigames/game_duelarena/configs/duelarena.constant:1-14`.
    pub fn rule_button(self, bit: u8) -> Option<i32> {
        self.rule_buttons
            .get(usize::from(bit))
            .copied()
            .filter(|id| *id > 0)
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

/// PvP V0 facts (PVP-DESIGN-1026 §3.3, §6.1).
///
/// One row per obj with `param=specwep,^true` (14 rows: the 12 weapons with a
/// `sa_energy` cost plus the dragon battleaxe and Excalibur, which fall back
/// to the 1000 default of `skill_combat/configs/combat.param:183-185`).
/// Generated by `tools/game-data/extractors/combat.ts` from the obj params
/// and the per-weapon scripts cited on each effect; consumed from V4 (specials).
/// Combat-tab codes follow `combat_tabs` (0-15); `style` uses the
/// `weapon_styles` bit (`1` melee, `2` ranged).
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct SpecialWeaponFact {
    /// Selected obj id of the special weapon.
    pub obj_id: i32,
    /// `sa_energy` param, default 1000
    /// (`skill_combat/configs/combat.param:183-185`).
    pub energy: u16,
    /// Combat-tab code whose `specbar` is clicked
    /// (`skill_combat/scripts/player/specwep.rs2:51-115`).
    pub tab: u8,
    /// Weapon style bit (`1` melee, `2` ranged).
    pub style: u8,
    /// Typed effect from the §6.1 table; never an obj-id dispatch.
    pub effect: SpecialEffect,
    /// Ammo the special consumes from the quiver (0 for melee).
    pub needs: Needs,
    /// What the local player shows when the special executes (§6.3 reads it).
    pub attribution: Attribution,
}

/// Typed special-attack effect (PVP-DESIGN-1026 §3.3, Codex N12).
///
/// Every distinction of the §6.1 effect table is a variant, so an implementer
/// consumes the fact and never dispatches on an obj id.
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SpecialEffect {
    /// One or two rolled hits on the target.
    Damage {
        /// Hit count (the dagger's second hit lands one tick later).
        hits: u8,
        /// Max-hit formula of each hit.
        max: MaxFormula,
        /// Hit-roll rule.
        accuracy: AccuracyRule,
        /// Timing beyond the ordinary attack delay.
        delay: ExtraDelay,
    },
    /// No damage; the target is pushed and script-locked, and our
    /// interaction is cleared (dragon spear).
    Shove {
        /// Script lock in ticks (`.p_delay(4)`).
        lock_ticks: u8,
        /// True: `p_stopaction` runs first, so a restoring `Attack` is owed.
        clears_interaction: bool,
    },
    /// Click-time stat effect, no attack; the bar never shows `true`
    /// (dragon battleaxe, Excalibur).
    Instant {
        /// Which instant special this weapon runs.
        effect: InstantEffect,
    },
}

/// Max-hit formula of one special-attack hit (PVP-DESIGN-1026 §3.3, §6.1).
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MaxFormula {
    /// `⌊num · M / den⌋` with `M` our ordinary melee max (dagger 115/100,
    /// longsword 125/100, mace 150/100, halberd 110/100, claws 110/100).
    MeleeScaled {
        /// Scale numerator.
        num: u16,
        /// Scale denominator.
        den: u16,
    },
    /// `combat_maxhit(combat_stat(ranged + level_plus, ammo.rangebonus))`:
    /// the ranged special basis, from the ranged level plus ten with the
    /// ammo's `rangebonus` only.
    RangedSpecialBasis {
        /// Ranged-level bonus (10 for every ranged special).
        level_plus: u8,
    },
}

/// Hit-roll rule of a special attack (PVP-DESIGN-1026 §3.3, §6.1).
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AccuracyRule {
    /// Ordinary melee roll with the attack roll scaled `num/den` against the
    /// named damage type; `side_targets` is the halberd's multi-only side
    /// roll (attack × 75/100 at the same max).
    MeleeScaled {
        /// Attack-roll numerator.
        num: u16,
        /// Attack-roll denominator.
        den: u16,
        /// Side-target roll for the halberd in multi-way combat, if any.
        side_targets: Option<SideRoll>,
    },
    /// Ordinary ranged roll scaled `num/den` (shortbow 10/7).
    RangedScaled {
        /// Attack-roll numerator.
        num: u16,
        /// Attack-roll denominator.
        den: u16,
    },
    /// Ordinary ranged roll, unscaled (thrownaxe).
    Ranged,
    /// No hit roll: the damage always rolls (magic longbow).
    AlwaysHits,
}

/// Side-target attack-roll scale for the dragon halberd in multi-way combat.
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct SideRoll {
    /// Attack-roll numerator (75 for the halberd's side targets).
    pub num: u16,
    /// Attack-roll denominator (100).
    pub den: u16,
}

/// Timing beyond the ordinary attack delay (PVP-DESIGN-1026 §3.3, §6.1).
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExtraDelay {
    /// No extra delay.
    None,
    /// Our next decision is one tick later (claws:
    /// `%action_delay = add(%action_delay, 1)`).
    NextDecisionPlusOne,
    /// The second hit is decided one tick after the first and reads the
    /// protect when it rolls (dagger).
    SecondHitNextTick,
}

/// Click-time instant special (PVP-DESIGN-1026 §3.3, §6.1).
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstantEffect {
    /// Dragon battleaxe (`specwep.rs2:61-79`).
    DragonBattleaxe,
    /// Excalibur (`specwep.rs2:89-105`).
    Excalibur,
}

/// Ammo a special attack consumes (PVP-DESIGN-1026 §3.3).
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct Needs {
    /// Accepted ammo units consumed: 0 for melee, 1 for the longbow and the
    /// thrownaxe, 2 for the shortbow.
    pub ammo: u8,
}

/// What the local player shows when the special executes
/// (PVP-DESIGN-1026 §3.3, §6.1 "Observable attribution" column).
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct Attribution {
    /// Attack animation id, when the script plays one.
    pub anim: Option<i32>,
    /// Attacker spotanim id, when the script plays one on us.
    pub spotanim: Option<i32>,
    /// Projectile spotanim id, when the script fires one.
    pub projectile: Option<i32>,
}

/// Complete signed combat bonuses of one wearable obj (§4.4).
///
/// Every wearable obj has one row. Strength/range stay in their existing width;
/// the ten signed bonuses are aggregated per object (C
/// `scripts/player/scripts/equip.rs2:239-252` in 274, `:268-281` in 289).
/// Missing params use selected `combat.param` defaults (C
/// `scripts/skill_combat/configs/combat.param:1-68`).
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct ItemCombatBonus {
    /// Selected obj id.
    pub obj_id: i32,
    /// `strengthbonus` param, or its content-declared default.
    pub strengthbonus: i32,
    /// `rangebonus` param, or its content-declared default.
    pub rangebonus: i32,
    pub stabattack: i16,
    pub slashattack: i16,
    pub crushattack: i16,
    pub magicattack: i16,
    pub rangeattack: i16,
    pub stabdefence: i16,
    pub slashdefence: i16,
    pub crushdefence: i16,
    pub magicdefence: i16,
    pub rangedefence: i16,
}

/// Melee attack tier of one wearable obj (PVP-DESIGN-1026 §4.2, V0).
///
/// Content: the `levelrequire` scripts (`scripts/levelrequire/scripts/*.rs2`,
/// for example `tier1.rs2:8`, `tier5.rs2:8`, `tier40.rs2:8`, `tier60.rs2:2-9`).
/// An obj with no tier script is tier 0
/// (`skill_combat/configs/combat.param:81-83`), never excluded. A quest-gated
/// row (`levelrequire_zanaris_quest_attack`, `levelrequire_heroes_quest_attack`,
/// `levelrequire_regicide_quest_attack_strength`) is `quest_gated` and is
/// unusable unless the quest varp is set, never a plain tier. Halberds also
/// gate Strength via `levelrequire_attack_and_strength`
/// (`scripts/levelrequire/scripts/levelrequire.rs2:186-192`, for example
/// `tier60.rs2:10` needs attack 60 and strength 30); V2 reads `strength`.
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct MeleeWeaponTier {
    /// Selected obj id.
    pub obj_id: i32,
    /// Attack tier (the `levelrequire_attack` level).
    pub tier: u8,
    /// True when the wield gate needs a quest varp besides the level.
    pub quest_gated: bool,
    /// Required Strength level from `levelrequire_attack_and_strength`
    /// (0 when the gate checks Attack only).
    #[serde(default)]
    pub strength: u8,
}

/// One inclusive duel-area rectangle on level 0 (PVP-DESIGN-1026 §3.2, V0).
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct DuelAreaRect {
    /// Minimum x (west edge).
    pub x1: i32,
    /// Minimum z (south edge).
    pub z1: i32,
    /// Maximum x (east edge).
    pub x2: i32,
    /// Maximum z (north edge).
    pub z2: i32,
}

/// One duel pen: a fight or obstacle arena rectangle.
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct DuelPenFact {
    /// Minimum x (west edge).
    pub x1: i32,
    /// Minimum z (south edge).
    pub z1: i32,
    /// Maximum x (east edge).
    pub x2: i32,
    /// Maximum z (north edge).
    pub z2: i32,
    /// True for the three `duel_arena_obstacle_fight_zones` pens.
    pub obstacles: bool,
}

/// Duel zone constants from the content dbrows (PVP-DESIGN-1026 §3.2, V0).
///
/// Content: the six `coord_pair` rows of
/// `minigames/game_duelarena/configs/duelarena.dbrow:1-11` (three
/// `duel_arena_fight_zones`, three `duel_arena_obstacle_fight_zones`),
/// the challenge row `:13-15`, and the lobby telejump of
/// `minigames/game_duelarena/scripts/duel_arena_finish.rs2:75`.
#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct DuelZoneFacts {
    /// Six pens in dbrow order (fights first, then obstacles).
    pub pens: Vec<DuelPenFact>,
    /// Challenge area (not a pen).
    pub challenge: DuelAreaRect,
    /// Lobby area (post-fight telejump target).
    pub lobby: DuelAreaRect,
}

/// One generated per-cast rune cost.
#[derive(Debug, Deserialize, Clone)]
pub struct SpellRune {
    pub alias: String,
    pub id: i32,
    pub name: String,
    pub count: i32,
}

/// Named combat spell from the selected cache: the 16 autocast-selectable staff
/// chooser spells plus manual-only damage spells (Crumble Undead, Iban Blast,
/// god strikes). `ssb` is the staff_spells grid index for selectable spells and
/// -1 otherwise; `component_id` is the manual magic-tab widget for UseWidgetOn.
/// `impact_spotanim` is the selected `spotanim_target` cache id, -1 when unknown;
/// the shared miss splash is a separate `failed_spell_impact` scalar, never a per-spell impact.
#[derive(Debug, Deserialize)]
pub struct SpellFact {
    pub name: String,
    pub source_row: String,
    pub ssb: i32,
    pub component_id: i32,
    pub autocast_selectable: bool,
    pub level: i32,
    pub continue_by_autocast: bool,
    #[serde(default)]
    pub spellcom: String,
    #[serde(default)]
    pub maxhit: i32,
    #[serde(default)]
    pub members: bool,
    #[serde(default)]
    pub wornrequired: Option<String>,
    #[serde(default)]
    pub worn_reqmessage: Option<String>,
    pub impact_spotanim: i32,
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
}

/// Mapsquare predicate for the enclosed Rune Essence mine (m45_75).
#[derive(Debug, Deserialize, Clone)]
pub struct EssenceRegionFact {
    pub mapsquare_mx: i32,
    pub mapsquare_mz: i32,
    pub predicate: String,
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
    pub return_anchor_role: String,
}

/// Nurmof pickaxe shop + essence region facts (no acquire routing).
#[derive(Debug, Deserialize, Clone)]
pub struct NurmofEssenceFacts {
    pub npc_alias: String,
    pub npc_id: i32,
    pub npc_name: String,
    pub shop_inv: String,
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
}

#[derive(Debug, Deserialize, Clone)]
pub struct FlourTileFact {
    pub x: i32,
    pub z: i32,
    pub plane: i32,
    #[serde(default)]
    pub role: Option<String>,
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

/// Six Murder Mystery flour facts: quest name, pots, barrel, and three tiles.
#[derive(Debug, Deserialize, Clone)]
pub struct FlourSixFacts {
    pub quest_name: String,
    pub pot: FlourItemFact,
    pub pot_flour: FlourItemFact,
    pub flour_barrel: FlourLocFact,
    pub flour_barrel_object_tile: FlourTileFact,
    pub flour_barrel_approach_tile: FlourTileFact,
    pub bank_tile: FlourTileFact,
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

/// Whether the progress varp is sent by content updates or remains local.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Transmission {
    PostedByContent,
    NotTransmitted,
    #[default]
    Unknown,
}

const QUEST_IDENTITY_UNKNOWN_SIDES: &[&str] = &[
    "progress_operand",
    "varp_pack_id",
    "completion_operand",
    "quest_points",
    "requirements",
];
/// One seed join. `id` is the seed id; `complete` is one constant, not a range.
/// `unknown_sides` names unproved aspects: `progress_operand`, `varp_pack_id`,
/// `completion_operand`, `quest_points`, or `requirements`. Enum index is not a field.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct QuestIdentityRow {
    pub id: String,
    pub component: String,
    pub display: String,
    pub varp: String,
    pub varp_id: i32,
    pub complete: i32,
    #[serde(default)]
    pub colour_authoritative: bool,
    #[serde(default)]
    pub stage_domain: Option<crate::selected::InclusiveRange>,
    #[serde(default)]
    pub transmission: Transmission,
    pub quest_points: i32,
    pub unknown_sides: Vec<String>,
    pub requirements: QuestRequirements,
    #[serde(default)]
    pub members: bool,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub journal_title: Option<String>,
    #[serde(default)]
    pub journal_script: Option<String>,
}
/// A content-script-derived quest start target. Rows are intentionally sparse:
/// no row means the map marker remains the generic `Quest Start` label.
#[derive(Debug, Deserialize, Clone)]
pub struct QuestStartRow {
    pub quest: String,
    pub target: QuestStartTarget,
    pub op: u8,
}

#[derive(Debug, Deserialize, Clone)]
pub struct QuestStartTarget {
    pub kind: String,
    pub id: i32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct QuestStartFacts {
    pub schema: u16,
    pub revision: i32,
    pub rows: Vec<QuestStartRow>,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NpcAttackKind {
    Ranged,
    Magic,
    Mixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragonfireKind {
    Elvarg,
    Chromatic,
    Metal,
    Other,
}

impl DragonfireKind {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "elvarg" => Some(Self::Elvarg),
            "chromatic" => Some(Self::Chromatic),
            "metal" => Some(Self::Metal),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

fn deserialize_dragonfire<'de, D>(deserializer: D) -> Result<Option<DragonfireKind>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct DragonfireVisitor;
    impl<'de> serde::de::Visitor<'de> for DragonfireVisitor {
        type Value = Option<DragonfireKind>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a dragonfire kind, null, or a legacy boolean")
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_none<E>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
            Ok(value.then_some(DragonfireKind::Other))
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            DragonfireKind::parse(value)
                .map(Some)
                .ok_or_else(|| E::custom(format!("unknown dragonfire kind {value}")))
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            self.visit_str(&value)
        }
    }
    deserializer.deserialize_any(DragonfireVisitor)
}

#[derive(Debug, Deserialize, Clone, Copy)]
pub struct StyleSequenceFact {
    pub seq_id: i32,
    pub style: u8,
}

#[derive(Debug, Deserialize, Clone)]
pub struct StyleSpotanimFact {
    pub spotanim_id: i32,
    pub style: u8,
    #[serde(rename = "where")]
    pub location: String,
}

#[derive(Debug, Deserialize, Clone, Copy)]
pub struct CombatTabFact {
    pub tab: u8,
    pub root_id: i32,
}

/// One source-joined melee attack-style button.
///
/// `slot` is the attackstyle index passed by `player_attackstyles.rs2`;
/// `mode` is the semantic `combat_damagestyles.constant` code for that slot.
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct MeleeModeFact {
    pub tab: u8,
    pub slot: u8,
    pub mode: u8,
    pub button: i32,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RangedAmmoFamily {
    Arrow,
    OgreArrow,
    Bolt,
    Thrown,
    Javelin,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct RangedWeaponFact {
    pub obj_id: i32,
    pub attackrange: u8,
    pub levelrequire: u8,
    pub ammo_family: RangedAmmoFamily,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct RangedAmmoFact {
    pub obj_id: i32,
    pub levelrequire: u8,
    pub family: RangedAmmoFamily,
}

/// One source-joined ranged attack-style button (`mode`: accurate 0, rapid 1, long-range 2).
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct RangedModeFact {
    pub tab: u8,
    pub slot: u8,
    pub mode: u8,
    pub button: i32,
}

/// One selected-content style slot per weapon, preserving table order. Damage
/// type/style codes come from C `combat_damagetypes.constant:1-6` and
/// `combat_damagestyles.constant:1-9`, with slot order from the selected
/// `combat_style_table` (C `combat.dbtable:1-3`, `combat.dbrow`); `style_bonus`
/// is attack/strength/defence from C `scripts/skill_combat/scripts/combat.rs2:51-61`.
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct WeaponStyleFact {
    pub obj_id: i32,
    pub style: u8,
    pub attackrate: u8,
    pub category: u8,
    #[serde(default)]
    pub tab: Option<u8>,
    pub style_slot: u8,
    pub damagetype: u8,
    pub damagestyle: u8,
    pub style_bonus: (u8, u8, u8),
}

fn deserialize_required_max_dealt<'de, D>(deserializer: D) -> Result<Option<i32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    i32::deserialize(deserializer).map(Some)
}

/// A source-proven NPC awakening that changes which combat facts apply.
///
/// This metadata affects combat interpretation only. It does not rename the
/// selected NPC kind or change its navigation geometry.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NpcCombatTransform {
    /// The NPC awakens as another selected NPC type.
    Known { npc_id: i32 },
    /// The content indicates an awakening but its combat target is unsupported.
    Unknown,
}

/// Pack-joined NPC identity for Path compile (`npc:cook`).
#[derive(Debug, Deserialize, Clone)]
pub struct NpcNameRow {
    pub id: i32,
    pub config: String,
    pub display: Option<String>,
    pub ops: Vec<String>,
    pub size: i32,
    /// Decoded wander range, including the supported engines' five-tile default.
    pub wanderrange: i32,
    /// Decoded tether range. Revision 274 defaults to seven; revision 289 derives
    /// an absent value as `wanderrange + 2` and clamps it to at least `wanderrange`.
    pub maxrange: i32,
    pub attackrange: i32,
    pub huntrange: i32,
    pub vislevel: i32,
    pub hitpoints: i32,
    /// NPC levels are `NpcType.stats` order attack/defence/strength/hitpoints/
    /// ranged/magic; defaults are E `NpcType.ts:107` (274) and `:106` (289).
    pub attack: i32,
    pub defence: i32,
    pub magic: i32,
    /// Selected damage-type constant. Omitted NPC params use the declared
    /// `npc_combat.param:1-3` default, resolved through `combat_damagetypes.constant`.
    pub damagetype: Option<String>,
    /// Selected-content NPC headicon **sprite index**, not a bitmask (unlike
    /// `PlayerView::headicons`): 3 protect melee, 4 protect missiles,
    /// 5 protect magic, 8 protect missiles + magic. Absent content is unknown.
    /// In 289 the icon is visual only; no server script grants the matching
    /// damage immunity.
    #[serde(default)]
    pub headicon: Option<i32>,
    #[serde(default)]
    pub strength: Option<i32>,
    #[serde(default)]
    pub ranged: Option<i32>,
    /// Signed attack/defence params read by C
    /// `scripts/skill_combat/scripts/npc/npc_combat.rs2:315-343`; absent values
    /// use selected `combat.param` defaults (C `combat.param:16-48,56-58`).
    pub attackbonus: i32,
    pub rangeattack: i32,
    pub magicattack: i32,
    pub stabdefence: i32,
    pub slashdefence: i32,
    pub crushdefence: i32,
    pub magicdefence: i32,
    pub rangedefence: i32,
    #[serde(default)]
    pub strengthbonus: Option<i32>,
    #[serde(default)]
    pub rangebonus: Option<i32>,
    #[serde(default)]
    pub undead: Option<i32>,
    #[serde(default)]
    pub ap_attack: bool,
    #[serde(default)]
    pub attack_kind: Option<NpcAttackKind>,
    #[serde(default)]
    pub forced_max_hit: Option<i32>,
    #[serde(default, deserialize_with = "deserialize_dragonfire")]
    pub dragonfire: Option<DragonfireKind>,
    /// NPC damage cap from `npc_param(max_dealt)` (C `npc_combat.rs2:391-428`);
    /// the selected `combat.param:159-162` default is 1000.
    #[serde(deserialize_with = "deserialize_required_max_dealt")]
    pub max_dealt: Option<i32>,
    #[serde(default)]
    pub attackrate: Option<i32>,
    #[serde(default)]
    pub bespoke: bool,
    /// True when an attack body reads `~check_protect_prayer` and will switch
    /// onto whatever protect we are not using. Combat holds one protect.
    #[serde(default)]
    pub counter_protect: bool,
    /// Source poison severity before conversion to per-tick damage. Absent means
    /// that this selected row carries no explicit poison severity fact.
    #[serde(default)]
    pub poison_severity: Option<i32>,
    /// Selected `npc_forcemulti` content parameter.
    #[serde(default)]
    pub forcemulti: bool,
    /// Source-proven awakening target for combat facts; absent when no
    /// awakening is known. `Unknown` fails closed during risk assessment.
    #[serde(default)]
    pub combat_transform: Option<NpcCombatTransform>,
}

/// The family version is checked before strict selected-data decoding.
#[derive(Debug, Deserialize, Clone)]
pub struct NpcNameFacts {
    #[serde(default)]
    pub combat_facts_version: u32,
    pub rows: Vec<NpcNameRow>,
}

#[derive(Deserialize)]
struct CombatVersionProbe {
    #[serde(default)]
    npc_names: Option<NpcCombatVersionProbe>,
}

#[derive(Deserialize)]
struct NpcCombatVersionProbe {
    #[serde(default)]
    combat_facts_version: u32,
    rows: IgnoredAny,
}

fn validate_combat_facts_version(version: Option<u32>) -> Result<(), GameDataError> {
    if version == Some(NPC_COMBAT_FACTS_VERSION) {
        Ok(())
    } else {
        Err(GameDataError::CombatFacts)
    }
}

fn preflight_combat_facts_version(reader: impl Read) -> Result<(), GameDataError> {
    let probe: CombatVersionProbe = serde_json::from_reader(reader).map_err(|error| {
        GameDataError::InvalidData(Arc::from(format!("generated game data decode: {error}")))
    })?;
    validate_combat_facts_version(probe.npc_names.map(|facts| {
        let _ = facts.rows;
        facts.combat_facts_version
    }))
}

/// Pack-joined loc identity for Path compile (`loc:hopper_lumbridge`).
#[derive(Debug, Deserialize, Clone)]
pub struct LocNameRow {
    pub id: i32,
    pub config: String,
    pub display: Option<String>,
    pub ops: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LocNameFacts {
    pub rows: Vec<LocNameRow>,
}

/// Salvaged jm2 NPC placements.
#[derive(Debug, Deserialize, Clone)]
pub struct NpcPlacementRow {
    pub npc_id: i32,
    pub x: i32,
    pub z: i32,
    pub plane: i32,
    pub mapsquare: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct NpcPlacementFacts {
    pub rows: Vec<NpcPlacementRow>,
}
/// Selected-content facts used by the Karamja banana-plantation recovery.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct KaramjaSpawn {
    pub config: String,
    pub x: i32,
    pub z: i32,
    pub plane: i32,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct KaramjaDialogue {
    pub employment: String,
    pub paid: String,
    pub incomplete: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct KaramjaFacts {
    pub luthas_spawn: KaramjaSpawn,
    pub crate_spawn: KaramjaSpawn,
    pub banana_tree_configs: Vec<String>,
    pub banana_tree_spawns: Vec<KaramjaSpawn>,
    pub crate_capacity: i32,
    pub coin_payout: i32,
    pub dialogue: KaramjaDialogue,
}

impl KaramjaSpawn {
    fn valid(&self) -> bool {
        !self.config.is_empty() && self.x >= 0 && self.z >= 0 && (0..4).contains(&self.plane)
    }
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

/// Roster identity. Absence is `None`, not an empty list.
/// A coverage record is a sibling, not a row.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
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

/// A content state that refuses a gather method while it holds. Wire names
/// are the extractor's `forbidden_states` strings.
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum GatherForbiddenState {
    /// The memberfish guard `~mm_wearing_greegree = true`: refused while the
    /// player wears a greegree (`mm_greegree` category).
    MonkeyForm,
}

/// One generated row of the selected core's `gather_resources` slice, carried so
/// the UI never decodes the gathering family. `key` is the selectable setting
/// value: the resource key for woodcutting/mining or the stable grouped key
/// for fishing. `methods` contains every catalog method admitted by the row;
/// `aliases` keeps pre-group fishing ids resolvable. A row with
/// `selectable == false` carries the admission `gap` code.
#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GatherResourceOption {
    pub skill: String,
    /// First method id, retained as the representative method for consumers
    /// that need one (e.g. resource-level rows).
    pub method: String,
    pub methods: Vec<String>,
    pub aliases: Vec<String>,
    pub key: String,
    pub resources: Vec<String>,
    pub label: String,
    #[serde(default)]
    pub level: u16,
    pub selectable: bool,
    #[serde(default)]
    pub gap: Option<String>,
    /// Content states that refuse this row's methods while they hold.
    #[serde(default)]
    pub forbidden_states: Vec<GatherForbiddenState>,
}

/// A build-time named gathering camp. Its box selects a resource anchor;
/// runtime gathering still uses the operator's radius around that anchor.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct GatherSiteOption {
    pub id: String,
    pub skill: String,
    pub label: String,
    pub region: crate::gather_methods::SceneRegionInput,
    pub keys: Vec<GatherSiteKey>,
    /// Walk requirements the Gatherer already enforces through nav. The label
    /// names them; no gate reads this list. Empty when nothing gates the site.
    #[serde(default)]
    pub requires: Vec<GatherSiteRequirement>,
    /// User-facing reason that Bank mode cannot use this site. Other dispositions remain available.
    #[serde(default)]
    pub banking_unavailable: Option<String>,
}

/// One requirement a published gather site names. `kind` selects the variant.
#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GatherSiteRequirement {
    /// A skill at or above `level` (`stat(skill) >= level`).
    Skill { skill: String, level: u16 },
    /// A quest that must be complete.
    Quest { name: String },
    /// An item that must be worn (`inv_total(worn, item) > 0`), named for the label.
    Worn { item: i32, name: String },
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GatherSiteKey {
    pub key: String,
    pub count: usize,
}

/// Source-proven identities for generic, quest-completion, and chapter scrolls and book continuation.
/// Chat message/objbox pages continue through the snapshot's chat controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DialogueUiIds {
    pub scroll_root: i32,
    pub quest_scroll_root: i32,
    /// Chapter-page main root. `None` only when the revision's interface pack
    /// has no `inter_199` row: that revision publishes no chapter pages at all,
    /// so no id here means "chapter page" (never a guessed one).
    pub chapter_root: Option<i32>,
    pub book_root: i32,
    pub book_forward: i32,
    pub book_close: i32,
    pub book_forward_marker: i32,
}

/// Generated chapter identity: an interface id, or the explicit `null` that
/// says this revision publishes no chapter page. The generated key itself is
/// required; an absent key is a decode error, never a silent `None`.
enum ChapterRootWire {
    Id(i32),
    Absent,
}

impl<'de> Deserialize<'de> for ChapterRootWire {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Visitor;

        struct WireVisitor;

        impl<'de> Visitor<'de> for WireVisitor {
            type Value = ChapterRootWire;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a dialogue interface id or null")
            }

            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(ChapterRootWire::Absent)
            }

            fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(ChapterRootWire::Absent)
            }

            fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                deserializer.deserialize_any(WireVisitor)
            }

            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
                i32::try_from(value)
                    .map(ChapterRootWire::Id)
                    .map_err(|_| E::custom("dialogue interface id is out of range"))
            }

            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
                i32::try_from(value)
                    .map(ChapterRootWire::Id)
                    .map_err(|_| E::custom("dialogue interface id is out of range"))
            }
        }

        // `deserialize_any` is also what serde's missing-field deserializer
        // reports, so a generated object that omits the key fails to decode.
        deserializer.deserialize_any(WireVisitor)
    }
}

impl<'de> Deserialize<'de> for DialogueUiIds {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            scroll_root: i32,
            quest_scroll_root: i32,
            chapter_root: ChapterRootWire,
            book_root: i32,
            book_forward: i32,
            book_close: i32,
            book_forward_marker: i32,
        }

        let wire = Wire::deserialize(deserializer)?;
        Ok(DialogueUiIds {
            scroll_root: wire.scroll_root,
            quest_scroll_root: wire.quest_scroll_root,
            chapter_root: match wire.chapter_root {
                ChapterRootWire::Id(id) => Some(id),
                ChapterRootWire::Absent => None,
            },
            book_root: wire.book_root,
            book_forward: wire.book_forward,
            book_close: wire.book_close,
            book_forward_marker: wire.book_forward_marker,
        })
    }
}

impl DialogueUiIds {
    fn available(&self) -> bool {
        let required = [
            self.scroll_root,
            self.quest_scroll_root,
            self.book_root,
            self.book_forward,
            self.book_close,
            self.book_forward_marker,
        ];
        required
            .iter()
            .enumerate()
            .all(|(index, id)| *id > 0 && !required[..index].contains(id))
            && self
                .chapter_root
                .is_none_or(|chapter| chapter > 0 && !required.contains(&chapter))
    }
}

/// Generated immutable facts for one client/cache revision.
#[derive(Debug, Deserialize)]
pub struct SelectedGameData {
    schema_version: u16,
    revision: i32,
    items: Vec<GameItem>,
    consumption: Vec<ConsumptionFact>,
    pickpocket: Vec<PickpocketFact>,
    #[serde(default)]
    spells: Vec<SpellFact>,
    #[serde(default)]
    staves: Vec<StaffFact>,
    #[serde(default)]
    failed_spell_impact: Option<i32>,
    #[serde(default)]
    autocast: Option<AutocastControls>,
    #[serde(default)]
    duel: Option<DuelControls>,
    #[serde(default)]
    special: Option<SpecialControls>,
    #[serde(default)]
    dialogue_ui: Option<DialogueUiIds>,
    #[serde(default, rename = "server_ordered")]
    server_ordered_wire: Option<crate::interact::server_ordered::CatalogWire>,
    #[serde(skip)]
    server_ordered: Option<crate::interact::server_ordered::ServerOrderedCatalog>,
    #[serde(skip)]
    server_ordered_status: Option<Result<(), crate::interact::server_ordered::ContractError>>,
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
    quest_identity: Option<QuestIdentityFacts>,
    #[serde(default)]
    quest_starts: Option<QuestStartFacts>,
    #[serde(default)]
    npc_names: Option<NpcNameFacts>,
    #[serde(default)]
    style_seqs: Vec<StyleSequenceFact>,
    #[serde(default)]
    style_spotanims: Vec<StyleSpotanimFact>,
    #[serde(default)]
    combat_tabs: Vec<CombatTabFact>,
    #[serde(default)]
    weapon_styles: Vec<WeaponStyleFact>,
    /// PvP V0 special-attack facts, one row per `specwep` obj (§3.3, §6.1).
    #[serde(default)]
    special_weapons: Vec<SpecialWeaponFact>,
    /// PvP V0 worn combat bonuses, one row per obj with a `strengthbonus`
    /// or `rangebonus` param (§3.3).
    #[serde(default)]
    item_combat_bonus: Vec<ItemCombatBonus>,
    /// PvP V0 melee attack tiers from the `levelrequire` scripts (§4.2).
    #[serde(default)]
    melee_weapon_tiers: Vec<MeleeWeaponTier>,
    /// PvP V0 duel zone constants from the content dbrows (§3.2).
    #[serde(default)]
    duel_zones: Option<DuelZoneFacts>,
    #[serde(default)]
    melee_modes: Vec<MeleeModeFact>,
    #[serde(default)]
    melee_mode_varp: Option<i32>,
    #[serde(default)]
    ranged_weapons: Vec<RangedWeaponFact>,
    #[serde(default)]
    ranged_ammo: Vec<RangedAmmoFact>,
    #[serde(default)]
    ranged_modes: Vec<RangedModeFact>,
    #[serde(default)]
    ranged_mode_varp: Option<i32>,
    #[serde(default)]
    loc_names: Option<LocNameFacts>,
    #[serde(default)]
    npc_placements: Option<NpcPlacementFacts>,
    #[serde(default)]
    karamja: Option<KaramjaFacts>,
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
    #[serde(default)]
    mining_hazards: Option<Vec<crate::gather_methods::MiningHazard>>,
    #[serde(default)]
    gather_resources: Vec<GatherResourceOption>,
    #[serde(default)]
    gather_sites: Vec<GatherSiteOption>,
    #[serde(default)]
    shops: Option<Vec<crate::shop_facts::ShopRecord>>,
    #[serde(skip)]
    item_id_index: Vec<Option<usize>>,
    #[serde(skip)]
    fixed_food_heals_index: Vec<(i32, String, i32)>,
    #[serde(skip)]
    fixed_food_heal_index: HashMap<u64, Vec<usize>>,
    #[serde(skip)]
    selected_revision: Option<ClientRevision>,
}

impl SelectedGameData {
    /// Validate the selected NPC combat family before any combat table consumes it.
    pub fn validate_combat_facts(&self) -> Result<(), GameDataError> {
        validate_combat_facts_version(
            self.npc_names
                .as_ref()
                .map(|facts| facts.combat_facts_version),
        )
    }

    /// Return NPC combat facts only when their version matches this API.
    pub fn npc_combat_facts(&self) -> Result<&NpcNameFacts, GameDataError> {
        self.validate_combat_facts()?;
        self.npc_names.as_ref().ok_or(GameDataError::CombatFacts)
    }
    pub(crate) fn selected_revision(&self) -> Option<ClientRevision> {
        self.selected_revision
    }

    /// Return the prepared server-ordered catalog without authenticating the
    /// local content file that supplied it.
    pub fn server_ordered_catalog(
        &self,
    ) -> Result<
        &crate::interact::server_ordered::ServerOrderedCatalog,
        crate::interact::server_ordered::ContractError,
    > {
        use crate::interact::server_ordered::ContractError;
        self.server_ordered_status
            .as_ref()
            .ok_or(ContractError::ContractUnavailable)?
            .as_ref()
            .map_err(|error| *error)?;
        self.server_ordered
            .as_ref()
            .ok_or(ContractError::ContractUnavailable)
    }

    /// Validate and prepare selected quest facts off-pump.
    pub fn prepare_quests(
        &self,
        worker: &mut crate::selected::FamilyPreparation,
    ) -> Result<Arc<crate::quest_facts::QuestCatalog>, crate::selected::FactError> {
        crate::quest_facts::prepare(self, worker)
    }

    /// Verify and prepare the selected prerequisite-step family off-pump.
    pub fn prepare_prereq_steps(
        &self,
        worker: &mut crate::selected::FamilyPreparation,
    ) -> Result<Arc<crate::prereq_steps::PrereqCatalog>, crate::selected::FactError> {
        crate::prereq_steps::prepare(self, worker)
    }

    /// Verify and decode the selected gathering family, or share the copy another user holds. Off-pump only.
    pub fn prepare_gathering(
        &self,
        worker: &mut crate::selected::FamilyPreparation,
    ) -> Result<Arc<crate::gather_methods::GatherCatalog>, crate::selected::FactError> {
        crate::gather_methods::prepare(self, worker)
    }

    /// Return the prepared quest catalog from cache. This never decodes or hashes.
    pub fn try_quests(&self) -> Option<Arc<crate::quest_facts::QuestCatalog>> {
        crate::quest_facts::cached(self)
    }

    /// Return the prepared prerequisite catalog without decoding or hashing.
    pub fn try_prereq_steps(&self) -> Option<Arc<crate::prereq_steps::PrereqCatalog>> {
        crate::prereq_steps::cached(self)
    }

    /// Cache-only: the prepared gathering catalog after its first process decode. Never decodes.
    pub fn try_gathering(&self) -> Option<Arc<crate::gather_methods::GatherCatalog>> {
        crate::gather_methods::cached(self)
    }

    fn decode(reader: impl Read, selected_revision: ClientRevision) -> Result<Arc<Self>, String> {
        let mut reader = BufReader::with_capacity(64 * 1024, reader);
        let mut data: Self = serde_json::from_reader(&mut reader)
            .map_err(|error| format!("generated game data decode: {error}"))?;
        data.validate_combat_facts()
            .map_err(|error| error.to_string())?;
        if data.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "generated game data schema mismatch: expected {SCHEMA_VERSION}, got {}",
                data.schema_version
            ));
        }
        if let Some(facts) = &data.karamja {
            let npc = data.npc_by_config(&facts.luthas_spawn.config);
            let crate_loc = data.loc_by_config(&facts.crate_spawn.config);
            if !facts.luthas_spawn.valid()
                || !facts.crate_spawn.valid()
                || facts.crate_capacity <= 0
                || facts.coin_payout <= 0
                || facts.banana_tree_configs.is_empty()
                || facts.banana_tree_configs.iter().any(|config| {
                    config.is_empty()
                        || data
                            .loc_by_config(config)
                            .is_none_or(|loc| loc.ops.is_empty())
                })
                || facts.banana_tree_spawns.is_empty()
                || facts.banana_tree_spawns.iter().any(|spawn| {
                    !spawn.valid()
                        || !facts.banana_tree_configs.contains(&spawn.config)
                        || data
                            .loc_by_config(&spawn.config)
                            .is_none_or(|loc| loc.ops.is_empty())
                })
                || facts.dialogue.employment.trim().is_empty()
                || facts.dialogue.paid.trim().is_empty()
                || facts.dialogue.incomplete.trim().is_empty()
                || npc.is_none_or(|row| {
                    row.display.as_deref().is_none_or(str::is_empty)
                        || !row.ops.iter().any(|op| op.eq_ignore_ascii_case("Talk-to"))
                })
                || crate_loc.is_none_or(|loc| loc.ops.is_empty())
            {
                return Err(
                    "karamja facts are incomplete or do not join selected content".to_string(),
                );
            }
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
            if facts.rows.iter().any(|row| {
                row.unknown_sides
                    .iter()
                    .any(|side| !QUEST_IDENTITY_UNKNOWN_SIDES.contains(&side.as_str()))
            }) {
                return Err(
                    "quest_identity contains an unsupported unknown_sides value".to_string()
                );
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
        if let Some(facts) = &data.quest_starts {
            if facts.schema != 1 {
                return Err("quest_starts schema mismatch".to_string());
            }
            if facts.rows.iter().any(|row| {
                row.quest.is_empty()
                    || row.op == 0
                    || row.op > 5
                    || row.target.id < 0
                    || !matches!(row.target.kind.as_str(), "npc" | "loc")
            }) {
                return Err("quest_starts contains an invalid target row".to_string());
            }
        }
        if let Some(facts) = &data.npc_names {
            if facts.rows.is_empty() {
                return Err("npc_names present with no rows".to_string());
            }
        }
        if let Some(facts) = &data.loc_names {
            if facts.rows.is_empty() {
                return Err("loc_names present with no rows".to_string());
            }
        }
        if let Some(facts) = &data.npc_placements {
            if facts.rows.is_empty() {
                return Err("npc_placements present with no rows".to_string());
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
        data.revision = selected_revision.as_i32();
        data.selected_revision = Some(selected_revision);
        data.build_indexes();
        let catalog = match data.server_ordered_wire.take() {
            Some(facts) => crate::interact::server_ordered::ServerOrderedCatalog::bind(
                facts,
                selected_revision,
            ),
            None => Err(crate::interact::server_ordered::ContractError::ContractUnavailable),
        };
        data.server_ordered_status = Some(match catalog {
            Ok(catalog) => {
                data.server_ordered = Some(catalog);
                Ok(())
            }
            Err(error) => Err(error),
        });
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
                self.fixed_food_heals_index
                    .push((fact.item.id, name.to_string(), heal));
            }
        }
        self.fixed_food_heal_index.clear();
        for (index, (_, name, _)) in self.fixed_food_heals_index.iter().enumerate() {
            self.fixed_food_heal_index
                .entry(ascii_fold_hash(name))
                .or_default()
                .push(index);
        }
    }

    pub fn revision(&self) -> i32 {
        self.revision
    }

    pub fn items(&self) -> &[GameItem] {
        &self.items
    }

    pub fn item_by_alias(&self, alias: &str) -> Option<&GameItem> {
        self.items
            .iter()
            .find(|item| item.alias.as_deref() == Some(alias))
    }
    /// Resolve an item key by exact alias, then by case-insensitive display
    /// name. Display-name collisions retain selected content order.
    pub fn resolve_item_name(&self, value: &str) -> Option<&GameItem> {
        if value.is_empty() {
            return None;
        }
        self.item_by_alias(value).or_else(|| {
            self.items.iter().find(|item| {
                item.name
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case(value))
            })
        })
    }

    /// Generated consumption rows, including source effect and next item stage.
    pub fn consumption_facts(&self) -> &[ConsumptionFact] {
        &self.consumption
    }

    pub fn consumable(&self, alias: &str) -> Option<&ConsumptionFact> {
        self.consumption
            .iter()
            .find(|fact| fact.item.alias == alias)
    }

    pub fn consume_next_stage(&self, alias: &str) -> Option<&str> {
        self.consumable(alias)?.next_stage.as_deref()
    }

    /// Generated consumption rows that are qualified as a fixed hitpoint heal.
    pub fn fixed_food_heals(&self) -> impl Iterator<Item = (&str, i32)> {
        self.fixed_food_heals_index
            .iter()
            .map(|(_, name, heal)| (name.as_str(), *heal))
    }

    /// Fixed hitpoint heals joined to the selected object id for native combat.
    pub fn fixed_food_heal_items(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.fixed_food_heals_index
            .iter()
            .map(|(id, _, heal)| (*id, *heal))
    }

    /// Fixed-heal food names for a settings choice list, best heal first
    /// (ties keep source order), matching the frozen list's best-first order.
    pub fn fixed_food_options_best_first(&self) -> Vec<&str> {
        let mut foods: Vec<(&str, i32)> = self.fixed_food_heals().collect();
        foods.sort_by_key(|&(_, heal)| std::cmp::Reverse(heal));
        foods.into_iter().map(|(name, _)| name).collect()
    }

    pub fn fixed_food_heal(&self, name: &str) -> Option<i32> {
        self.fixed_food_heal_index
            .get(&ascii_fold_hash(name))?
            .iter()
            .find_map(|index| {
                let (_, known, heal) = &self.fixed_food_heals_index[*index];
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

    /// Required Thieving level for a generated pickpocket NPC id.
    ///
    /// Conflicting selected rows are unknown: the caller must not choose a
    /// gate from a display name shared by unrelated NPCs.
    pub fn required_thieving_npc(&self, id: i32) -> Option<i32> {
        let mut facts = self
            .pickpocket
            .iter()
            .filter(|fact| fact.npcs.iter().any(|npc| npc.id == id));
        let level = facts.next()?.level;
        facts.all(|fact| fact.level == level).then_some(level)
    }

    /// Selected NPC config's respawn window, in game ticks, for a generated
    /// pickpocket NPC id.
    ///
    /// The generator writes the config's `respawnrate`, or the engine default
    /// 100 when the config declares none (`engine/src/cache/config/NpcType.ts:105`).
    /// Conflicting selected rows are unknown, so a caller never waits a value
    /// one row does not declare.
    pub fn thieving_npc_respawn_ticks(&self, id: i32) -> Option<u32> {
        let mut rates = self
            .pickpocket
            .iter()
            .flat_map(|fact| fact.npcs.iter())
            .filter(|npc| npc.id == id)
            .map(|npc| npc.respawn_ticks);
        let rate = rates.next()?;
        rates.all(|other| other == rate).then_some(rate)
    }

    pub fn spells(&self) -> &[SpellFact] {
        &self.spells
    }

    pub fn staves(&self) -> &[StaffFact] {
        &self.staves
    }

    /// Selected `failedspell_impact` cache id for the shared miss splash.
    /// `None` when the selected cache does not publish it. Per-spell
    /// `impact_spotanim` excludes this shared splash, so consumers can distinguish
    /// failed casts from successful spell impacts without a style-fact join.
    pub fn failed_spell_impact(&self) -> Option<i32> {
        self.failed_spell_impact
    }

    /// Packed choose/grid/toggle identities from the selected cache.
    pub fn autocast_controls(&self) -> Option<&AutocastControls> {
        self.autocast
            .as_ref()
            .filter(|controls| controls.available())
    }

    /// Minimal selected main-dialogue controls; no opt-in debug catalog is loaded.
    pub fn dialogue_ui(&self) -> Option<&DialogueUiIds> {
        self.dialogue_ui.as_ref().filter(|ids| ids.available())
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

    /// PvP V0 `special_button(tab)`: the `specbar` component of one combat
    /// tab (codes 0-15 per `combat_tabs`), or `None` when the tab has no bar.
    ///
    /// Content: one `[if_button,combat_<tab>:specbar] @toggle_sa;` per tab
    /// (`skill_combat/scripts/player/specwep.rs2:51-115`). Joins
    /// `combat_tabs` (tab to interface root) with `special.bars`
    /// (root to bar); the staff and scythe tabs have no bar.
    pub fn special_button(&self, tab: u8) -> Option<i32> {
        let root = self
            .combat_tabs
            .iter()
            .find(|row| row.tab == tab)
            .map(|row| row.root_id)?;
        let bar = self
            .special_controls()
            .map(|controls| controls.bar_for_root(root))?;
        (bar >= 0).then_some(bar)
    }

    /// PvP V0 special-attack facts, one row per `specwep` obj (§3.3, §6.1).
    pub fn special_weapons(&self) -> &[SpecialWeaponFact] {
        &self.special_weapons
    }

    /// PvP V0 special-attack fact for one selected obj id, or `None` when
    /// the obj has no special.
    pub fn special_weapon(&self, obj_id: i32) -> Option<SpecialWeaponFact> {
        self.special_weapons
            .iter()
            .find(|row| row.obj_id == obj_id)
            .copied()
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

    /// Roster identity. `None` is family absence, not an empty extract.
    pub fn quest_identity(&self) -> Option<&QuestIdentityFacts> {
        self.quest_identity.as_ref()
    }
    pub fn quest_starts(&self) -> Option<&QuestStartFacts> {
        self.quest_starts.as_ref()
    }

    pub fn npc_names(&self) -> Option<&NpcNameFacts> {
        self.npc_names.as_ref()
    }
    pub fn npc_name(&self, id: i32) -> Option<&NpcNameRow> {
        let rows = &self.npc_names.as_ref()?.rows;
        rows.binary_search_by_key(&id, |row| row.id)
            .ok()
            .map(|index| &rows[index])
    }

    pub fn style_seqs(&self) -> &[StyleSequenceFact] {
        &self.style_seqs
    }

    pub fn style_spotanims(&self) -> &[StyleSpotanimFact] {
        &self.style_spotanims
    }

    pub fn combat_tabs(&self) -> &[CombatTabFact] {
        &self.combat_tabs
    }

    pub fn weapon_styles(&self) -> &[WeaponStyleFact] {
        &self.weapon_styles
    }

    /// PvP V0 worn combat bonuses, one complete row per wearable obj with
    /// selected attack/defence and strength/range params or their defaults.
    pub fn item_combat_bonus_rows(&self) -> &[ItemCombatBonus] {
        &self.item_combat_bonus
    }

    /// PvP V0 worn combat bonus for one selected wearable obj, or `None` for a
    /// non-wearable or unknown obj id.
    pub fn item_combat_bonus(&self, obj_id: i32) -> Option<ItemCombatBonus> {
        self.item_combat_bonus
            .iter()
            .find(|row| row.obj_id == obj_id)
            .copied()
    }

    /// PvP V0 melee attack tiers from the `levelrequire` scripts (§4.2).
    pub fn melee_weapon_tier_rows(&self) -> &[MeleeWeaponTier] {
        &self.melee_weapon_tiers
    }

    /// PvP V0 melee attack tier for one selected obj id, or `None` when the
    /// obj is not a melee weapon. A `quest_gated` row is unusable unless the
    /// quest varp is set, never a plain tier.
    pub fn melee_weapon_tier(&self, obj_id: i32) -> Option<MeleeWeaponTier> {
        self.melee_weapon_tiers
            .iter()
            .find(|row| row.obj_id == obj_id)
            .copied()
    }

    /// PvP V0 duel zone constants from the content dbrows (§3.2), or `None`
    /// for generated data written before V0.
    pub fn duel_zones(&self) -> Option<&DuelZoneFacts> {
        self.duel_zones.as_ref()
    }

    pub fn melee_modes(&self) -> &[MeleeModeFact] {
        &self.melee_modes
    }

    pub fn melee_mode_varp(&self) -> Option<i32> {
        self.melee_mode_varp
    }

    pub fn ranged_weapons(&self) -> &[RangedWeaponFact] {
        &self.ranged_weapons
    }

    pub fn ranged_ammo(&self) -> &[RangedAmmoFact] {
        &self.ranged_ammo
    }

    pub fn ranged_modes(&self) -> &[RangedModeFact] {
        &self.ranged_modes
    }

    pub fn ranged_mode_varp(&self) -> Option<i32> {
        self.ranged_mode_varp
    }

    pub fn loc_names(&self) -> Option<&LocNameFacts> {
        self.loc_names.as_ref()
    }

    pub fn npc_placements(&self) -> Option<&NpcPlacementFacts> {
        self.npc_placements.as_ref()
    }

    pub fn npc_by_config(&self, config: &str) -> Option<&NpcNameRow> {
        self.npc_names
            .as_ref()?
            .rows
            .iter()
            .find(|row| row.config == config)
    }

    pub fn loc_by_config(&self, config: &str) -> Option<&LocNameRow> {
        self.loc_names
            .as_ref()?
            .rows
            .iter()
            .find(|row| row.config == config)
    }
    pub fn karamja(&self) -> Option<&KaramjaFacts> {
        self.karamja.as_ref()
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

    pub fn bank_placements(&self) -> Result<&crate::named_banks::BankPlacementFacts, FactError> {
        self.bank_placements
            .as_ref()
            .ok_or_else(|| FactError::FamilyUnavailable((*LOCAL_GAME_DATA_FAMILY).clone()))
    }

    /// Every cook surface placed in the selected map pack, in generator order.
    pub fn cook_surfaces(&self) -> Result<&[crate::cook_locations::CookSurface], FactError> {
        self.cook_surfaces
            .as_ref()
            .map(|facts| facts.rows.as_slice())
            .ok_or_else(|| FactError::FamilyUnavailable((*LOCAL_GAME_DATA_FAMILY).clone()))
    }
    pub(crate) fn shop_records(&self) -> Result<&[crate::shop_facts::ShopRecord], FactError> {
        self.shops
            .as_deref()
            .ok_or_else(|| FactError::FamilyUnavailable((*LOCAL_GAME_DATA_FAMILY).clone()))
    }

    /// The gathering family's hazard (gas) rock slice, carried by the core so it never needs the family
    /// decoded. `None` when not generated.
    pub fn mining_hazards(&self) -> Option<&[crate::gather_methods::MiningHazard]> {
        self.mining_hazards.as_deref()
    }

    /// Generated per-skill gather option rows in generator order; empty
    /// when not generated. The UI reads these, never the gathering family.
    pub fn gather_resources(&self) -> &[GatherResourceOption] {
        &self.gather_resources
    }

    /// Generated gather option rows for one skill (`woodcutting`, `mining`,
    /// `fishing`; trimmed, ASCII case-insensitive), in generator order.
    pub fn gather_resources_for<'a>(
        &'a self,
        skill: &'a str,
    ) -> impl Iterator<Item = &'a GatherResourceOption> {
        let wanted = skill.trim();
        self.gather_resources
            .iter()
            .filter(move |row| row.skill.eq_ignore_ascii_case(wanted))
    }

    /// One generated gather option by skill and selectable key or legacy alias
    /// (both trimmed, ASCII case-insensitive). Used by prepare to resolve a
    /// setting value to its method group.
    pub fn gather_option(&self, skill: &str, key: &str) -> Option<&GatherResourceOption> {
        let skill = skill.trim();
        let key = key.trim();
        self.gather_resources.iter().find(|row| {
            row.skill.eq_ignore_ascii_case(skill)
                && (row.key.eq_ignore_ascii_case(key)
                    || row
                        .aliases
                        .iter()
                        .any(|alias| alias.eq_ignore_ascii_case(key)))
        })
    }

    /// Named sites in generator order, without decoding the gathering family.
    pub fn gather_sites(&self) -> &[GatherSiteOption] {
        &self.gather_sites
    }

    pub fn gather_sites_for<'a>(
        &'a self,
        skill: &'a str,
    ) -> impl Iterator<Item = &'a GatherSiteOption> {
        let wanted = skill.trim();
        self.gather_sites
            .iter()
            .filter(move |row| row.skill.eq_ignore_ascii_case(wanted))
    }

    pub fn gather_site(&self, skill: &str, id: &str) -> Option<&GatherSiteOption> {
        let skill = skill.trim();
        let id = id.trim();
        self.gather_sites
            .iter()
            .find(|row| row.skill.eq_ignore_ascii_case(skill) && row.id.eq_ignore_ascii_case(id))
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

    /// Staff-spell grid component for a known autocast-selectable spell, otherwise -1.
    /// Manual-only spells resolve through their own `component_id`, never this grid.
    /// Posted selected-cache `spell_grid_base` wins over the frozen 1830 audit.
    pub fn spell_button_com(&self, spell_name: &str) -> i32 {
        let base = self
            .autocast
            .as_ref()
            .map(|controls| controls.spell_grid_base)
            .unwrap_or(STAFF_SPELLS_COM0);
        self.spell(spell_name)
            .filter(|spell| spell.autocast_selectable)
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

/// Decode selected game data from caller-provided local bytes.
pub fn decode_game_data(
    bytes: &[u8],
    revision: ClientRevision,
) -> Result<Arc<SelectedGameData>, GameDataError> {
    preflight_combat_facts_version(bytes)?;
    let data = SelectedGameData::decode(bytes, revision)
        .map_err(|message| decode_error(revision, message))?;
    validate_prayer_layout(&data, revision)?;
    Ok(data)
}
#[cfg(test)]
pub(crate) fn test_data(revision: ClientRevision) -> Option<Arc<SelectedGameData>> {
    let (path, description) = match revision {
        ClientRevision::R274 => (
            concat!(env!("CARGO_MANIFEST_DIR"), "/data/game-data/274.json"),
            "revision 274 game data for API tests",
        ),
        ClientRevision::R289 => (
            concat!(env!("CARGO_MANIFEST_DIR"), "/data/game-data/289.json"),
            "revision 289 game data for API tests",
        ),
        ClientRevision::R377 => (
            concat!(env!("CARGO_MANIFEST_DIR"), "/data/game-data/377.json"),
            "revision 377 game data for API tests",
        ),
    };
    crate::test_support::local_asset_path("", path, description)?;
    if !crate::test_support::embedded_game_data_available() {
        return None;
    }
    Some(for_revision(revision).expect("embedded selected game data"))
}

/// Decode quest identity facts from caller-provided local bytes.
pub fn decode_quest_identity(bytes: &[u8]) -> Result<(QuestIdentityFacts, u16), FactError> {
    crate::quest_facts::decode(bytes)
}

/// Load one revision exactly once and share it for the process lifetime.
#[cfg(all(feature = "embedded-game-data", not(no_local_game_data)))]
pub fn for_revision(revision: ClientRevision) -> Result<Arc<SelectedGameData>, GameDataError> {
    let (cell, bytes) = match revision {
        ClientRevision::R274 => (&DATA_274, REVISION_274),
        ClientRevision::R289 => (&DATA_289, REVISION_289),
        #[cfg(not(no_local_game_data_377))]
        ClientRevision::R377 => (&DATA_377, REVISION_377),
        #[cfg(no_local_game_data_377)]
        ClientRevision::R377 => return Err(GameDataError::LocalDataUnavailable),
    };
    match cell.get_or_init(|| {
        preflight_combat_facts_version(GzDecoder::new(bytes))?;
        let data = SelectedGameData::decode(GzDecoder::new(bytes), revision)
            .map_err(|message| decode_error(revision, message))?;
        validate_prayer_layout(&data, revision)?;
        Ok(data)
    }) {
        Ok(data) => Ok(Arc::clone(data)),
        Err(error) => Err(error.clone()),
    }
}

/// Local game data is absent in tool-only or toy-scale builds.
#[cfg(not(all(feature = "embedded-game-data", not(no_local_game_data))))]
pub fn for_revision(_revision: ClientRevision) -> Result<Arc<SelectedGameData>, GameDataError> {
    Err(GameDataError::LocalDataUnavailable)
}

fn decode_error(revision: ClientRevision, message: String) -> GameDataError {
    GameDataError::Decode {
        revision,
        message: Arc::from(message),
    }
}

fn validate_prayer_layout(
    data: &SelectedGameData,
    revision: ClientRevision,
) -> Result<(), GameDataError> {
    let rows = data.prayers.len();
    let supported = crate::prayer::PRAYER_COUNT;
    if rows > supported {
        return Err(GameDataError::PrayerLayout {
            revision,
            rows,
            supported,
            detail: Arc::from(format!(
                "generated game data prayer layout mismatch for revision {}: {} rows exceed the supported count {}",
                revision.as_i32(),
                rows,
                supported,
            )),
        });
    }
    for (index, prayer) in data.prayers.iter().enumerate() {
        let expected = crate::prayer::PRAYER_VARP0 + index as i32;
        if prayer.varp != expected {
            return Err(GameDataError::PrayerLayout {
                revision,
                rows,
                supported,
                detail: Arc::from(format!(
                    "generated game data prayer layout mismatch for revision {}: row {index} has varp {}, expected {expected}",
                    revision.as_i32(),
                    prayer.varp,
                )),
            });
        }
    }
    Ok(())
}
#[cfg(test)]
#[path = "game_data_tests.rs"]
mod tests;
#[cfg(all(
    test,
    not(all(feature = "embedded-game-data", not(no_local_game_data)))
))]
mod missing_data_tests {
    use super::*;

    #[test]
    fn unavailable_build_reports_missing_local_data() {
        assert!(!local_game_data_available());
        assert!(matches!(
            for_revision(ClientRevision::R289),
            Err(GameDataError::LocalDataUnavailable)
        ));
    }
}

#[cfg(all(
    test,
    feature = "embedded-game-data",
    not(no_local_game_data),
    no_local_game_data_377
))]
mod missing_377_data_tests {
    use super::*;

    #[test]
    fn absent_377_assets_preserve_embedded_legacy_revisions() {
        assert!(local_game_data_available());
        for revision in [ClientRevision::R274, ClientRevision::R289] {
            assert_eq!(
                for_revision(revision).unwrap().revision(),
                revision.as_i32()
            );
        }
        assert!(matches!(
            for_revision(ClientRevision::R377),
            Err(GameDataError::LocalDataUnavailable)
        ));
    }
}
