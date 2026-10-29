//! Exact absorbing-state calculation, with analytic elimination of zero-damage cycles.
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
    #[error("{0}")]
    Input(String),
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
fn default_weapon() -> Selector {
    Selector::Name("bronze_scimitar".into())
}
fn one() -> u8 {
    1
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Style {
    #[default]
    Accurate,
    Aggressive,
    Defensive,
    Controlled,
}
impl Style {
    fn bonuses(self) -> [u32; 3] {
        match self {
            Self::Accurate => [3, 0, 0],
            Self::Aggressive => [0, 3, 0],
            Self::Defensive => [0, 0, 3],
            Self::Controlled => [1, 1, 1],
        }
    }
    fn code(self) -> u8 {
        match self {
            Self::Accurate => 0,
            Self::Aggressive => 1,
            Self::Controlled => 2,
            Self::Defensive => 3,
        }
    }
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct Stats {
    pub attack: u8,
    pub strength: u8,
    pub defence: u8,
    pub hitpoints: u8,
}
impl Stats {
    fn policy(level: u8) -> Self {
        Self {
            attack: level,
            strength: level,
            defence: level,
            hitpoints: level.max(10),
        }
    }
    fn valid(self) -> bool {
        [self.attack, self.strength, self.defence, self.hitpoints]
            .iter()
            .all(|v| (1..=99).contains(v))
    }
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FightQuery {
    pub npc: Selector,
    pub facts: PathBuf,
    #[serde(default = "default_weapon")]
    pub weapon: Selector,
    #[serde(default)]
    pub style: Style,
    /// Omit candidate to request only the minimum-policy search.
    #[serde(default)]
    pub candidate: Option<Stats>,
    /// One NPC only. Larger counts are explicitly not modelled.
    #[serde(default = "one")]
    pub npc_count: u8,
    /// Default assumes an aggressive NPC ready at tick zero. Player damage lands at tick one.
    #[serde(default)]
    pub player_initiates: bool,
}

#[derive(Debug, Deserialize)]
struct Facts {
    revision: i32,
    npcs: Vec<NpcRoll>,
    weapons: Vec<Weapon>,
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
    max_dealt: u16,
    source: String,
}
#[derive(Debug, Deserialize)]
struct Weapon {
    id: i32,
    config: String,
    attack_bonus: [i32; 3],
    defence_bonus: [i32; 3],
    strength_bonus: i32,
    attackrate: u8,
    required_attack: u8,
    /// One entry per supported semantic mode. Each value is stab=0, slash=1, crush=2.
    damage_types: [Option<usize>; 4],
    poisoned: bool,
    source: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Outcome {
    pub win_probability: f64,
    pub death_probability: f64,
    pub expected_ticks: f64,
}
#[derive(Debug, Serialize)]
pub struct Evaluation {
    pub stats: Stats,
    pub outcome: Outcome,
    pub player_hit_chance: f64,
    pub npc_hit_chance: f64,
    pub player_max_hit: u16,
    pub npc_max_hit: u16,
    pub player_interval: u8,
    pub npc_interval: u8,
}
#[derive(Debug, Serialize)]
pub struct FightResult {
    pub status: &'static str,
    pub npc_id: i32,
    pub npc: String,
    pub weapon: String,
    pub style: Style,
    pub poison_changes_result: &'static str,
    pub reasons: Vec<String>,
    pub candidate: Option<Evaluation>,
    pub minimum: Option<Evaluation>,
    pub previous_level: Option<Evaluation>,
    pub sources: Vec<String>,
    pub timing: &'static str,
}

fn unsupported(row: &NpcNameRow, tables: &CombatTables, count: u8, weapon: &Weapon) -> Vec<String> {
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
    if row.poison_severity != Some(0) || weapon.poisoned {
        reasons.push("poison changes survival and timing; its probability is not modelled".into());
    }
    if let Some(unknown) = risk.unknown_for(true) {
        reasons.push(format!("product risk class: {unknown:?}"));
    }
    reasons
}

pub fn calculate(query: &FightQuery, tables: &CombatTables) -> Result<FightResult, FightError> {
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
    let weapons: Vec<_> = facts
        .weapons
        .iter()
        .filter(|weapon| {
            query.weapon.matches(
                weapon.id,
                &weapon.config,
                selected
                    .items()
                    .iter()
                    .find(|i| i.id == weapon.id)
                    .and_then(|i| i.name.as_deref()),
            )
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
    let reasons = unsupported(row, tables, query.npc_count, weapon);
    let mut result = FightResult {
        status: if reasons.is_empty() {
            "modelled"
        } else {
            "not_modelled"
        },
        npc_id: row.id,
        npc: row.config.clone(),
        weapon: weapon.config.clone(),
        style: query.style,
        poison_changes_result: if row.poison_severity == Some(0) && !weapon.poisoned {
            "no: neither participant inflicts poison"
        } else {
            "yes: poison can change survival; calibration needed"
        },
        reasons,
        candidate: None,
        minimum: None,
        previous_level: None,
        sources: vec![weapon.source.clone()],
        timing: if query.player_initiates {
            "player swing tick 0, damage tick 1; NPC first attack tick 1 + floor(interval/2), from retaliation flinch; queued player damage wins ties"
        } else {
            "NPC ready at tick 0; player swing tick 0, damage tick 1; queued player damage wins ties"
        },
    };
    if !result.reasons.is_empty() {
        return Ok(result);
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
    validate_weapon(selected, weapon, query.style)?;
    if let Some(stats) = query.candidate {
        result.candidate = Some(evaluate(
            stats,
            row,
            npc,
            weapon,
            query.style,
            query.player_initiates,
        )?);
    }
    for level in weapon.required_attack.max(1)..=99 {
        let evaluation = evaluate(
            Stats::policy(level),
            row,
            npc,
            weapon,
            query.style,
            query.player_initiates,
        )?;
        if evaluation.outcome.death_probability <= 0.01 {
            result.minimum = Some(evaluation);
            break;
        }
        result.previous_level = Some(evaluation);
    }
    if result.minimum.is_none() {
        result.status = "no_seed_within_policy";
    }
    Ok(result)
}

fn validate_weapon(
    selected: &SelectedGameData,
    weapon: &Weapon,
    style: Style,
) -> Result<(), FightError> {
    if !selected
        .items()
        .iter()
        .any(|item| item.id == weapon.id && item.alias.as_deref() == Some(&weapon.config))
    {
        return Err(FightError::Input(
            "weapon supplement disagrees with selected item identity".into(),
        ));
    }
    if selected
        .item_combat_bonus(weapon.id)
        .map(|b| b.strengthbonus)
        != Some(weapon.strength_bonus)
    {
        return Err(FightError::Input(
            "weapon strength supplement disagrees with selected facts".into(),
        ));
    }
    let tier = selected
        .melee_weapon_tier(weapon.id)
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
        .find(|fact| fact.obj_id == weapon.id)
        .ok_or_else(|| FightError::Input("selected weapon style missing".into()))?;
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
    Ok(())
}
fn roll(stat: u32, bonus: i32) -> Result<u32, FightError> {
    let value = i64::from(stat) * (i64::from(bonus) + 64);
    u32::try_from(value)
        .map_err(|_| FightError::Input("negative or overflowing combat roll".into()))
}
fn evaluate(
    stats: Stats,
    row: &NpcNameRow,
    npc: &NpcRoll,
    weapon: &Weapon,
    style: Style,
    initiates: bool,
) -> Result<Evaluation, FightError> {
    if !stats.valid() || stats.attack < weapon.required_attack {
        return Err(FightError::Input(
            "invalid levels or weapon Attack requirement not met".into(),
        ));
    }
    let [attack, strength, defence] = style.bonuses();
    let damage_type = weapon.damage_types[usize::from(style.code())]
        .ok_or_else(|| FightError::Input("style unavailable".into()))?;
    let npc_type = match row.damagetype.as_deref() {
        Some("^stab_style") => 0,
        Some("^slash_style") => 1,
        Some("^crush_style") => 2,
        _ => {
            return Err(FightError::Input(
                "NPC melee damage type unavailable".into(),
            ))
        }
    };
    let player_hit_chance = hit_chance(
        roll(
            u32::from(stats.attack) + 8 + attack,
            weapon.attack_bonus[damage_type],
        )?,
        roll(npc.defence + 9, npc.defence_bonus[damage_type])?,
    );
    let npc_hit_chance = hit_chance(
        roll(npc.attack + 9, npc.attack_bonus)?,
        roll(
            u32::from(stats.defence) + 8 + defence,
            weapon.defence_bonus[npc_type],
        )?,
    );
    let player_max_hit = u16::try_from(
        (roll(
            u32::from(stats.strength) + 8 + strength,
            weapon.strength_bonus,
        )? + 320)
            / 640,
    )
    .map_err(|_| FightError::Input("player maximum hit overflow".into()))?
    .min(npc.max_dealt);
    let npc_max_hit = u16::from(
        npc_max_hit(row).ok_or_else(|| FightError::Input("NPC maximum hit unavailable".into()))?,
    );
    let model = Model {
        player_hp: u16::from(stats.hitpoints),
        npc_hp: u16::try_from(row.hitpoints)
            .map_err(|_| FightError::Input("NPC Hitpoints out of range".into()))?,
        player_chance: player_hit_chance,
        npc_chance: npc_hit_chance,
        player_max: player_max_hit,
        npc_max: npc_max_hit,
        player_interval: weapon.attackrate,
        npc_interval: npc.attackrate,
        npc_first_tick: if initiates { 1 + npc.attackrate / 2 } else { 0 },
    };
    Ok(Evaluation {
        stats,
        outcome: solve(model)?,
        player_hit_chance,
        npc_hit_chance,
        player_max_hit,
        npc_max_hit,
        player_interval: weapon.attackrate,
        npc_interval: npc.attackrate,
    })
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
#[derive(Clone, Copy)]
struct Event {
    tick: u32,
    player: bool,
    maximum: u16,
    positive: f64,
    zero: f64,
}
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
fn solve(model: Model) -> Result<Outcome, FightError> {
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
                        win += event.positive;
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
    let start = index(usize::from(model.player_hp), usize::from(model.npc_hp), 0);
    let win = wins[start].clamp(0.0, 1.0);
    Ok(Outcome {
        win_probability: win,
        death_probability: 1.0 - win,
        expected_ticks: lengths[start] + f64::from(events[0].tick),
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
        }
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
            let exact = solve(model).unwrap();
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
        let outcome = solve(model).unwrap();
        assert!((outcome.win_probability - 2.0 / 3.0).abs() < 1e-14);
        assert!((outcome.expected_ticks - (1.0 + 4.0 / 3.0)).abs() < 1e-14);
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
        let outcome = solve(harmless).unwrap();
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
            id: 1321,
            config: "bronze_scimitar".into(),
            attack_bonus: [1, 7, -2],
            defence_bonus: [0, 1, 0],
            strength_bonus: 6,
            attackrate: 4,
            required_attack: 1,
            damage_types: [Some(1), Some(1), Some(0), Some(1)],
            poisoned: false,
            source: "test".into(),
        };
        validate_weapon(tables.selected(), &weapon, Style::Accurate).unwrap();
        let mut row = tables.npc(95).unwrap().clone();
        assert!(unsupported(&row, &tables, 1, &weapon).is_empty());
        assert!(!unsupported(&row, &tables, 2, &weapon).is_empty());
        row.bespoke = true;
        assert!(unsupported(&row, &tables, 1, &weapon)
            .iter()
            .any(|r| r.contains("bespoke")));
        row.bespoke = false;
        row.poison_severity = Some(5);
        assert!(unsupported(&row, &tables, 1, &weapon)
            .iter()
            .any(|r| r.contains("poison changes")));
        row.poison_severity = Some(0);
        row.strength = None;
        assert!(unsupported(&row, &tables, 1, &weapon)
            .iter()
            .any(|r| r.contains("unknown NPC maximum")));
    }
    #[test]
    fn npc_names_do_not_select_an_arbitrary_variant() {
        let selected = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
        let tables = CombatTables::build(selected).unwrap();
        let query = FightQuery {
            npc: Selector::Name("Skeleton".into()),
            facts: PathBuf::from("not-read"),
            weapon: default_weapon(),
            style: Style::Accurate,
            candidate: None,
            npc_count: 1,
            player_initiates: false,
        };
        assert!(calculate(&query, &tables)
            .unwrap_err()
            .to_string()
            .contains("ambiguous"));
        assert_eq!(Stats::policy(1).hitpoints, 10);
        assert_eq!(Stats::policy(11).hitpoints, 11);
    }
}
