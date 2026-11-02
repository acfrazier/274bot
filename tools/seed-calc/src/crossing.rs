//! Product-backed route crossing thresholds for explicitly declared zones.
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use api::condition::PoisonState;
use api::game_data::SelectedGameData;
use api::snapshot::WorldTile;
use host_play::admission::{self, Admission, RiskInput, RiskPolicy, WalkAllow};
use nav_planner::producer;
use nav_planner::router::{self, FindOptions};
use nav_planner::world::NavWorld;
use nav_planner::world_state::WorldState;
use nav_planner::zones::{TooMany, ZoneExempt};
use script::combat::risk::{RouteAssessment, Verdict};
use script::combat::tables::CombatStat;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const FIRST_LEVEL: u8 = 1;
const LAST_LEVEL: u8 = 99;
const POLICY_C_BASE_LEVEL: u8 = 40;
const LOBSTER_ID: i32 = 379;
const MELEE_KIT: [i32; 5] = [1163, 1113, 1079, 1201, 1333];

/// One product route query. Zone names are exact pack identities, not labels.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CrossQuery {
    pub navpack: PathBuf,
    pub from: [i32; 3],
    pub to: [i32; 3],
    pub zones: Vec<String>,
    #[serde(default)]
    pub label: Option<String>,
    /// Search held lobsters from zero through this limit at the policy-C seed.
    #[serde(default)]
    pub lobster_search_limit: Option<u8>,
    #[serde(default)]
    pub route_state: RouteState,
}

/// Noncombat facts retained from the campaign seed, not extra survival supplies.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RouteState {
    #[serde(deserialize_with = "numeric_map")]
    pub skills: BTreeMap<i32, u8>,
    #[serde(deserialize_with = "numeric_map")]
    pub varps: BTreeMap<i32, i32>,
    pub completed_quests: Vec<String>,
    #[serde(deserialize_with = "numeric_map")]
    pub inventory: BTreeMap<i32, i32>,
    pub worn: Vec<i32>,
}

// Internally tagged Query deserialization buffers JSON keys as strings.
fn numeric_map<'de, D, V>(deserializer: D) -> Result<BTreeMap<i32, V>, D::Error>
where
    D: serde::Deserializer<'de>,
    V: Deserialize<'de>,
{
    struct NumericMap<V>(std::marker::PhantomData<V>);
    impl<'de, V: Deserialize<'de>> serde::de::Visitor<'de> for NumericMap<V> {
        type Value = BTreeMap<i32, V>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("an object with numeric id keys")
        }

        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            let mut values = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, V>()? {
                let id = key.parse().map_err(serde::de::Error::custom)?;
                values.insert(id, value);
            }
            Ok(values)
        }
    }
    deserializer.deserialize_map(NumericMap(std::marker::PhantomData))
}

/// Product risk verdict, preserved without reimplementing its decision rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "reason", rename_all = "snake_case")]
pub enum CrossVerdict {
    Survivable,
    Unsurvivable,
    Unknown(String),
    NoRoute(String),
}

/// One evaluated candidate, including the host's admission refusal when denied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CrossCandidate {
    pub level: u8,
    pub hp: u8,
    pub defence: u8,
    pub combat_level: u8,
    pub food_count: Option<u8>,
    pub admitted: bool,
    pub verdict: CrossVerdict,
    pub refusal: Option<String>,
    pub reason: String,
}

/// How the ascending product search ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CrossStatus {
    Admitted,
    NoAdmissionThrough99 { at_99: CrossCandidate },
    NoAdmissionWithinFoodSlots { at_limit: CrossCandidate },
}

/// Product route zones and the first candidate accepted by host admission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CrossThreshold {
    pub route_zones: Vec<String>,
    pub lowest_level: Option<u8>,
    pub lowest_hp: Option<u8>,
    pub defence: Option<u8>,
    pub food_search_limit: Option<u8>,
    pub lowest_food_count: Option<u8>,
    pub available_food_slots: Option<u8>,
    /// Whether the product risk input uses the queried Defence directly.
    pub defence_used_directly_by_risk: bool,
    pub below_floor: Option<CrossCandidate>,
    pub at_floor: Option<CrossCandidate>,
    pub status: CrossStatus,
}

