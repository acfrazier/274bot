//! QUEST-WATCH seed-only scenarios: log one fixed account in, apply one
//! authored seed plan with per-account cheats, prove the exact account
//! state from the snapshot, then log out. No script ever Starts here;
//! the panel run that later logs the account in owns the quest.
//!
//! The plan is a JSON file named by `QUEST_WATCH_SEED`, translated from
//! a QUEST-TERRA `seed.json` by the evidence-tree harness
//! (`QUEST-WATCH-1029/scripts/quest_watch.py`). Completed-quest varps
//! and values arrive already resolved from the quest catalog; this
//! module only resolves item aliases and skill names against the
//! selected revision's content and refuses anything it cannot apply
//! exactly.
//!
//! `quest_watch_speed` is the matching one-cheat scenario: log in,
//! send `::speed <ms>` (`QUEST_WATCH_SPEED_MS`), wait for the fresh
//! system-chat confirmation, log out. `::speed` is world-wide; the
//! harness CLI refuses to select this scenario anywhere but Engine Q.

use crate::{Proof, Scenario, ScenarioSettings, Seed, Step, StepKind, Wait};
use api::selected::ClientRevision;
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::skill::Skill;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

/// Quest-list colour of a completed quest (`ctx::quest_status`).
const COLOUR_COMPLETE: i32 = 0xF800;
/// Quest-list colour of a quest that has not started.
const COLOUR_NOT_STARTED: i32 = 0xF80000;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SeedPlan {
    /// Catalog id of the quest the account is seeded for (receipts only).
    quest: String,
    /// The quest's own content varp; the seed resets it to not started.
    quest_varp: String,
    /// Further varps reset to 0 beside `quest_varp` (Shield of Arrav
    /// tracks one varp per gang; a reseed must clear both).
    #[serde(default)]
    reset_varps: Vec<String>,
    /// The quest's quest-tab display name, for the colour proof.
    quest_display: String,
    /// Exact base levels by skill name. A skill the plan omits must sit
    /// at its `minme` default (1, or 10 for Hitpoints).
    #[serde(default)]
    stats: BTreeMap<String, i32>,
    #[serde(default)]
    coins: i32,
    #[serde(default)]
    items: Vec<SeedItem>,
    #[serde(default)]
    completed: Vec<CompletedQuest>,
    /// Exact value for the `qp` varp (max of the seed's stated need and
    /// the catalog quest points of `completed`, computed by the CLI).
    #[serde(default)]
    quest_points: i32,
    /// Varp id of `qp` in the selected content pack, for the readback.
    /// The CLI refuses a plan with quest points when it cannot read it.
    #[serde(default)]
    qp_varp_id: Option<i32>,
    start_tile: [i32; 3],
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SeedItem {
    alias: String,
    count: i32,
    /// `inventory`, `bank` or `worn`.
    #[serde(rename = "where")]
    place: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompletedQuest {
    varp: String,
    value: i32,
    display: String,
}

fn command_step(name: &'static str, command: String, proof: Proof) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(move |client, _| api::interact::cheat(client, &command).is_sent()),
        },
        wait: Wait {
            arm: proof,
            budget_ticks: 100,
        },
    }
}

fn totals(rows: &[api::snapshot::ItemView]) -> BTreeMap<i32, i32> {
    let mut out = BTreeMap::new();
    for row in rows {
        if row.count > 0 {
            *out.entry(row.def.id).or_insert(0) += row.count;
        }
    }
    out
}

fn colour_is(snapshot: &GameSnapshot, display: &str, colour: i32) -> bool {
    snapshot
        .quest_statuses()
        .iter()
        .any(|row| row.name.eq_ignore_ascii_case(display) && row.colour == colour)
}

/// Runtime-selected `quest_watch_seed` scenario (`QUEST_WATCH_SEED`).
pub fn quest_watch_seed_scenario_from_env() -> Result<Scenario, String> {
    let path = PathBuf::from(
        std::env::var_os("QUEST_WATCH_SEED")
            .ok_or("QUEST_WATCH_SEED is required; set it to the seed plan JSON path")?,
    );
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    let plan: SeedPlan =
        serde_json::from_str(&text).map_err(|error| format!("parse QUEST_WATCH_SEED: {error}"))?;
    build_seed_scenario(plan)
}

