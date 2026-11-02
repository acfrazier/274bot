//! Finite-state absorbing fight model with product-backed food decisions.
use crate::food;
use std::collections::HashMap;
use std::path::PathBuf;

use api::game_data::{NpcNameRow, SelectedGameData};
use script::combat::risk::KindRisk;
use script::combat::select::facts::npc_max_hit;
use script::combat::CombatTables;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FightError {
    #[error("cannot read fight facts: {0}")]
    Io(#[from] std::io::Error),
    #[error("cannot decode fight facts: {0}")]
    Json(#[from] serde_json::Error),
    #[error("survival state graph exceeded {limit} states after reaching {states}")]
    StateLimit { limit: usize, states: usize },
    #[error("{0}")]
    Input(String),
    #[error("{0}")]
    NotModelled(String),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Selector {
    Id(i32),
    Name(String),
}
impl Selector {
    fn matches(&self, id: i32, alias: &str, display: Option<&str>) -> bool {
        match self {
            Self::Id(wanted) => *wanted == id,
            Self::Name(wanted) => {
                wanted.eq_ignore_ascii_case(alias)
                    || display.is_some_and(|name| wanted.eq_ignore_ascii_case(name))
            }
        }
    }
}
fn default_weapon() -> Option<Selector> {
    Some(Selector::Name("bronze_scimitar".into()))
}
fn one() -> u8 {
    1
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Style {
    #[default]
    Accurate,
    Aggressive,
    Defensive,
    Controlled,
    Ranged,
    Magic,
}
impl Style {
    fn bonuses(self) -> [u32; 3] {
        match self {
            Self::Accurate => [3, 0, 0],
            Self::Aggressive => [0, 3, 0],
            Self::Defensive => [0, 0, 3],
            Self::Controlled => [1, 1, 1],
            Self::Ranged | Self::Magic => [0; 3],
        }
    }
    fn code(self) -> u8 {
        match self {
            Self::Accurate => 0,
            Self::Aggressive => 1,
            Self::Controlled => 2,
            Self::Defensive => 3,
            Self::Ranged | Self::Magic => 0,
        }
    }
    fn melee(self) -> bool {
        !matches!(self, Self::Ranged | Self::Magic)
    }
}
fn base40() -> u8 {
    40
}
fn default_free_slots() -> u8 {
    28
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Stats {
    pub attack: u8,
    pub strength: u8,
    pub defence: u8,
    pub hitpoints: u8,
    #[serde(default = "base40")]
    pub magic: u8,
    #[serde(default = "base40")]
    pub ranged: u8,
}
impl Stats {
    fn policy(level: u8, style: Style) -> Self {
        Self {
            attack: level,
            strength: level,
            defence: level,
            hitpoints: level.max(10),
            magic: 40,
            ranged: if style == Style::Ranged { 50 } else { 40 },
        }
    }
    fn with_raise(mut self, stat: CombatStat, level: u8) -> Self {
        match stat {
            CombatStat::Attack => self.attack = level,
            CombatStat::Strength => self.strength = level,
            CombatStat::Defence => self.defence = level,
            CombatStat::Hitpoints => self.hitpoints = level,
        }
        self
    }
    fn valid(self) -> bool {
        [
            self.attack,
            self.strength,
            self.defence,
            self.hitpoints,
            self.magic,
            self.ranged,
        ]
        .iter()
        .all(|v| (1..=99).contains(v))
    }
}

#[derive(Clone, Copy)]
enum CombatStat {
    Attack,
    Strength,
    Defence,
    Hitpoints,
}

impl CombatStat {
    const POLICY_ORDER: [Self; 4] = [Self::Attack, Self::Strength, Self::Defence, Self::Hitpoints];

    fn name(self) -> &'static str {
        match self {
            Self::Attack => "Attack",
            Self::Strength => "Strength",
            Self::Defence => "Defence",
            Self::Hitpoints => "Hitpoints",
        }
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FightQuery {
    #[serde(default)]
    pub label: Option<String>,
    pub npc: Selector,
    pub facts: PathBuf,
    #[serde(default = "default_weapon")]
    pub weapon: Option<Selector>,
    #[serde(default)]
    pub style: Style,
    #[serde(default)]
    pub worn_items: Vec<Selector>,
    #[serde(default)]
    pub food: Option<Selector>,
    #[serde(default)]
    pub ammunition: Option<Selector>,
    #[serde(default)]
    pub spell: Option<String>,
    /// Maximum available count to search; each food occupies one free slot.
    #[serde(default)]
    pub food_count: u8,
    #[serde(default = "default_free_slots", alias = "free_inventory_slots")]
    pub free_slots: u8,
    /// Omit candidate to request only the minimum-policy search.
    #[serde(default)]
    pub candidate: Option<Stats>,
    /// One NPC only. Larger counts are explicitly not modelled.
    #[serde(default = "one")]
    pub npc_count: u8,
    /// Consecutive single-NPC fights, carrying HP, food and regeneration.
    #[serde(default = "one")]
    pub sequential_kills: u8,
    /// Default assumes an aggressive NPC ready at tick zero. Player damage lands at tick one.
    #[serde(default)]
    pub player_initiates: bool,
}
#[derive(Debug, Deserialize)]
struct Facts {
    revision: i32,
    npcs: Vec<NpcRoll>,
    weapons: Vec<Weapon>,
    #[serde(default)]
    items: Vec<ItemFact>,
    #[serde(default)]
    foods: Vec<FoodSource>,
}
#[derive(Debug, Deserialize)]
struct FoodSource {
    id: i32,
    alias: String,
    source: String,
}
#[derive(Debug, Deserialize)]
struct NpcRoll {
    id: i32,
    config: String,
    attack: u32,
    defence: u32,
    attack_bonus: i32,
    defence_bonus: [i32; 3],
    hitpoints: i32,
    strength: i32,
    strength_bonus: i32,
    attackrate: u8,
    source: String,
}
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum WeaponKind {
    Melee,
    Ranged,
}
#[derive(Debug, Deserialize)]
struct WearRequirement {
    attack: u8,
    strength: u8,
    defence: u8,
    ranged: u8,
    magic: u8,
    quest_gated: bool,
}
#[derive(Debug, Deserialize)]
struct ItemFact {
    id: i32,
    config: String,
    attack_bonus: [i32; 5],
    defence_bonus: [i32; 5],
    strength_bonus: i32,
    range_bonus: i32,
    wear: WearRequirement,
    source: String,
    wear_source: String,
}
#[derive(Debug, Deserialize)]
struct Weapon {
    id: Option<i32>,
    config: String,
    kind: WeaponKind,
    attack_bonus: [i32; 5],
    defence_bonus: [i32; 5],
    strength_bonus: i32,
    range_bonus: i32,
    attackrate: u8,
    required_attack: u8,
    required_ranged: u8,
    /// One entry per supported semantic melee mode. Each value is stab=0, slash=1, crush=2.
    damage_types: [Option<usize>; 4],
    poisoned: bool,
    source: String,
    wear_source: String,
}
fn unarmed_weapon() -> Weapon {
    Weapon {
        id: None,
        config: "unarmed".into(),
        kind: WeaponKind::Melee,
        attack_bonus: [0; 5],
        defence_bonus: [0; 5],
        strength_bonus: 0,
        range_bonus: 0,
        attackrate: 4,
        required_attack: 1,
        required_ranged: 1,
        damage_types: [Some(2), Some(2), None, Some(2)],
        poisoned: false,
        source: "content/scripts/skill_combat/scripts/combat.rs2:14-39,63-71; content/scripts/skill_combat/configs/combat.dbrow:140-148; content/scripts/player/scripts/equip.rs2:244-285; content/scripts/skill_combat/scripts/player/player_melee.rs2:52-57".into(),
        wear_source: String::new(),
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Outcome {
    pub win_probability: f64,
    pub death_probability: f64,
    pub expected_ticks: f64,
    pub explored_states: usize,
}
#[derive(Debug, Serialize)]
pub struct Evaluation {
    pub stats: Stats,
    pub food_count: u8,
    pub outcome: Outcome,
    pub player_hit_chance: f64,
    pub npc_hit_chance: f64,
    pub player_max_hit: u16,
    pub npc_max_hit: u16,
    pub player_interval: u8,
    pub npc_interval: u8,
    pub equipment_attack_bonus: [i32; 3],
    pub equipment_defence_bonus: [i32; 3],
    pub equipment_strength_bonus: i32,
    pub equipment_range_bonus: i32,
}
#[derive(Debug, Serialize)]
pub struct FightResult {
    pub label: Option<String>,
    pub status: &'static str,
    pub npc_id: i32,
    pub npc: String,
    pub weapon: String,
    pub worn_items: Vec<String>,
    pub food: Option<String>,
    pub ammunition: Option<String>,
    pub spell: Option<String>,
    pub food_search_limit: u8,
    pub free_slots: u8,
    pub selected_food_count: Option<u8>,
    pub accepted_death_bound: Option<&'static str>,
    pub style: Style,
    pub sequential_kills: u8,
    pub poison_changes_result: &'static str,
    pub reasons: Vec<String>,
    pub candidate: Option<Evaluation>,
    pub minimum: Option<Evaluation>,
    pub previous_candidate: Option<Evaluation>,
    pub max_explored_states: usize,
    pub policy_reason: String,
    pub sources: Vec<String>,
    pub timing: &'static str,
}

fn unsupported(
    row: &NpcNameRow,
    tables: &CombatTables,
    count: u8,
    weapon: &Weapon,
    style: Style,
) -> Vec<String> {
    let mut reasons = Vec::new();
    let risk = KindRisk::from_npc(row, tables);
    if count != 1 {
        reasons.push("several NPCs in one fight need calibration".into());
    }
    if row.bespoke {
        reasons.push("NPC risk facts flag bespoke combat".into());
    }
    if npc_max_hit(row).is_none() {
        reasons.push("unknown NPC maximum hit".into());
    }
    if row.dragonfire.is_some() {
        reasons.push("dragonfire needs calibration".into());
    }
    if row.forced_max_hit.is_some() || row.counter_protect {
        reasons.push("special attack needs calibration".into());
    }
    if row.attack_kind.is_some() {
        reasons.push(
            "ranged projectile flight/posthumous hits or magic spell effects are not modelled"
                .into(),
        );
    }
    if style == Style::Ranged || weapon.kind == WeaponKind::Ranged {
        reasons.push("player ranged projectile flight and queued damage are not modelled".into());
    }
    if style == Style::Magic {
        reasons.push("player spell-dependent damage, effects, and timing are not modelled".into());
    }
    if row.poison_severity != Some(0) || weapon.poisoned {
        reasons.push("poison changes survival and timing; its probability is not modelled".into());
    }
    if let Some(unknown) = risk.unknown_for() {
        reasons.push(format!("product risk class: {unknown:?}"));
    }
    reasons
}

fn display_name(selected: &SelectedGameData, id: i32) -> Option<&str> {
    selected
        .items()
        .iter()
        .find(|item| item.id == id)
        .and_then(|item| item.name.as_deref())
}

fn resolve_item<'a>(
    selected: &SelectedGameData,
    facts: &'a [ItemFact],
    selector: &Selector,
) -> Result<&'a ItemFact, FightError> {
    let matches: Vec<_> = facts
        .iter()
        .filter(|item| selector.matches(item.id, &item.config, display_name(selected, item.id)))
        .collect();
    match matches.as_slice() {
        [item] => Ok(*item),
        [] => Err(FightError::Input(
            "worn item missing from source supplement".into(),
        )),
        _ => Err(FightError::Input(
            "worn item selector is ambiguous; use an id".into(),
        )),
    }
}
fn resolve_selected_ammunition<'a>(
    selected: &'a SelectedGameData,
    selector: &Selector,
) -> Result<&'a api::game_data::GameItem, FightError> {
    let mut matches = selected.items().iter().filter(|item| {
        selector.matches(
            item.id,
            item.alias.as_deref().unwrap_or(""),
            item.name.as_deref(),
        )
    });
    let Some(item) = matches.next() else {
        return Err(FightError::Input("ammunition item not found".into()));
    };
    if matches.next().is_some() {
        return Err(FightError::Input(
            "ammunition selector is ambiguous; use an id".into(),
        ));
    }
    Ok(item)
}

pub fn calculate(query: &FightQuery, tables: &CombatTables) -> Result<FightResult, FightError> {
    if query.sequential_kills == 0 {
        return Err(FightError::Input(
            "sequential_kills must be positive".into(),
        ));
    }
    if query.free_slots > 28 || query.food_count > query.free_slots {
        return Err(FightError::Input(
            "food_count must fit within the 28-slot inventory and declared free_slots".into(),
        ));
    }
    if query.food_count > 0 && query.food.is_none() {
        return Err(FightError::Input(
            "food_count requires one selected food item".into(),
        ));
    }
    let selected = tables.selected();
    let rows = &selected
        .npc_names()
        .ok_or_else(|| FightError::Input("NPC facts unavailable".into()))?
        .rows;
    let matches: Vec<_> = rows
        .iter()
        .filter(|row| {
            query
                .npc
                .matches(row.id, &row.config, row.display.as_deref())
        })
        .collect();
    let row = match matches.as_slice() {
        [row] => *row,
        [] => return Err(FightError::Input("NPC not found".into())),
        _ => {
            return Err(FightError::Input(format!(
                "NPC name is ambiguous; use one of ids {:?}",
                matches.iter().map(|r| r.id).collect::<Vec<_>>()
            )))
        }
    };
    let facts: Facts = serde_json::from_reader(std::fs::File::open(&query.facts)?)?;
    if facts.revision != 289 {
        return Err(FightError::Input(
            "supplement must describe revision 289".into(),
        ));
    }
    let unarmed = unarmed_weapon();
    let weapon = if let Some(selector) = &query.weapon {
        let weapons: Vec<_> = facts
            .weapons
            .iter()
            .filter(|weapon| {
                weapon.id.is_some_and(|id| {
                    selector.matches(id, &weapon.config, display_name(selected, id))
                })
            })
            .collect();
        let weapon = match weapons.as_slice() {
            [weapon] => *weapon,
            _ => {
                return Err(FightError::Input(
                    "weapon missing or ambiguous in source supplement".into(),
                ))
            }
        };
        let weapon_id = weapon
            .id
            .ok_or_else(|| FightError::Input("weapon fact has no item id".into()))?;
        let weapon_item = resolve_item(selected, &facts.items, &Selector::Id(weapon_id))?;
        if weapon_item.attack_bonus != weapon.attack_bonus[..5]
            || weapon_item.defence_bonus != weapon.defence_bonus[..5]
            || weapon_item.strength_bonus != weapon.strength_bonus
            || weapon_item.range_bonus != weapon.range_bonus
        {
            return Err(FightError::Input(
                "weapon roll supplement disagrees with its source item facts".into(),
            ));
        }
        weapon
    } else {
        &unarmed
    };
    if (query.style == Style::Ranged && weapon.kind != WeaponKind::Ranged)
        || (query.style.melee() && weapon.kind != WeaponKind::Melee)
    {
        return Err(FightError::Input(
            "weapon kind does not match requested combat style".into(),
        ));
    }
    let mut worn = Vec::with_capacity(query.worn_items.len());
    for selector in &query.worn_items {
        let item = resolve_item(selected, &facts.items, selector)?;
        if weapon.id == Some(item.id) || worn.iter().any(|known: &&ItemFact| known.id == item.id) {
            return Err(FightError::Input(
                "worn_items must not repeat the weapon or another worn item".into(),
            ));
        }
        if selected.item_combat_bonus(item.id).is_some_and(|bonus| {
            bonus.strengthbonus != item.strength_bonus || bonus.rangebonus != item.range_bonus
        }) {
            return Err(FightError::Input(
                "worn item strength or ranged bonus disagrees with selected facts".into(),
            ));
        }
        worn.push(item);
    }
    let food_item = if let Some(selector) = &query.food {
        let matches: Vec<_> = selected
            .items()
            .iter()
            .filter(|item| {
                selector.matches(
                    item.id,
                    item.alias.as_deref().unwrap_or(""),
                    item.name.as_deref(),
                )
            })
            .collect();
        match matches.as_slice() {
            [item] => Some(*item),
            [] => return Err(FightError::Input("food item not found".into())),
            _ => {
                return Err(FightError::Input(
                    "food selector is ambiguous; use an id".into(),
                ))
            }
        }
    } else {
        None
    };
    let ammunition_item = query
        .ammunition
        .as_ref()
        .map(|selector| resolve_selected_ammunition(selected, selector))
        .transpose()?;
    let mut reasons = unsupported(row, tables, query.npc_count, weapon, query.style);
    if (query.style == Style::Ranged && weapon.kind != WeaponKind::Ranged)
        || (query.style.melee() && weapon.kind != WeaponKind::Melee)
    {
        return Err(FightError::Input(
            "weapon kind does not match requested combat style".into(),
        ));
    }
    if query.style != Style::Magic {
        validate_weapon(selected, weapon, query.style)?;
        validate_wear(Stats::policy(40, query.style), weapon, &worn)?;
    }
    if let Some(food) = food_item {
        if tables.food(food.id).is_none() {
            return Err(FightError::Input(format!(
                "{} is not recognized by product food facts",
                food.alias.as_deref().unwrap_or("selected food")
            )));
        }
    }
    let food_source = if let Some(food) = food_item {
        let alias = food.alias.as_deref().ok_or_else(|| {
            FightError::Input("selected food has no content alias for source validation".into())
        })?;
        Some(
            facts
                .foods
                .iter()
                .find(|source| source.id == food.id && source.alias == alias)
                .ok_or_else(|| {
                    FightError::Input("food source row missing from source supplement".into())
                })?,
        )
    } else {
        None
    };
    let mut result = FightResult {
        label: query.label.clone(),
        status: if reasons.is_empty() {
            "modelled"
        } else {
            "not_modelled"
        },
        npc_id: row.id,
        npc: row.config.clone(),
        weapon: weapon.config.clone(),
        worn_items: worn.iter().map(|item| item.config.clone()).collect(),
        food: food_item.and_then(|item| item.alias.clone()),
        ammunition: ammunition_item.and_then(|item| item.alias.clone()),
        spell: query.spell.clone(),
        food_search_limit: query.food_count,
        free_slots: query.free_slots,
        selected_food_count: None,
        accepted_death_bound: None,
        style: query.style,
        sequential_kills: query.sequential_kills,
        poison_changes_result: if row.poison_severity == Some(0) && !weapon.poisoned {
            "no: neither participant inflicts poison"
        } else {
            "yes: poison can change survival; calibration needed"
        },
        reasons: std::mem::take(&mut reasons),
        candidate: None,
        minimum: None,
        previous_candidate: None,
        max_explored_states: 0,
        policy_reason: "not evaluated because combat is outside the modeled scope".into(),
        sources: vec![
            weapon.source.clone(),
            "content/scripts/skill_combat/scripts/player/player_combat_stat.rs2:62-100".into(),
            "content/scripts/skill_combat/scripts/player/player_combat.rs2:52-85".into(),
            "content/scripts/skill_combat/scripts/player/player_melee.rs2:18-28,46-57".into(),
            "content/scripts/skill_combat/scripts/npc/npc_combat_melee.rs2:2-48".into(),
            "content/scripts/skill_combat/scripts/npc/npc_combat.rs2:7-24,315-343,391-428".into(),
            "content/scripts/login_logout/login.rs2:34-38".into(),
            "content/scripts/player/scripts/stat.rs2:58-60".into(),
        ],
        timing: if query.player_initiates {
            "player swing tick 0; queued damage lands one tick later; NPC first attacks at 1 + floor(interval/2); eat action adds its content skill delay to the next player action; HP regeneration ticks every 100"
        } else {
            "NPC and player are ready at tick 0; queued player damage lands one tick later; NPC damage resolves before food and player actions; eat action adds its content skill delay to the next player action; HP regeneration ticks every 100"
        },
    };
    for item in &worn {
        result.sources.push(item.source.clone());
        if !item.wear_source.is_empty() {
            result.sources.push(item.wear_source.clone());
        }
    }
    if !weapon.wear_source.is_empty() {
        result.sources.push(weapon.wear_source.clone());
    }
    if food_item.is_some() {
        if let Some(source) = food_source {
            result.sources.push(source.source.clone());
        }
        result
            .sources
            .push("content/scripts/player/scripts/consumption/consume.rs2:114-134".into());
        result.sources.push(
            "content/scripts/player/scripts/consumption/effects/scripts/consume_effects.rs2:39-74"
                .into(),
        );
        result
            .sources
            .push("crates/script/src/combat/guard.rs:470-483".into());
        result
            .sources
            .push("crates/script/src/combat/policy.rs:333-386".into());
        result
            .sources
            .push("crates/script/src/combat/schedule.rs:85-87,118-131".into());
    }
    let npc = facts
        .npcs
        .iter()
        .find(|n| n.id == row.id && n.config == row.config)
        .ok_or_else(|| FightError::Input("NPC roll supplement missing".into()))?;
    if npc.hitpoints != row.hitpoints
        || Some(npc.strength) != row.strength
        || Some(npc.strength_bonus) != row.strengthbonus
        || npc.attackrate != tables.npc_rate(row)
    {
        return Err(FightError::Input(
            "source supplement disagrees with selected NPC facts".into(),
        ));
    }
    result.sources.push(npc.source.clone());
    if !result.reasons.is_empty() {
        return Ok(result);
    }
    let setup = EvaluationSetup {
        row,
        npc,
        weapon,
        worn: &worn,
        style: query.style,
        initiates: query.player_initiates,
        sequential_kills: query.sequential_kills,
    };
    if let Some(stats) = query.candidate {
        let candidate_food = None;
        match evaluate(stats, &setup, candidate_food, 0) {
            Ok(candidate) => {
                result.max_explored_states = result
                    .max_explored_states
                    .max(candidate.outcome.explored_states);
                result.candidate = Some(candidate);
            }
            Err(error @ FightError::StateLimit { states, .. }) => {
                result.max_explored_states = result.max_explored_states.max(states);
                result
                    .reasons
                    .push(format!("candidate comparison not modelled: {error}"));
            }
            Err(FightError::NotModelled(reason)) => {
                result
                    .reasons
                    .push(format!("candidate comparison not modelled: {reason}"));
            }
            Err(error) => return Err(error),
        }
    }
    let food_limit = if food_item.is_some() {
        query.food_count
    } else {
        0
    };
    let policy = search_policy(query.style, |stats| {
        validate_wear(stats, weapon, &worn)?;
        let food_rule = food_item
            .map(|food| food::build(tables, row, food, stats.hitpoints))
            .transpose()
            .map_err(FightError::Input)?;
        search_food_candidates(stats, food_limit, |stats, food_count| {
            evaluate(stats, &setup, food_rule.as_ref(), food_count)
        })
    });
    let policy = match policy {
        Ok(policy) => policy,
        Err(error @ FightError::StateLimit { states, .. }) => {
            result.max_explored_states = result.max_explored_states.max(states);
            result.status = "not_modelled";
            result.reasons.push(error.to_string());
            result.policy_reason = "Policy C search stopped at the survival state limit.".into();
            return Ok(result);
        }
        Err(FightError::NotModelled(reason)) => {
            result.status = "not_modelled";
            result.reasons.push(reason);
            result.policy_reason = "Policy C search could not be completed.".into();
            return Ok(result);
        }
        Err(error) => return Err(error),
    };
    result.max_explored_states = result.max_explored_states.max(policy.max_explored_states);
    result.previous_candidate = policy.previous_candidate;
    result.policy_reason = policy.reason;
    if let Some((evaluation, bound)) = policy.accepted {
        result.selected_food_count = Some(evaluation.food_count);
        result.accepted_death_bound = Some(bound);
        result.minimum = Some(evaluation);
    } else {
        result.status = "no_seed_within_policy";
    }
    Ok(result)
}

struct FoodSearch {
    accepted: Option<(Evaluation, &'static str)>,
    previous_candidate: Option<Evaluation>,
    max_explored_states: usize,
}

fn search_food_candidates(
    stats: Stats,
    food_limit: u8,
    mut evaluate: impl FnMut(Stats, u8) -> Result<Evaluation, FightError>,
) -> Result<FoodSearch, FightError> {
    let mut within_five = None;
    let mut previous_candidate: Option<Evaluation> = None;
    let mut max_explored_states = 0;
    for food_count in 0..=food_limit {
        let evaluation = evaluate(stats, food_count)?;
        max_explored_states = max_explored_states.max(evaluation.outcome.explored_states);
        let death_probability = evaluation.outcome.death_probability;
        if death_probability <= 0.01 {
            return Ok(FoodSearch {
                accepted: Some((evaluation, "1%")),
                previous_candidate,
                max_explored_states,
            });
        }
        if death_probability <= 0.05 {
            if within_five.is_none() {
                within_five = Some(evaluation);
            }
            continue;
        }
        if previous_candidate
            .as_ref()
            .is_none_or(|current| death_probability < current.outcome.death_probability)
        {
            previous_candidate = Some(evaluation);
        }
    }
    if let Some(evaluation) = within_five {
        return Ok(FoodSearch {
            accepted: Some((evaluation, "5%")),
            previous_candidate,
            max_explored_states,
        });
    }
    Ok(FoodSearch {
        accepted: None,
        previous_candidate,
        max_explored_states,
    })
}

struct PolicySearch {
    accepted: Option<(Evaluation, &'static str)>,
    previous_candidate: Option<Evaluation>,
    max_explored_states: usize,
    reason: String,
}

fn search_policy(
    style: Style,
    mut search: impl FnMut(Stats) -> Result<FoodSearch, FightError>,
) -> Result<PolicySearch, FightError> {
    let base = search(Stats::policy(40, style))?;
    let mut max_explored_states = base.max_explored_states;
    let mut previous_candidate = base.previous_candidate;
    if let Some((evaluation, bound)) = base.accepted {
        return Ok(PolicySearch {
            accepted: Some((evaluation, bound)),
            previous_candidate,
            max_explored_states,
            reason: format!("Policy C: base-40 stats meet {bound} without a stat raise."),
        });
    }

    for level in 41..=99 {
        for stat in CombatStat::POLICY_ORDER {
            let candidate = search(Stats::policy(40, style).with_raise(stat, level))?;
            max_explored_states = max_explored_states.max(candidate.max_explored_states);
            previous_candidate = candidate.previous_candidate;
            if let Some((evaluation, bound)) = candidate.accepted {
                return Ok(PolicySearch {
                    accepted: Some((evaluation, bound)),
                    previous_candidate,
                    max_explored_states,
                    reason: format!(
                        "Policy C: no base-40 food count met 5%; {} alone raised to {level} is the smallest single-stat raise that meets {bound}.",
                        stat.name()
                    ),
                });
            }
        }
    }

    for level in 41..=99 {
        let candidate = search(Stats::policy(level, style))?;
        max_explored_states = max_explored_states.max(candidate.max_explored_states);
        previous_candidate = candidate.previous_candidate;
        if let Some((evaluation, bound)) = candidate.accepted {
            return Ok(PolicySearch {
                accepted: Some((evaluation, bound)),
                previous_candidate,
                max_explored_states,
                reason: format!(
                    "Policy C: no base-40 or single-stat food search met 5%; the combined combat-stat raise to {level} is the first that meets {bound}."
                ),
            });
        }
    }

    Ok(PolicySearch {
        accepted: None,
        previous_candidate,
        max_explored_states,
        reason: "Policy C: no base-40, single-stat, or combined candidate through 99 met 5%."
            .into(),
    })
}

fn validate_weapon(
    selected: &SelectedGameData,
    weapon: &Weapon,
    style: Style,
) -> Result<(), FightError> {
    let Some(id) = weapon.id else {
        if !style.melee()
            || style == Style::Controlled
            || weapon.damage_types[usize::from(style.code())].is_none()
        {
            return Err(FightError::Input(
                "unarmed attacks support accurate, aggressive, and defensive styles only".into(),
            ));
        }
        return Ok(());
    };
    if !selected
        .items()
        .iter()
        .any(|item| item.id == id && item.alias.as_deref() == Some(&weapon.config))
    {
        return Err(FightError::Input(
            "weapon supplement disagrees with selected item identity".into(),
        ));
    }
    if let Some(bonus) = selected.item_combat_bonus(id) {
        if bonus.strengthbonus != weapon.strength_bonus || bonus.rangebonus != weapon.range_bonus {
            return Err(FightError::Input(
                "weapon strength or ranged bonus disagrees with selected facts".into(),
            ));
        }
    }
    match weapon.kind {
        WeaponKind::Melee => {
            if !style.melee() {
                return Err(FightError::Input(
                    "melee weapon does not match requested combat style".into(),
                ));
            }
            let tier = selected
                .melee_weapon_tier(id)
                .ok_or_else(|| FightError::Input("selected weapon requirement missing".into()))?;
            if tier.quest_gated
                || tier.strength > 0
                || weapon.required_attack != tier.tier.max(1)
                || weapon
                    .damage_types
                    .iter()
                    .flatten()
                    .any(|damage_type| *damage_type > 2)
            {
                return Err(FightError::Input(
                    "weapon requirement or damage type disagrees with generic melee model".into(),
                ));
            }
            let style_fact = selected
                .weapon_styles()
                .iter()
                .find(|fact| fact.obj_id == id && fact.category == 0)
                .ok_or_else(|| FightError::Input("selected melee weapon style missing".into()))?;
            if style_fact.attackrate != weapon.attackrate
                || style_fact.style != 1
                || weapon.damage_types[usize::from(style.code())].is_none()
                || !selected
                    .melee_modes()
                    .iter()
                    .any(|fact| Some(fact.tab) == style_fact.tab && fact.mode == style.code())
            {
                return Err(FightError::Input(
                    "weapon does not offer requested melee style or interval disagrees".into(),
                ));
            }
        }
        WeaponKind::Ranged => {
            if style != Style::Ranged {
                return Err(FightError::Input(
                    "ranged weapon requires the ranged combat style".into(),
                ));
            }
            let fact = selected
                .ranged_weapons()
                .iter()
                .find(|fact| fact.obj_id == id)
                .ok_or_else(|| FightError::Input("selected ranged weapon fact missing".into()))?;
            let style = selected
                .weapon_styles()
                .iter()
                .find(|fact| fact.obj_id == id && fact.category == 1)
                .ok_or_else(|| FightError::Input("selected ranged weapon style missing".into()))?;
            if fact.levelrequire != weapon.required_ranged || style.attackrate != weapon.attackrate
            {
                return Err(FightError::Input(
                    "ranged weapon requirement or interval disagrees with selected facts".into(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_wear(stats: Stats, weapon: &Weapon, worn: &[&ItemFact]) -> Result<(), FightError> {
    if !stats.valid() {
        return Err(FightError::Input(
            "combat stats must be between 1 and 99".into(),
        ));
    }
    if stats.attack < weapon.required_attack || stats.ranged < weapon.required_ranged {
        return Err(FightError::Input(format!(
            "stats cannot wield {} (Attack {}, Ranged {})",
            weapon.config, weapon.required_attack, weapon.required_ranged
        )));
    }
    for item in worn {
        let requirement = &item.wear;
        if requirement.quest_gated {
            return Err(FightError::Input(format!(
                "{} has an unmet quest-gated wear requirement",
                item.config
            )));
        }
        if stats.attack < requirement.attack
            || stats.strength < requirement.strength
            || stats.defence < requirement.defence
            || stats.ranged < requirement.ranged
            || stats.magic < requirement.magic
        {
            return Err(FightError::Input(format!(
                "stats cannot wear {} (Attack {}, Strength {}, Defence {}, Ranged {}, Magic {})",
                item.config,
                requirement.attack,
                requirement.strength,
                requirement.defence,
                requirement.ranged,
                requirement.magic
            )));
        }
    }
    Ok(())
}

fn roll(stat: u32, bonus: i32) -> Result<u32, FightError> {
    let value = i64::from(stat) * (i64::from(bonus) + 64);
    u32::try_from(value)
        .map_err(|_| FightError::Input("negative or overflowing combat roll".into()))
}

struct EvaluationSetup<'a> {
    row: &'a NpcNameRow,
    npc: &'a NpcRoll,
    weapon: &'a Weapon,
    worn: &'a [&'a ItemFact],
    style: Style,
    initiates: bool,
    sequential_kills: u8,
}

fn evaluate(
    stats: Stats,
    setup: &EvaluationSetup<'_>,
    food_rule: Option<&food::FoodRule>,
    food_count: u8,
) -> Result<Evaluation, FightError> {
    validate_wear(stats, setup.weapon, setup.worn)?;
    if !setup.style.melee() {
        return Err(FightError::Input(
            "ranged and magic attacks are not modelled".into(),
        ));
    }
    if food_count > 0 && food_rule.is_none() {
        return Err(FightError::Input(
            "food count has no product-backed food rule".into(),
        ));
    }
    let [attack, strength, defence] = setup.style.bonuses();
    let damage_type = setup.weapon.damage_types[usize::from(setup.style.code())]
        .ok_or_else(|| FightError::Input("style unavailable".into()))?;
    let npc_type = match setup.row.damagetype.as_deref() {
        Some("^stab_style") => 0,
        Some("^slash_style") => 1,
        Some("^crush_style") => 2,
        _ => {
            return Err(FightError::Input(
                "NPC melee damage type unavailable".into(),
            ))
        }
    };
    let mut equipment_attack_bonus = [0i32; 3];
    let mut equipment_defence_bonus = [0i32; 3];
    for damage in 0..3 {
        equipment_attack_bonus[damage] = setup.weapon.attack_bonus[damage]
            + setup
                .worn
                .iter()
                .map(|item| item.attack_bonus[damage])
                .sum::<i32>();
        equipment_defence_bonus[damage] = setup.weapon.defence_bonus[damage]
            + setup
                .worn
                .iter()
                .map(|item| item.defence_bonus[damage])
                .sum::<i32>();
    }
    let equipment_strength_bonus = setup.weapon.strength_bonus
        + setup
            .worn
            .iter()
            .map(|item| item.strength_bonus)
            .sum::<i32>();
    let equipment_range_bonus =
        setup.weapon.range_bonus + setup.worn.iter().map(|item| item.range_bonus).sum::<i32>();
    let player_hit_chance = hit_chance(
        roll(
            u32::from(stats.attack) + 8 + attack,
            equipment_attack_bonus[damage_type],
        )?,
        roll(setup.npc.defence + 9, setup.npc.defence_bonus[damage_type])?,
    );
    let npc_hit_chance = hit_chance(
        roll(setup.npc.attack + 9, setup.npc.attack_bonus)?,
        roll(
            u32::from(stats.defence) + 8 + defence,
            equipment_defence_bonus[npc_type],
        )?,
    );
    let player_max_hit = u16::try_from(
        (roll(
            u32::from(stats.strength) + 8 + strength,
            equipment_strength_bonus,
        )? + 320)
            / 640,
    )
    .map_err(|_| FightError::Input("player maximum hit overflow".into()))?;
    let npc_max_hit = u16::from(
        npc_max_hit(setup.row)
            .ok_or_else(|| FightError::NotModelled("NPC maximum hit unavailable".into()))?,
    );
    let model = SurvivalModel {
        player_hp: stats.hitpoints,
        npc_hp: u16::try_from(setup.row.hitpoints)
            .map_err(|_| FightError::Input("NPC Hitpoints out of range".into()))?,
        player_chance: player_hit_chance,
        npc_chance: npc_hit_chance,
        player_max: player_max_hit,
        npc_max: npc_max_hit,
        player_interval: setup.weapon.attackrate,
        npc_interval: setup.npc.attackrate,
        npc_first_tick: if setup.initiates {
            1 + setup.npc.attackrate / 2
        } else {
            0
        },
        sequential_kills: setup.sequential_kills,
        food_rule,
        food_count,
    };
    Ok(Evaluation {
        stats,
        food_count,
        outcome: solve_survival(model)?,
        player_hit_chance,
        npc_hit_chance,
        player_max_hit,
        npc_max_hit,
        player_interval: setup.weapon.attackrate,
        npc_interval: setup.npc.attackrate,
        equipment_attack_bonus,
        equipment_defence_bonus,
        equipment_strength_bonus,
        equipment_range_bonus,
    })
}

#[derive(Clone, Copy)]
struct SurvivalModel<'a> {
    player_hp: u8,
    npc_hp: u16,
    player_chance: f64,
    npc_chance: f64,
    player_max: u16,
    npc_max: u16,
    player_interval: u8,
    npc_interval: u8,
    npc_first_tick: u8,
    sequential_kills: u8,
    food_rule: Option<&'a food::FoodRule>,
    food_count: u8,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
struct SurvivalState {
    player_hp: u8,
    npc_hp: u16,
    food: u8,
    player_wait: u16,
    npc_wait: u16,
    pending_damage: u16,
    pending_wait: u8,
    regen_wait: u16,
    food_wait: u16,
    kills_left: u8,
}

#[derive(Clone, Copy)]
struct Branch {
    state: SurvivalState,
    probability: f64,
    terminal: Option<bool>,
    ate: bool,
}

#[derive(Clone, Copy)]
struct RawTransition {
    next: Option<SurvivalState>,
    won: bool,
    probability: f64,
    ticks: u16,
}

struct Edge {
    next: Option<usize>,
    won: bool,
    probability: f64,
    ticks: u16,
}

struct Node {
    state: SurvivalState,
    edges: Vec<Edge>,
}

const MAX_SURVIVAL_STATES: usize = 8_000_000;
const MAX_VALUE_ITERATIONS: usize = 100_000;

fn damage_distribution(chance: f64, maximum: u16) -> Vec<(u16, f64)> {
    let positive = chance / (f64::from(maximum) + 1.0);
    std::iter::once((0, 1.0 - positive * f64::from(maximum)))
        .chain((1..=maximum).map(|damage| (damage, positive)))
        .collect()
}

fn spawn_next(model: SurvivalModel<'_>, state: &mut SurvivalState) {
    state.npc_hp = model.npc_hp;
    state.kills_left -= 1;
    state.pending_damage = 0;
    state.pending_wait = 0;
    state.player_wait = 0;
    state.npc_wait = u16::from(model.npc_first_tick);
}

fn next_food_delay(rule: &food::FoodRule, hp: u8, remaining: u16) -> Option<u16> {
    (0..remaining).rev().find_map(|cooldown| {
        let cooldown = u8::try_from(cooldown).ok()?;
        rule.should_eat(hp, cooldown)
            .then_some(remaining - u16::from(cooldown))
    })
}

fn advance(state: &mut SurvivalState, model: SurvivalModel<'_>) -> Result<u16, FightError> {
    let mut delay = state.player_wait.min(state.npc_wait).min(state.regen_wait);
    if state.pending_damage > 0 {
        delay = delay.min(u16::from(state.pending_wait));
    }
    if state.food > 0 {
        if let Some(food_delay) = model
            .food_rule
            .and_then(|rule| next_food_delay(rule, state.player_hp, state.food_wait))
        {
            delay = delay.min(food_delay);
        }
    }
    if delay == 0 || delay == u16::MAX {
        return Err(FightError::Input(
            "survival event scheduler failed to advance".into(),
        ));
    }
    state.player_wait -= delay;
    state.npc_wait -= delay;
    state.regen_wait -= delay;
    state.food_wait = state.food_wait.saturating_sub(delay);
    if state.pending_damage > 0 {
        state.pending_wait = state
            .pending_wait
            .saturating_sub(u8::try_from(delay).unwrap_or(u8::MAX));
    }
    Ok(delay)
}

fn expand_survival(
    original: SurvivalState,
    model: SurvivalModel<'_>,
) -> Result<Vec<RawTransition>, FightError> {
    let mut branches = vec![Branch {
        state: original,
        probability: 1.0,
        terminal: None,
        ate: false,
    }];

    for branch in &mut branches {
        if branch.state.pending_damage == 0 || branch.state.pending_wait != 0 {
            continue;
        }
        let damage = branch.state.pending_damage;
        branch.state.pending_damage = 0;
        branch.state.pending_wait = 0;
        branch.state.npc_hp = branch.state.npc_hp.saturating_sub(damage);
        if branch.state.npc_hp == 0 {
            if branch.state.kills_left == 1 {
                branch.terminal = Some(true);
            } else {
                spawn_next(model, &mut branch.state);
            }
        }
    }

    let mut after_npc = Vec::new();
    for branch in branches {
        if branch.terminal.is_some() || branch.state.npc_wait != 0 {
            after_npc.push(branch);
            continue;
        }
        for (damage, probability) in damage_distribution(model.npc_chance, model.npc_max) {
            let mut next = branch;
            next.state.player_hp = next
                .state
                .player_hp
                .saturating_sub(u8::try_from(damage).unwrap_or(u8::MAX));
            next.state.npc_wait = u16::from(model.npc_interval);
            next.probability *= probability;
            if next.state.player_hp == 0 {
                next.terminal = Some(false);
            }
            after_npc.push(next);
        }
    }

    let mut after_regen = Vec::with_capacity(after_npc.len());
    for mut branch in after_npc {
        if branch.terminal.is_none() && branch.state.regen_wait == 0 {
            branch.state.player_hp = branch
                .state
                .player_hp
                .saturating_add(1)
                .min(model.player_hp);
            branch.state.regen_wait = 100;
        }
        after_regen.push(branch);
    }

    let mut after_food = Vec::with_capacity(after_regen.len());
    for mut branch in after_regen {
        if branch.terminal.is_none()
            && branch.state.food > 0
            && model.food_rule.is_some_and(|rule| {
                rule.should_eat(branch.state.player_hp, branch.state.food_wait as u8)
            })
        {
            let rule = model.food_rule.expect("food choice needs its rule");
            branch.state.food -= 1;
            branch.state.player_hp = branch
                .state
                .player_hp
                .saturating_add(rule.heal)
                .min(model.player_hp);
            branch.state.food_wait = u16::from(rule.eat_delay);
            branch.state.player_wait = branch
                .state
                .player_wait
                .saturating_add(u16::from(rule.skill_delay));
            branch.ate = true;
        }
        after_food.push(branch);
    }

    let mut after_player = Vec::new();
    for branch in after_food {
        if branch.terminal.is_some() || branch.ate || branch.state.player_wait != 0 {
            after_player.push(branch);
            continue;
        }
        for (damage, probability) in damage_distribution(model.player_chance, model.player_max) {
            let mut next = branch;
            next.state.player_wait = u16::from(model.player_interval);
            if damage > 0 {
                next.state.pending_damage = damage.min(next.state.npc_hp);
                next.state.pending_wait = 1;
            }
            next.probability *= probability;
            after_player.push(next);
        }
    }

    let mut transitions = Vec::with_capacity(after_player.len());
    for mut branch in after_player {
        if let Some(won) = branch.terminal {
            transitions.push(RawTransition {
                next: None,
                won,
                probability: branch.probability,
                ticks: 0,
            });
        } else {
            let ticks = advance(&mut branch.state, model)?;
            transitions.push(RawTransition {
                next: Some(branch.state),
                won: false,
                probability: branch.probability,
                ticks,
            });
        }
    }
    Ok(transitions)
}

fn solve_survival(model: SurvivalModel<'_>) -> Result<Outcome, FightError> {
    if model.player_interval < 2
        || model.npc_interval == 0
        || model.player_hp == 0
        || model.npc_hp == 0
        || model.sequential_kills == 0
    {
        return Err(FightError::Input(
            "invalid survival model timing or Hitpoints".into(),
        ));
    }
    if model.player_max == 0 || model.player_chance == 0.0 {
        return Err(FightError::Input(
            "player cannot deal positive damage".into(),
        ));
    }
    if model.food_count > 0 && model.food_rule.is_none() {
        return Err(FightError::Input(
            "food count has no product-backed food rule".into(),
        ));
    }
    let initial = SurvivalState {
        player_hp: model.player_hp,
        npc_hp: model.npc_hp,
        food: model.food_count,
        player_wait: 0,
        npc_wait: u16::from(model.npc_first_tick),
        pending_damage: 0,
        pending_wait: 0,
        regen_wait: 100,
        food_wait: 0,
        kills_left: model.sequential_kills,
    };
    let mut nodes = vec![Node {
        state: initial,
        edges: Vec::new(),
    }];
    let mut indices = HashMap::from([(initial, 0usize)]);
    let mut cursor = 0;
    while cursor < nodes.len() {
        if nodes.len() > MAX_SURVIVAL_STATES {
            return Err(FightError::StateLimit {
                limit: MAX_SURVIVAL_STATES,
                states: nodes.len(),
            });
        }
        let transitions = expand_survival(nodes[cursor].state, model)?;
        let mut edges = Vec::with_capacity(transitions.len());
        for transition in transitions {
            let next = transition.next.map(|state| {
                if let Some(index) = indices.get(&state) {
                    *index
                } else {
                    let index = nodes.len();
                    nodes.push(Node {
                        state,
                        edges: Vec::new(),
                    });
                    indices.insert(state, index);
                    index
                }
            });
            edges.push(Edge {
                next,
                won: transition.won,
                probability: transition.probability,
                ticks: transition.ticks,
            });
        }
        nodes[cursor].edges = edges;
        cursor += 1;
    }

    let mut wins = vec![0.0; nodes.len()];
    let mut lengths = vec![0.0; nodes.len()];
    for _ in 0..MAX_VALUE_ITERATIONS {
        let mut max_win_delta: f64 = 0.0;
        let mut max_tick_delta: f64 = 0.0;
        for index in (0..nodes.len()).rev() {
            let mut win = 0.0;
            let mut ticks = 0.0;
            for edge in &nodes[index].edges {
                if let Some(next) = edge.next {
                    win += edge.probability * wins[next];
                    ticks += edge.probability * (f64::from(edge.ticks) + lengths[next]);
                } else {
                    win += edge.probability * f64::from(edge.won);
                    ticks += edge.probability * f64::from(edge.ticks);
                }
            }
            max_win_delta = max_win_delta.max((win - wins[index]).abs());
            max_tick_delta = max_tick_delta.max((ticks - lengths[index]).abs());
            wins[index] = win;
            lengths[index] = ticks;
        }
        if max_win_delta < 1e-13 && max_tick_delta < 1e-9 {
            return Ok(Outcome {
                win_probability: wins[0].clamp(0.0, 1.0),
                death_probability: (1.0 - wins[0]).clamp(0.0, 1.0),
                expected_ticks: lengths[0],
                explored_states: nodes.len(),
            });
        }
    }
    Err(FightError::NotModelled(
        "survival state graph did not converge within 100000 value iterations".into(),
    ))
}

/// P(U[0,A] > U[0,D]). A tie misses, including A=D=0.
pub fn hit_chance(attack: u32, defence: u32) -> f64 {
    let a = f64::from(attack);
    let d = f64::from(defence);
    if attack > defence {
        1.0 - (d + 2.0) / (2.0 * (a + 1.0))
    } else {
        a / (2.0 * (d + 1.0))
    }
}
#[cfg(test)]
#[derive(Clone, Copy)]
struct Model {
    player_hp: u16,
    npc_hp: u16,
    player_chance: f64,
    npc_chance: f64,
    player_max: u16,
    npc_max: u16,
    player_interval: u8,
    npc_interval: u8,
    npc_first_tick: u8,
}
#[cfg(test)]
#[derive(Clone, Copy)]
struct Event {
    tick: u32,
    player: bool,
    maximum: u16,
    positive: f64,
    zero: f64,
}
#[cfg(test)]
fn events(model: Model) -> Result<(Vec<Event>, u32), FightError> {
    if model.player_interval < 2
        || model.npc_interval == 0
        || model.npc_first_tick > model.npc_interval
        || model.player_hp == 0
        || model.npc_hp == 0
    {
        return Err(FightError::Input(
            "invalid fight timing or Hitpoints".into(),
        ));
    }
    let mut divisor = u32::from(model.player_interval);
    let mut other = u32::from(model.npc_interval);
    while other != 0 {
        (divisor, other) = (other, divisor % other);
    }
    let period = u32::from(model.player_interval) / divisor * u32::from(model.npc_interval);
    let mut events = Vec::new();
    let start = u32::from(model.npc_first_tick > 0);
    for tick in start..period + start {
        // NPC queues run before NPC attacks. A killing queued melee hit cancels that NPC's attack.
        for (due, player, maximum, chance) in [
            (
                tick % u32::from(model.player_interval) == 1,
                true,
                model.player_max,
                model.player_chance,
            ),
            (
                tick % u32::from(model.npc_interval)
                    == u32::from(model.npc_first_tick) % u32::from(model.npc_interval),
                false,
                model.npc_max,
                model.npc_chance,
            ),
        ] {
            if due {
                let positive = chance / (f64::from(maximum) + 1.0);
                events.push(Event {
                    tick,
                    player,
                    maximum,
                    positive,
                    zero: 1.0 - positive * f64::from(maximum),
                });
            }
        }
    }
    Ok((events, period))
}
#[cfg(test)]
fn solve(model: Model, sequential_kills: u8) -> Result<Outcome, FightError> {
    let (events, period) = events(model)?;
    let phases = events.len();
    let cycle_zero = events.iter().map(|e| e.zero).product::<f64>();
    if cycle_zero >= 1.0 {
        return Err(FightError::Input(
            "neither participant can deal damage".into(),
        ));
    }
    let width = usize::from(model.npc_hp) + 1;
    let cells = (usize::from(model.player_hp) + 1)
        .checked_mul(width)
        .and_then(|n| n.checked_mul(phases))
        .filter(|n| *n <= 50_000_000)
        .ok_or_else(|| FightError::Input("fight state space too large".into()))?;
    let mut wins = vec![0.0; cells];
    let mut lengths = vec![0.0; cells];
    let mut b_win = vec![0.0; phases];
    let mut b_length = vec![0.0; phases];
    let index = |hp: usize, npc: usize, phase: usize| (hp * width + npc) * phases + phase;
    let mut remaining_wins = vec![1.0; usize::from(model.player_hp) + 1];
    let mut remaining_lengths = vec![0.0; remaining_wins.len()];
    for _ in 0..sequential_kills {
        for hp in 1..=usize::from(model.player_hp) {
            for npc in 1..=usize::from(model.npc_hp) {
                for (phase, event) in events.iter().enumerate() {
                    let next = (phase + 1) % phases;
                    let gap = f64::from(if next == 0 {
                        period + events[0].tick - event.tick
                    } else {
                        events[next].tick - event.tick
                    });
                    let mut win = 0.0;
                    let mut length = event.zero * gap;
                    for damage in 1..=usize::from(event.maximum) {
                        if event.player && damage >= npc {
                            win += event.positive * remaining_wins[hp];
                            length += event.positive * remaining_lengths[hp];
                        } else if !event.player && damage >= hp { /* Death is absorbing. */
                        } else {
                            let child = if event.player {
                                index(hp, npc - damage, next)
                            } else {
                                index(hp - damage, npc, next)
                            };
                            win += event.positive * wins[child];
                            length += event.positive * (gap + lengths[child]);
                        }
                    }
                    b_win[phase] = win;
                    b_length[phase] = length;
                }
                // Compose all phase equations and solve the one remaining zero-damage loop.
                let mut win = 0.0;
                let mut length = 0.0;
                let mut weight = 1.0;
                for phase in 0..phases {
                    win += weight * b_win[phase];
                    length += weight * b_length[phase];
                    weight *= events[phase].zero;
                }
                win /= 1.0 - cycle_zero;
                length /= 1.0 - cycle_zero;
                wins[index(hp, npc, 0)] = win;
                lengths[index(hp, npc, 0)] = length;
                for phase in (1..phases).rev() {
                    win = b_win[phase] + events[phase].zero * win;
                    length = b_length[phase] + events[phase].zero * length;
                    wins[index(hp, npc, phase)] = win;
                    lengths[index(hp, npc, phase)] = length;
                }
            }
        }
        // A new encounter resets both clocks, not the player's remaining HP.
        for hp in 1..=usize::from(model.player_hp) {
            let start = index(hp, usize::from(model.npc_hp), 0);
            remaining_wins[hp] = wins[start];
            remaining_lengths[hp] = lengths[start] + f64::from(events[0].tick);
        }
    }
    let start = index(usize::from(model.player_hp), usize::from(model.npc_hp), 0);
    let win = wins[start].clamp(0.0, 1.0);
    Ok(Outcome {
        win_probability: win,
        death_probability: 1.0 - win,
        expected_ticks: lengths[start] + f64::from(events[0].tick),
        explored_states: cells,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inclusive_roll_hand_cases() {
        // A=3,D=1 has 5 successful pairs among 8. A=1,D=3 has 1 among 8.
        assert_eq!(hit_chance(3, 1), 5.0 / 8.0);
        assert_eq!(hit_chance(1, 3), 1.0 / 8.0);
        assert_eq!(hit_chance(2, 2), 3.0 / 9.0);
        assert_eq!(hit_chance(0, 0), 0.0);
        for a in 0..=12 {
            for d in 0..=12 {
                let successes = (0..=a)
                    .map(|x| (0..=d).filter(|y| x > *y).count())
                    .sum::<usize>();
                assert!(
                    (hit_chance(a, d) - successes as f64 / f64::from((a + 1) * (d + 1))).abs()
                        < 1e-14
                );
            }
        }
    }
    #[test]
    fn armour_bonus_reduces_incoming_hit_chance_hand_case() {
        let npc_attack_roll = roll(18, 0).unwrap();
        let bare_defence_roll = roll(48, 0).unwrap();
        let armoured_defence_roll = roll(48, 40).unwrap();
        assert_eq!(bare_defence_roll, 3_072);
        assert_eq!(armoured_defence_roll, 4_992);
        assert_eq!(
            hit_chance(npc_attack_roll, armoured_defence_roll),
            1_152.0 / (2.0 * 4_993.0)
        );
        assert!(
            hit_chance(npc_attack_roll, armoured_defence_roll)
                < hit_chance(npc_attack_roll, bare_defence_roll)
        );
    }
    fn sample(seed: &mut u64) -> f64 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        (*seed >> 11) as f64 / ((1u64 << 53) as f64)
    }
    fn monte_carlo(model: Model) -> Outcome {
        let (events, period) = events(model).unwrap();
        let mut seed = 0x0192_dab4_bafe_7135u64;
        let mut wins = 0;
        let mut ticks = 0u64;
        const TRIALS: u32 = 200_000;
        for _ in 0..TRIALS {
            let mut hp = model.player_hp;
            let mut npc = model.npc_hp;
            let mut cycle = 0;
            'fight: loop {
                for event in &events {
                    // Independent accuracy and uniform damage draws, not the DP's compressed probabilities.
                    let chance = if event.player {
                        model.player_chance
                    } else {
                        model.npc_chance
                    };
                    let damage = if sample(&mut seed) < chance {
                        (sample(&mut seed) * f64::from(event.maximum + 1)) as u16
                    } else {
                        0
                    };
                    if event.player {
                        npc = npc.saturating_sub(damage);
                    } else {
                        hp = hp.saturating_sub(damage);
                    }
                    if hp == 0 || npc == 0 {
                        wins += u32::from(npc == 0);
                        ticks += u64::from(cycle + event.tick);
                        break 'fight;
                    }
                }
                cycle += period;
            }
        }
        let win = f64::from(wins) / f64::from(TRIALS);
        Outcome {
            win_probability: win,
            death_probability: 1.0 - win,
            expected_ticks: ticks as f64 / f64::from(TRIALS),
            explored_states: 0,
        }
    }
    fn product_food_rule(hp_max: u8) -> food::FoodRule {
        let selected = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
        let tables = CombatTables::build(selected).unwrap();
        let npc = tables.npc(95).unwrap().clone();
        let lobster = tables
            .selected()
            .items()
            .iter()
            .find(|item| item.alias.as_deref() == Some("lobster"))
            .unwrap();
        food::build(&tables, &npc, lobster, hp_max).unwrap()
    }

    fn test_evaluation(stats: Stats, food_count: u8, death_probability: f64) -> Evaluation {
        Evaluation {
            stats,
            food_count,
            outcome: Outcome {
                win_probability: 1.0 - death_probability,
                death_probability,
                expected_ticks: 10.0,
                explored_states: 128,
            },
            player_hit_chance: 0.5,
            npc_hit_chance: 0.5,
            player_max_hit: 1,
            npc_max_hit: 1,
            player_interval: 4,
            npc_interval: 4,
            equipment_attack_bonus: [0; 3],
            equipment_defence_bonus: [0; 3],
            equipment_strength_bonus: 0,
            equipment_range_bonus: 0,
        }
    }

    #[test]
    fn only_a_positive_food_count_meets_the_bound() {
        let stats = Stats::policy(40, Style::Accurate);
        let search = search_food_candidates(stats, 2, |stats, food_count| {
            let death_probability = if food_count == 0 { 0.06 } else { 0.04 };
            Ok(test_evaluation(stats, food_count, death_probability))
        })
        .unwrap();

        let (evaluation, bound) = search.accepted.unwrap();
        assert_eq!(evaluation.food_count, 1);
        assert_eq!(bound, "5%");
        assert_eq!(search.previous_candidate.unwrap().food_count, 0);
    }

    #[test]
    fn policy_search_prefers_the_smallest_single_stat_raise() {
        let mut searched = Vec::new();
        let policy = search_policy(Style::Accurate, |stats| {
            searched.push(stats);
            let reaches_bound = stats.attack == 40
                && stats.strength == 42
                && stats.defence == 40
                && stats.hitpoints == 40;
            Ok(FoodSearch {
                accepted: reaches_bound.then(|| (test_evaluation(stats, 0, 0.04), "5%")),
                previous_candidate: Some(test_evaluation(stats, 0, 0.06)),
                max_explored_states: 128,
            })
        })
        .unwrap();
        let (evaluation, _) = policy.accepted.unwrap();
        assert_eq!(evaluation.stats.strength, 42);
        assert_eq!(evaluation.stats.attack, 40);
        assert_eq!(evaluation.stats.defence, 40);
        assert_eq!(evaluation.stats.hitpoints, 40);
        assert_eq!(searched.len(), 7);
        assert!(policy.reason.contains("Strength alone raised to 42"));
    }

    fn sampled_damage(seed: &mut u64, chance: f64, maximum: u16) -> u16 {
        if sample(seed) < chance {
            (sample(seed) * f64::from(maximum + 1)) as u16
        } else {
            0
        }
    }

    fn monte_carlo_survival(model: SurvivalModel<'_>, trials: u32) -> Result<Outcome, FightError> {
        let initial = SurvivalState {
            player_hp: model.player_hp,
            npc_hp: model.npc_hp,
            food: model.food_count,
            player_wait: 0,
            npc_wait: u16::from(model.npc_first_tick),
            pending_damage: 0,
            pending_wait: 0,
            regen_wait: 100,
            food_wait: 0,
            kills_left: model.sequential_kills,
        };
        let mut seed = 0x0137_ade1_17ab_901du64;
        let mut wins = 0u32;
        let mut ticks = 0u64;
        for _ in 0..trials {
            let mut state = initial;
            let mut elapsed = 0u64;
            let won = loop {
                if state.pending_damage > 0 && state.pending_wait == 0 {
                    state.npc_hp = state.npc_hp.saturating_sub(state.pending_damage);
                    state.pending_damage = 0;
                    if state.npc_hp == 0 {
                        if state.kills_left == 1 {
                            break true;
                        }
                        spawn_next(model, &mut state);
                    }
                }
                if state.npc_wait == 0 {
                    let damage = sampled_damage(&mut seed, model.npc_chance, model.npc_max);
                    state.player_hp = state
                        .player_hp
                        .saturating_sub(u8::try_from(damage).unwrap_or(u8::MAX));
                    state.npc_wait = u16::from(model.npc_interval);
                    if state.player_hp == 0 {
                        break false;
                    }
                }
                if state.regen_wait == 0 {
                    state.player_hp = state.player_hp.saturating_add(1).min(model.player_hp);
                    state.regen_wait = 100;
                }
                let ate = state.food > 0
                    && model.food_rule.is_some_and(|rule| {
                        rule.should_eat(state.player_hp, state.food_wait as u8)
                    });
                if ate {
                    let rule = model.food_rule.expect("food choice needs its rule");
                    state.food -= 1;
                    state.player_hp = state
                        .player_hp
                        .saturating_add(rule.heal)
                        .min(model.player_hp);
                    state.food_wait = u16::from(rule.eat_delay);
                    state.player_wait = state
                        .player_wait
                        .saturating_add(u16::from(rule.skill_delay));
                } else if state.player_wait == 0 {
                    let damage = sampled_damage(&mut seed, model.player_chance, model.player_max);
                    state.player_wait = u16::from(model.player_interval);
                    if damage > 0 {
                        state.pending_damage = damage.min(state.npc_hp);
                        state.pending_wait = 1;
                    }
                }
                let delay = advance(&mut state, model)?;
                elapsed += u64::from(delay);
            };
            wins += u32::from(won);
            ticks += elapsed;
        }
        let win_probability = f64::from(wins) / f64::from(trials);
        Ok(Outcome {
            win_probability,
            death_probability: 1.0 - win_probability,
            expected_ticks: ticks as f64 / f64::from(trials),
            explored_states: 0,
        })
    }

    #[test]
    fn product_food_causes_one_heal_and_three_tick_player_delay() {
        let rule = product_food_rule(40);
        let hp = (1..40)
            .find(|hp| rule.should_eat(*hp, 0))
            .expect("product food policy should select lobster below its eat line");
        let model = SurvivalModel {
            player_hp: 40,
            npc_hp: 1,
            player_chance: 1.0,
            npc_chance: 0.0,
            player_max: 1,
            npc_max: 0,
            player_interval: 4,
            npc_interval: 4,
            npc_first_tick: 0,
            sequential_kills: 1,
            food_rule: Some(&rule),
            food_count: 1,
        };
        let state = SurvivalState {
            player_hp: hp,
            npc_hp: 1,
            food: 1,
            player_wait: 0,
            npc_wait: 100,
            pending_damage: 0,
            pending_wait: 0,
            regen_wait: 100,
            food_wait: 0,
            kills_left: 1,
        };
        let transitions = expand_survival(state, model).unwrap();
        assert_eq!(transitions.len(), 1);
        let transition = transitions[0];
        let next = transition.next.unwrap();
        assert_eq!(transition.ticks, 3);
        assert_eq!(rule.eat_delay, 3);
        assert_eq!(rule.skill_delay, 3);
        assert_eq!(next.player_hp, hp.saturating_add(rule.heal).min(40));
        assert_eq!(next.food, 0);
        assert_eq!(next.player_wait, 0);
        assert_eq!(next.food_wait, 0);
    }

    #[test]
    fn absorbing_solver_matches_deterministic_food_and_regeneration_simulation() {
        let rule = product_food_rule(24);
        assert!(
            (1..=24).any(|hp| rule.should_eat(hp, 0)),
            "product-backed food rule must select lobster in this fixture"
        );
        let model = SurvivalModel {
            player_hp: 24,
            npc_hp: 4,
            player_chance: 0.05,
            npc_chance: 0.65,
            player_max: 1,
            npc_max: 1,
            player_interval: 4,
            npc_interval: 4,
            npc_first_tick: 0,
            sequential_kills: 1,
            food_rule: Some(&rule),
            food_count: 1,
        };
        let exact = solve_survival(model).unwrap();
        assert!(exact.expected_ticks > 100.0);
        let sampled = monte_carlo_survival(model, 30_000).unwrap();
        assert!(
            (exact.death_probability - sampled.death_probability).abs() < 0.02,
            "{exact:?} {sampled:?}"
        );
        assert!(
            (exact.expected_ticks - sampled.expected_ticks).abs()
                < exact.expected_ticks * 0.08 + 2.0,
            "{exact:?} {sampled:?}"
        );
    }
    #[test]
    fn dp_matches_two_brute_force_monte_carlo_cases() {
        for model in [
            Model {
                player_hp: 3,
                npc_hp: 4,
                player_chance: 0.6,
                npc_chance: 0.4,
                player_max: 2,
                npc_max: 2,
                player_interval: 4,
                npc_interval: 4,
                npc_first_tick: 0,
            },
            Model {
                player_hp: 5,
                npc_hp: 3,
                player_chance: 0.35,
                npc_chance: 0.7,
                player_max: 3,
                npc_max: 1,
                player_interval: 3,
                npc_interval: 2,
                npc_first_tick: 1,
            },
        ] {
            let exact = solve(model, 1).unwrap();
            let sampled = monte_carlo(model);
            // 0.006 absolute probability and 0.25 tick tolerances, 200,000 deterministic trials per case.
            assert!(
                (exact.win_probability - sampled.win_probability).abs() < 0.006,
                "{exact:?} {sampled:?}"
            );
            assert!(
                (exact.expected_ticks - sampled.expected_ticks).abs() < 0.25,
                "{exact:?} {sampled:?}"
            );
        }
    }
    #[test]
    fn geometric_zero_cycles_and_tie_priority() {
        let model = Model {
            player_hp: 1,
            npc_hp: 1,
            player_chance: 1.0,
            npc_chance: 1.0,
            player_max: 1,
            npc_max: 1,
            player_interval: 4,
            npc_interval: 4,
            npc_first_tick: 1,
        };
        let outcome = solve(model, 1).unwrap();
        assert!((outcome.win_probability - 2.0 / 3.0).abs() < 1e-14);
        assert!((outcome.expected_ticks - (1.0 + 4.0 / 3.0)).abs() < 1e-14);
    }
    #[test]
    fn sequential_kills_carry_remaining_hp_and_absorb_death() {
        let model = Model {
            player_hp: 1,
            npc_hp: 1,
            player_chance: 1.0,
            npc_chance: 1.0,
            player_max: 1,
            npc_max: 1,
            player_interval: 4,
            npc_interval: 4,
            npc_first_tick: 1,
        };
        let single = solve(model, 1).unwrap();
        let triple = solve(model, 3).unwrap();
        assert!((triple.win_probability - (2.0_f64 / 3.0).powi(3)).abs() < 1e-14);
        assert!(
            (triple.expected_ticks - single.expected_ticks * (1.0 + 2.0 / 3.0 + 4.0 / 9.0)).abs()
                < 1e-14
        );
        let two_hp = solve(
            Model {
                player_hp: 2,
                ..model
            },
            2,
        )
        .unwrap();
        assert!((two_hp.win_probability - 20.0 / 27.0).abs() < 1e-14);
    }
    #[test]
    fn harmless_npc_and_retaliation_flinch_have_finite_exact_lengths() {
        let harmless = Model {
            player_hp: 1,
            npc_hp: 1,
            player_chance: 1.0,
            npc_chance: 1.0,
            player_max: 1,
            npc_max: 0,
            player_interval: 4,
            npc_interval: 4,
            npc_first_tick: 0,
        };
        let outcome = solve(harmless, 1).unwrap();
        assert_eq!(outcome.win_probability, 1.0);
        assert!((outcome.expected_ticks - 5.0).abs() < 1e-14);
        let passive = Model {
            npc_max: 1,
            npc_first_tick: 3,
            ..harmless
        };
        let (schedule, _) = events(passive).unwrap();
        assert_eq!((schedule[0].tick, schedule[0].player), (1, true));
        assert_eq!((schedule[1].tick, schedule[1].player), (3, false));
        let one_tick = Model {
            npc_interval: 1,
            npc_first_tick: 1,
            ..passive
        };
        let (schedule, _) = events(one_tick).unwrap();
        assert_eq!((schedule[0].tick, schedule[0].player), (1, true));
    }
    #[test]
    fn selected_facts_and_unsupported_classes_stay_explicit() {
        let selected = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
        let tables = CombatTables::build(selected).unwrap();
        let weapon = Weapon {
            id: Some(1321),
            config: "bronze_scimitar".into(),
            kind: WeaponKind::Melee,
            attack_bonus: [1, 7, -2, 0, 0],
            defence_bonus: [0, 1, 0, 0, 0],
            strength_bonus: 6,
            range_bonus: 0,
            attackrate: 4,
            required_attack: 1,
            required_ranged: 1,
            damage_types: [Some(1), Some(1), Some(0), Some(1)],
            poisoned: false,
            source: "test".into(),
            wear_source: "test".into(),
        };
        validate_weapon(tables.selected(), &weapon, Style::Accurate).unwrap();
        let mut row = tables.npc(95).unwrap().clone();
        assert!(unsupported(&row, &tables, 1, &weapon, Style::Accurate).is_empty());
        assert!(!unsupported(&row, &tables, 2, &weapon, Style::Accurate).is_empty());
        row.bespoke = true;
        assert!(unsupported(&row, &tables, 1, &weapon, Style::Accurate)
            .iter()
            .any(|r| r.contains("bespoke")));
        row.bespoke = false;
        row.poison_severity = Some(5);
        assert!(unsupported(&row, &tables, 1, &weapon, Style::Accurate)
            .iter()
            .any(|r| r.contains("poison changes")));
        row.poison_severity = Some(0);
        row.strength = None;
        assert!(unsupported(&row, &tables, 1, &weapon, Style::Accurate)
            .iter()
            .any(|r| r.contains("unknown NPC maximum")));
    }
    #[test]
    fn null_weapon_is_unarmed_and_omitted_weapon_keeps_the_default() {
        let default: FightQuery =
            serde_json::from_str(r#"{"npc":"wolf","facts":"facts"}"#).unwrap();
        assert!(matches!(
            default.weapon,
            Some(Selector::Name(alias)) if alias == "bronze_scimitar"
        ));

        let unarmed: FightQuery =
            serde_json::from_str(r#"{"npc":"wolf","facts":"facts","weapon":null}"#).unwrap();
        assert!(unarmed.weapon.is_none());
    }

    #[test]
    fn unarmed_weapon_uses_the_content_crush_table_and_four_tick_interval() {
        let selected = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
        let tables = CombatTables::build(selected).unwrap();
        let weapon = unarmed_weapon();
        assert_eq!(weapon.id, None);
        assert_eq!(weapon.attackrate, 4);
        assert_eq!(weapon.attack_bonus, [0; 5]);
        assert_eq!(weapon.strength_bonus, 0);
        assert_eq!(weapon.damage_types, [Some(2), Some(2), None, Some(2)]);
        for style in [Style::Accurate, Style::Aggressive, Style::Defensive] {
            validate_weapon(tables.selected(), &weapon, style).unwrap();
        }
        assert!(validate_weapon(tables.selected(), &weapon, Style::Controlled).is_err());
    }

    #[test]
    fn npc_names_do_not_select_an_arbitrary_variant() {
        let selected = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
        let tables = CombatTables::build(selected).unwrap();
        let query = FightQuery {
            label: None,
            npc: Selector::Name("Skeleton".into()),
            facts: PathBuf::from("not-read"),
            weapon: default_weapon(),
            style: Style::Accurate,
            worn_items: Vec::new(),
            food: None,
            ammunition: None,
            spell: None,
            food_count: 0,
            free_slots: 28,
            candidate: None,
            npc_count: 1,
            player_initiates: false,
            sequential_kills: 1,
        };
        assert!(calculate(&query, &tables)
            .unwrap_err()
            .to_string()
            .contains("ambiguous"));
        assert_eq!(Stats::policy(1, Style::Accurate).hitpoints, 10);
        assert_eq!(Stats::policy(11, Style::Accurate).hitpoints, 11);
    }
}
