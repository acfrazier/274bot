//! Compact, selected-revision NPC and curated-hazard risk facts.
//!
//! NPC hit maxima and attack intervals deliberately reuse the shared combat
//! helpers. Unsupported or incomplete facts retain an explicit class instead
//! of acquiring a guessed zero-risk interpretation.

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

use api::game_data::{NpcAttackKind, NpcNameRow};
use nav::zones::{ZoneKind, ZoneTable};

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
        let poison = poison_per_tick(row.poison_severity);
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

fn compact_ranges(row: &NpcNameRow) -> Option<(u8, u8)> {
    if row.maxrange < 0 || row.attackrange < 0 || row.size <= 0 {
        return None;
    }
    let attackrange = i64::from(row.attackrange);
    let radius = i64::from(row.maxrange).checked_add(attackrange.max(1))?;
    let reach = (if row.ap_attack { attackrange } else { 1 })
        .checked_add(i64::from(row.size))?
        .checked_sub(1)?;
    Some((u8::try_from(radius).ok()?, u8::try_from(reach).ok()?))
}

fn flight_ticks(attackrange: i32) -> Option<u8> {
    if attackrange < 0 {
        return None;
    }
    let duration = i64::from(attackrange).checked_mul(5)?.checked_add(32)?;
    u8::try_from((duration + 29) / 30).ok()
}

fn poison_per_tick(severity: Option<i32>) -> Option<u8> {
    match severity {
        None => None,
        Some(value) if value >= 0 => u8::try_from((i64::from(value) + 4) / 5).ok(),
        Some(_) => None,
    }
}

#[derive(Debug, Clone, Copy)]
struct NpcRows {
    npc_id: i32,
    first: u16,
    len: u16,
}

/// Immutable per-zone-kind risk facts and an allocation-free NPC→zone-row index.
#[derive(Debug)]
pub struct RiskTables {
    /// Same order and length as `ZoneTable::kinds()`.
    pub kinds: Box<[KindRisk]>,
    /// Maximum unknown-poison pulse from every selected NPC row, not only zoned kinds.
    pub poison_unknown_v: u8,
    rows_by_kind: Box<[NpcRows]>,
    row_indices: Box<[u16]>,
}

#[cfg(test)]
static RISK_TABLE_BUILD_COUNT: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
pub(super) fn build_count_for_test() -> usize {
    RISK_TABLE_BUILD_COUNT.load(Ordering::Relaxed)
}

impl RiskTables {
    /// Build once from all zone kinds and the full selected NPC extract.
    pub fn build(zones: &ZoneTable, combat: &CombatTables) -> Self {
        #[cfg(test)]
        RISK_TABLE_BUILD_COUNT.fetch_add(1, Ordering::Relaxed);

        let kinds = zones
            .kinds()
            .iter()
            .map(|kind| build_kind(kind, combat))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let (rows_by_kind, row_indices) = build_row_index(zones);
        Self {
            kinds,
            poison_unknown_v: poison_unknown_v(combat),
            rows_by_kind,
            row_indices,
        }
    }

    pub fn kind(&self, index: u16) -> Option<&KindRisk> {
        self.kinds.get(usize::from(index))
    }

    /// Every zone row for this packed NPC id, including rows not engaged by a route.
    pub fn rows_for_npc(&self, id: i32) -> &[u16] {
        let Ok(index) = self
            .rows_by_kind
            .binary_search_by_key(&id, |rows| rows.npc_id)
        else {
            return &[];
        };
        let rows = self.rows_by_kind[index];
        let first = usize::from(rows.first);
        let end = first + usize::from(rows.len);
        &self.row_indices[first..end]
    }
}

fn build_kind(kind: &ZoneKind, combat: &CombatTables) -> KindRisk {
    match kind.npc_id {
        -1 => KindRisk::from_hazard(nav::zones::curated::hazard_damage(kind.id.as_ref())),
        id if id >= 0 => combat
            .npc(id)
            .map(|row| KindRisk::from_npc(row, combat))
            .unwrap_or_else(KindRisk::missing_facts),
        _ => KindRisk::missing_facts(),
    }
}

fn build_row_index(zones: &ZoneTable) -> (Box<[NpcRows]>, Box<[u16]>) {
    let mut by_npc = Vec::with_capacity(zones.zones().len());
    for (index, zone) in zones.zones().iter().enumerate() {
        let kind = &zones.kinds()[usize::from(zone.kind)];
        if kind.npc_id < 0 {
            continue;
        }
        let index = u16::try_from(index).expect("validated zone rows fit u16");
        by_npc.push((kind.npc_id, index));
    }
    by_npc.sort_unstable();

    let mut row_indices = Vec::with_capacity(by_npc.len());
    let mut rows_by_kind = Vec::new();
    let mut start = 0;
    while start < by_npc.len() {
        let npc_id = by_npc[start].0;
        let first = u16::try_from(row_indices.len()).expect("validated zone rows fit u16");
        let mut end = start;
        while end < by_npc.len() && by_npc[end].0 == npc_id {
            row_indices.push(by_npc[end].1);
            end += 1;
        }
        let len = u16::try_from(end - start).expect("validated zone rows fit u16");
        rows_by_kind.push(NpcRows { npc_id, first, len });
        start = end;
    }
    (
        rows_by_kind.into_boxed_slice(),
        row_indices.into_boxed_slice(),
    )
}

