//! Compact, selected-revision NPC and curated-hazard risk facts.
//!
//! NPC hit maxima and attack intervals deliberately reuse the shared combat
//! helpers. Unsupported or incomplete facts retain an explicit class instead
//! of acquiring a guessed zero-risk interpretation.

use std::future::Future;
#[cfg(any(test, feature = "test-hooks"))]
use std::sync::atomic::{AtomicUsize, Ordering};

use api::game_data::{NpcAttackKind, NpcCombatTransform, NpcNameRow};
use nav_planner::contract::ZoneKindView;

use super::super::select::facts::npc_max_hit;
use super::super::tables::CombatTables;
use super::{Style, UnknownWhy};

/// Why a kind is not supported by the bounded risk model.
///
/// `Known` is the compact known sentinel. Poison is a numeric fact, not an
/// unsupported kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum UnknownKind {
    Known = 0,
    Bespoke,
    Magic,
    Mixed,
    Dragonfire,
    CounterProtect,
    MissingStat,
    ZeroRate,
    FlightRate,
    HazardDamage,
    MissingFacts,
    Overflow,
}

/// All steady facts for one zone kind. This representation is at most 12 bytes.
///
/// `max_hit` is the shared one-event maximum (including a forced multi-roll
/// total); `rate` is ticks between NPC attacks. For a hazard, `rate == 1` is a
/// nonzero N/A sentinel: geometry/replay use `hazard()` and charge its damage
/// exactly once at acquisition, never as a repeated attack. When `unknown` is
/// not `Known`/world-applicable, numeric fields are inert placeholders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct KindRisk {
    pub max_hit: u8,
    pub rate: u8,
    /// The pursuit envelope radius: `maxrange + max(1, attackrange)`.
    pub r: u8,
    /// The live reach term: `(ap ? attackrange : 1) + size - 1`.
    pub reach: u8,
    pub style: Style,
    pub unknown: UnknownKind,
    /// Poison damage per 30-tick pulse: `ceil(poison_severity / 5)`.
    pub poison: u8,
    flags: u8,
    /// Checked `(stat + 9) * (bonus + 64)` from selected combat facts.
    pub attack_roll: u32,
}

const _: () = assert!(std::mem::size_of::<KindRisk>() <= 12);

impl KindRisk {
    const SHARED_ATTACK: u8 = 1;
    const FORCEMULTI: u8 = 1 << 1;
    const AP: u8 = 1 << 2;
    const HAZARD: u8 = 1 << 3;

    /// Build a compact fact for synthetic fixtures and focused callers.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        max_hit: u8,
        rate: u8,
        r: u8,
        reach: u8,
        style: Style,
        unknown: UnknownKind,
        poison: u8,
        attack_roll: u32,
        shared_attack: bool,
        forcemulti: bool,
        ap: bool,
        hazard: bool,
    ) -> Self {
        Self {
            max_hit,
            rate,
            r,
            reach,
            style,
            unknown,
            poison,
            attack_roll,
            flags: (shared_attack as u8)
                | ((forcemulti as u8) << 1)
                | ((ap as u8) << 2)
                | ((hazard as u8) << 3),
        }
    }

    /// Derive one NPC's facts using the shared maximum-hit and attack-rate rules.
    pub fn from_npc(row: &NpcNameRow, combat: &CombatTables) -> Self {
        let max_hit = npc_max_hit(row);
        let rate = combat.npc_rate(row);
        let range = compact_ranges(row);
        let poison = poison_pulse(row.poison_severity);
        let style = npc_style(row);
        let attack_roll = npc_attack_roll(row);

        let unknown = if row.poison_severity.is_none() {
            UnknownKind::MissingFacts
        } else if range.is_none()
            || attack_roll.is_none()
            || poison.is_none()
            || max_hit.is_none() && row.forced_max_hit.is_some()
        {
            UnknownKind::Overflow
        } else if row.dragonfire.is_some() {
            UnknownKind::Dragonfire
        } else if row.counter_protect {
            UnknownKind::CounterProtect
        } else if row.attack_kind == Some(NpcAttackKind::Magic) {
            UnknownKind::Magic
        } else if row.attack_kind == Some(NpcAttackKind::Mixed) {
            UnknownKind::Mixed
        } else if max_hit.is_none() && row.bespoke {
            UnknownKind::Bespoke
        } else if max_hit.is_none() && missing_main_stat(row) {
            UnknownKind::MissingStat
        } else if max_hit.is_none() {
            UnknownKind::Overflow
        } else if rate == 0 {
            UnknownKind::ZeroRate
        } else if row.attack_kind == Some(NpcAttackKind::Ranged)
            && flight_ticks(row.attackrange)
                .is_none_or(|flight| i32::from(flight) > super::consts::PROJ_LAG || rate <= flight)
        {
            UnknownKind::FlightRate
        } else {
            UnknownKind::Known
        };

        let (r, reach) = range.unwrap_or((0, 0));
        Self::new(
            max_hit.unwrap_or(0),
            rate,
            r,
            reach,
            style,
            unknown,
            poison.unwrap_or(u8::MAX),
            attack_roll.unwrap_or(0),
            !row.bespoke && row.forced_max_hit.is_none(),
            row.forcemulti,
            row.ap_attack,
            false,
        )
    }

    /// Static unsupported classes apply in either world. Poison pulses remain
    /// available as numeric facts in both world modes.
    pub const fn unknown_for(self) -> Option<UnknownKind> {
        match self.unknown {
            UnknownKind::Known => None,
            unknown => Some(unknown),
        }
    }

    pub const fn shared_attack(self) -> bool {
        self.flags & Self::SHARED_ATTACK != 0
    }

    pub const fn forcemulti(self) -> bool {
        self.flags & Self::FORCEMULTI != 0
    }

    pub const fn ap(self) -> bool {
        self.flags & Self::AP != 0
    }

    pub const fn hazard(self) -> bool {
        self.flags & Self::HAZARD != 0
    }

    fn missing_facts() -> Self {
        Self::new(
            0,
            0,
            0,
            0,
            Style::Unknown,
            UnknownKind::MissingFacts,
            0,
            0,
            false,
            false,
            false,
            false,
        )
    }

    fn from_hazard(damage: Option<u8>) -> Self {
        let (max_hit, unknown) = match damage {
            Some(damage) => (damage, UnknownKind::Known),
            None => (0, UnknownKind::HazardDamage),
        };
        Self::new(
            max_hit,
            1,
            0,
            0,
            Style::Unknown,
            unknown,
            0,
            0,
            false,
            false,
            false,
            true,
        )
    }
}