impl CrossThreshold {
    /// Produce one stable human-readable line for the CLI summary stream.
    pub fn summary(&self) -> String {
        let zones = if self.route_zones.is_empty() {
            "none".to_owned()
        } else {
            self.route_zones.join(", ")
        };
        match &self.status {
            CrossStatus::Admitted if self.lowest_food_count.is_some() => {
                let Some(at_floor) = self.at_floor.as_ref() else {
                    return format!("crossing: inconsistent food admission; route zones: {zones}");
                };
                let food_count = self.lowest_food_count.unwrap_or_default();
                let below = self.below_floor.as_ref().map_or_else(
                    || "no lower food count".to_owned(),
                    |candidate| {
                        format!(
                            "N={} refused ({:?}: {})",
                            candidate.food_count.unwrap_or_default(),
                            candidate.verdict,
                            candidate.refusal.as_deref().unwrap_or("no refusal detail"),
                        )
                    },
                );
                format!(
                    "crossing: minimum lobster count N={food_count} at base40, admitted ({:?}); below: {below}; search limit={}, available slots={}; risk uses Defence and signed worn-armour bonuses; route zones: {zones}",
                    at_floor.verdict,
                    self.food_search_limit.unwrap_or_default(),
                    self.available_food_slots.unwrap_or_default(),
                )
            }
            CrossStatus::Admitted => {
                match (&self.at_floor, self.lowest_level, self.lowest_hp, self.defence) {
                    (Some(at_floor), Some(level), Some(hp), Some(defence)) => {
                        let below = self.below_floor.as_ref().map_or_else(
                            || "no lower candidate".to_owned(),
                            |candidate| {
                                format!(
                                    "L={} HP={} Defence={} combat={} refused ({:?}: {})",
                                    candidate.level,
                                    candidate.hp,
                                    candidate.defence,
                                    candidate.combat_level,
                                    candidate.verdict,
                                    candidate.refusal.as_deref().unwrap_or("no refusal detail"),
                                )
                            },
                        );
                        format!(
                            "crossing: floor L={level} HP={hp} Defence={defence} combat={}, admitted ({:?}); below: {below}; risk uses Defence directly={}; route zones: {zones}",
                            at_floor.combat_level,
                            at_floor.verdict,
                            self.defence_used_directly_by_risk,
                        )
                    }
                    _ => format!("crossing: inconsistent admitted result; route zones: {zones}"),
                }
            }
            CrossStatus::NoAdmissionThrough99 { at_99 } => format!(
                "crossing: no admission through 99; L=99 HP={} Defence={} combat={} verdict={:?} refusal={}; risk uses Defence directly={}; route zones: {zones}",
                at_99.hp,
                at_99.defence,
                at_99.combat_level,
                at_99.verdict,
                at_99.refusal.as_deref().unwrap_or("none"),
                self.defence_used_directly_by_risk,
            ),
            CrossStatus::NoAdmissionWithinFoodSlots { at_limit } => format!(
                "crossing: no admission for N=0..{} ({} free slots); at N={} verdict={:?} refusal={}; route zones: {zones}",
                self.food_search_limit.unwrap_or_default(),
                self.available_food_slots.unwrap_or_default(),
                at_limit.food_count.unwrap_or_default(),
                at_limit.verdict,
                at_limit.refusal.as_deref().unwrap_or("none"),
            ),
        }
    }
}

/// A route or input error. Risk unknowns remain successful, explicit results.
#[derive(Debug, Error)]
pub enum CrossError {
    #[error("failed to load navpack {path}: {source}")]
    Navpack {
        path: PathBuf,
        #[source]
        source: nav_planner::pack::PackError,
    },
    #[error("navpack has no zone table for the declared cross zones")]
    MissingZoneTable,
    #[error("declared cross zone {0:?} is absent from the navpack")]
    UnknownZone(String),
    #[error("at most eight declared cross zones are supported")]
    TooManyZones(#[source] TooMany),
    #[error("could not bind product route zones: {0}")]
    ZoneIdentity(String),
    #[error("the candidate search did not evaluate any level")]
    NoCandidateLevels,
    #[error("product admission returned no assessment for a routed candidate")]
    MissingAssessment,
    #[error("invalid route state: {0}")]
    InvalidState(String),
}

/// Consent admission and no-grant survivability are different product decisions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CrossResult {
    pub label: Option<String>,
    pub declared_cross_admission: CrossThreshold,
    pub ungranted_survivability: CrossThreshold,
}

impl CrossResult {
    pub fn summary(&self) -> String {
        let label = self
            .label
            .as_deref()
            .map_or_else(String::new, |label| format!("{label}: "));
        format!(
            "{label}declared {}; ungranted survivability {}",
            self.declared_cross_admission.summary(),
            self.ungranted_survivability.summary(),
        )
    }
}

struct EvaluatedCandidate {
    level: u8,
    hp: u8,
    combat_level: u8,
    food_count: Option<u8>,
    assessment: Option<Arc<RouteAssessment>>,
    admitted: bool,
    route_refusal: Option<String>,
    reason: String,
}

/// Load the requested navpack, plan the running route, and query product admission.
pub fn calculate(query: &CrossQuery) -> Result<CrossResult, CrossError> {
    let world = NavWorld::load_pack(&query.navpack).map_err(|source| CrossError::Navpack {
        path: query.navpack.clone(),
        source,
    })?;
    let (declared_cross_admission, ungranted_survivability) =
        if let Some(limit) = query.lobster_search_limit {
            (
                calculate_food_in_world(query, &world, true, limit)?,
                calculate_food_in_world(query, &world, false, limit)?,
            )
        } else {
            (
                calculate_in_world(query, &world, true)?,
                calculate_in_world(query, &world, false)?,
            )
        };
    Ok(CrossResult {
        label: query.label.clone(),
        declared_cross_admission,
        ungranted_survivability,
    })
}

