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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Style {
    Melee,
    Ranged,
    Mage,
}
impl Style {
    /// The three styles in their canonical order (also the `StyleKits` order).
    pub const ALL: [Self; 3] = [Self::Melee, Self::Ranged, Self::Mage];
    /// Dense index into a per-style array.
    pub const fn index(self) -> usize {
        self as usize
    }
    /// The `headicons` bit of the protection prayer that blocks this style:
    /// Protect from Melee, Missiles and Magic show as bits 3, 4 and 5
    /// (`headicon.constant:4-6`).
    pub const fn protect_bit(self) -> u8 {
        1 << (3 + self as u8)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
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
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
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
    /// The special-attack weapon of this style's loadout when it is not the
    /// kit's main right hand (PVP-DESIGN-1026 §6.2). Read from V4 on.
    pub special: Option<i32>,
}
/// One loadout per style (PVP-DESIGN-1026 §4.1). A style with no kit is never
/// a switch target; the opening style may still run without one (the machine
/// then picks its weapon from the pack by content facts, §4.2).
#[derive(Debug, Clone, Default)]
pub struct StyleKits {
    pub melee: Option<Arc<CompiledKit>>,
    pub ranged: Option<Arc<CompiledKit>>,
    pub mage: Option<Arc<CompiledKit>>,
}
impl StyleKits {
    pub fn get(&self, style: Style) -> Option<&Arc<CompiledKit>> {
        match style {
            Style::Melee => self.melee.as_ref(),
            Style::Ranged => self.ranged.as_ref(),
            Style::Mage => self.mage.as_ref(),
        }
    }
    /// The same kits with `kit` as the loadout of `style`.
    #[must_use]
    pub fn with(mut self, style: Style, kit: Arc<CompiledKit>) -> Self {
        match style {
            Style::Melee => self.melee = Some(kit),
            Style::Ranged => self.ranged = Some(kit),
            Style::Mage => self.mage = Some(kit),
        }
        self
    }
}
/// When the machine may change attack style mid-fight (PVP-DESIGN-1026 §4.1,
/// §4.3). A trigger only ever targets a style that has a kit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwitchPolicy {
    /// T2/T3: leave a style the opponent protects against, and return to
    /// `prefer[0]` once the opponent shows no protection.
    pub on_protect: bool,
    /// T1: switch instead of aborting when ammo or runes run out.
    pub on_exhaustion: bool,
    /// T4: leave melee when the opponent stays out of reach.
    pub on_reach: bool,
    /// The order tried: the opening style first by default, then the rest.
    pub prefer: [Style; 3],
}
impl SwitchPolicy {
    /// Every trigger on, `first` preferred, the other styles in canonical order.
    pub fn for_style(first: Style) -> Self {
        let mut prefer = [first; 3];
        let mut next = 1;
        for style in Style::ALL {
            if style != first {
                prefer[next] = style;
                next += 1;
            }
        }
        Self {
            on_protect: true,
            on_exhaustion: true,
            on_reach: true,
            prefer,
        }
    }
}
impl Default for SwitchPolicy {
    fn default() -> Self {
        Self::for_style(Style::Melee)
    }
}
#[derive(Debug, Clone)]
pub struct SpellRef {
    pub alias: Arc<str>,
}
/// What the machine may do on the bot's behalf. Every flag defaults to allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Allowances {
    pub prayer: bool,
    pub food: bool,
    pub potions: bool,
    pub equipment: bool,
    pub retaliate_toggle: bool,
    /// Wear positions the machine must never change, one bit per slot
    /// (`player/configs/equip.constant:2-15`): the duel rules `NoWeapons`,
    /// `NoArmour` and `NoJewelry` lock slots. A locked slot has no wanted item.
    pub locked_slots: u16,
    /// Whether a special attack may be used (false under the duel's `no_specs`).
    pub special: bool,
}
impl Default for Allowances {
    fn default() -> Self {
        Self {
            prayer: true,
            food: true,
            potions: true,
            equipment: true,
            retaliate_toggle: true,
            locked_slots: 0,
            special: true,
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
    /// One loadout per style; the opening style's kit is `kits.get(style)`.
    pub kits: StyleKits,
    pub switch: SwitchPolicy,
    /// An explicit order always uses manual casts, even with an armed staff.
    /// `None` selects the strongest castable spell (the 16-row chooser when
    /// a staff is desired, otherwise manual). Manual casts are five ticks apart.
    pub spells: Option<Arc<[SpellRef]>>,
    /// After no entry of a fixed manual order remains castable, permit the
    /// strongest castable unrejected spell. Defaults to false; this does not
    /// change the request's cast mode or bypass spell-specific refusals.
    pub fallback_spells: bool,
    /// Approach this fight spot before attacking. Failed approaches share a
    /// three-walk bound and stop at once for policy or quest refusals.
    pub stand: Option<WorldTile>,
    pub search_bounds: Option<Arc<[SceneRegionInput]>>,
    /// Named zone grants for this fight's approach, return, and escape routes.
    pub cross: Option<Arc<[Arc<str>]>>,
    pub engage_radius: u8,
    pub lost_radius: u8,
    pub budget_ticks: u16,
    pub allow: Allowances,
    pub fallback: Fallback,
    pub intruder: IntruderPolicy,
    pub retaliate: bool,
    pub prayer_mode: PrayerMode,
    pub until_ticks: u16,
    /// The script's opt-in to START a fight with a named player. The global
    /// operator setting `pvp.start_fights` is the other source; one predicate,
    /// [`super::select::pvp_initiation_allowed`], reads both at `begin` and only
    /// for [`Target::Player`]. Self-defence never reads this field.
    pub pvp: bool,
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
            kits: StyleKits::default(),
            switch: SwitchPolicy::default(),
            spells: None,
            fallback_spells: false,
            melee_mode: None,
            ranged_style: RangedMode::default(),
            stand: None,
            search_bounds: None,
            cross: None,
            engage_radius: 12,
            lost_radius: 20,
            budget_ticks: 1500,
            allow: Allowances::default(),
            fallback: Fallback::Abort,
            intruder: IntruderPolicy::default(),
            retaliate: true,
            prayer_mode: PrayerMode::Hold,
            until_ticks: 0,
            pvp: false,
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
pub enum PrepReadiness {
    CombatRootMissing,
    CombatRootWrong,
    StyleEchoMissing,
}
/// Why a player cannot be fought now (PVP-DESIGN-1026 §2.1). Carried by
/// `ActionError::PvpRefused` at `begin` and by [`AbortReason::PvpRefused`]
/// after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, thiserror::Error)]
pub enum PvpRefusal {
    /// `Target::Player` with neither `request.pvp` nor the global setting.
    #[error("pvp-not-allowed")]
    NotOptedIn,
    /// The player row has neither the "Attack" nor the "Fight" op.
    #[error("pvp-no-attack-op")]
    NoAttackOp,
    /// The bot stands outside the Wilderness and outside a duel pen.
    #[error("pvp-outside-wilderness")]
    OutsideWilderness,
    /// The opponent stands outside the Wilderness.
    #[error("pvp-opponent-outside")]
    OpponentOutside,
    /// The level difference exceeds the Wilderness level.
    #[error("pvp-level-range")]
    LevelRange {
        /// The lower of the two Wilderness levels.
        wilderness: u8,
        /// The absolute combat-level difference.
        difference: u8,
    },
    /// In the duel challenge area, not in a pen.
    #[error("pvp-challenge-area")]
    ChallengeArea,
    /// The arena refused: the player is not the duel partner.
    #[error("pvp-not-opponent")]
    NotOpponent,
    /// The duel has not started.
    #[error("pvp-duel-not-started")]
    DuelNotStarted,
}

impl PvpRefusal {
    /// The stable code scripts match, equal to the variant's display text.
    pub const fn code(self) -> &'static str {
        match self {
            Self::NotOptedIn => "pvp-not-allowed",
            Self::NoAttackOp => "pvp-no-attack-op",
            Self::OutsideWilderness => "pvp-outside-wilderness",
            Self::OpponentOutside => "pvp-opponent-outside",
            Self::LevelRange { .. } => "pvp-level-range",
            Self::ChallengeArea => "pvp-challenge-area",
            Self::NotOpponent => "pvp-not-opponent",
            Self::DuelNotStarted => "pvp-duel-not-started",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AbortReason {
    Unprotected(Unprotected),
    Unattackable,
    SafespotBroken,
    Retreated,
    RetreatFailed,
    PrepFailed(PrepItem),
    PrepReadiness(PrepReadiness),
    Unresponsive,
    /// A mid-fight refusal: the server or the zone gate stopped the attack on
    /// a player.
    PvpRefused(PvpRefusal),
    /// Single-way combat refused the attack on an NPC in three 8-tick windows
    /// in a row with no confirmed attack in between (refusals inside one
    /// window count once). After each refusal the machine attacked an NPC that
    /// was hitting the bot and that the request may fight, or held the attack
    /// for the rest of the window; the content allows the bot to attack only
    /// the NPC that last attacked it (`player_combat.rs2:115`).
    SingleWayBlocked,
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
    /// Player targets the candidate check refused to engage this run.
    pub pvp_blocked: u8,
    /// Which source authorised STARTING a fight; `None` for self-defence.
    pub pvp_source: Option<super::select::PvpSource>,
    /// Whether a server kill line was seen (never a gate; the zero bar is).
    pub kill_line_seen: bool,
    /// Completed style switches (PVP-DESIGN-1026 §4.1); not the protect-prayer
    /// switches of `protect_switches`.
    pub switches: u8,
    /// Switches that ended `Failed`: the old style kept fighting.
    pub switch_failed: u8,
    /// The style the machine was executing when the fight ended.
    pub active_style: Style,
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
