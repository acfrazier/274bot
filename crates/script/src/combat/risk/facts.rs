//! Compact, selected-revision NPC and curated-hazard risk facts.
//!
//! NPC hit maxima and attack intervals deliberately reuse the shared combat
//! helpers. Unsupported or incomplete facts retain an explicit class instead
//! of acquiring a guessed zero-risk interpretation.

use std::future::Future;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

use api::game_data::{NpcAttackKind, NpcNameRow};
use nav_planner::contract::ZoneKindView;

use super::super::select::facts::npc_max_hit;
use super::super::tables::CombatTables;
use super::Style;

/// Why a kind is not supported by the bounded risk model.
///
/// `Known` is the compact known sentinel. Poison is conditional: it is unknown
/// only on members worlds, while its damage-per-tick value remains available
/// to the F2P estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum UnknownKind {
    Known = 0,
    Bespoke,
    Magic,
    Mixed,
    Dragonfire,
    CounterProtect,
    Poison,
    MissingStat,
    ZeroRate,
    FlightRate,
    HazardDamage,
    MissingFacts,
    Overflow,
}

/// All steady facts for one zone kind. This representation is at most 8 bytes.
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
}

const _: () = assert!(std::mem::size_of::<KindRisk>() <= 8);

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

        let unknown = if row.poison_severity.is_none() {
            UnknownKind::MissingFacts
        } else if range.is_none()
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
        } else if poison.is_some_and(|value| value > 0) {
            UnknownKind::Poison
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
            !row.bespoke && row.forced_max_hit.is_none(),
            row.forcemulti,
            row.ap_attack,
            false,
        )
    }

    /// Static unsupported classes apply in either world; poison applies only on
    /// members. This is also the origin-attacker support check.
    pub const fn unknown_for(self, map_members: bool) -> Option<UnknownKind> {
        match self.unknown {
            UnknownKind::Known | UnknownKind::Poison => {
                if map_members && self.poison > 0 {
                    Some(UnknownKind::Poison)
                } else {
                    None
                }
            }
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
            false,
            false,
            false,
            true,
        )
    }
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

/// Immutable risk facts for every selected NPC, keyed by NPC id (a
/// game-data identity, unique per pack kind), and the selected poison
/// bound. Independent of any nav pack: a zone's facts are read through its
/// facade kind view, so no kind-row order or pack index reaches script.
#[derive(Debug)]
pub struct RiskTables {
    /// `(npc id, facts)` in the selected NPC rows' ascending id order.
    npcs: Box<[(i32, KindRisk)]>,
    /// Maximum unknown-poison pulse from every selected NPC row, not only zoned kinds.
    pub poison_unknown_v: u8,
}

#[cfg(test)]
static RISK_TABLE_BUILD_COUNT: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
pub(super) fn build_count_for_test() -> usize {
    RISK_TABLE_BUILD_COUNT.load(Ordering::Relaxed)
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
    /// The async state contains only the input borrow, scalar indexes, and
    /// the one output vector. Callers account for its future layout, the
    /// vector/boxed-slice conversion overlap, and the returned table.
    pub async fn build_cooperative<F, U>(
        combat: &CombatTables,
        yield_unit: &mut F,
    ) -> Result<Self, std::collections::TryReserveError>
    where
        F: FnMut() -> U,
        U: Future<Output = ()>,
    {
        #[cfg(test)]
        RISK_TABLE_BUILD_COUNT.fetch_add(1, Ordering::Relaxed);

        let rows = combat
            .selected()
            .npc_names()
            .map_or(&[][..], |names| &names.rows);
        let mut npcs = Vec::new();
        npcs.try_reserve_exact(rows.len())?;
        for row in rows {
            npcs.push((row.id, KindRisk::from_npc(row, combat)));
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
            damagetype: None,
            headicon: None,
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
        }
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
        assert_eq!(poison.unknown_for(false), None);
        assert_eq!(poison.unknown_for(true), Some(UnknownKind::Poison));

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
        assert_eq!(missing.unknown_for(false), Some(UnknownKind::MissingFacts));
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
        assert_eq!(known.unknown_for(false), None);
        let unknown = KindRisk::from_hazard(None);
        assert_eq!(unknown.unknown, UnknownKind::HazardDamage);
        assert_eq!(unknown.unknown_for(false), Some(UnknownKind::HazardDamage));
        assert!(risk.npc(-1).is_none(), "a hazard is never an NPC row");
    }

    #[test]
    fn u2_npc_facts_cover_every_selected_npc_and_poison_bound_uses_all_of_them() {
        let combat = combat();
        let ice = combat.selected().npc_by_config("icewarrior").unwrap();
        let risk = RiskTables::build(&combat);
        let rows = &combat.selected().npc_names().unwrap().rows;
        for row in rows {
            assert_eq!(
                risk.npc(row.id),
                Some(&KindRisk::from_npc(row, &combat)),
                "{}",
                row.config
            );
            assert_eq!(
                risk.npc_or_missing(row.id),
                KindRisk::from_npc(combat.npc(row.id).unwrap(), &combat)
            );
        }
        assert_eq!(
            risk.npc_or_missing(-7).unknown,
            UnknownKind::MissingFacts,
            "an NPC id below -1 is unsupported, not a hazard"
        );
        assert!(risk.npc(-1).is_none());
        assert!(risk.npc(i32::MAX).is_none());
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
            assert_eq!(fact.unknown_for(false), None, "{config}");
        }
        let poison_spider = combat.selected().npc_by_config("poisonspider").unwrap();
        let fact = KindRisk::from_npc(poison_spider, &combat);
        assert_eq!(fact.max_hit, 7);
        assert_eq!(fact.unknown_for(false), None);
        assert_eq!(fact.unknown_for(true), Some(UnknownKind::Poison));
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