fn calculate_in_world(
    query: &CrossQuery,
    world: &NavWorld,
    consent: bool,
) -> Result<CrossThreshold, CrossError> {
    let from = tile(query.from);
    let to = tile(query.to);
    let zones = declared_zone_exemptions(world, &query.zones)?;
    let options = FindOptions {
        zones,
        ..FindOptions::default()
    };
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289)
        .map_err(|error| CrossError::InvalidState(format!("{error:?}")))?;
    let free_slots = inventory_free_slots(&query.route_state, &selected)?;
    let mut state = player_state(&query.route_state, &selected)?;
    let mut input = risk_input(
        from,
        10,
        FIRST_LEVEL,
        player_combat_level(FIRST_LEVEL, 10),
        free_slots,
    );
    bind_worn_facts(&mut input, &state, &selected);

    let mut previous = None;
    for level in FIRST_LEVEL..=LAST_LEVEL {
        let hp = level.max(10);
        let combat_level = player_combat_level(level, hp);
        state.combat_level = Some(i32::from(combat_level));
        state
            .stats
            .insert(CombatStat::Defence.index() as i32, i32::from(level));
        state
            .stats
            .insert(CombatStat::Hitpoints.index() as i32, i32::from(hp));
        input.hp = hp;
        input.hp_max = hp;
        input.defence = level;
        input.combat = Some(combat_level);
        let evaluated =
            match router::find_with(&world.collision, &world.graph, from, to, options, &state) {
                Ok(route) => {
                    let admission =
                        product_admission(input, if consent { zones } else { ZoneExempt::NONE });
                    let assessment = admission::assess(&route, world, &admission);
                    let admitted = admission::permits(&route, world, &admission, &assessment);
                    EvaluatedCandidate {
                        level,
                        hp,
                        combat_level,
                        food_count: None,
                        assessment: Some(assessment),
                        admitted,
                        route_refusal: None,
                        reason: String::new(),
                    }
                }
                Err(reason) => EvaluatedCandidate {
                    level,
                    hp,
                    combat_level,
                    food_count: None,
                    assessment: None,
                    admitted: false,
                    route_refusal: Some(format!("NoRoute({reason:?})")),
                    reason: format!("Product route unavailable: {reason:?}"),
                },
            };
        if evaluated.admitted {
            let assessment = evaluated
                .assessment
                .as_ref()
                .ok_or(CrossError::MissingAssessment)?;
            let route_zones = product_route_zones(world, assessment)?;
            let at_floor = candidate(&evaluated);
            let below_floor = previous.as_ref().map(candidate);
            return Ok(CrossThreshold {
                route_zones,
                lowest_level: Some(level),
                lowest_hp: Some(hp),
                defence: Some(level),
                food_search_limit: None,
                lowest_food_count: None,
                available_food_slots: None,
                defence_used_directly_by_risk: true,
                below_floor,
                at_floor: Some(at_floor),
                status: CrossStatus::Admitted,
            });
        }
        previous = Some(evaluated);
    }

    let Some(at_99) = previous else {
        return Err(CrossError::NoCandidateLevels);
    };
    let route_zones = match at_99.assessment.as_ref() {
        Some(assessment) => product_route_zones(world, assessment)?,
        None => Vec::new(),
    };
    Ok(CrossThreshold {
        route_zones,
        lowest_level: None,
        lowest_hp: None,
        defence: None,
        food_search_limit: None,
        lowest_food_count: None,
        available_food_slots: None,
        defence_used_directly_by_risk: true,
        below_floor: None,
        at_floor: None,
        status: CrossStatus::NoAdmissionThrough99 {
            at_99: candidate(&at_99),
        },
    })
}