/// Checked NPC combat roll from `npc_combat_melee.rs2:12-17` and
/// `npc_combat_ranged.rs2:1-5`. A negative bonus factor or overflow has no roll.
pub const fn combat_roll(stat: u16, bonus: i16) -> Option<u32> {
    let factor = bonus as i32 + 64;
    if factor < 0 {
        return None;
    }
    (stat as u32 + 9).checked_mul(factor as u32)
}

/// Player defence rolls for stab, slash, crush and ranged damage.
///
/// `equip.rs2:275-279` sums signed worn bonuses. `combat.rs2:51-61`
/// supplies the style bonus. Prayer and boosts receive no credit.
pub fn defence_rolls(defence: u8, style_def: u8, worn: [i16; 4]) -> Result<[u32; 4], UnknownWhy> {
    let effective = u32::from(defence)
        .checked_add(8)
        .and_then(|stat| stat.checked_add(u32::from(style_def)))
        .ok_or(UnknownWhy::MissingFacts)?;
    let mut rolls = [0; 4];
    for (roll, bonus) in rolls.iter_mut().zip(worn) {
        let factor = u32::try_from(i32::from(bonus) + 64).map_err(|_| UnknownWhy::MissingFacts)?;
        *roll = effective
            .checked_mul(factor)
            .ok_or(UnknownWhy::MissingFacts)?;
    }
    Ok(rolls)
}

fn npc_attack_roll(row: &NpcNameRow) -> Option<u32> {
    let (stat, bonus) = match row.attack_kind {
        Some(NpcAttackKind::Ranged) => (row.ranged?, row.rangeattack),
        Some(NpcAttackKind::Magic) => (row.magic, row.magicattack),
        Some(NpcAttackKind::Mixed) | None => (row.attack, row.attackbonus),
    };
    combat_roll(u16::try_from(stat).ok()?, i16::try_from(bonus).ok()?)
}

fn npc_style(row: &NpcNameRow) -> Style {
    if row.dragonfire.is_some() {
        return Style::Unknown;
    }
    match row.attack_kind {
        Some(NpcAttackKind::Ranged) => Style::Ranged,
        Some(NpcAttackKind::Magic | NpcAttackKind::Mixed) => Style::Unknown,
        None => Style::Melee,
    }
}

fn missing_main_stat(row: &NpcNameRow) -> bool {
    match row.attack_kind {
        Some(NpcAttackKind::Ranged) => row.ranged.is_none(),
        Some(NpcAttackKind::Magic | NpcAttackKind::Mixed) => false,
        None => row.strength.is_none(),
    }
}

pub(super) fn npc_envelope_radius(row: &NpcNameRow) -> Option<u8> {
    if row.maxrange < 0 || row.attackrange < 0 {
        return None;
    }
    let radius = i64::from(row.maxrange).checked_add(i64::from(row.attackrange).max(1))?;
    u8::try_from(radius).ok()
}

