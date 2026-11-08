//! Exact finite-state model for guide steps that damage the player on failure.
use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use script::combat::tables::CombatTables;

const ROLL_BUCKETS: u16 = 256;
const REGEN_TICKS: u8 = 100;
const INVENTORY_SLOTS: u8 = 28;
const ROPE_START_TILE: [i64; 2] = [38 * 64 + 30, 151 * 64 + 35];
const DEATH_BOUND: f64 = 0.01;
const RELAXED_DEATH_BOUND: f64 = 0.05;

#[derive(Debug, Error)]
pub enum SelfDamageError {
    #[error("cannot read self-damage input: {0}")]
    Io(#[from] std::io::Error),
    #[error("cannot decode self-damage input: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Input(String),
    #[error("self-damage model unavailable: {0}")]
    NotModelled(String),
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SelfDamageQuery {
    #[serde(default)]
    pub label: Option<String>,
    pub guide: PathBuf,
    pub seed: PathBuf,
    /// Override the seed's Hitpoints level for a controlled comparison.
    #[serde(default)]
    pub hitpoints: Option<u8>,
    /// Search at most this many lobsters. The default is one full inventory.
    #[serde(default = "default_lobster_limit")]
    pub lobster_limit: u8,
    /// Required only when a guide has more than one role.
    #[serde(default)]
    pub role: Option<String>,
}

fn default_lobster_limit() -> u8 {
    INVENTORY_SLOTS
}

#[derive(Debug, Serialize)]
pub struct SelfDamageResult {
    pub label: String,
    pub guide: String,
    pub role: Option<String>,
    pub hitpoints: u8,
    pub agility: u8,
    pub crafting: u8,
    pub food_result_condition: &'static str,
    pub seed_lobsters: u8,
    pub eat_line: u8,
    pub lobster_heal: u8,
    pub no_food_death_probability: f64,
    pub no_food_budget_stop_probability: f64,
    pub minimum_lobsters: Option<u8>,
    pub accepted_death_bound: Option<f64>,
    pub death_probability_with_policy_food: f64,
    pub budget_stop_probability_with_policy_food: f64,
    pub food_search_limit: u8,
    pub budget_stops: Vec<BudgetStopResult>,
    pub steps: Vec<StepModelResult>,
    pub exact_state_recursion: bool,
    pub model_limit: String,
}

#[derive(Debug, Serialize)]
pub struct BudgetStopResult {
    pub stop: String,
    pub probability_without_food: f64,
    pub probability_with_policy_food: f64,
}

#[derive(Debug, Serialize)]
pub struct StepModelResult {
    pub id: String,
    pub skill: String,
    pub level: u8,
    pub successes_per_256: u16,
    pub attempts_per_entry: u8,
    pub entries_per_parent_pass: u8,
    pub failure_damage: String,
    pub attempt_ticks: String,
    pub source: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PreRoll {
    /// Delayed ticks from `p_delay` calls before the setup walk.
    delay_before_walk: u8,
    /// Teleport steps of a forced walk. Each step ends in `p_delay(0)` inside `agility_walk`
    /// (`content/scripts/skill_agility/scripts/agility.rs2:40-41`), so its tick is delayed.
    walk_ticks: u8,
    /// Delayed ticks from `p_delay` calls after the setup walk.
    delay_after_walk: u8,
}

#[derive(Clone, Copy, Debug)]
struct Timing {
    first_attempt_before_roll: PreRoll,
    retry_before_roll: PreRoll,
    failure_before_damage: u8,
    after_failure: u8,
    after_success: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TickAccess {
    Available,
    /// The player cannot run normal timers because of a delay or modal state.
    Blocked,
}

#[derive(Clone, Copy, Debug)]
enum Skill {
    Agility,
    Crafting,
}

impl Skill {
    fn name(self) -> &'static str {
        match self {
            Self::Agility => "agility",
            Self::Crafting => "crafting",
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Damage {
    Fixed(u8),
    CurrentPercentPlus { percent: u8, plus: u8 },
    BasePercentCapped { percent: u8, cap: u8 },
}

impl Damage {
    fn amount(self, current_hp: u8, base_hp: u8) -> u8 {
        match self {
            Self::Fixed(amount) => amount,
            Self::CurrentPercentPlus { percent, plus } => {
                ((u16::from(current_hp) * u16::from(percent)) / 100 + u16::from(plus))
                    .min(u16::from(u8::MAX)) as u8
            }
            Self::BasePercentCapped { percent, cap } => {
                ((u16::from(base_hp) * u16::from(percent)) / 100).min(u16::from(cap)) as u8
            }
        }
    }

    fn description(self) -> String {
        match self {
            Self::Fixed(amount) => format!("{amount} HP"),
            Self::CurrentPercentPlus { percent, plus } => {
                format!("floor(current HP * {percent} / 100) + {plus}")
            }
            Self::BasePercentCapped { percent, cap } => {
                format!("min(floor(base HP * {percent} / 100), {cap})")
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Hazard {
    id: &'static str,
    skill: Skill,
    level: u8,
    low: i32,
    high: i32,
    damage: Damage,
    timing: Timing,
    source: &'static str,
    timing_note: &'static str,
}

impl Hazard {
    fn successes_per_256(self) -> u16 {
        stat_random_successes(self.level, self.low, self.high)
    }
}

#[derive(Clone, Debug)]
struct BoundedStep {
    id: String,
    attempts: u8,
    stop: String,
    hazard: Hazard,
    entries_per_parent_pass: u8,
}

#[derive(Clone, Debug)]
enum Operation {
    Bounded(BoundedStep),
    SwingAcquire {
        id: String,
        restarts: u8,
        stop: String,
        swing: Hazard,
        recovery: Vec<BoundedStep>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct State {
    hp: u8,
    regen_phase: u8,
    food: u8,
    food_cooldown: u8,
}

/// States are visited in `Ord` order at every merge and every sum. Floating-point addition is
/// not associative, so a map with a per-process order (a hash map) changes the stored bits from
/// run to run. Sorting only at print would not help: the sums are built during the merge.
type Distribution = BTreeMap<State, f64>;

#[derive(Default)]
struct Run {
    live: Distribution,
    death_probability: f64,
    budget_stops: BTreeMap<String, f64>,
}

#[derive(Default)]
struct OnceBranches {
    success: Distribution,
    failure: Distribution,
    death_probability: f64,
}

#[derive(Clone, Copy)]
struct FoodPolicy {
    line: u8,
    heal: u8,
    eat_delay: u8,
    skill_delay: u8,
}

#[derive(Clone, Copy)]
struct FoodFacts {
    policy: FoodPolicy,
    seed_count: u8,
}

struct Candidate {
    count: u8,
    death_probability: f64,
    budget_stops: BTreeMap<String, f64>,
}

pub fn calculate(
    query: &SelfDamageQuery,
    tables: &CombatTables,
) -> Result<SelfDamageResult, SelfDamageError> {
    if query.lobster_limit > INVENTORY_SLOTS {
        return Err(SelfDamageError::Input(format!(
            "lobster_limit cannot exceed {INVENTORY_SLOTS} inventory slots"
        )));
    }

    let guide: Value = read_json(&query.guide)?;
    let seed: Value = read_json(&query.seed)?;
    let guide_name = guide
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            query
                .guide
                .file_stem()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "guide".to_owned());
    let role_value = select_role(&guide, query.role.as_deref())?;
    let role_name = role_value
        .get("role")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let hitpoints = query
        .hitpoints
        .or_else(|| value_u8(seed.pointer("/stats/hitpoints")))
        .ok_or_else(|| {
            SelfDamageError::Input(
                "seed has no stats.hitpoints; pass hitpoints to set the starting level".into(),
            )
        })?;
    if !(1..=99).contains(&hitpoints) {
        return Err(SelfDamageError::Input(
            "Hitpoints must be between 1 and 99".into(),
        ));
    }
    let agility = seed_level(&seed, "agility")?;
    let crafting = seed_level(&seed, "crafting")?;
    let plan = extract_plan(&guide, role_value, agility, crafting)?;
    if plan.is_empty() {
        return Err(SelfDamageError::NotModelled(
            "the selected guide role has no supported self-damaging steps".into(),
        ));
    }

    let food_facts = food_facts(tables, &seed, hitpoints)?;
    let initial = State {
        hp: hitpoints,
        regen_phase: 0,
        food: 0,
        food_cooldown: 0,
    };
    let no_food = execute(&plan, initial, hitpoints, None);

    let mut candidates = Vec::with_capacity(usize::from(query.lobster_limit) + 1);
    for count in 0..=query.lobster_limit {
        let food_state = State {
            food: count,
            ..initial
        };
        let run = execute(&plan, food_state, hitpoints, Some(food_facts.policy));
        candidates.push(Candidate {
            count,
            death_probability: run.death_probability,
            budget_stops: run.budget_stops,
        });
    }

    let selected = candidates
        .iter()
        .find(|candidate| candidate.death_probability <= DEATH_BOUND)
        .map(|candidate| (candidate, DEATH_BOUND))
        .or_else(|| {
            candidates
                .iter()
                .find(|candidate| candidate.death_probability <= RELAXED_DEATH_BOUND)
                .map(|candidate| (candidate, RELAXED_DEATH_BOUND))
        });
    let displayed = selected
        .map(|(candidate, bound)| (Some(candidate), Some(bound)))
        .unwrap_or_else(|| (candidates.last(), None));
    let selected_candidate = displayed
        .0
        .ok_or_else(|| SelfDamageError::Input("lobster search produced no candidate".into()))?;

    let mut stop_names = BTreeMap::<String, (f64, f64)>::new();
    for (name, probability) in &no_food.budget_stops {
        stop_names.entry(name.clone()).or_default().0 = *probability;
    }
    for (name, probability) in &selected_candidate.budget_stops {
        stop_names.entry(name.clone()).or_default().1 = *probability;
    }
    let budget_stops = stop_names
        .into_iter()
        .map(
            |(stop, (probability_without_food, probability_with_policy_food))| BudgetStopResult {
                stop,
                probability_without_food,
                probability_with_policy_food,
            },
        )
        .collect();

    Ok(SelfDamageResult {
        label: query.label.clone().unwrap_or_else(|| guide_name.clone()),
        guide: guide_name,
        role: role_name,
        hitpoints,
        agility,
        crafting,
        seed_lobsters: food_facts.seed_count,
        food_result_condition:
            "Conditional: the live Quester does not eat during these modeled steps.",
        eat_line: food_facts.policy.line,
        lobster_heal: food_facts.policy.heal,
        no_food_death_probability: no_food.death_probability,
        no_food_budget_stop_probability: sum_stops(&no_food.budget_stops),
        minimum_lobsters: selected.map(|(candidate, _)| candidate.count),
        accepted_death_bound: selected.map(|(_, bound)| bound),
        death_probability_with_policy_food: selected_candidate.death_probability,
        budget_stop_probability_with_policy_food: sum_stops(&selected_candidate.budget_stops),
        food_search_limit: query.lobster_limit,
        budget_stops,
        steps: describe_plan(&plan),
        exact_state_recursion: true,
        model_limit: "Exact for listed self-damage actions, their authored retry limits, observed p_delay and same-tick arrival delays, per-tile rope setup movement (each forcemove tile ends in p_delay(0)), and the 100-tick health_regen timer from phase zero. Delayed and modal ticks, including every p_delay(0) and rope setup walk tick, do not regenerate health; each eligible tick processes at most one overdue timer event (engine/src/engine/World.ts:687-709, engine/src/engine/entity/Player.ts:816-830,945-959, engine/src/engine/script/handlers/PlayerOps.ts:359-380). It excludes travel outside those actions, controller handoff time, unrelated guide actions, and their elapsed ticks.".into(),
    })
}

fn read_json(path: &PathBuf) -> Result<Value, SelfDamageError> {
    let file = File::open(path)?;
    Ok(serde_json::from_reader(BufReader::new(file))?)
}

fn value_u8(value: Option<&Value>) -> Option<u8> {
    value
        .and_then(Value::as_u64)
        .and_then(|number| u8::try_from(number).ok())
}

fn seed_level(seed: &Value, name: &str) -> Result<u8, SelfDamageError> {
    let value = seed
        .pointer(&format!("/skills/{name}"))
        .or_else(|| seed.pointer(&format!("/stats/{name}")));
    let Some(level) = value_u8(value) else {
        return Ok(1);
    };
    if !(1..=99).contains(&level) {
        return Err(SelfDamageError::Input(format!(
            "seed skill {name} must be between 1 and 99"
        )));
    }
    Ok(level)
}

fn select_role<'a>(
    guide: &'a Value,
    requested: Option<&str>,
) -> Result<&'a Value, SelfDamageError> {
    let roles = guide
        .get("roles")
        .and_then(Value::as_array)
        .ok_or_else(|| SelfDamageError::Input("guide has no roles array".into()))?;
    if let Some(requested) = requested {
        return roles
            .iter()
            .find(|role| role.get("role").and_then(Value::as_str) == Some(requested))
            .ok_or_else(|| {
                SelfDamageError::Input(format!("guide has no role named {requested:?}"))
            });
    }
    if roles.len() != 1 {
        return Err(SelfDamageError::Input(
            "guide has multiple roles; select one with the role field".into(),
        ));
    }
    roles
        .first()
        .ok_or_else(|| SelfDamageError::Input("guide has no roles".into()))
}

fn extract_plan(
    guide: &Value,
    role: &Value,
    agility: u8,
    crafting: u8,
) -> Result<Vec<Operation>, SelfDamageError> {
    let sequences = role
        .get("sequences")
        .and_then(Value::as_array)
        .ok_or_else(|| SelfDamageError::Input("guide role has no sequences array".into()))?;
    let mut plan = Vec::new();
    for sequence in sequences {
        let steps = sequence
            .get("steps")
            .and_then(Value::as_array)
            .ok_or_else(|| SelfDamageError::Input("guide sequence has no steps array".into()))?;
        for step in steps {
            if let Some(loc) = step.pointer("/args/target/loc").and_then(Value::as_str) {
                if loc == "rockslide2_obstacle_upass" {
                    plan.push(Operation::Bounded(parse_bounded_step(
                        step,
                        rockslide_hazard(agility),
                        None,
                        1,
                    )?));
                    continue;
                }
                if loc.starts_with("mcannonrailing") {
                    plan.push(Operation::Bounded(parse_bounded_step(
                        step,
                        railing_hazard(crafting),
                        None,
                        1,
                    )?));
                    continue;
                }
            }
            if step.pointer("/args/recipe").and_then(Value::as_str) == Some("swing-and-recover") {
                plan.push(parse_swing_acquire(step, guide, agility)?);
            }
        }
    }
    Ok(plan)
}

fn parse_bounded_step(
    step: &Value,
    hazard: Hazard,
    restarts: Option<u8>,
    entries_per_parent_pass: u8,
) -> Result<BoundedStep, SelfDamageError> {
    let id = step
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| SelfDamageError::Input("self-damage step has no id".into()))?
        .to_owned();
    let attempts = if let Some(max_attempts) = value_u8(step.get("max_attempts")) {
        max_attempts
    } else if let Some(max_restarts) = restarts {
        max_restarts.saturating_add(1)
    } else {
        return Err(SelfDamageError::NotModelled(format!(
            "self-damage step {id} has no max_attempts budget"
        )));
    };
    if attempts == 0 {
        return Err(SelfDamageError::Input(format!(
            "self-damage step {id} has a zero attempt budget"
        )));
    }
    let stop = step
        .get("stop")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{id} exhausted after {attempts} attempts"));
    Ok(BoundedStep {
        id,
        attempts,
        stop,
        hazard,
        entries_per_parent_pass,
    })
}

fn parse_swing_acquire(
    parent: &Value,
    guide: &Value,
    agility: u8,
) -> Result<Operation, SelfDamageError> {
    let id = parent
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| SelfDamageError::Input("swing acquire step has no id".into()))?
        .to_owned();
    let restarts = value_u8(parent.pointer("/args/max_restarts")).ok_or_else(|| {
        SelfDamageError::NotModelled(format!("{id} does not declare max_restarts"))
    })?;
    let acquire = guide
        .pointer("/quest/acquire")
        .and_then(Value::as_object)
        .ok_or_else(|| SelfDamageError::NotModelled(format!("{id} has no quest acquire map")))?;
    let recipe = acquire
        .get("swing-and-recover")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            SelfDamageError::NotModelled(format!("{id} has no swing-and-recover recipe"))
        })?;
    let swing_step = recipe
        .iter()
        .find(|step| step.get("id").and_then(Value::as_str) == Some("swing-attempt"))
        .ok_or_else(|| SelfDamageError::NotModelled("swing recipe has no swing-attempt".into()))?;
    let setup_ticks = rope_setup_ticks(swing_step)?;
    let swing = rope_hazard(agility, setup_ticks);
    if swing_step.pointer("/args/item").and_then(Value::as_str) != Some("rope") {
        return Err(SelfDamageError::NotModelled(
            "swing-attempt does not use the modeled rope".into(),
        ));
    }

    let mut recovery = Vec::new();
    for child in recipe {
        let Some(recipe_name) = child.pointer("/args/recipe").and_then(Value::as_str) else {
            continue;
        };
        if !recipe_name.starts_with("recovery-rockslide-") {
            continue;
        }
        let child_id = child
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| SelfDamageError::Input("recovery step has no id".into()))?;
        let max_restarts = value_u8(child.pointer("/args/max_restarts")).ok_or_else(|| {
            SelfDamageError::NotModelled(format!("{child_id} has no max_restarts"))
        })?;
        let child_recipe = acquire
            .get(recipe_name)
            .and_then(Value::as_array)
            .ok_or_else(|| {
                SelfDamageError::NotModelled(format!("{child_id} has no {recipe_name} recipe"))
            })?;
        let attempt = child_recipe
            .iter()
            .find(|step| {
                step.pointer("/args/target/loc").and_then(Value::as_str)
                    == Some("rockslide2_obstacle_upass")
            })
            .ok_or_else(|| {
                SelfDamageError::NotModelled(format!("{child_id} recipe has no rockslide attempt"))
            })?;
        let mut fields = attempt
            .as_object()
            .cloned()
            .ok_or_else(|| SelfDamageError::Input("rockslide attempt is not an object".into()))?;
        fields.insert("id".into(), Value::String(child_id.to_owned()));
        let child_step = Value::Object(fields);
        recovery.push(parse_bounded_step(
            &child_step,
            rockslide_hazard(agility),
            Some(max_restarts),
            1,
        )?);
    }
    if recovery.len() != 8 {
        return Err(SelfDamageError::NotModelled(format!(
            "{id} recovery recipe has {} rockslides, expected the authored eight",
            recovery.len()
        )));
    }
    Ok(Operation::SwingAcquire {
        id: id.clone(),
        restarts,
        stop: format!(
            "{id} exhausted after {} attempts",
            restarts.saturating_add(1)
        ),
        swing,
        recovery,
    })
}

fn rockslide_hazard(level: u8) -> Hazard {
    Hazard {
        id: "rockslide2_obstacle_upass",
        skill: Skill::Agility,
        level,
        low: 160,
        high: 300,
        damage: Damage::Fixed(3),
        timing: Timing {
            first_attempt_before_roll: PreRoll {
                delay_before_walk: 1,
                ..PreRoll::default()
            },
            retry_before_roll: PreRoll::default(),
            failure_before_damage: 1,
            after_failure: 0,
            after_success: 3,
        },
        source: "content/scripts/quests/quest_upass/scripts/upass_obstacles.rs2:19-31",
        timing_note: "p_arrivedelay waits one tick only after movement in the current tick. Failure p_delay(0) adds 1 delayed tick before damage; success agility_exactmove movement_delay=2 calls p_delay(2), adding 3 delayed ticks.",
    }
}
fn rope_setup_ticks(swing_step: &Value) -> Result<u8, SelfDamageError> {
    let tile = swing_step
        .pointer("/args/anchor/tile")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            SelfDamageError::NotModelled("swing-attempt has no guide anchor tile".into())
        })?;
    if tile.len() != 3 || tile[2].as_i64() != Some(0) {
        return Err(SelfDamageError::NotModelled(
            "swing-attempt anchor is not on the modeled rope plane".into(),
        ));
    }
    let x = tile[0]
        .as_i64()
        .ok_or_else(|| SelfDamageError::Input("swing anchor has no x coordinate".into()))?;
    let z = tile[1]
        .as_i64()
        .ok_or_else(|| SelfDamageError::Input("swing anchor has no z coordinate".into()))?;
    let ticks = x
        .abs_diff(ROPE_START_TILE[0])
        .max(z.abs_diff(ROPE_START_TILE[1]));
    u8::try_from(ticks)
        .map_err(|_| SelfDamageError::NotModelled("rope setup walk is too long to model".into()))
}

fn rope_hazard(level: u8, setup_ticks: u8) -> Hazard {
    let (delay_before_walk, delay_after_walk) = (1, 4);
    Hazard {
        id: "swing-attempt",
        skill: Skill::Agility,
        level,
        low: 100,
        high: 410,
        damage: Damage::CurrentPercentPlus {
            percent: 15,
            plus: 1,
        },
        timing: Timing {
            first_attempt_before_roll: PreRoll {
                delay_before_walk,
                walk_ticks: setup_ticks,
                delay_after_walk,
            },
            retry_before_roll: PreRoll {
                delay_before_walk,
                walk_ticks: setup_ticks,
                delay_after_walk,
            },
            failure_before_damage: 2,
            after_failure: 0,
            after_success: 3,
        },
        source: "content/scripts/quests/quest_upass/scripts/upass_obstacles.rs2:93-128",
        timing_note: "Five p_delay ticks precede the roll, with the guide-anchor forcemove walk between the first and later delays. Each walk step is a p_delay(0) inside agility_walk (content/scripts/skill_agility/scripts/agility.rs2:38-43), so every walk tick is delayed and cannot regenerate. Failure p_delay(1) adds 2 delayed ticks before damage; success adds 3 delayed ticks.",
    }
}

fn railing_hazard(level: u8) -> Hazard {
    Hazard {
        id: "mcannonrailing",
        skill: Skill::Crafting,
        level,
        low: 60,
        high: 150,
        damage: Damage::BasePercentCapped { percent: 10, cap: 3 },
        timing: Timing {
            first_attempt_before_roll: PreRoll {
                delay_before_walk: 9,
                ..PreRoll::default()
            },
            retry_before_roll: PreRoll {
                delay_before_walk: 9,
                ..PreRoll::default()
            },
            failure_before_damage: 0,
            after_failure: 0,
            after_success: 3,
        },
        source: "content/scripts/quests/quest_mcannon/scripts/locs/railings.rs2:64-92",
        timing_note: "Three p_delay(2) calls add 9 delayed ticks before each roll. Success p_delay(2) adds 3 delayed ticks; failure damages immediately.",
    }
}

/// Return the exact number of successful byte-roll values in STAT_RANDOM.
fn stat_random_successes(level: u8, low: i32, high: i32) -> u16 {
    let level = i32::from(level.min(99));
    let value = (low * (99 - level)) / 98 + (high * (level - 1)) / 98 + 1;
    value.clamp(0, i32::from(ROLL_BUCKETS)) as u16
}

fn food_facts(
    tables: &CombatTables,
    seed: &Value,
    hitpoints: u8,
) -> Result<FoodFacts, SelfDamageError> {
    let selected = tables.selected();
    let lobster = selected.item_by_alias("lobster").ok_or_else(|| {
        SelfDamageError::NotModelled("revision-289 selected data has no lobster item".into())
    })?;
    let fact = tables.food(lobster.id).ok_or_else(|| {
        SelfDamageError::NotModelled("lobster has no selected fixed-heal food fact".into())
    })?;
    let heal = u8::try_from(fact.heal).map_err(|_| {
        SelfDamageError::NotModelled("lobster heal is outside the model range".into())
    })?;
    let eat_delay = u8::try_from(
        fact.eat_delay_arg
            .ok_or_else(|| SelfDamageError::NotModelled("lobster eat delay is unknown".into()))?
            .saturating_add(1),
    )
    .map_err(|_| {
        SelfDamageError::NotModelled("lobster eat delay is outside the model range".into())
    })?;
    let skill_delay =
        u8::try_from(fact.skill_delay_arg.ok_or_else(|| {
            SelfDamageError::NotModelled("lobster skill delay is unknown".into())
        })?)
        .map_err(|_| {
            SelfDamageError::NotModelled("lobster skill delay is outside the model range".into())
        })?;
    let line = script::combat::select::lines(None, i32::from(hitpoints))
        .eat
        .clamp(0, i32::from(hitpoints)) as u8;
    Ok(FoodFacts {
        policy: FoodPolicy {
            line,
            heal,
            eat_delay,
            skill_delay,
        },
        seed_count: seed_lobsters(seed),
    })
}

fn seed_lobsters(seed: &Value) -> u8 {
    let total = seed
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("alias").and_then(Value::as_str) == Some("lobster"))
        .filter_map(|item| value_u8(item.get("count")))
        .fold(0u16, |sum, count| sum.saturating_add(u16::from(count)));
    total.min(u16::from(u8::MAX)) as u8
}

fn execute(
    plan: &[Operation],
    initial: State,
    base_hp: u8,
    food_policy: Option<FoodPolicy>,
) -> Run {
    let mut distribution = Distribution::new();
    distribution.insert(initial, 1.0);
    let mut run = Run {
        live: distribution,
        ..Run::default()
    };
    for operation in plan {
        if run.live.is_empty() {
            break;
        }
        match operation {
            Operation::Bounded(step) => run_bounded(&mut run, step, base_hp, food_policy),
            Operation::SwingAcquire {
                restarts,
                stop,
                swing,
                recovery,
                ..
            } => run_swing_acquire(
                &mut run,
                *restarts,
                stop,
                *swing,
                recovery,
                base_hp,
                food_policy,
            ),
        }
    }
    run
}

fn run_bounded(run: &mut Run, step: &BoundedStep, base_hp: u8, food_policy: Option<FoodPolicy>) {
    let successes = step.hazard.successes_per_256();
    let chance = f64::from(successes) / f64::from(ROLL_BUCKETS);
    let miss_chance = 1.0 - chance;
    let mut pending = std::mem::take(&mut run.live);
    let mut completed = Distribution::new();
    for attempt in 0..step.attempts {
        let mut next_attempt = Distribution::new();
        for (state, probability) in pending {
            let before = if attempt == 0 {
                step.hazard.timing.first_attempt_before_roll
            } else {
                step.hazard.timing.retry_before_roll
            };
            let ready = advance_pre_roll(state, before, base_hp);
            let success_probability = probability * chance;
            if success_probability > 0.0 {
                let successful = advance(
                    ready,
                    u16::from(step.hazard.timing.after_success),
                    base_hp,
                    TickAccess::Blocked,
                );
                let successful = apply_due_regen(successful, base_hp);
                let successful = food_policy
                    .map(|policy| eat_at_boundary(successful, policy, base_hp))
                    .unwrap_or(successful);
                add_state(&mut completed, successful, success_probability);
            }
            let failure_probability = probability * miss_chance;
            if failure_probability > 0.0 {
                let before_damage = advance(
                    ready,
                    u16::from(step.hazard.timing.failure_before_damage),
                    base_hp,
                    TickAccess::Blocked,
                );
                let damage = step.hazard.damage.amount(before_damage.hp, base_hp);
                if before_damage.hp <= damage {
                    run.death_probability += failure_probability;
                    continue;
                }
                let failed = State {
                    hp: before_damage.hp - damage,
                    ..before_damage
                };
                let failed = advance(
                    failed,
                    u16::from(step.hazard.timing.after_failure),
                    base_hp,
                    TickAccess::Blocked,
                );
                let failed = apply_due_regen(failed, base_hp);
                if attempt + 1 == step.attempts {
                    add_stop(&mut run.budget_stops, &step.stop, failure_probability);
                } else {
                    let failed = food_policy
                        .map(|policy| eat_at_boundary(failed, policy, base_hp))
                        .unwrap_or(failed);
                    add_state(&mut next_attempt, failed, failure_probability);
                }
            }
        }
        pending = next_attempt;
    }
    run.live = completed;
}

fn run_swing_acquire(
    run: &mut Run,
    restarts: u8,
    stop: &str,
    swing: Hazard,
    recovery: &[BoundedStep],
    base_hp: u8,
    food_policy: Option<FoodPolicy>,
) {
    let mut pending = std::mem::take(&mut run.live);
    let mut completed = Distribution::new();
    for pass in 0..=restarts {
        if pending.is_empty() {
            break;
        }
        let branches = run_once(std::mem::take(&mut pending), swing, base_hp);
        run.death_probability += branches.death_probability;
        let mut failed = branches.failure;
        let mut successful = branches.success;
        if let Some(policy) = food_policy {
            apply_food(&mut successful, policy, base_hp);
            apply_food(&mut failed, policy, base_hp);
        }
        merge_states(&mut completed, successful);

        for recovery_step in recovery {
            if failed.is_empty() {
                break;
            }
            let mut child = Run {
                live: failed,
                ..Run::default()
            };
            run_bounded(&mut child, recovery_step, base_hp, food_policy);
            run.death_probability += child.death_probability;
            merge_stops(&mut run.budget_stops, child.budget_stops);
            failed = child.live;
        }

        if pass == restarts {
            for (_, probability) in failed {
                add_stop(&mut run.budget_stops, stop, probability);
            }
        } else {
            pending = failed;
        }
    }
    run.live = completed;
}

fn run_once(input: Distribution, hazard: Hazard, base_hp: u8) -> OnceBranches {
    let chance = f64::from(hazard.successes_per_256()) / f64::from(ROLL_BUCKETS);
    let miss_chance = 1.0 - chance;
    let mut branches = OnceBranches::default();
    for (state, probability) in input {
        let ready = advance_pre_roll(state, hazard.timing.first_attempt_before_roll, base_hp);
        let success_probability = probability * chance;
        if success_probability > 0.0 {
            let successful = advance(
                ready,
                u16::from(hazard.timing.after_success),
                base_hp,
                TickAccess::Blocked,
            );
            add_state(
                &mut branches.success,
                apply_due_regen(successful, base_hp),
                success_probability,
            );
        }
        let failure_probability = probability * miss_chance;
        if failure_probability > 0.0 {
            let before_damage = advance(
                ready,
                u16::from(hazard.timing.failure_before_damage),
                base_hp,
                TickAccess::Blocked,
            );
            let damage = hazard.damage.amount(before_damage.hp, base_hp);
            if before_damage.hp <= damage {
                branches.death_probability += failure_probability;
            } else {
                let failed = advance(
                    State {
                        hp: before_damage.hp - damage,
                        ..before_damage
                    },
                    u16::from(hazard.timing.after_failure),
                    base_hp,
                    TickAccess::Blocked,
                );
                add_state(
                    &mut branches.failure,
                    apply_due_regen(failed, base_hp),
                    failure_probability,
                );
            }
        }
    }
    branches
}

fn advance_pre_roll(state: State, before_roll: PreRoll, base_hp: u8) -> State {
    // Every pre-roll tick is delayed. `P_DELAY` sets `delayed` and suspends the script through
    // the following tick even with a zero argument (`engine/src/engine/script/handlers/PlayerOps.ts:376-380`),
    // and normal timers require `canAccess`, which rejects a delayed player
    // (`engine/src/engine/entity/Player.ts:821-831,945-955`).
    // A forced walk is a chain of `p_delay(0)` steps inside `agility_walk`
    // (`content/scripts/skill_agility/scripts/agility.rs2:18-43`), reached from `forcemove`
    // (`agility.rs2:125-128`, `content/scripts/quests/quest_upass/scripts/upass_obstacles.rs2:102`).
    // The resumed script delays again before timers run (`engine/src/engine/World.ts:692-704`),
    // so the walk ticks cannot regenerate health either.
    let state = advance(
        state,
        u16::from(before_roll.delay_before_walk),
        base_hp,
        TickAccess::Blocked,
    );
    let state = advance(
        state,
        u16::from(before_roll.walk_ticks),
        base_hp,
        TickAccess::Blocked,
    );
    advance(
        state,
        u16::from(before_roll.delay_after_walk),
        base_hp,
        TickAccess::Blocked,
    )
}

fn advance(mut state: State, ticks: u16, base_hp: u8, access: TickAccess) -> State {
    for _ in 0..ticks {
        state.food_cooldown = state.food_cooldown.saturating_sub(1);
        state.regen_phase = state.regen_phase.saturating_add(1).min(REGEN_TICKS);
        if access == TickAccess::Available {
            state = apply_due_regen(state, base_hp);
        }
    }
    state
}

fn apply_due_regen(mut state: State, base_hp: u8) -> State {
    if state.regen_phase == REGEN_TICKS {
        state.hp = state.hp.saturating_add(1).min(base_hp);
        state.regen_phase = 0;
    }
    state
}

fn eat_at_boundary(mut state: State, policy: FoodPolicy, base_hp: u8) -> State {
    while state.food > 0 && state.hp <= policy.line {
        if state.food_cooldown > 0 {
            state = advance(
                state,
                u16::from(state.food_cooldown),
                base_hp,
                TickAccess::Available,
            );
            if state.hp > policy.line {
                break;
            }
        }
        state.food -= 1;
        state.hp = state.hp.saturating_add(policy.heal).min(base_hp);
        state.food_cooldown = policy.eat_delay;
        let wait = if state.hp <= policy.line && state.food > 0 {
            policy.eat_delay.max(policy.skill_delay)
        } else {
            policy.skill_delay
        };
        state = advance(state, u16::from(wait), base_hp, TickAccess::Available);
    }
    state
}

fn apply_food(states: &mut Distribution, policy: FoodPolicy, base_hp: u8) {
    let input = std::mem::take(states);
    for (state, probability) in input {
        add_state(states, eat_at_boundary(state, policy, base_hp), probability);
    }
}

fn add_state(states: &mut Distribution, state: State, probability: f64) {
    *states.entry(state).or_default() += probability;
}

fn merge_states(target: &mut Distribution, source: Distribution) {
    for (state, probability) in source {
        add_state(target, state, probability);
    }
}

fn add_stop(stops: &mut BTreeMap<String, f64>, name: &str, probability: f64) {
    *stops.entry(name.to_owned()).or_default() += probability;
}

fn merge_stops(target: &mut BTreeMap<String, f64>, source: BTreeMap<String, f64>) {
    for (name, probability) in source {
        add_stop(target, &name, probability);
    }
}

fn sum_stops(stops: &BTreeMap<String, f64>) -> f64 {
    stops.values().sum()
}

fn describe_plan(plan: &[Operation]) -> Vec<StepModelResult> {
    let mut output = Vec::new();
    for operation in plan {
        match operation {
            Operation::Bounded(step) => output.push(describe_step(step)),
            Operation::SwingAcquire {
                id,
                restarts,
                swing,
                recovery,
                ..
            } => {
                output.push(StepModelResult {
                    id: format!("{id}/{}", swing.id),
                    skill: swing.skill.name().into(),
                    level: swing.level,
                    successes_per_256: swing.successes_per_256(),
                    attempts_per_entry: restarts.saturating_add(1),
                    entries_per_parent_pass: 1,
                    failure_damage: swing.damage.description(),
                    attempt_ticks: swing.timing_note.into(),
                    source: swing.source.into(),
                });
                for step in recovery {
                    output.push(describe_step(step));
                }
            }
        }
    }
    output
}

fn describe_step(step: &BoundedStep) -> StepModelResult {
    StepModelResult {
        id: step.id.clone(),
        skill: step.hazard.skill.name().into(),
        level: step.hazard.level,
        successes_per_256: step.hazard.successes_per_256(),
        attempts_per_entry: step.attempts,
        entries_per_parent_pass: step.entries_per_parent_pass,
        failure_damage: step.hazard.damage.description(),
        attempt_ticks: step.hazard.timing_note.into(),
        source: step.hazard.source.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_hazard(successes: u16, damage: Damage) -> Hazard {
        Hazard {
            id: "test",
            skill: Skill::Agility,
            level: 1,
            low: 0,
            high: 0,
            damage,
            timing: Timing {
                first_attempt_before_roll: PreRoll::default(),
                retry_before_roll: PreRoll::default(),
                failure_before_damage: 0,
                after_failure: 0,
                after_success: 0,
            },
            source: "test",
            timing_note: "test",
        }
        .with_test_successes(successes)
    }

    impl Hazard {
        fn with_test_successes(mut self, successes: u16) -> Self {
            self.low = i32::from(successes) - 1;
            self.high = i32::from(successes) - 1;
            self
        }
    }

    #[test]
    fn two_step_chain_matches_hand_computed_death_and_budget_stop() {
        let first = BoundedStep {
            id: "first".into(),
            attempts: 1,
            stop: "first-stop".into(),
            hazard: test_hazard(128, Damage::Fixed(1)),
            entries_per_parent_pass: 1,
        };
        let second = BoundedStep {
            id: "second".into(),
            attempts: 1,
            stop: "second-stop".into(),
            hazard: test_hazard(128, Damage::Fixed(2)),
            entries_per_parent_pass: 1,
        };
        let run = execute(
            &[Operation::Bounded(first), Operation::Bounded(second)],
            State {
                hp: 2,
                regen_phase: 0,
                food: 0,
                food_cooldown: 0,
            },
            2,
            None,
        );

        assert!((run.death_probability - 0.25).abs() < 1e-12);
        assert!((run.budget_stops["first-stop"] - 0.5).abs() < 1e-12);
        assert!((run.live.values().sum::<f64>() - 0.25).abs() < 1e-12);
    }

    #[test]
    fn five_attempt_three_damage_rockslide_uses_the_engine_roll() {
        let success_count = stat_random_successes(1, 160, 300);
        assert_eq!(success_count, 161);
        let step = BoundedStep {
            id: "cross-rockslide-1".into(),
            attempts: 5,
            stop: "rockslide-1-attempts-exhausted".into(),
            hazard: rockslide_hazard(1),
            entries_per_parent_pass: 1,
        };
        let run = execute(
            &[Operation::Bounded(step)],
            State {
                hp: 40,
                regen_phase: 0,
                food: 0,
                food_cooldown: 0,
            },
            40,
            None,
        );
        let expected_stop = (95.0_f64 / 256.0).powi(5);
        assert_eq!(run.death_probability, 0.0);
        assert!((run.budget_stops["rockslide-1-attempts-exhausted"] - expected_stop).abs() < 1e-12);
        let state_after_four_fails = State {
            hp: 28,
            regen_phase: 8,
            food: 0,
            food_cooldown: 0,
        };
        let expected_after_four_fails = (95.0_f64 / 256.0).powi(4) * (161.0_f64 / 256.0);
        let actual_after_four_fails = run
            .live
            .get(&state_after_four_fails)
            .copied()
            .unwrap_or_default();
        assert!((actual_after_four_fails - expected_after_four_fails).abs() < 1e-12);
    }

    #[test]
    fn a_due_regen_during_the_rope_setup_walk_cannot_heal_before_the_fall() {
        // `forcemove` walks the player to the rope anchor through `agility_walk`, and every
        // walked tile ends in `p_delay(0)` (`content/scripts/skill_agility/scripts/agility.rs2:40-41`;
        // reached from `content/scripts/quests/quest_upass/scripts/upass_obstacles.rs2:102`).
        // `P_DELAY` keeps the player delayed for the following tick even with a zero argument
        // (`engine/src/engine/script/handlers/PlayerOps.ts:376-380`), and normal timers need
        // `canAccess` (`engine/src/engine/entity/Player.ts:821-831,945-955`), so no walk tick heals.
        let before_roll = PreRoll {
            delay_before_walk: 1,
            walk_ticks: 2,
            delay_after_walk: 4,
        };
        let initial = State {
            hp: 1,
            regen_phase: REGEN_TICKS - 3,
            food: 0,
            food_cooldown: 0,
        };
        let ready = advance_pre_roll(initial, before_roll, 40);
        assert_eq!(
            ready.hp, initial.hp,
            "the rope setup walk must not regenerate health"
        );
        assert_eq!(ready.regen_phase, REGEN_TICKS);

        let swing = rope_hazard(1, 2);
        assert_eq!(swing.timing.first_attempt_before_roll, before_roll);
        let run = execute(
            &[Operation::SwingAcquire {
                id: "swing-rope".into(),
                restarts: 0,
                stop: "swing-rope exhausted".into(),
                swing,
                recovery: Vec::new(),
            }],
            initial,
            40,
            None,
        );
        let miss_chance = 1.0 - f64::from(swing.successes_per_256()) / f64::from(ROLL_BUCKETS);
        assert!(
            (run.death_probability - miss_chance).abs() < 1e-12,
            "a due heal landed before the rope fall: death {} != {miss_chance}",
            run.death_probability
        );
    }

    #[test]
    fn delayed_and_modal_ticks_defer_one_due_regeneration() {
        let initial = State {
            hp: 10,
            regen_phase: 99,
            food: 0,
            food_cooldown: 0,
        };
        let blocked = advance(initial, 300, 20, TickAccess::Blocked);
        assert_eq!(blocked.hp, 10);
        assert_eq!(blocked.regen_phase, REGEN_TICKS);

        let still_blocked = advance(blocked, 1, 20, TickAccess::Blocked);
        assert_eq!(still_blocked.hp, 10);
        assert_eq!(still_blocked.regen_phase, REGEN_TICKS);

        let available = advance(still_blocked, 1, 20, TickAccess::Available);
        assert_eq!(available.hp, 11);
        assert_eq!(available.regen_phase, 0);
    }

    #[test]
    fn a_due_regen_cannot_prevent_death_after_delay_damage() {
        let mut hazard = test_hazard(0, Damage::Fixed(1));
        hazard.timing.failure_before_damage = 1;
        let step = BoundedStep {
            id: "delayed-damage".into(),
            attempts: 1,
            stop: "attempts-exhausted".into(),
            hazard,
            entries_per_parent_pass: 1,
        };
        let run = execute(
            &[Operation::Bounded(step)],
            State {
                hp: 1,
                regen_phase: REGEN_TICKS - 1,
                food: 0,
                food_cooldown: 0,
            },
            2,
            None,
        );
        assert_eq!(run.death_probability, 1.0);
    }

    #[test]
    fn current_upass_attempt_fields_control_the_modeled_budgets() {
        let mut acquire = serde_json::Map::new();
        let mut swing_recipe = vec![serde_json::json!({
            "id": "swing-attempt",
            "args": {
                "item": "rope",
                "anchor": { "tile": [2460, 9699, 0] }
            }
        })];
        for index in 1..=8 {
            let recipe_name = format!("recovery-rockslide-{index}");
            swing_recipe.push(serde_json::json!({
                "id": recipe_name.clone(),
                "args": { "recipe": recipe_name.clone(), "max_restarts": 7 }
            }));
            acquire.insert(
                recipe_name,
                serde_json::json!([{
                    "args": { "target": { "loc": "rockslide2_obstacle_upass" } }
                }]),
            );
        }
        acquire.insert(
            "swing-and-recover".into(),
            serde_json::Value::Array(swing_recipe),
        );
        let guide = serde_json::json!({ "quest": { "acquire": acquire } });
        let parent = serde_json::json!({
            "id": "swing-rope",
            "args": { "max_restarts": 11 }
        });

        let Operation::SwingAcquire {
            restarts,
            swing,
            recovery,
            ..
        } = parse_swing_acquire(&parent, &guide, 1).unwrap()
        else {
            panic!("swing recipe did not produce a swing acquire operation");
        };
        assert_eq!(restarts, 11);
        assert_eq!(swing.successes_per_256(), 101);
        assert_eq!(
            swing.timing.first_attempt_before_roll,
            PreRoll {
                delay_before_walk: 1,
                walk_ticks: 2,
                delay_after_walk: 4,
            }
        );
        assert_eq!(recovery.len(), 8);
        assert!(recovery.iter().all(|step| step.attempts == 8));

        let rockslide = serde_json::json!({
            "id": "cross-rockslide-1",
            "max_attempts": 8,
            "stop": "rockslide-1-attempts-exhausted"
        });
        let rockslide = parse_bounded_step(&rockslide, rockslide_hazard(1), None, 1).unwrap();
        assert_eq!(rockslide.attempts, 8);
        let mut altered_parent = parent;
        altered_parent["args"]["max_restarts"] = serde_json::json!(3);
        let mut altered_guide = guide;
        altered_guide["quest"]["acquire"]["swing-and-recover"][1]["args"]["max_restarts"] =
            serde_json::json!(2);
        let Operation::SwingAcquire {
            restarts, recovery, ..
        } = parse_swing_acquire(&altered_parent, &altered_guide, 1).unwrap()
        else {
            panic!("altered swing recipe did not produce a swing acquire operation");
        };
        assert_eq!(restarts, 3);
        assert_eq!(recovery[0].attempts, 3);
        assert_eq!(recovery[1].attempts, 8);

        let rockslide = serde_json::json!({
            "id": "cross-rockslide-1",
            "max_attempts": 4,
            "stop": "rockslide-1-attempts-exhausted"
        });
        let rockslide = parse_bounded_step(&rockslide, rockslide_hazard(1), None, 1).unwrap();
        assert_eq!(rockslide.attempts, 4);
    }

    #[test]
    fn level_one_crafting_matches_the_61_of_256_guide_fact() {
        assert_eq!(stat_random_successes(1, 60, 150), 61);
    }

    fn sorted_probability_bits(states: &Distribution) -> Vec<((u8, u8, u8, u8), u64)> {
        let mut rows: Vec<((u8, u8, u8, u8), u64)> = states
            .iter()
            .map(|(state, probability)| {
                (
                    (state.hp, state.regen_phase, state.food, state.food_cooldown),
                    probability.to_bits(),
                )
            })
            .collect();
        rows.sort_unstable();
        rows
    }

    fn sorted_stop_bits(stops: &BTreeMap<String, f64>) -> Vec<(String, u64)> {
        stops
            .iter()
            .map(|(name, probability)| (name.clone(), probability.to_bits()))
            .collect()
    }

    #[test]
    fn the_same_model_twice_stores_bit_identical_probabilities() {
        // The railing shape below combines a 9-tick pre-roll delay, a 3-tick success tail and a
        // food policy. Eating folds several states into one key during a merge, so a stored
        // probability is a sum of several terms. Floating-point addition is not associative, and
        // a hash map visits those source states in a per-process order.
        let railing = |index: u8| {
            Operation::Bounded(BoundedStep {
                id: format!("railing-{index}"),
                attempts: 16,
                stop: format!("railing-{index}-exhausted"),
                hazard: Hazard {
                    id: "mcannonrailing",
                    skill: Skill::Crafting,
                    level: 1,
                    low: 0,
                    high: 0,
                    damage: Damage::Fixed(3),
                    timing: Timing {
                        first_attempt_before_roll: PreRoll {
                            delay_before_walk: 9,
                            ..PreRoll::default()
                        },
                        retry_before_roll: PreRoll {
                            delay_before_walk: 9,
                            ..PreRoll::default()
                        },
                        failure_before_damage: 0,
                        after_failure: 0,
                        after_success: 3,
                    },
                    source: "test",
                    timing_note: "test",
                }
                .with_test_successes(61),
                entries_per_parent_pass: 1,
            })
        };
        let plan: Vec<Operation> = (1..=4u8).map(railing).collect();
        let policy = FoodPolicy {
            line: 20,
            heal: 12,
            eat_delay: 2,
            skill_delay: 3,
        };
        let initial = State {
            hp: 40,
            regen_phase: 0,
            food: 8,
            food_cooldown: 0,
        };

        let first = execute(&plan, initial, 40, Some(policy));
        let second = execute(&plan, initial, 40, Some(policy));

        assert_eq!(
            first.death_probability.to_bits(),
            second.death_probability.to_bits(),
            "death probability changed between two runs of the same model"
        );
        assert_eq!(
            sorted_probability_bits(&first.live),
            sorted_probability_bits(&second.live),
            "stored probabilities changed between two runs of the same model"
        );
        assert_eq!(
            sorted_stop_bits(&first.budget_stops),
            sorted_stop_bits(&second.budget_stops),
            "budget stop probabilities changed between two runs of the same model"
        );
    }
}