fn calculate_food_in_world(
    query: &CrossQuery,
    world: &NavWorld,
    consent: bool,
    requested_limit: u8,
) -> Result<CrossThreshold, CrossError> {
    let missing_item = MELEE_KIT
        .iter()
        .find(|&&id| !query.route_state.worn.contains(&id));
    if let Some(id) = missing_item {
        return Err(CrossError::InvalidState(format!(
            "policy-C crossing search requires melee kit item {id} worn"
        )));
    }
    if query.route_state.inventory.contains_key(&LOBSTER_ID) {
        return Err(CrossError::InvalidState(
            "policy-C crossing search adds lobsters and rejects an existing lobster stack".into(),
        ));
    }

    let from = tile(query.from);
    let to = tile(query.to);
    let zones = declared_zone_exemptions(world, &query.zones)?;
    let options = FindOptions {
        zones,
        ..FindOptions::default()
    };
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289)
        .map_err(|error| CrossError::InvalidState(format!("{error:?}")))?;
    let available_food_slots = inventory_free_slots(&query.route_state, &selected)?;
    let food_search_limit = requested_limit.min(available_food_slots);
    let mut state = player_state(&query.route_state, &selected)?;
    for (stat, level) in [
        (CombatStat::Attack, POLICY_C_BASE_LEVEL),
        (CombatStat::Strength, POLICY_C_BASE_LEVEL),
        (CombatStat::Defence, POLICY_C_BASE_LEVEL),
        (CombatStat::Hitpoints, POLICY_C_BASE_LEVEL),
        (CombatStat::Ranged, POLICY_C_BASE_LEVEL),
        (CombatStat::Prayer, 1),
        (CombatStat::Magic, POLICY_C_BASE_LEVEL),
    ] {
        state.stats.insert(stat.index() as i32, i32::from(level));
    }
    let hp = POLICY_C_BASE_LEVEL;
    let combat_level = player_combat_level_base40();
    state.combat_level = Some(i32::from(combat_level));
    let mut input = risk_input(
        from,
        hp,
        POLICY_C_BASE_LEVEL,
        combat_level,
        available_food_slots,
    );
    bind_worn_facts(&mut input, &state, &selected);

    let mut previous = None;
    for food_count in 0..=food_search_limit {
        if food_count == 0 {
            state.inv.remove(&LOBSTER_ID);
        } else {
            state.inv.insert(LOBSTER_ID, i32::from(food_count));
        }
        let free_slots = available_food_slots - food_count;
        let evaluated =
            match router::find_with(&world.collision, &world.graph, from, to, options, &state) {
                Ok(route) => {
                    let admission = product_admission(
                        risk_input_with_food(input, free_slots, food_count),
                        if consent { zones } else { ZoneExempt::NONE },
                    );
                    let assessment = admission::assess(&route, world, &admission);
                    let admitted = admission::permits(&route, world, &admission, &assessment);
                    EvaluatedCandidate {
                        level: POLICY_C_BASE_LEVEL,
                        hp,
                        combat_level,
                        food_count: Some(food_count),
                        assessment: Some(assessment),
                        admitted,
                        route_refusal: None,
                        reason: String::new(),
                    }
                }
                Err(reason) => EvaluatedCandidate {
                    level: POLICY_C_BASE_LEVEL,
                    hp,
                    combat_level,
                    food_count: Some(food_count),
                    assessment: None,
                    admitted: false,
                    route_refusal: Some(format!("NoRoute({reason:?})")),
                    reason: format!("Product route unavailable: {reason:?}"),
                },
            };
        if evaluated.admitted {
            let assessment = evaluated
                .assessment
                .as_ref()
                .ok_or(CrossError::MissingAssessment)?;
            return Ok(CrossThreshold {
                route_zones: product_route_zones(world, assessment)?,
                lowest_level: Some(POLICY_C_BASE_LEVEL),
                lowest_hp: Some(hp),
                defence: Some(POLICY_C_BASE_LEVEL),
                food_search_limit: Some(food_search_limit),
                lowest_food_count: Some(food_count),
                available_food_slots: Some(available_food_slots),
                defence_used_directly_by_risk: true,
                below_floor: previous.as_ref().map(candidate),
                at_floor: Some(candidate(&evaluated)),
                status: CrossStatus::Admitted,
            });
        }
        previous = Some(evaluated);
    }

    let Some(at_limit) = previous else {
        return Err(CrossError::NoCandidateLevels);
    };
    let route_zones = match at_limit.assessment.as_ref() {
        Some(assessment) => product_route_zones(world, assessment)?,
        None => Vec::new(),
    };
    Ok(CrossThreshold {
        route_zones,
        lowest_level: None,
        lowest_hp: None,
        defence: None,
        food_search_limit: Some(food_search_limit),
        lowest_food_count: None,
        available_food_slots: Some(available_food_slots),
        defence_used_directly_by_risk: true,
        below_floor: None,
        at_floor: None,
        status: CrossStatus::NoAdmissionWithinFoodSlots {
            at_limit: candidate(&at_limit),
        },
    })
}

fn declared_zone_exemptions(world: &NavWorld, names: &[String]) -> Result<ZoneExempt, CrossError> {
    let view = producer::zones_view(world)
        .map_err(|error| CrossError::ZoneIdentity(format!("{error:?}")))?;
    let Some(view) = view else {
        return if names.is_empty() {
            Ok(ZoneExempt::NONE)
        } else {
            Err(CrossError::MissingZoneTable)
        };
    };
    let mut keys = Vec::with_capacity(names.len());
    for name in names {
        let key = view
            .resolve(name)
            .ok_or_else(|| CrossError::UnknownZone(name.clone()))?;
        keys.push(key.key());
    }
    ZoneExempt::named(&keys).map_err(CrossError::TooManyZones)
}

fn product_route_zones(
    world: &NavWorld,
    assessment: &RouteAssessment,
) -> Result<Vec<String>, CrossError> {
    let view = producer::zones_view(world)
        .map_err(|error| CrossError::ZoneIdentity(format!("{error:?}")))?;
    let Some(view) = view else {
        return Ok(Vec::new());
    };
    let mut names = Vec::with_capacity(assessment.plan.intervals.len());
    for interval in assessment.plan.intervals.iter() {
        let key = interval
            .key(view)
            .map_err(|error| CrossError::ZoneIdentity(format!("{error:?}")))?;
        let bound = producer::bind_key(view, key)
            .map_err(|error| CrossError::ZoneIdentity(format!("{error:?}")))?;
        names.push(
            view.name(bound)
                .map_err(|error| CrossError::ZoneIdentity(format!("{error:?}")))?,
        );
    }
    names.sort_unstable();
    names.dedup();
    Ok(names)
}