fn compact_ranges(row: &NpcNameRow) -> Option<(u8, u8)> {
    if row.maxrange < 0 || row.attackrange < 0 || row.size <= 0 {
        return None;
    }
    let attackrange = i64::from(row.attackrange);
    let radius = npc_envelope_radius(row)?;
    let reach = (if row.ap_attack { attackrange } else { 1 })
        .checked_add(i64::from(row.size))?
        .checked_sub(1)?;
    Some((radius, u8::try_from(reach).ok()?))
}

fn flight_ticks(attackrange: i32) -> Option<u8> {
    if attackrange < 0 {
        return None;
    }
    let duration = i64::from(attackrange).checked_mul(5)?.checked_add(32)?;
    u8::try_from((duration + 29) / 30).ok()
}

/// Convert an engine poison severity to its per-fire pulse.
///
/// The engine subtracts one severity per poison fire and deals
/// `ceil(severity / 5)` damage.
pub fn poison_pulse(severity: Option<i32>) -> Option<u8> {
    match severity {
        None => None,
        Some(value) if value >= 0 => u8::try_from((i64::from(value) + 4) / 5).ok(),
        Some(_) => None,
    }
}

/// Immutable risk facts keyed by the selected NPC identity. Source-proven
/// awakening chains provide their final target's combat facts while preserving
/// the original identity used by navigation zones.
#[derive(Debug)]
pub struct RiskTables {
    /// `(NPC id, effective combat facts)` in selected rows' ascending id order.
    npcs: Box<[(i32, KindRisk)]>,
    /// Maximum poison pulse from every raw selected NPC row.
    pub poison_unknown_v: u8,
}

#[cfg(any(test, feature = "test-hooks"))]
static RISK_TABLE_BUILD_COUNT: AtomicUsize = AtomicUsize::new(0);

#[cfg(any(test, feature = "test-hooks"))]
pub(super) fn build_count_for_test() -> usize {
    RISK_TABLE_BUILD_COUNT.load(Ordering::Relaxed)
}

fn resolve_awake_npc(rows: &[NpcNameRow], id: i32) -> Option<&NpcNameRow> {
    let mut current = rows
        .binary_search_by_key(&id, |row| row.id)
        .ok()
        .and_then(|index| rows.get(index))?;
    let mut transforms = 0;
    loop {
        match current.combat_transform {
            None => return Some(current),
            Some(NpcCombatTransform::Unknown) => return None,
            Some(NpcCombatTransform::Known { npc_id }) => {
                if transforms >= rows.len() {
                    return None;
                }
                transforms += 1;
                let index = rows.binary_search_by_key(&npc_id, |row| row.id).ok()?;
                current = rows.get(index)?;
            }
        }
    }
}

fn npc_risk_for_row(row: &NpcNameRow, rows: &[NpcNameRow], combat: &CombatTables) -> KindRisk {
    if row.combat_transform.is_none() {
        return KindRisk::from_npc(row, combat);
    }
    resolve_awake_npc(rows, row.id)
        .map(|target| KindRisk::from_npc(target, combat))
        .unwrap_or_else(KindRisk::missing_facts)
}

#[cfg(test)]
fn npc_risk_from_rows(rows: &[NpcNameRow], id: i32, combat: &CombatTables) -> KindRisk {
    rows.binary_search_by_key(&id, |row| row.id)
        .ok()
        .and_then(|index| rows.get(index))
        .map(|row| npc_risk_for_row(row, rows, combat))
        .unwrap_or_else(KindRisk::missing_facts)
}

/// The effective selected NPC row, resolving content-proven awakenings.
pub(super) fn effective_npc_row(id: i32, combat: &CombatTables) -> Option<&NpcNameRow> {
    let rows = combat
        .selected()
        .npc_names()
        .map_or(&[][..], |names| &names.rows);
    resolve_awake_npc(rows, id)
}

impl RiskTables {
    /// Build once from the full selected NPC extract.
    pub fn build(combat: &CombatTables) -> Self {
        let mut yield_unit = || std::future::ready(());
        super::drive_synchronously(Self::build_cooperative(combat, &mut yield_unit))
            .expect("risk table allocation")
    }