fn build_seed_scenario(plan: SeedPlan) -> Result<Scenario, String> {
    let selected = api::game_data::for_revision(ClientRevision::R289)?;
    let stand = WorldTile {
        x: plan.start_tile[0],
        z: plan.start_tile[1],
        level: plan.start_tile[2],
    };
    if !(0..=16383).contains(&stand.x)
        || !(0..=16383).contains(&stand.z)
        || !(0..=3).contains(&stand.level)
    {
        return Err(format!(
            "field start_tile: ({},{},{}) is outside world coordinates",
            stand.x, stand.z, stand.level
        ));
    }
    if plan.quest_varp.is_empty()
        || !plan
            .quest_varp
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(format!(
            "field quest_varp: {:?} is not a content variable alias",
            plan.quest_varp
        ));
    }
    if !(0..=500).contains(&plan.quest_points) {
        return Err(format!(
            "field quest_points: {} is outside 0..=500",
            plan.quest_points
        ));
    }
    if plan.quest_points > 0 && plan.qp_varp_id.is_none() {
        return Err(
            "field qp_varp_id: required to read quest points back when quest_points > 0".into(),
        );
    }
    if plan.coins < 0 {
        return Err(format!("field coins: {} is negative", plan.coins));
    }

    // Exact expected stats: the plan's levels, `minme` defaults elsewhere.
    let mut expected_stats: BTreeMap<i32, i32> = BTreeMap::new();
    for (index, name) in Skill::names.iter().enumerate() {
        let default = if name.eq_ignore_ascii_case("hitpoints") {
            10
        } else {
            1
        };
        expected_stats.insert(index as i32, default);
    }
    let mut listed_stats: Vec<(i32, i32)> = Vec::new();
    for (name, level) in &plan.stats {
        let index = Skill::names
            .iter()
            .position(|candidate| candidate.eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("field stats: unknown skill {name:?}"))?;
        if !(1..=99).contains(level) {
            return Err(format!(
                "field stats: {name} level {level} is outside 1..=99"
            ));
        }
        expected_stats.insert(index as i32, *level);
        listed_stats.push((index as i32, *level));
    }

    let resolve = |name: &str| -> Result<(i32, String), String> {
        let item = selected
            .item_by_alias(name)
            .or_else(|| {
                selected.items().iter().find(|item| {
                    item.name
                        .as_deref()
                        .is_some_and(|known| known.eq_ignore_ascii_case(name))
                })
            })
            .ok_or_else(|| format!("field items: alias {name:?} is not in selected content"))?;
        let alias = item
            .alias
            .as_deref()
            .ok_or_else(|| format!("field items: {name:?} has no config alias for give"))?;
        Ok((item.id, alias.to_owned()))
    };

    let mut expected_inventory: BTreeMap<i32, (String, i32)> = BTreeMap::new();
    let mut expected_bank: BTreeMap<i32, (String, i32)> = BTreeMap::new();
    let mut worn: Vec<(i32, String)> = Vec::new();
    let mut merge = |map: &mut BTreeMap<i32, (String, i32)>,
                     alias: &str,
                     count: i32|
     -> Result<(), String> {
        let (id, canonical) = resolve(alias)?;
        let entry = map.entry(id).or_insert((canonical, 0));
        entry.1 = entry
            .1
            .checked_add(count)
            .ok_or("field items: quantity overflow")?;
        Ok(())
    };
    if plan.coins > 0 {
        merge(&mut expected_inventory, "coins", plan.coins)?;
    }
    for item in &plan.items {
        if item.count < 1 {
            return Err(format!(
                "field items: {:?} count {} must be positive",
                item.alias, item.count
            ));
        }
        match item.place.as_str() {
            "inventory" => merge(&mut expected_inventory, &item.alias, item.count)?,
            "bank" => merge(&mut expected_bank, &item.alias, item.count)?,
            "worn" => {
                if item.count != 1 {
                    return Err(format!(
                        "field items: worn {:?} count {} must be 1",
                        item.alias, item.count
                    ));
                }
                worn.push(resolve(&item.alias)?);
            }
            other => {
                return Err(format!(
                    "field items: {:?} has unknown where {other:?} (inventory, bank or worn)",
                    item.alias
                ));
            }
        }
    }
    for (index, done) in plan.completed.iter().enumerate() {
        if done.varp.is_empty()
            || !done
                .varp
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            || plan.completed[..index]
                .iter()
                .any(|other| other.varp == done.varp)
        {
            return Err(format!(
                "field completed: varp {:?} must be a unique content variable alias",
                done.varp
            ));
        }
        if done.varp == plan.quest_varp {
            return Err(format!(
                "field completed: the target quest {:?} cannot also be completed",
                plan.quest
            ));
        }
    }

    let tab = Proof::SideTabAvailable { index: 3 };
    let mut steps = crate::script_live_seed_steps();
    steps.push(command_step(
        "reset stats before seeding",
        "minme".into(),
        Proof::StatAtMost { id: 1, max: 1 },
    ));
    steps.push(command_step("clear worn items", "~clearinv worn".into(), tab));
    steps.push(command_step("clear inventory", "~clearinv".into(), tab));
    steps.push(command_step("clear bank", "~clearbank".into(), tab));
    for (index, level) in &listed_stats {
        steps.push(command_step(
            "seed exact skill level",
            format!("setstat {} {}", Skill::names[*index as usize], level),
            tab,
        ));
    }
    steps.push(command_step(
        "reset the target quest to not started",
        format!("setvar {} 0", plan.quest_varp),
        tab,
    ));
    for varp in &plan.reset_varps {
        if varp.is_empty()
            || !varp
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            || *varp == plan.quest_varp
        {
            return Err(format!(
                "field reset_varps: {varp:?} must be a content variable alias other than quest_varp"
            ));
        }
        steps.push(command_step(
            "reset one further quest varp to not started",
            format!("setvar {varp} 0"),
            tab,
        ));
    }
    for done in &plan.completed {
        steps.push(command_step(
            "seed one completed quest varp",
            format!("setvar {} {}", done.varp, done.value),
            tab,
        ));
    }
    steps.push(command_step(
        "seed exact quest points",
        format!("setvar qp {}", plan.quest_points),
        tab,
    ));
    for (id, (alias, count)) in &expected_bank {
        let _ = id;
        steps.push(command_step(
            "seed one bank item",
            format!("givebank {alias} {count}"),
            tab,
        ));
    }
    let mut expected_equipment: Vec<i32> = Vec::new();
    for (id, alias) in &worn {
        steps.push(command_step(
            "seed one worn item",
            format!("give {alias} 1"),
            Proof::ItemId { id: *id, count: 1 },
        ));
        steps.push(crate::scenarios::combat::wear_combat_item_step(
            "wear one seeded item",
            *id,
        ));
        expected_equipment.push(*id);
    }
    for (id, (alias, count)) in &expected_inventory {
        steps.push(command_step(
            "seed one inventory item",
            format!("give {alias} {count}"),
            Proof::ItemId {
                id: *id,
                count: *count,
            },
        ));
    }
    steps.push(Step {
        name: "relog after the seed so quest colours refresh",
        kind: StepKind::Relog,
        wait: Wait {
            arm: tab,
            budget_ticks: 600,
        },
    });
    steps.push(command_step(
        "stand on the seed start tile",
        api::interact::tele_args(stand.level, stand.x, stand.z),
        Proof::Arrived {
            x: stand.x,
            z: stand.z,
            level: stand.level,
        },
    ));

    let gate_stats = expected_stats.clone();
    let gate_listed: Vec<i32> = listed_stats.iter().map(|(index, _)| *index).collect();
    let gate_inventory: BTreeMap<i32, i32> = expected_inventory
        .iter()
        .map(|(id, (_, count))| (*id, *count))
        .collect();
    let gate_equipment = expected_equipment.clone();
    let gate_completed: Vec<String> = plan
        .completed
        .iter()
        .map(|done| done.display.clone())
        .collect();
    let gate_display = plan.quest_display.clone();
    let gate_points = plan.quest_points;
    let gate_qp_id = plan.qp_varp_id;
    steps.push(Step {
        name: "prove the exact seeded stats, kit, tile and quest state",
        kind: StepKind::Await {
            evidence: "exact quest-watch seed: stats, inventory, worn kit, stand, quest colours and quest points",
            ready: Box::new(move |snapshot| {
                if !snapshot.ingame() || snapshot.scene_state() != 2 {
                    return false;
                }
                if snapshot.tile() != Some((stand.x, stand.z, stand.level)) {
                    return false;
                }
                for (index, expected) in &gate_stats {
                    let Some(stat) = snapshot.stats().iter().find(|stat| stat.index == *index)
                    else {
                        if gate_listed.contains(index) {
                            return false;
                        }
                        continue;
                    };
                    if stat.base != *expected || stat.effective != *expected {
                        return false;
                    }
                }
                if totals(snapshot.inventory()) != gate_inventory {
                    return false;
                }
                let worn_ids: Vec<i32> = snapshot
                    .equipment()
                    .iter()
                    .filter(|item| item.count > 0)
                    .map(|item| item.def.id)
                    .collect();
                if worn_ids.len() != gate_equipment.len()
                    || !gate_equipment.iter().all(|id| worn_ids.contains(id))
                {
                    return false;
                }
                if !gate_completed
                    .iter()
                    .all(|display| colour_is(snapshot, display, COLOUR_COMPLETE))
                {
                    return false;
                }
                if !colour_is(snapshot, &gate_display, COLOUR_NOT_STARTED) {
                    return false;
                }
                if let Some(id) = gate_qp_id {
                    let observed = snapshot
                        .varps()
                        .iter()
                        .find(|row| row.index == id)
                        .map_or(0, |row| row.value);
                    if observed != gate_points {
                        return false;
                    }
                }
                true
            }),
            summary: None,
        },
        wait: Wait {
            arm: tab,
            budget_ticks: 200,
        },
    });

    // Bank contents are only observable with the bank open. `~bank`
    // opens it anywhere; the relog after the proof closes it and lands
    // the account back on its saved tile.
    steps.push(command_step(
        "open the bank to prove its seeded contents",
        "~bank".into(),
        Proof::BankOpen,
    ));
    let gate_bank: BTreeMap<i32, i32> = expected_bank
        .iter()
        .map(|(id, (_, count))| (*id, *count))
        .collect();
    steps.push(Step {
        name: "prove the exact seeded bank",
        kind: StepKind::Await {
            evidence: "exact quest-watch seed bank contents",
            ready: Box::new(move |snapshot| {
                snapshot.bank_loaded() && totals(snapshot.bank()) == gate_bank
            }),
            summary: None,
        },
        wait: Wait {
            arm: Proof::BankOpen,
            budget_ticks: 200,
        },
    });
    steps.push(Step {
        name: "relog to close the bank on the start tile",
        kind: StepKind::Relog,
        wait: Wait {
            arm: tab,
            budget_ticks: 600,
        },
    });
    steps.push(Step {
        name: "prove the start tile after the bank relog",
        kind: StepKind::Await {
            evidence: "quest-watch seed start tile after the final relog",
            ready: Box::new(move |snapshot| {
                snapshot.ingame()
                    && snapshot.scene_state() == 2
                    && snapshot.tile() == Some((stand.x, stand.z, stand.level))
                    && snapshot.bank_component_id() < 0
            }),
            summary: None,
        },
        wait: Wait {
            arm: tab,
            budget_ticks: 200,
        },
    });
    steps.push(Step {
        name: "log out with the seed saved",
        kind: StepKind::Perform {
            send: Box::new(|client, _| {
                let ifaces = std::sync::Arc::clone(&client.ifaces);
                api::interact::logout(client, ifaces.as_slice())
            }),
        },
        wait: Wait {
            arm: Proof::LoggedOut,
            budget_ticks: 200,
        },
    });

    Ok(Scenario {
        name: "quest_watch_seed",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::LoggedOut,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: Duration::from_secs(900),
            start_script: None,
            terminal_shot: Some("quest_watch_seed"),
            nav: crate::gold_script_nav(),
            ..Default::default()
        },
    })
}