fn product_admission(input: RiskInput, grants: ZoneExempt) -> Admission {
    Admission {
        policy: RiskPolicy::Avoid,
        enforce: true,
        grants,
        allow: WalkAllow::default(),
        input,
        generation: 1,
        compat_v1: false,
        escape: None,
    }
}

fn player_state(
    profile: &RouteState,
    selected: &SelectedGameData,
) -> Result<WorldState, CrossError> {
    let mut state = WorldState::empty();
    state.inventory_ready = true;
    state.equipment_ready = true;
    state.map_members = true;
    state
        .inv
        .extend(profile.inventory.iter().map(|(&id, &count)| (id, count)));
    for &id in &profile.worn {
        let item = selected
            .items()
            .iter()
            .find(|item| item.id == id)
            .ok_or_else(|| CrossError::InvalidState(format!("unknown worn item {id}")))?;
        if item.wear_position < 0 && item.wear_position_2 < 0 && item.wear_position_3 < 0 {
            return Err(CrossError::InvalidState(format!(
                "item {id} cannot be worn"
            )));
        }
        state.worn.insert(id);
    }
    state
        .varps
        .extend(profile.varps.iter().map(|(&id, &value)| (id, value)));
    state
        .quests
        .extend(profile.completed_quests.iter().cloned());
    for (&skill, &level) in &profile.skills {
        if !(7..=20).contains(&skill) || !(1..=99).contains(&level) {
            return Err(CrossError::InvalidState(
                "skills must be noncombat indices 7..20, levels 1..99".into(),
            ));
        }
        state.stats.insert(skill, i32::from(level));
    }
    for (stat, value) in [
        (CombatStat::Attack, 1),
        (CombatStat::Defence, 1),
        (CombatStat::Strength, 1),
        (CombatStat::Hitpoints, 10),
        (CombatStat::Ranged, 1),
        (CombatStat::Prayer, 1),
        (CombatStat::Magic, 1),
    ] {
        state.stats.insert(stat.index() as i32, value);
    }
    Ok(state)
}

fn inventory_free_slots(
    profile: &RouteState,
    selected: &SelectedGameData,
) -> Result<u8, CrossError> {
    let mut slots = 0;
    for (&id, &count) in &profile.inventory {
        if count <= 0 {
            return Err(CrossError::InvalidState(
                "inventory counts must be positive".into(),
            ));
        }
        let item = selected
            .items()
            .iter()
            .find(|item| item.id == id)
            .ok_or_else(|| CrossError::InvalidState(format!("unknown inventory item {id}")))?;
        let used = if item.stackable { 1 } else { count };
        if used > 28 - slots {
            return Err(CrossError::InvalidState(
                "inventory exceeds 28 slots".into(),
            ));
        }
        slots += used;
    }
    Ok(28 - slots as u8)
}

/// Match Player.getCombatLevel with fresh A/S/R/M/P=1 and D/HP at the query levels.
fn player_combat_level(defence: u8, hp: u8) -> u8 {
    let sum = u16::from(defence) + u16::from(hp);
    ((5 * sum + 13) / 20) as u8
}

/// Match Player.getCombatLevel with all six combat stats at 40 and Prayer at 1.
fn player_combat_level_base40() -> u8 {
    let level = 40u16;
    let style = (level + level).max(level + level / 2);
    // Prayer is level 1 here, so integer division contributes zero.
    let sum = level + level;
    ((10 * sum + 13 * style) / 40) as u8
}

fn risk_input(from: WorldTile, hp: u8, defence: u8, combat_level: u8, free_slots: u8) -> RiskInput {
    let mut input = RiskInput::default();
    input.pos = from;
    input.poison = PoisonState::Clear;
    input.hp = hp;
    input.hp_max = hp;
    input.defence = defence;
    input.prayer = 0;
    input.prayer_base = 1;
    input.combat = Some(combat_level);
    input.free_slots = free_slots;
    input.set_map_members(true);
    input.set_run(true);
    input.set_other_prayers_on(false);
    input.set_off_debt(false);
    input.set_worn_shield(false);
    input.set_missing_facts(false);
    input.set_unattributed(false);
    input.set_input_locked(false);
    input.set_overflow(false);
    input
}

fn bind_worn_facts(input: &mut RiskInput, state: &WorldState, selected: &SelectedGameData) {
    // kiteshields.obj:179-207 defines rune 1201 as ordinary armour.
    // king_black_dragon.rs2:338-340 checks only antidragonbreathshield.
    match selected.item_by_alias("antidragonbreathshield") {
        Some(shield) => input.set_worn_shield(state.worn.contains(&shield.id)),
        None => input.set_missing_facts(true),
    }
    for &id in &state.worn {
        let Some(bonus) = selected.item_combat_bonus(id) else {
            input.set_missing_facts(true);
            continue;
        };
        for (index, value) in [
            bonus.stabdefence,
            bonus.slashdefence,
            bonus.crushdefence,
            bonus.rangedefence,
        ]
        .into_iter()
        .enumerate()
        {
            match input.worn_defence[index].checked_add(value) {
                Some(total) => input.worn_defence[index] = total,
                None => {
                    input.set_missing_facts(true);
                    input.set_overflow(true);
                }
            }
        }
    }
}

