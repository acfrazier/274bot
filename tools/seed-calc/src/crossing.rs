//! Product-backed route crossing thresholds for explicitly declared zones.
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use api::condition::PoisonState;
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

/// One product route query. Zone names are exact pack identities, not labels.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CrossQuery {
    pub navpack: PathBuf,
    pub from: [i32; 3],
    pub to: [i32; 3],
    pub zones: Vec<String>,
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
    FixableWith,
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
}

/// Product route zones and the first candidate accepted by host admission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CrossThreshold {
    pub route_zones: Vec<String>,
    pub lowest_level: Option<u8>,
    pub lowest_hp: Option<u8>,
    pub defence: Option<u8>,
    /// Whether the product risk input uses Defence directly. It currently does not.
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
    pub declared_cross_admission: CrossThreshold,
    pub ungranted_survivability: CrossThreshold,
}

impl CrossResult {
    pub fn summary(&self) -> String {
        format!(
            "declared {}; ungranted survivability {}",
            self.declared_cross_admission.summary(),
            self.ungranted_survivability.summary(),
        )
    }
}

struct EvaluatedCandidate {
    level: u8,
    hp: u8,
    combat_level: u8,
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
    Ok(CrossResult {
        declared_cross_admission: calculate_in_world(query, &world, true)?,
        ungranted_survivability: calculate_in_world(query, &world, false)?,
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
    let free_slots = inventory_free_slots(&query.route_state)?;
    let mut state = player_state(&query.route_state)?;

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
        let evaluated =
            match router::find_with(&world.collision, &world.graph, from, to, options, &state) {
                Ok(route) => {
                    let admission = product_admission(
                        risk_input(from, hp, combat_level, free_slots),
                        if consent { zones } else { ZoneExempt::NONE },
                    );
                    let assessment = admission::assess(&route, world, &admission);
                    let admitted = admission::permits(&route, world, &admission, &assessment);
                    EvaluatedCandidate {
                        level,
                        hp,
                        combat_level,
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
                defence_used_directly_by_risk: false,
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
        defence_used_directly_by_risk: false,
        below_floor: None,
        at_floor: None,
        status: CrossStatus::NoAdmissionThrough99 {
            at_99: candidate(&at_99),
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

fn player_state(profile: &RouteState) -> Result<WorldState, CrossError> {
    let mut state = WorldState::empty();
    state.inventory_ready = true;
    state.equipment_ready = true;
    state.map_members = true;
    state
        .inv
        .extend(profile.inventory.iter().map(|(&id, &count)| (id, count)));
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
        state.stats.insert(stat.index() as i32, i32::from(value));
    }
    Ok(state)
}

fn inventory_free_slots(profile: &RouteState) -> Result<u8, CrossError> {
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289)
        .map_err(|error| CrossError::InvalidState(format!("{error:?}")))?;
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

fn risk_input(from: WorldTile, hp: u8, combat_level: u8, free_slots: u8) -> RiskInput {
    RiskInput {
        pos: from,
        poison: PoisonState::Clear,
        hp,
        hp_max: hp,
        prayer: 0,
        prayer_base: 1,
        combat: Some(combat_level),
        free_slots,
        map_members: true,
        run: true,
        other_prayers_on: false,
        off_debt: false,
        worn_shield: false,
        missing_facts: false,
        unattributed: false,
        input_locked: false,
        overflow: false,
        ..RiskInput::default()
    }
}

fn candidate(value: &EvaluatedCandidate) -> CrossCandidate {
    let (verdict, refusal, reason) = match value.assessment.as_ref() {
        Some(assessment) => {
            let verdict = match assessment.verdict {
                Verdict::Survivable => CrossVerdict::Survivable,
                Verdict::FixableWith => CrossVerdict::FixableWith,
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
            route_state: RouteState::default(),
        }
    }

    fn declared_fixture_zones() -> Vec<String> {
        [9].into_iter()
            .map(|x| format!("icewarrior@{x},1,0"))
            .collect()
    }

    fn assert_real_floor(result: CrossThreshold) {
        assert!(!result.defence_used_directly_by_risk);
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
    }

    #[test]
    fn route_state_retains_required_skills_quests_and_tools() {
        let mut profile = RouteState {
            skills: BTreeMap::from([(13, 15), (14, 15)]),
            varps: BTreeMap::from([(188, 15), (347, 10)]),
            completed_quests: vec!["Hero's Quest".into(), "The Fremennik Trials".into()],
            inventory: BTreeMap::from([(995, 15), (1265, 1), (1351, 1)]),
        };
        let state = player_state(&profile).unwrap();
        assert_eq!(state.stats.get(&14), Some(&15));
        assert_eq!(state.varps.get(&347), Some(&10));
        assert!(state.quests.contains("Hero's Quest"));
        assert_eq!(state.inv.get(&995), Some(&15));
        assert!(state.quest_evidence.is_none());
        assert_eq!(inventory_free_slots(&profile).unwrap(), 25);
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
        profile.skills.insert(0, 99);
        assert!(player_state(&profile).is_err());
        profile.inventory.insert(1351, 29);
        assert!(inventory_free_slots(&profile).is_err());
    }

    #[test]
    fn product_admission_refuses_below_and_accepts_at_the_first_floor() {
        let world = fixture_world();
        let result = calculate_in_world(&fixture_query(declared_fixture_zones()), &world, false)
            .expect("product route and admission calculation");
        assert_real_floor(result);
    }

    #[test]
    fn named_cross_consent_is_not_a_survivability_proof() {
        let world = fixture_world();
        let result = calculate_in_world(&fixture_query(declared_fixture_zones()), &world, true)
            .expect("named-cross product admission");
        assert_eq!(result.lowest_level, Some(1));
        let candidate = result.at_floor.expect("declared crossing admitted");
        assert!(candidate.admitted);
        assert_ne!(candidate.verdict, CrossVerdict::Survivable);
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