/// Runtime-selected `quest_watch_speed` scenario (`QUEST_WATCH_SPEED_MS`).
/// World-wide effect: Engine Q only, enforced by the harness CLI.
pub fn quest_watch_speed_scenario_from_env() -> Result<Scenario, String> {
    let raw = std::env::var("QUEST_WATCH_SPEED_MS")
        .map_err(|_| "QUEST_WATCH_SPEED_MS is required (world tick ms, Engine Q only)")?;
    let ms: u32 = raw
        .parse()
        .map_err(|error| format!("QUEST_WATCH_SPEED_MS {raw:?}: {error}"))?;
    if !(50..=5000).contains(&ms) {
        return Err(format!("QUEST_WATCH_SPEED_MS {ms} is outside 50..=5000"));
    }
    let baseline = std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0));
    let send_baseline = std::sync::Arc::clone(&baseline);
    let mut steps = crate::script_live_seed_steps();
    steps.push(Step {
        name: "send the world tick speed once",
        kind: StepKind::Perform {
            send: Box::new(move |client, snapshot| {
                send_baseline.store(
                    crate::quest_fast::chat_baseline(snapshot),
                    std::sync::atomic::Ordering::SeqCst,
                );
                crate::quest_fast::send_tick_speed(client, ms).is_ok()
            }),
        },
        wait: Wait {
            arm: Proof::SideTabAvailable { index: 3 },
            budget_ticks: 100,
        },
    });
    steps.push(Step {
        name: "prove the fresh world speed confirmation",
        kind: StepKind::Await {
            evidence: "fresh exact system-chat world speed confirmation",
            ready: Box::new(move |snapshot| {
                crate::quest_fast::tick_speed_confirmed(
                    snapshot,
                    baseline.load(std::sync::atomic::Ordering::SeqCst),
                    ms,
                )
            }),
            summary: None,
        },
        wait: Wait {
            arm: Proof::SideTabAvailable { index: 3 },
            budget_ticks: 200,
        },
    });
    steps.push(Step {
        name: "log out after the speed change",
        kind: StepKind::Perform {
            send: Box::new(|client, _| {
                let ifaces = std::sync::Arc::clone(&client.ifaces);
                api::interact::logout(client, ifaces.as_slice())
            }),
        },
        wait: Wait {
            arm: Proof::LoggedOut,
            budget_ticks: 200,
        },
    });
    Ok(Scenario {
        name: "quest_watch_speed",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::LoggedOut,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: Duration::from_secs(600),
            start_script: None,
            terminal_shot: Some("quest_watch_speed"),
            nav: crate::gold_script_nav(),
            ..Default::default()
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(text: &str) -> SeedPlan {
        serde_json::from_str(text).expect("plan parses")
    }

    fn refusal(result: Result<Scenario, String>, what: &str) -> String {
        match result {
            Ok(_) => panic!("{what} must refuse"),
            Err(error) => error,
        }
    }

    #[test]
    fn seed_plan_rejects_unknown_fields_and_places() {
        let bad_place = plan(
            r#"{"quest":"priest","quest_varp":"prieststart","quest_display":"The Restless Ghost","items":[{"alias":"lobster","count":1,"where":"pocket"}],"start_tile":[3243,3206,0]}"#,
        );
        let error = refusal(build_seed_scenario(bad_place), "unknown where");
        assert!(error.contains("field items"), "{error}");
        assert!(error.contains("pocket"), "{error}");
    }

    #[test]
    fn seed_plan_rejects_unknown_skill_and_alias() {
        let bad_skill = plan(
            r#"{"quest":"priest","quest_varp":"prieststart","quest_display":"The Restless Ghost","stats":{"luck":5},"start_tile":[3243,3206,0]}"#,
        );
        let error = refusal(build_seed_scenario(bad_skill), "unknown skill");
        assert!(error.contains("field stats"), "{error}");

        let bad_alias = plan(
            r#"{"quest":"priest","quest_varp":"prieststart","quest_display":"The Restless Ghost","items":[{"alias":"not_an_item_xyz","count":1,"where":"bank"}],"start_tile":[3243,3206,0]}"#,
        );
        let error = refusal(build_seed_scenario(bad_alias), "unknown alias");
        assert!(error.contains("field items"), "{error}");
    }

    #[test]
    fn seed_plan_builds_a_logout_terminal_scenario() {
        let priest = plan(
            r#"{"quest":"priest","quest_varp":"prieststart","quest_display":"The Restless Ghost","stats":{"hitpoints":30,"defence":30},"start_tile":[3243,3206,0]}"#,
        );
        let scenario = build_seed_scenario(priest).expect("priest plan builds");
        assert_eq!(scenario.name, "quest_watch_seed");
        assert!(matches!(scenario.proof, Proof::LoggedOut));
        assert_eq!(scenario.settings.start_script, None);
        assert!(scenario.seed.mainland);
    }
}