fn risk_input_with_food(mut input: RiskInput, free_slots: u8, food_count: u8) -> RiskInput {
    input.free_slots = free_slots;
    if food_count != 0 {
        input.food_ids = [LOBSTER_ID, 0, 0, 0, 0, 0];
        input.food_counts = [food_count, 0, 0, 0, 0, 0];
        input.food_len = 1;
    }
    input
}

fn candidate(value: &EvaluatedCandidate) -> CrossCandidate {
    let (verdict, refusal, reason) = match value.assessment.as_ref() {
        Some(assessment) => {
            let verdict = match assessment.verdict {
                Verdict::Survivable => CrossVerdict::Survivable,
                Verdict::Unsurvivable => CrossVerdict::Unsurvivable,
                Verdict::Unknown(reason) => CrossVerdict::Unknown(format!("{reason:?}")),
            };
            let refusal = (!value.admitted)
                .then(|| format!("{:?}", admission::verdict_refusal(assessment.verdict)));
            (verdict, refusal, assessment.reason.to_string())
        }
        None => {
            let reason = value
                .route_refusal
                .clone()
                .unwrap_or_else(|| "NoRoute".to_owned());
            (
                CrossVerdict::NoRoute(reason.clone()),
                Some(reason),
                value.reason.clone(),
            )
        }
    };
    CrossCandidate {
        level: value.level,
        hp: value.hp,
        defence: value.level,
        combat_level: value.combat_level,
        food_count: value.food_count,
        admitted: value.admitted,
        verdict,
        refusal,
        reason,
    }
}