    /// Build selected NPC facts with one borrowed checkpoint per NPC row.
    ///
    /// Awakening targets are resolved without per-row storage. The async state
    /// contains only the input borrow, scalar indexes, and the one output
    /// vector. Callers account for its future layout, the vector/boxed-slice
    /// conversion overlap, and the returned table.
    pub async fn build_cooperative<F, U>(
        combat: &CombatTables,
        yield_unit: &mut F,
    ) -> Result<Self, std::collections::TryReserveError>
    where
        F: FnMut() -> U,
        U: Future<Output = ()>,
    {
        #[cfg(any(test, feature = "test-hooks"))]
        RISK_TABLE_BUILD_COUNT.fetch_add(1, Ordering::Relaxed);

        let rows = combat
            .selected()
            .npc_names()
            .map_or(&[][..], |names| &names.rows);
        let mut npcs = Vec::new();
        npcs.try_reserve_exact(rows.len())?;
        for row in rows {
            npcs.push((row.id, npc_risk_for_row(row, rows, combat)));
            yield_unit().await;
        }
        let poison_unknown_v = combat.unknown_poison_pulse();
        Ok(Self {
            npcs: npcs.into_boxed_slice(),
            poison_unknown_v,
        })
    }

    /// Facts of one selected NPC, or `None` when the selection lacks it.
    pub fn npc(&self, id: i32) -> Option<&KindRisk> {
        self.npcs
            .binary_search_by_key(&id, |(npc, _)| *npc)
            .ok()
            .map(|index| &self.npcs[index].1)
    }

    /// Test override of one NPC's facts.
    #[cfg(test)]
    pub(super) fn npc_mut(&mut self, id: i32) -> Option<&mut KindRisk> {
        self.npcs
            .binary_search_by_key(&id, |(npc, _)| *npc)
            .ok()
            .map(|index| &mut self.npcs[index].1)
    }

    /// Facts of the kind a facade zone names: its NPC's, its curated hazard
    /// damage, or missing facts for an NPC the selection lacks. This is the
    /// one zone-kind path; a hazard's curated damage is read only for a
    /// hazard kind.
    pub fn zone_kind(&self, kind: ZoneKindView<'_>) -> KindRisk {
        match kind.npc_id() {
            None => KindRisk::from_hazard(kind.hazard_damage()),
            Some(id) => self.npc_or_missing(id),
        }
    }

    fn npc_or_missing(&self, id: i32) -> KindRisk {
        if id < 0 {
            return KindRisk::missing_facts();
        }
        self.npc(id)
            .copied()
            .unwrap_or_else(KindRisk::missing_facts)
    }
}