fn poison_unknown_v(combat: &CombatTables) -> u8 {
    let Some(npc_names) = combat.selected().npc_names() else {
        return u8::MAX;
    };
    poison_unknown_v_rows(&npc_names.rows)
}

fn poison_unknown_v_rows(rows: &[NpcNameRow]) -> u8 {
    if rows.is_empty() {
        return u8::MAX;
    }

    let mut maximum = 0;
    for row in rows {
        let Some(severity) = row.poison_severity else {
            return u8::MAX;
        };
        let Some(per_tick) = poison_per_tick(Some(severity)) else {
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
    use api::WorldTile;
    use nav::router::AvoidRect;
    use nav::transport::WildernessRules;
    use nav::zones::{Zone, ZoneClass};
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

    fn tile(x: i32) -> WorldTile {
        WorldTile {
            x,
            z: 100,
            level: 0,
        }
    }

    fn zone_table(zones: Vec<Zone>, kinds: Vec<ZoneKind>) -> ZoneTable {
        ZoneTable::from_parts(
            zones,
            kinds,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            4096,
            4096,
            &WildernessRules::default(),
        )
        .unwrap()
    }

    fn npc_kind(id: i32, kind_id: &str) -> ZoneKind {
        ZoneKind::new(kind_id, kind_id, id, 1, false, false)
    }

    fn one_npc_zone(npc_id: i32, kind_id: &str) -> ZoneTable {
        zone_table(
            vec![Zone::npc(tile(120), 1, ZoneClass::Always, u16::MAX, 0)],
            vec![npc_kind(npc_id, kind_id)],
        )
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
        assert_eq!(poison_per_tick(None), None);
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

        let missing = RiskTables::build(&one_npc_zone(i32::MAX, "missing"), &combat);
        assert_eq!(missing.kind(0).unwrap().unknown, UnknownKind::MissingFacts);
        assert_eq!(
            missing.kind(0).unwrap().unknown_for(false),
            Some(UnknownKind::MissingFacts)
        );
        assert!(missing.kind(1).is_none());
    }

    #[test]
    fn u2_curated_hazards_keep_known_damage_and_unknown_absence() {
        let zones = zone_table(
            vec![
                Zone::hazard(
                    AvoidRect {
                        min_x: 100,
                        max_x: 100,
                        min_z: 100,
                        max_z: 100,
                        level: Some(0),
                    },
                    0,
                    0,
                ),
                Zone::hazard(
                    AvoidRect {
                        min_x: 101,
                        max_x: 101,
                        min_z: 100,
                        max_z: 100,
                        level: Some(0),
                    },
                    0,
                    1,
                ),
            ],
            vec![
                ZoneKind::new("ikov-lava-bridge", "Ikov lava", -1, 0, false, false),
                ZoneKind::new("unknown-hazard", "Unknown hazard", -1, 0, false, false),
            ],
        );
        let risk = RiskTables::build(&zones, &combat());
        let known = risk.kind(0).unwrap();
        assert_eq!(known.max_hit, 20);
        assert_eq!(known.rate, 1);
        assert!(known.hazard());
        assert_eq!(known.unknown_for(false), None);
        let unknown = risk.kind(1).unwrap();
        assert_eq!(unknown.unknown, UnknownKind::HazardDamage);
        assert_eq!(unknown.unknown_for(false), Some(UnknownKind::HazardDamage));
    }

    #[test]
    fn u2_rows_cover_every_zone_and_poison_bound_uses_all_selected_npcs() {
        let combat = combat();
        let ice = combat.selected().npc_by_config("icewarrior").unwrap();
        let zones = zone_table(
            (0..4)
                .map(|index| Zone::npc(tile(120 + index * 10), 1, ZoneClass::Always, u16::MAX, 0))
                .collect(),
            vec![npc_kind(ice.id, "icewarrior")],
        );
        let risk = RiskTables::build(&zones, &combat);
        assert_eq!(risk.rows_for_npc(ice.id), &[0, 1, 2, 3]);
        assert!(risk.rows_for_npc(-1).is_empty());
        assert!(risk.rows_for_npc(i32::MAX).is_empty());
        assert_eq!(risk.poison_unknown_v, 11);
        assert_eq!(risk.kind(0).unwrap().max_hit, 6);

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
}