fn tile(coords: [i32; 3]) -> WorldTile {
    WorldTile {
        x: coords[0],
        z: coords[1],
        level: coords[2],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::selected::ClientRevision;
    use client::dash3d::CollisionFlag;
    use nav_planner::collision::{pack_walk, WorldCollision};
    use nav_planner::transport::TransportGraph;
    use nav_planner::zones::{Zone, ZoneClass, ZoneKind, ZoneTable};
    use script::combat::CombatTables;

    fn tile(x: i32, z: i32) -> WorldTile {
        WorldTile { x, z, level: 0 }
    }

    fn fixture_world() -> NavWorld {
        let selected = api::game_data::for_revision(ClientRevision::R289)
            .expect("selected revision-289 game facts");
        let combat = CombatTables::build(selected).expect("selected combat facts");
        let ice_warrior = combat
            .selected()
            .npc_by_config("icewarrior")
            .expect("selected ice-warrior identity");
        let kind = ZoneKind::new(
            "icewarrior",
            "Ice warrior",
            ice_warrior.id,
            u16::try_from(ice_warrior.vislevel).expect("valid ice-warrior visibility level"),
            false,
            false,
        );

        let origin = tile(0, 0);
        let width = 22;
        let height = 3;
        let mut flags = vec![CollisionFlag::WALK_SCENERY as u32; width * height * 4];
        for level in 0..4 {
            for x in 0..width {
                flags[level * width * height + width + x] = 0;
            }
        }
        let (walk, blocked) = pack_walk(&flags);
        let collision = WorldCollision {
            origin,
            width,
            height,
            walk,
            blocked,
            flags: None,
        };
        let zones = [9]
            .into_iter()
            .map(|x| Zone::npc(tile(x, 1), 0, ZoneClass::Always, u16::MAX, 0))
            .collect();
        let mut graph = TransportGraph::default();
        graph.zones = Some(
            ZoneTable::from_parts(
                zones,
                vec![kind],
                Vec::new(),
                Vec::new(),
                Vec::new(),
                origin,
                width as u32,
                height as u32,
                &graph.wilderness,
            )
            .expect("valid fixture zone table"),
        );
        NavWorld::from_parts(collision, graph, Vec::new())
    }

    fn fixture_query(zones: Vec<String>) -> CrossQuery {
        CrossQuery {
            navpack: PathBuf::new(),
            from: [0, 1, 0],
            to: [21, 1, 0],
            zones,
            label: None,
            lobster_search_limit: None,
            route_state: RouteState::default(),
        }
    }

    fn declared_fixture_zones() -> Vec<String> {
        [9].into_iter()
            .map(|x| format!("icewarrior@{x},1,0"))
            .collect()
    }

    fn assert_real_floor(result: CrossThreshold) {
        assert!(result.defence_used_directly_by_risk);
        let CrossStatus::Admitted = result.status else {
            panic!("the fixture route has no admission floor: {result:?}");
        };
        let floor = result.lowest_level.expect("admitted result has a floor");
        let at_floor = result
            .at_floor
            .expect("admitted result has an at-floor check");
        assert!(at_floor.admitted);
        assert!(at_floor.refusal.is_none());
        assert_eq!(at_floor.level, floor);
        assert_eq!(at_floor.hp, floor.max(10));
        assert_eq!(at_floor.defence, floor);
        assert_eq!(
            at_floor.combat_level,
            player_combat_level(floor, floor.max(10))
        );
        let below = result
            .below_floor
            .expect("fixture requires an actual refusal below the floor");
        assert!(!below.admitted);
        assert_eq!(below.level + 1, floor);
        assert!(below.refusal.is_some());
    }

    #[test]
    fn fresh_profile_combat_level_matches_engine_formula() {
        assert_eq!(player_combat_level(1, 10), 3);
        assert_eq!(player_combat_level(99, 99), 50);
        assert_eq!(player_combat_level_base40(), 46);
    }

    #[test]
    fn crossing_input_binds_profile_armour_and_real_dragonfire_shield() {
        let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let profile = RouteState {
            worn: vec![1201],
            ..RouteState::default()
        };
        let state = player_state(&profile, &selected).unwrap();
        let mut input = risk_input(tile(0, 1), 40, 40, 46, 28);
        bind_worn_facts(&mut input, &state, &selected);
        let input = risk_input_with_food(input, 23, 5);

        assert_eq!(input.hp, 40);
        assert_eq!(input.hp_max, 40);
        assert_eq!(input.defence, 40);
        assert_eq!(input.combat, Some(46));
        assert_eq!(input.free_slots, 23);
        assert_eq!(input.worn_defence, [44, 48, 46, 46]);
        assert_eq!(input.food_iter().collect::<Vec<_>>(), [(LOBSTER_ID, 5)]);
        assert!(!input.worn_shield());
        assert!(input.map_members());
        assert!(input.run());
        assert!(!input.other_prayers_on());
        assert!(!input.off_debt());
        assert!(!input.missing_facts());
        assert!(!input.unattributed());
        assert!(!input.input_locked());
        assert!(!input.overflow());
        assert_eq!(input.poison, PoisonState::Clear);

        let shield = selected.item_by_alias("antidragonbreathshield").unwrap();
        let profile = RouteState {
            worn: vec![shield.id],
            ..RouteState::default()
        };
        let state = player_state(&profile, &selected).unwrap();
        let mut input = risk_input(tile(0, 1), 10, 1, 3, 28);
        bind_worn_facts(&mut input, &state, &selected);
        assert!(input.worn_shield());
        assert_eq!(input.defence, 1);
        assert_eq!(input.hp, 10);
        assert_eq!(input.combat, Some(3));
        assert!(input.food_iter().next().is_none());
    }

    #[test]
    fn missing_and_overflowing_worn_facts_remain_typed_unknown_inputs() {
        let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let mut state = WorldState::empty();
        state.worn.insert(-1);
        let mut input = risk_input(tile(0, 1), 10, 1, 3, 28);
        bind_worn_facts(&mut input, &state, &selected);
        assert!(input.missing_facts());
        assert!(!input.overflow());

        state.worn.clear();
        state.worn.insert(1201);
        let mut input = risk_input(tile(0, 1), 10, 1, 3, 28);
        input.worn_defence = [-45, -49, -47, -47];
        bind_worn_facts(&mut input, &state, &selected);
        assert_eq!(input.worn_defence, [-1; 4]);
        assert!(!input.missing_facts());

        input.worn_defence = [i16::MAX; 4];
        bind_worn_facts(&mut input, &state, &selected);
        assert!(input.missing_facts());
        assert!(input.overflow());
    }

    #[test]
    fn crossing_output_serializes_only_current_product_verdicts() {
        for (verdict, expected) in [
            (
                CrossVerdict::Survivable,
                serde_json::json!({"kind": "survivable"}),
            ),
            (
                CrossVerdict::Unsurvivable,
                serde_json::json!({"kind": "unsurvivable"}),
            ),
            (
                CrossVerdict::Unknown("Input".into()),
                serde_json::json!({"kind": "unknown", "reason": "Input"}),
            ),
            (
                CrossVerdict::NoRoute("Blocked".into()),
                serde_json::json!({"kind": "no_route", "reason": "Blocked"}),
            ),
        ] {
            assert_eq!(serde_json::to_value(verdict).unwrap(), expected);
        }
    }

    #[test]
    fn route_state_retains_required_skills_quests_and_tools() {
        let mut profile = RouteState {
            skills: BTreeMap::from([(13, 15), (14, 15)]),
            varps: BTreeMap::from([(188, 15), (347, 10)]),
            completed_quests: vec!["Hero's Quest".into(), "The Fremennik Trials".into()],
            inventory: BTreeMap::from([(995, 15), (1265, 1), (1351, 1)]),
            worn: vec![1333],
        };
        let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let state = player_state(&profile, &selected).unwrap();
        assert_eq!(state.stats.get(&14), Some(&15));
        assert_eq!(state.varps.get(&347), Some(&10));
        assert!(state.quests.contains("Hero's Quest"));
        assert_eq!(state.inv.get(&995), Some(&15));
        assert!(state.worn.contains(&1333));
        assert!(state.quest_evidence.is_none());
        assert_eq!(inventory_free_slots(&profile, &selected).unwrap(), 25);
        let query = crate::Query::Crossing(CrossQuery {
            route_state: profile.clone(),
            ..fixture_query(Vec::new())
        });
        let encoded = serde_json::to_string(&query).unwrap();
        let crate::Query::Crossing(decoded) = serde_json::from_str(&encoded).unwrap() else {
            panic!("crossing query must retain its mode");
        };
        assert_eq!(decoded.route_state.inventory, profile.inventory);
        assert_eq!(decoded.route_state.skills, profile.skills);
        assert_eq!(decoded.route_state.worn, profile.worn);
        profile.skills.insert(0, 99);
        assert!(player_state(&profile, &selected).is_err());
        profile.inventory.insert(1351, 29);
        assert!(inventory_free_slots(&profile, &selected).is_err());
    }

    #[test]
    fn product_admission_refuses_below_and_accepts_at_the_first_floor() {
        let world = fixture_world();
        let result = calculate_in_world(&fixture_query(declared_fixture_zones()), &world, false)
            .expect("product route and admission calculation");
        assert_real_floor(result);
    }

    #[test]
    fn named_cross_consent_still_requires_a_survivable_floor() {
        let world = fixture_world();
        let result = calculate_in_world(&fixture_query(declared_fixture_zones()), &world, true)
            .expect("named-cross product admission");
        assert_eq!(
            result.at_floor.as_ref().unwrap().verdict,
            CrossVerdict::Survivable
        );
        assert_real_floor(result);
    }

    #[test]
    fn policy_c_search_reports_first_food_admission_or_full_refusal() {
        let world = fixture_world();
        let mut query = fixture_query(declared_fixture_zones());
        query.route_state.worn = MELEE_KIT.to_vec();
        let threshold = calculate_food_in_world(&query, &world, false, 4)
            .expect("fixed policy-C crossing search");

        assert_eq!(threshold.food_search_limit, Some(4));
        assert_eq!(threshold.available_food_slots, Some(28));
        match threshold.status {
            CrossStatus::Admitted => {
                let count = threshold
                    .lowest_food_count
                    .expect("an admitted food search has a threshold");
                assert_eq!(threshold.lowest_level, Some(POLICY_C_BASE_LEVEL));
                assert_eq!(threshold.lowest_hp, Some(POLICY_C_BASE_LEVEL));
                assert_eq!(threshold.defence, Some(POLICY_C_BASE_LEVEL));
                let at_floor = threshold.at_floor.expect("admitted candidate");
                assert_eq!(at_floor.food_count, Some(count));
                assert!(at_floor.admitted);
                if let Some(below) = threshold.below_floor {
                    assert_eq!(below.food_count, Some(count - 1));
                    assert!(!below.admitted);
                } else {
                    assert_eq!(count, 0);
                }
            }
            CrossStatus::NoAdmissionWithinFoodSlots { at_limit } => {
                assert_eq!(threshold.lowest_food_count, None);
                assert_eq!(at_limit.food_count, Some(4));
                assert!(!at_limit.admitted);
            }
            CrossStatus::NoAdmissionThrough99 { .. } => {
                panic!("fixed food search must not use the level-search status")
            }
        }
    }

    #[test]
    fn route_search_does_not_exempt_an_undeclared_zone() {
        let world = fixture_world();
        let mut zones = declared_fixture_zones();
        zones.pop();
        let result = calculate_in_world(&fixture_query(zones), &world, false)
            .expect("all candidate route refusals are preserved in the result");
        let CrossStatus::NoAdmissionThrough99 { at_99 } = result.status else {
            panic!("the undeclared hostile zone must block every candidate");
        };
        assert!(!at_99.admitted);
        assert!(matches!(at_99.verdict, CrossVerdict::NoRoute(_)));
    }

    #[test]
    #[ignore = "requires CROSS_CALC_NAVPACK pointing to the prepared reference navpack"]
    fn reference_crossing_requests_preserve_product_admission_or_refusal() {
        let navpack =
            PathBuf::from(std::env::var_os("CROSS_CALC_NAVPACK").expect("set CROSS_CALC_NAVPACK"));
        for (from, to, zones) in [
            (
                [2772, 3230, 0],
                [2956, 3147, 0],
                vec![
                    "scorpion@2865,3156,0",
                    "scorpion@2870,3159,0",
                    "snake@2875,3164,0",
                    "scorpion@2861,3165,0",
                    "scorpion@2863,3169,0",
                    "scorpion@2862,3180,0",
                ],
            ),
            (
                [2629, 3693, 0],
                [2602, 3234, 0],
                vec!["highwayman@2735,3521,0", "wolf@2647,3584,0"],
            ),
        ] {
            let query = CrossQuery {
                navpack: navpack.clone(),
                from,
                to,
                zones: zones.into_iter().map(str::to_owned).collect(),
                label: None,
                lobster_search_limit: None,
                route_state: RouteState::default(),
            };
            let result = calculate(&query).expect("reference route must be calculable");
            for threshold in [
                result.declared_cross_admission,
                result.ungranted_survivability,
            ] {
                if let Some(at_floor) = threshold.at_floor {
                    assert!(at_floor.admitted);
                    assert_eq!(threshold.lowest_hp, Some(at_floor.hp));
                    if let Some(below) = threshold.below_floor {
                        assert!(!below.admitted);
                    }
                } else {
                    assert!(threshold.lowest_hp.is_none());
                    let CrossStatus::NoAdmissionThrough99 { at_99 } = threshold.status else {
                        panic!("a refusal must retain the last product decision");
                    };
                    assert!(!at_99.admitted);
                }
            }
        }
    }
}