/// Maximum poison pulse across all selected NPC rows, independent of whether
/// a row's combat facts resolve through an awakening.
pub(crate) fn poison_unknown_v_rows(rows: &[NpcNameRow]) -> u8 {
    if rows.is_empty() {
        return u8::MAX;
    }

    let mut maximum = 0;
    for row in rows {
        let Some(severity) = row.poison_severity else {
            return u8::MAX;
        };
        let Some(per_tick) = poison_pulse(Some(severity)) else {
            return u8::MAX;
        };
        maximum = maximum.max(per_tick);
    }
    maximum
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::game_data::{DragonfireKind, NpcAttackKind};
    use api::selected::ClientRevision;
    use std::sync::Arc;

    fn combat() -> Arc<CombatTables> {
        CombatTables::build(api::game_data::for_revision(ClientRevision::R289).unwrap()).unwrap()
    }

    fn npc() -> NpcNameRow {
        NpcNameRow {
            id: 1,
            config: "risk_fixture".to_string(),
            display: None,
            ops: vec!["Attack".to_string()],
            size: 1,
            wanderrange: 5,
            maxrange: 7,
            attackrange: 0,
            huntrange: 0,
            vislevel: 1,
            hitpoints: 10,
            max_dealt: Some(1000),
            damagetype: None,
            headicon: None,
            attack: 30,
            defence: 30,
            magic: 30,
            attackbonus: 0,
            rangeattack: 0,
            magicattack: 0,
            stabdefence: 0,
            slashdefence: 0,
            crushdefence: 0,
            magicdefence: 0,
            rangedefence: 0,
            strength: Some(30),
            ranged: Some(30),
            strengthbonus: Some(0),
            rangebonus: Some(0),
            undead: Some(0),
            ap_attack: false,
            attack_kind: None,
            forced_max_hit: None,
            dragonfire: None,
            attackrate: Some(4),
            bespoke: false,
            counter_protect: false,
            poison_severity: Some(0),
            forcemulti: false,
            combat_transform: None,
        }
    }

    #[test]
    fn npc_transform_resolution_follows_chains_and_fails_closed() {
        let combat = combat();
        let mut dormant = npc();
        dormant.id = 1;
        dormant.combat_transform = Some(NpcCombatTransform::Known { npc_id: 2 });
        dormant.poison_severity = Some(0);
        let mut middle = npc();
        middle.id = 2;
        middle.combat_transform = Some(NpcCombatTransform::Known { npc_id: 3 });
        let mut awake = npc();
        awake.id = 3;
        awake.attack = 77;
        awake.poison_severity = Some(11);
        let chain = [dormant, middle, awake];
        assert_eq!(resolve_awake_npc(&chain, 1).map(|row| row.id), Some(3));
        assert_eq!(
            npc_risk_from_rows(&chain, 1, &combat),
            KindRisk::from_npc(&chain[2], &combat)
        );
        assert_eq!(npc_risk_from_rows(&chain, 1, &combat).poison, 3);

        let mut unknown = npc();
        unknown.combat_transform = Some(NpcCombatTransform::Unknown);
        unknown.poison_severity = Some(11);
        let unknown_id = unknown.id;
        let unknown_rows = [unknown];
        assert_eq!(
            npc_risk_from_rows(&unknown_rows, unknown_id, &combat).unknown,
            UnknownKind::MissingFacts
        );
        assert_eq!(poison_unknown_v_rows(&unknown_rows), 3);

        let mut missing = npc();
        missing.combat_transform = Some(NpcCombatTransform::Known { npc_id: 2 });
        let missing_id = missing.id;
        let missing_rows = [missing];
        assert_eq!(
            npc_risk_from_rows(&missing_rows, missing_id, &combat).unknown,
            UnknownKind::MissingFacts
        );
        assert_eq!(poison_unknown_v_rows(&missing_rows), 0);

        let mut first = npc();
        first.combat_transform = Some(NpcCombatTransform::Known { npc_id: 2 });
        let mut second = npc();
        second.id = 2;
        second.combat_transform = Some(NpcCombatTransform::Known { npc_id: 1 });
        let cycle = [first, second];
        assert_eq!(
            npc_risk_from_rows(&cycle, 1, &combat).unknown,
            UnknownKind::MissingFacts
        );
        assert_eq!(poison_unknown_v_rows(&cycle), 0);
    }

    #[test]
    fn u2_known_and_unknown_classes_fail_closed() {
        let combat = combat();
        let mut row = npc();
        let fact = KindRisk::from_npc(&row, &combat);
        assert_eq!(fact.unknown, UnknownKind::Known);
        assert_eq!(fact.style, Style::Melee);
        assert_eq!(fact.r, 8);
        assert_eq!(fact.reach, 1);
        assert_eq!(fact.max_hit, npc_max_hit(&row).unwrap());
        assert!(fact.shared_attack());

        row.attack_kind = Some(NpcAttackKind::Ranged);
        row.attackrange = 10;
        row.attackrate = Some(4);
        row.ap_attack = true;
        row.size = 3;
        row.forcemulti = true;
        let ranged = KindRisk::from_npc(&row, &combat);
        assert_eq!(ranged.unknown, UnknownKind::Known);
        assert_eq!(ranged.style, Style::Ranged);
        assert_eq!(ranged.r, 17);
        assert_eq!(ranged.reach, 12);
        assert!(ranged.forcemulti());
        assert!(ranged.ap());

        row = npc();
        row.bespoke = true;
        assert_eq!(
            KindRisk::from_npc(&row, &combat).unknown,
            UnknownKind::Bespoke
        );

        row = npc();
        row.bespoke = true;
        row.strength = None;
        row.forced_max_hit = Some(14);
        let forced = KindRisk::from_npc(&row, &combat);
        assert_eq!(forced.unknown, UnknownKind::Known);
        assert_eq!(forced.max_hit, 14);
        assert!(!forced.shared_attack());

        row = npc();
        row.attack_kind = Some(NpcAttackKind::Magic);
        assert_eq!(
            KindRisk::from_npc(&row, &combat).unknown,
            UnknownKind::Magic
        );
        row.attack_kind = Some(NpcAttackKind::Mixed);
        assert_eq!(
            KindRisk::from_npc(&row, &combat).unknown,
            UnknownKind::Mixed
        );

        row = npc();
        row.dragonfire = Some(DragonfireKind::Other);
        assert_eq!(
            KindRisk::from_npc(&row, &combat).unknown,
            UnknownKind::Dragonfire
        );
        row = npc();
        row.counter_protect = true;
        assert_eq!(
            KindRisk::from_npc(&row, &combat).unknown,
            UnknownKind::CounterProtect
        );

        row = npc();
        row.strength = None;
        assert_eq!(
            KindRisk::from_npc(&row, &combat).unknown,
            UnknownKind::MissingStat
        );
        row = npc();
        row.attackrate = Some(0);
        assert_eq!(
            KindRisk::from_npc(&row, &combat).unknown,
            UnknownKind::ZeroRate
        );
        row = npc();
        row.attack_kind = Some(NpcAttackKind::Ranged);
        row.attackrange = 10;
        row.attackrate = Some(3);
        assert_eq!(flight_ticks(row.attackrange), Some(3));
        assert_eq!(
            KindRisk::from_npc(&row, &combat).unknown,
            UnknownKind::FlightRate
        );
        row = npc();
        row.attack_kind = Some(NpcAttackKind::Ranged);
        row.attackrange = 15;
        row.attackrate = Some(6);
        assert_eq!(flight_ticks(row.attackrange), Some(4));
        assert_eq!(
            KindRisk::from_npc(&row, &combat).unknown,
            UnknownKind::FlightRate
        );

        row = npc();
        row.poison_severity = None;
        let missing_poison = KindRisk::from_npc(&row, &combat);
        assert_eq!(missing_poison.unknown, UnknownKind::MissingFacts);
        assert_eq!(missing_poison.poison, u8::MAX);
        assert_eq!(poison_pulse(None), None);
        assert_eq!(poison_unknown_v_rows(&[row]), u8::MAX);
        assert_eq!(poison_unknown_v_rows(&[npc()]), 0);

        row = npc();
        row.poison_severity = Some(11);
        let poison = KindRisk::from_npc(&row, &combat);
        assert_eq!(poison.poison, 3);
        assert_eq!(poison.unknown_for(), None);

        row = npc();
        row.maxrange = 250;
        row.attackrange = 10;
        assert_eq!(
            KindRisk::from_npc(&row, &combat).unknown,
            UnknownKind::Overflow
        );
        row = npc();
        row.forced_max_hit = Some(256);
        assert_eq!(
            KindRisk::from_npc(&row, &combat).unknown,
            UnknownKind::Overflow
        );

        let missing = RiskTables::build(&combat).npc_or_missing(i32::MAX);
        assert_eq!(missing.unknown, UnknownKind::MissingFacts);
        assert_eq!(missing.unknown_for(), Some(UnknownKind::MissingFacts));
    }

    #[test]
    fn u2_curated_hazards_keep_known_damage_and_unknown_absence() {
        // The host reads curated damage through `ZoneKindView::hazard_damage`
        // (`ikov-lava-bridge` is 20; an uncurated hazard is unknown).
        let risk = RiskTables::build(&combat());
        let known = KindRisk::from_hazard(Some(20));
        assert_eq!(known.max_hit, 20);
        assert_eq!(known.rate, 1);
        assert!(known.hazard());
        assert_eq!(known.unknown_for(), None);
        let unknown = KindRisk::from_hazard(None);
        assert_eq!(unknown.unknown, UnknownKind::HazardDamage);
        assert_eq!(unknown.unknown_for(), Some(UnknownKind::HazardDamage));
        assert!(risk.npc(-1).is_none(), "a hazard is never an NPC row");
    }

    #[test]
    fn u2_npc_facts_cover_every_selected_npc_and_poison_bound_uses_all_of_them() {
        let combat = combat();
        let ice = combat.selected().npc_by_config("icewarrior").unwrap();
        let risk = RiskTables::build(&combat);
        let rows = &combat.selected().npc_names().unwrap().rows;
        for row in rows {
            let expected = npc_risk_from_rows(rows, row.id, &combat);
            assert_eq!(risk.npc(row.id), Some(&expected), "{}", row.config);
            assert_eq!(risk.npc_or_missing(row.id), expected);
            if row.combat_transform.is_none() {
                assert_eq!(expected, KindRisk::from_npc(row, &combat), "{}", row.config);
                assert_eq!(
                    effective_npc_row(row.id, &combat).map(|effective| effective.id),
                    Some(row.id)
                );
            }
        }

        assert_eq!(
            risk.npc_or_missing(-7).unknown,
            UnknownKind::MissingFacts,
            "an NPC id below -1 is unsupported, not a hazard"
        );
        assert!(risk.npc(-1).is_none());
        assert!(risk.npc(i32::MAX).is_none());
        assert_eq!(risk.poison_unknown_v, poison_unknown_v_rows(rows));
        assert_eq!(risk.poison_unknown_v, 11);
        assert_eq!(risk.npc_or_missing(ice.id).max_hit, 6);

        let worked = [
            ("icewarrior", 6, 4),
            ("zombie_armed", 3, 5),
            ("giantspider2", 3, 4),
            ("pirate_aggressive", 4, 5),
            ("jailguard", 3, 5),
            ("pack_wolf", 3, 4),
            ("wolfpack_leader", 7, 4),
            ("mugger", 1, 4),
        ];
        for (config, max_hit, rate) in worked {
            let row = combat.selected().npc_by_config(config).unwrap();
            let fact = KindRisk::from_npc(row, &combat);
            assert_eq!((fact.max_hit, fact.rate), (max_hit, rate), "{config}");
            assert_eq!(fact.unknown_for(), None, "{config}");
        }
        let poison_spider = combat.selected().npc_by_config("poisonspider").unwrap();
        let fact = KindRisk::from_npc(poison_spider, &combat);
        assert_eq!(fact.max_hit, 7);
        assert_eq!(fact.unknown_for(), None);
    }

    #[test]
    fn facts_rock_crab_dormant_kinds_use_awake_combat_facts() {
        let combat = combat();
        let risk = RiskTables::build(&combat);
        for (dormant_config, awake_config) in [
            ("horror_rockcrab_inactive", "horror_rockcrab"),
            ("horror_rockcrab_small_inactive", "horror_rockcrab_small"),
        ] {
            let dormant = combat.selected().npc_by_config(dormant_config).unwrap();
            let awake = combat.selected().npc_by_config(awake_config).unwrap();
            assert_eq!(
                dormant.combat_transform,
                Some(NpcCombatTransform::Known { npc_id: awake.id }),
                "{dormant_config}"
            );
            assert_eq!(
                effective_npc_row(dormant.id, &combat).map(|effective| effective.id),
                Some(awake.id),
                "{dormant_config}"
            );
            let expected = KindRisk::from_npc(awake, &combat);
            assert_eq!(risk.npc(dormant.id), Some(&expected), "{dormant_config}");
            assert_eq!(
                risk.npc_or_missing(dormant.id),
                expected,
                "{dormant_config}"
            );
            assert_eq!(
                risk.npc(dormant.id),
                risk.npc(awake.id),
                "{dormant_config} must retain awake facts under its original id"
            );
        }
    }

    #[test]
    fn facts_rune_kit_defence_rolls_match_content() {
        // Old: no defence_rolls or selected worn defence fields exist, so this
        // test does not compile. New: the selected rune kit gives these rolls.
        let combat = combat();
        let mut worn = [0i16; 4];
        for alias in [
            "rune_full_helm",
            "rune_chainbody",
            "rune_platelegs",
            "rune_kiteshield",
            "rune_scimitar",
        ] {
            let item = combat.selected().item_by_alias(alias).unwrap();
            let bonus = combat.selected().item_combat_bonus(item.id).unwrap();
            for (sum, value) in worn.iter_mut().zip([
                bonus.stabdefence,
                bonus.slashdefence,
                bonus.crushdefence,
                bonus.rangedefence,
            ]) {
                *sum = sum.checked_add(value).unwrap();
            }
        }
        assert_eq!(worn, [188, 202, 198, 190]);
        assert_eq!(defence_rolls(40, 0, worn), Ok([12096, 12768, 12576, 12192]));
    }

    #[test]
    fn facts_wolf_attack_roll_is_3776() {
        // Old: KindRisk has no attack_roll, so this assertion does not compile.
        // New: all.npc:2389 attack=50 and combat.param attackbonus default=0.
        let combat = combat();
        let row = combat.selected().npc_by_config("wolf").unwrap();
        assert_eq!(row.attack, 50);
        assert_eq!(row.attackbonus, 0);
        let risks = RiskTables::build(&combat);
        assert_eq!(risks.npc(row.id).unwrap().attack_roll, 3776);
        assert_eq!(combat_roll(50, 0), Some(3776));
    }

    #[test]
    fn facts_rock_crab_uses_content_defaults() {
        // Old: attack and attackbonus are not selected facts and no attack_roll
        // exists. New: omitted attack uses 1 and omitted attackbonus uses 0.
        let combat = combat();
        let row = combat.selected().npc_by_config("horror_rockcrab").unwrap();
        assert_eq!((row.attack, row.attackbonus), (1, 0));
        let risks = RiskTables::build(&combat);
        assert_eq!(risks.npc(row.id).unwrap().attack_roll, 640);
    }

    #[test]
    fn facts_checked_rolls_reject_negative_factors_and_invalid_npc_stats() {
        assert_eq!(combat_roll(0, -64), Some(0));
        assert_eq!(combat_roll(1, -65), None);
        assert_eq!(combat_roll(u16::MAX, i16::MAX), Some(2_151_875_064));
        assert_eq!(defence_rolls(40, 0, [-64; 4]), Ok([0; 4]));
        assert_eq!(
            defence_rolls(40, 0, [-65, 0, 0, 0]),
            Err(UnknownWhy::MissingFacts)
        );
        let combat = combat();
        let mut row = npc();
        row.attack = -1;
        assert_eq!(
            KindRisk::from_npc(&row, &combat).unknown,
            UnknownKind::Overflow
        );
        row.attack = 65_536;
        assert_eq!(
            KindRisk::from_npc(&row, &combat).unknown,
            UnknownKind::Overflow
        );
        row.attack = 1;
        row.attackbonus = -65;
        assert_eq!(
            KindRisk::from_npc(&row, &combat).unknown,
            UnknownKind::Overflow
        );
        row.attackbonus = 32_768;
        assert_eq!(
            KindRisk::from_npc(&row, &combat).unknown,
            UnknownKind::Overflow
        );
    }

    #[test]
    fn facts_ranged_attack_roll_uses_ranged_level_and_bonus() {
        let combat = combat();
        let mut row = npc();
        row.attack_kind = Some(NpcAttackKind::Ranged);
        row.attack = 1;
        row.attackbonus = 100;
        row.ranged = Some(50);
        row.rangeattack = 10;
        let fact = KindRisk::from_npc(&row, &combat);
        assert_eq!(fact.unknown, UnknownKind::Known);
        assert_eq!(fact.attack_roll, 4366);
    }

    #[test]
    fn facts_process_table_sizes_are_bounded() {
        assert_eq!(std::mem::size_of::<KindRisk>(), 12);
        assert_eq!(std::mem::size_of::<(i32, KindRisk)>(), 16);
        let combat = combat();
        let rows = combat.selected().npc_names().unwrap().rows.len();
        let risks = RiskTables::build(&combat);
        assert_eq!(risks.npcs.len(), rows);
        eprintln!(
            "P1A sizes KindRisk={} RiskInput={} RiskTables={} npc_rows={} npc_table_bytes={} npc_table_growth={}",
            std::mem::size_of::<KindRisk>(),
            std::mem::size_of::<super::super::RiskInput>(),
            std::mem::size_of::<RiskTables>(),
            rows,
            std::mem::size_of_val(risks.npcs.as_ref()),
            rows * 4,
        );
        eprintln!(
            "P1A generated sizes NpcNameRow={} npc_rows={} ItemCombatBonus={} bonus_rows={} WeaponStyleFact={} weapon_styles={}",
            std::mem::size_of::<NpcNameRow>(),
            rows,
            std::mem::size_of::<api::game_data::ItemCombatBonus>(),
            combat.selected().item_combat_bonus_rows().len(),
            std::mem::size_of::<api::game_data::WeaponStyleFact>(),
            combat.selected().weapon_styles().len(),
        );
    }

    #[test]
    fn u2_content_proved_bespoke_melee_uses_shared_damage_facts() {
        let combat = combat();
        let rows = &combat.selected().npc_names().unwrap().rows;

        let spider = rows.iter().find(|row| row.id == 977).unwrap();
        assert!(!spider.bespoke);
        let spider_fact = KindRisk::from_npc(spider, &combat);
        assert_eq!(spider_fact.style, Style::Melee);
        assert_eq!(spider_fact.max_hit, 4);
        assert_eq!(spider_fact.rate, 4);
        assert_eq!(spider_fact.unknown_for(), None);
        assert_eq!(
            crate::combat::select::facts::npc_style_max_hit(
                spider,
                crate::combat::threats::StyleObs::Melee,
            ),
            Some(4),
        );

        let highwayman = rows.iter().find(|row| row.id == 180).unwrap();
        assert!(!highwayman.bespoke);
        let highwayman_fact = KindRisk::from_npc(highwayman, &combat);
        assert_eq!(highwayman_fact.style, Style::Melee);
        assert_eq!(highwayman_fact.rate, 4);
        assert_eq!(highwayman_fact.unknown_for(), None);

        let shadow_spider = combat.selected().npc_by_config("shadow_spider").unwrap();
        assert!(shadow_spider.bespoke);
        assert_eq!(
            KindRisk::from_npc(shadow_spider, &combat).unknown,
            UnknownKind::Bespoke,
        );
        assert_eq!(
            crate::combat::select::facts::npc_style_max_hit(
                shadow_spider,
                crate::combat::threats::StyleObs::Melee,
            ),
            None,
        );
    }

    #[test]
    fn u2_cooperative_risk_table_matches_sync_and_checkpoints_every_npc() {
        let combat = combat();
        let rows = &combat.selected().npc_names().unwrap().rows;
        let mut checkpoints = 0;
        let mut yield_unit = || {
            checkpoints += 1;
            std::future::ready(())
        };
        let cooperative = super::super::drive_synchronously(RiskTables::build_cooperative(
            &combat,
            &mut yield_unit,
        ))
        .unwrap();
        let synchronous = RiskTables::build(&combat);
        // The shared CombatTables already caches the revision's pulse maximum.
        // This builder now has one NPC pass, with one checkpoint per row.
        assert_eq!(checkpoints, rows.len());
        assert_eq!(cooperative.npcs.as_ref(), synchronous.npcs.as_ref());
        assert_eq!(cooperative.poison_unknown_v, synchronous.poison_unknown_v);
    }
}
