//! One engagement's immutable intent and compact, observed outcome.
use api::gather_methods::SceneRegionInput;
use api::quest_progress::EvidenceStamp;
pub use api::snapshot::ActorKind;
use api::WorldTile;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ActorRef {
    pub kind: ActorKind,
    pub index: u16,
}
impl ActorRef {
    pub fn matches(self, other: api::snapshot::ActorTargetView) -> bool {
        self.kind == other.kind && usize::from(self.index) == other.index
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Melee,
    Ranged,
    Mage,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum MeleeMode {
    Accurate,
    Aggressive,
    Defensive,
    Controlled,
}
impl MeleeMode {
    pub(crate) const fn xp_mask(self) -> u8 {
        match self {
            Self::Accurate => 1,
            Self::Aggressive => 2,
            Self::Defensive => 4,
            Self::Controlled => 7,
        }
    }
    pub(crate) const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Accurate),
            1 => Some(Self::Aggressive),
            2 => Some(Self::Defensive),
            3 => Some(Self::Controlled),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum RangedMode {
    Accurate,
    #[default]
    Rapid,
    LongRange,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PrayerMode {
    #[default]
    Hold,
    Flick,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tactic {
    Open,
    Safespot,
    LureCorner,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Nearest,
    Random,
    LowestHealth,
}
#[derive(Debug, Clone)]
pub enum Target {
    Npc {
        types: Arc<[i32]>,
        pick: Pick,
        not_targeting_others: bool,
    },
    Attacker {
        npcs: bool,
        players: bool,
    },
    Player {
        name: Arc<str>,
    },
}
/// Resolved kit rows, not a loadout resolver. The caller applies the overlay
/// and selected-content resolution once before constructing this shared DTO.
#[derive(Debug, Clone, Default)]
pub struct CompiledKit {
    pub worn: Arc<[(u8, i32)]>,
    pub carry: Arc<[(i32, u32)]>,
}
#[derive(Debug, Clone)]
pub struct SpellRef {
    pub alias: Arc<str>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Allowances {
    pub prayer: bool,
    pub food: bool,
    pub potions: bool,
    pub equipment: bool,
    pub retaliate_toggle: bool,
}
impl Default for Allowances {
    fn default() -> Self {
        Self {
            prayer: true,
            food: true,
            potions: true,
            equipment: true,
            retaliate_toggle: true,
        }
    }
}
#[derive(Debug, Clone)]
pub enum Fallback {
    Abort,
    Retreat { tile: WorldTile },
    FaceTank { kit: Option<Arc<CompiledKit>> },
    Fight,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntruderRule {
    Ignore,
    FightBack,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntruderPolicy {
    pub player: IntruderRule,
    pub npc: IntruderRule,
}
impl Default for IntruderPolicy {
    fn default() -> Self {
        Self {
            player: IntruderRule::FightBack,
            npc: IntruderRule::Ignore,
        }
    }
}
#[derive(Debug, Clone)]
pub struct CombatRequest {
    pub target: Target,
    pub tactic: Tactic,
    pub style: Style,
    pub melee_mode: Option<MeleeMode>,
    pub ranged_style: RangedMode,
    pub kit: Option<Arc<CompiledKit>>,
    pub spells: Option<Arc<[SpellRef]>>,
    pub stand: Option<WorldTile>,
    pub search_bounds: Option<Arc<[SceneRegionInput]>>,
    pub engage_radius: u8,
    pub lost_radius: u8,
    pub budget_ticks: u16,
    pub allow: Allowances,
    pub fallback: Fallback,
    pub intruder: IntruderPolicy,
    pub retaliate: bool,
    pub prayer_mode: PrayerMode,
    pub until_ticks: u16,
}
impl Default for CombatRequest {
    fn default() -> Self {
        Self {
            target: Target::Attacker {
                npcs: true,
                players: true,
            },
            tactic: Tactic::Open,
            style: Style::Melee,
            kit: None,
            spells: None,
            melee_mode: None,
            ranged_style: RangedMode::default(),
            stand: None,
            search_bounds: None,
            engage_radius: 12,
            lost_radius: 20,
            budget_ticks: 1500,
            allow: Allowances::default(),
            fallback: Fallback::Abort,
            intruder: IntruderPolicy::default(),
            retaliate: true,
            prayer_mode: PrayerMode::Hold,
            until_ticks: 0,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Unprotected {
    NoFood,
    Dragonfire,
    NoAmmo,
    NoRunes,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PrepItem {
    Weapon,
    Shield,
    Ammo,
    Staff,
    Arm,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AbortReason {
    Unprotected(Unprotected),
    Unattackable,
    SafespotBroken,
    Retreated,
    RetreatFailed,
    PrepFailed(PrepItem),
    Unresponsive,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CombatEnd {
    Killed,
    TargetGone,
    NoTarget,
    Budget,
    Died,
    Aborted(AbortReason),
}
#[derive(Debug, Clone, Copy, Serialize)]
pub struct CombatReport {
    pub end: CombatEnd,
    #[serde(serialize_with = "serialize_evidence")]
    pub evidence: EvidenceStamp,
    pub engaged: Option<ActorRef>,
    pub engaged_npc_type: i32,
    pub ticks: u16,
    pub swings: u16,
    pub casts: u16,
    pub damage_taken: u16,
    pub food: u8,
    pub prayer_doses: u8,
    pub boost_doses: u8,
    pub antifire_doses: u8,
    pub hits_while_protected: u8,
    pub protect_switches: u8,
    pub intruders: u8,
    pub ammo_pickups: u8,
    pub restorations: u8,
    pub locked_ticks: u8,
    pub multi_op_plans: u8,
    pub melee_mode_fallback: Option<MeleeMode>,
    pub flick_resets: u8,
    pub flick_misses: u8,
    pub flick_fallback: bool,
}
const _: () = assert!(std::mem::size_of::<CombatReport>() <= 80);

fn serialize_evidence<S: serde::Serializer>(
    stamp: &EvidenceStamp,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeStruct;
    let mut value = serializer.serialize_struct("Evidence", 2)?;
    value.serialize_field("tick", &stamp.tick)?;
    value.serialize_field("sequence", &stamp.sequence)?;
    value.end()
}
