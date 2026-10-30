//! Live native-runner poison interruption and resume proofs using real content.
#![cfg(all(feature = "live-harness", feature = "test-support"))]

#[path = "common/quester_live.rs"]
mod quester_live;

use api::condition::{Exposure, PoisonState};
use api::interact::{Interactions, OpTarget, SendResult};
use api::snapshot::{AnimationUpdateView, GameSnapshot, WorldTile};
use quester_live::{Cell, LiveProfileControl, Mode, PathCell};
use script::native::{NativePhase, ScriptStatus, StatusValue};
use script::quester::registry::{self, FolderSource};
use script::survival::{
    cure::CureWhy,
    math::{NeedWhy, SurvivalNeed},
    Decision, Owner,
};
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const GATHERER_STAND: WorldTile = WorldTile {
    x: 3120,
    z: 3267,
    level: 0,
};
const MINING_STAND: WorldTile = WorldTile {
    x: 3253,
    z: 3420,
    level: 0,
};
const DESERT_CAPTAIN_STAND: WorldTile = WorldTile {
    x: 3270,
    z: 3029,
    level: 0,
};
// m40_148.jm2:4888 places the chest at local (1,35) on level 0.
const TWISTED_CHEST_TILE: WorldTile = WorldTile {
    x: 40 * 64 + 1,
    z: 148 * 64 + 35,
    level: 0,
};

const WOODCUTTING_STAT: i32 = 8;
const MINING_STAT: i32 = 14;
const HITPOINTS_STAT: i32 = 3;
const WILLOW_LOG_ID: i32 = 1519;
const COPPER_ORE_ID: i32 = 436;
const ANTIPOISON_ALIAS: &str = "1doseantipoison";
const RUNE_AXE_ALIAS: &str = "rune_axe";
const RUNE_PICKAXE_ALIAS: &str = "rune_pickaxe";
const SINISTER_KEY_ALIAS: &str = "sinister_key";
const SINISTER_KEY_ID: i32 = 993;
const TWISTED_CHEST_LOC_ID: i32 = 377;
// agility_dungeon.rs2:113-120 adds nine reward items.
const TWISTED_CHEST_REWARD_COUNT: i32 = 9;
// p_delay(3) resumes at +4 ticks; queue(..., 0) runs on the next eligible
// player queue pass: PlayerOps.ts:376-379, Player.ts:903-912.
const TWISTED_CHEST_POISON_SIGNAL_TICKS: u32 = 5;
// The debugproc's queue(..., 0) runs on the first eligible queue pass;
// cheat_other.rs2:1-3, Player.ts:903-912.
const DEBUGPROC_POISON_SIGNAL_TICKS: u32 = 2;
const DESERT_STAGE_REPLY: &str = "get desertrescue: 3";
const DESERT_CAPTAIN_OPENING_OPTIONS: [&str; 3] =
    ["Wow! A real captain!", "You there!", "Hey ugly!"];
const POISON_CHAT: &str = "You have been poisoned!";
// Only the provisioning cell uses this Engine L debugproc to seed its
// account's own state (`cheat_other.rs2:1-3`); other cells use the authored
// Yanille chest. Both sources reach this chat through `poison.rs2:12-18`.
const POISON_DEBUG_COMMAND: &str = "~poison 27";
const CURE_CHAT: &str = "You drink some of your antipoison potion.";
// Login schedules the poison timer at 30 ticks; the next observed game tick
// proves its zero-value clear: poison.rs2:20-23,69-75.
const TIMER_CLEAR_TICKS: u32 = 31;
const Q_UNKNOWN: u8 = 11;
const LETHAL_OUTLOOK: u16 = 330;
const BASE40_SKILL_LEVEL: i32 = 40;

const COOK_PATH_JSON: &str = include_str!("../../script/paths/289/cook.json");

#[derive(Clone, Copy, PartialEq, Eq)]
enum Activity {
    Woodcutting,
    Mining,
    Dialogue,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PoisonSource {
    TwistedChest,
    AccountStateDebugProc,
}

struct Witness {
    activity: Activity,
    poison_source: PoisonSource,
    expected_key_at_start: i32,
    expected_inventory_extra: i32,
    poison_during_provisioning: bool,
    skill_name: &'static str,
    skill_stat: i32,
    resource_id: Option<i32>,
    tool_id: Option<i32>,
    antipoison_id: i32,
    stand: WorldTile,
    desert_stage_reply: Option<String>,
    desert_stage_reply_tick: Option<u32>,
    baseline_chat: Option<i32>,
    baseline_xp: i32,
    baseline_resource: i32,
    baseline_tool: i32,
    expected_tool_at_start: i32,
    key_id: i32,
    baseline_key: i32,
    baseline_antipoison: i32,
    baseline_animation_serial: Option<u32>,
    login_tick: Option<u32>,
    timer_phase_ready_tick: Option<u32>,
    provisioning_tick: Option<u32>,
    work_active_tick: Option<u32>,
    work_animation_tick: Option<u32>,
    work_animation: Option<AnimationUpdateView>,
    poison_source_tick: Option<u32>,
    poison_seed_bank_open: Option<bool>,
    poison_seed_tool_count: Option<i32>,
    poison_source_modal: Option<i32>,
    poison_tick: Option<u32>,
    first_kind2_mark_tick: Option<u32>,
    need_tick: Option<u32>,
    need_xp: Option<i32>,
    need_resource: Option<i32>,
    need_status: Option<String>,
    need_phase: Option<String>,
    need_step: Option<String>,
    need_report_decision: Option<String>,
    need_report_why: Option<String>,
    need_report: Option<String>,
    need_report_owner: Option<String>,
    need_condition_tick: Option<u32>,
    need_condition_poison: Option<String>,
    need_unlatched_poison: Option<bool>,
    need_outlook_total: Option<u16>,
    need_outlook_per_fire: Option<u8>,
    need_outlook_pending: Option<bool>,
    need_hp: Option<u8>,
    need_hp_max: Option<u8>,
    need_food: Option<u8>,
    need_antipoison: Option<u8>,
    need_in_combat: Option<bool>,
    need_no_hit_at_edge: Option<bool>,
    need_page_open_tick: Option<u32>,
    need_page_options: Vec<String>,
    cure_tick: Option<u32>,
    dose_drop_tick: Option<u32>,
    drink_decision_tick: Option<u32>,
    drink_decision_modal: Option<i32>,
    settled_tick: Option<u32>,
    immune_tick: Option<u32>,
    dialogue_page_tick: Option<u32>,
    profile_change_tick: Option<u32>,
    profile_change_modal: Option<i32>,
    profile_change_options: Vec<String>,
    dialogue_closed_tick: Option<u32>,
    dialogue_reopened_tick: Option<u32>,
    dialogue_reopened_options: Vec<String>,
    dialogue_reopened_step: Option<String>,
}

fn new_witness(
    activity: Activity,
    skill_name: &'static str,
    skill_stat: i32,
    resource_id: Option<i32>,
    tool_alias: Option<&str>,
    poison_source: PoisonSource,
    stand: WorldTile,
) -> Arc<Mutex<Witness>> {
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289)
        .expect("selected R289 data");
    let item_id = |alias: &str| {
        selected
            .item_by_alias(alias)
            .unwrap_or_else(|| panic!("missing selected item alias {alias}"))
            .id
    };
    let key_id = item_id(SINISTER_KEY_ALIAS);
    let (expected_key_at_start, expected_inventory_extra) =
        match (poison_source, activity) {
            (PoisonSource::TwistedChest, Activity::Dialogue) => {
                (0, TWISTED_CHEST_REWARD_COUNT)
            }
            (PoisonSource::TwistedChest, _) => (1, 0),
            (PoisonSource::AccountStateDebugProc, _) => (0, 0),
        };
    let resource_id = resource_id.inspect(|&id| {
        selected
            .item_by_id(id)
            .unwrap_or_else(|| panic!("missing selected resource item id {id}"));
    });
    Arc::new(Mutex::new(Witness {
        activity,
        poison_source,
        expected_key_at_start,
        expected_inventory_extra,
        poison_during_provisioning: false,
        skill_name,
        skill_stat,
        resource_id,
        tool_id: tool_alias.map(item_id),
        antipoison_id: item_id(ANTIPOISON_ALIAS),
        key_id,
        stand,
        desert_stage_reply: None,
        desert_stage_reply_tick: None,
        baseline_chat: None,
        baseline_xp: 0,
        baseline_resource: 0,
        baseline_tool: 0,
        expected_tool_at_start: i32::from(tool_alias.is_some()),
        baseline_antipoison: 0,
        baseline_key: 0,
        baseline_animation_serial: None,
        login_tick: None,
        timer_phase_ready_tick: None,
        provisioning_tick: None,
        work_active_tick: None,
        work_animation_tick: None,
        work_animation: None,
        poison_source_tick: None,
        poison_seed_bank_open: None,
        poison_seed_tool_count: None,
        poison_source_modal: None,
        poison_tick: None,
        first_kind2_mark_tick: None,
        need_tick: None,
        need_xp: None,
        need_resource: None,
        need_status: None,
        need_phase: None,
        need_step: None,
        need_report_decision: None,
        need_report_why: None,
        need_report: None,
        need_report_owner: None,
        need_condition_tick: None,
        need_condition_poison: None,
        need_unlatched_poison: None,
        need_outlook_total: None,
        need_outlook_per_fire: None,
        need_outlook_pending: None,
        need_hp: None,
        need_hp_max: None,
        need_food: None,
        need_antipoison: None,
        need_in_combat: None,
        need_no_hit_at_edge: None,
        need_page_open_tick: None,
        need_page_options: Vec::new(),
        cure_tick: None,
        dose_drop_tick: None,
        drink_decision_tick: None,
        drink_decision_modal: None,
        settled_tick: None,
        immune_tick: None,
        dialogue_page_tick: None,
        profile_change_tick: None,
        profile_change_modal: None,
        profile_change_options: Vec::new(),
        dialogue_closed_tick: None,
        dialogue_reopened_tick: None,
        dialogue_reopened_options: Vec::new(),
        dialogue_reopened_step: None,
    }))
}

fn status_text<'a>(status: Option<&'a ScriptStatus>, key: &str) -> Option<&'a str> {
    status?.fields.iter().find_map(|field| match &field.value {
        StatusValue::Text(value) if field.key == key => Some(value.as_ref()),
        _ => None,
    })
}

fn stat_xp(snapshot: &GameSnapshot, id: i32) -> i32 {
    snapshot
        .stats()
        .iter()
        .find(|stat| stat.index == id)
        .map_or(0, |stat| stat.xp)
}

fn stat_base(snapshot: &GameSnapshot, id: i32) -> i32 {
    snapshot
        .stats()
        .iter()
        .find(|stat| stat.index == id)
        .map_or(0, |stat| stat.base)
}

fn latest_chat(snapshot: &GameSnapshot) -> i32 {
    snapshot
        .chat_lines()
        .iter()
        .map(|line| line.sequence)
        .max()
        .unwrap_or(0)
}

fn item_count(snapshot: &GameSnapshot, id: i32) -> i32 {
    snapshot.inv_count(id)
}

fn inventory_total(snapshot: &GameSnapshot) -> i32 {
    snapshot.inv().iter().map(|(_, count)| *count).sum()
}

fn active_hitmark(snapshot: &GameSnapshot, kind: Option<i32>) -> Option<bool> {
    let hitmarks = snapshot.hitmarks()?;
    Some(hitmarks.marks.iter().any(|mark| {
        kind.is_none_or(|kind| mark.kind == kind)
            && mark.value > 0
            && mark.cycle > hitmarks.loop_cycle
    }))
}

fn in_combat(snapshot: &GameSnapshot) -> Option<bool> {
    snapshot
        .local_player()
        .map(|player| player.player.actor.in_combat)
}

fn insert_before_start(scenario: &mut scenario::Scenario, steps: Vec<scenario::Step>) {
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, scenario::StepKind::StartScript))
        .expect("fixture starts its native script");
    scenario.steps.splice(start..start, steps);
}

fn fixture(
    activity: Activity,
    label: &str,
    stand: WorldTile,
    extra_items: &[(&str, i32)],
    seed_vars: &[(&str, i32)],
    before_relog: Vec<scenario::Step>,
    mode: Mode,
) -> Cell {
    let mut cell = quester_live::path_cell(PathCell {
        quest: "cook",
        display: "Cook's Assistant",
        label: label.to_owned(),
        stage: Some("cook:0"),
        loadout: None,
        extra_items,
        seed_vars,
        stand,
        mode,
        before_relog,
    })
    .expect("build account-owned live fixture");

    let mut before_start = Vec::new();
    if activity == Activity::Dialogue {
        before_start.push(scenario::Step {
            name: "prove the Desert Rescue stage from the server",
            kind: scenario::StepKind::Perform {
                send: Box::new(|client, _| {
                    api::interact::cheat(client, "getvar desertrescue").is_sent()
                }),
            },
            wait: scenario::Wait {
                arm: scenario::Proof::FreshChat {
                    needle: DESERT_STAGE_REPLY,
                },
                budget_ticks: 3,
            },
        });
    }
    before_start.push(scenario::Step {
        name: "seed Hitpoints 30 after the fixture profile proof",
        kind: scenario::StepKind::Perform {
            send: Box::new(|client, _| {
                api::interact::cheat(client, "setstat hitpoints 30").is_sent()
            }),
        },
        wait: scenario::Wait {
            arm: scenario::Proof::Stat {
                id: HITPOINTS_STAT,
                min: 30,
            },
            budget_ticks: 10,
        },
    });
    insert_before_start(&mut cell.scenario, before_start);
    cell
}

fn bank_seed_step(alias: &'static str) -> scenario::Step {
    let command = format!("givebank {alias} 1");
    scenario::Step {
        name: "seed the rune pickaxe in the account bank before fixture relog",
        kind: scenario::StepKind::Perform {
            send: Box::new(move |client, _| api::interact::cheat(client, &command).is_sent()),
        },
        wait: scenario::Wait {
            arm: scenario::Proof::SideTabAvailable { index: 3 },
            budget_ticks: 3,
        },
    }
}

fn add_login_timer_wait_before_start(
    scenario: &mut scenario::Scenario,
    witness: Arc<Mutex<Witness>>,
) {
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, scenario::StepKind::StartScript))
        .expect("fixture starts its native script");
    let wait_witness = Arc::clone(&witness);
    let wait = scenario::Step {
        name: "wait for the 31-tick login poison timer to clear before provisioning",
        kind: scenario::StepKind::Await {
            evidence: "31 observed ticks after login; poison.rs2:20-23,69-75",
            ready: Box::new(move |snapshot| {
                if !snapshot.ingame()
                    || snapshot.scene_state() != 2
                    || snapshot.local_player().is_none()
                {
                    return false;
                }
                let mut witness = wait_witness.lock().expect("survival witness");
                let login_tick = *witness.login_tick.get_or_insert(snapshot.tick());
                snapshot.tick().saturating_sub(login_tick) >= TIMER_CLEAR_TICKS
            }),
            summary: None,
        },
        wait: scenario::Wait {
            arm: scenario::Proof::Arrived {
                x: witness.lock().expect("survival witness").stand.x,
                z: witness.lock().expect("survival witness").stand.z,
                level: witness.lock().expect("survival witness").stand.level,
            },
            budget_ticks: TIMER_CLEAR_TICKS,
        },
    };
    scenario.steps.splice(start..start, [wait]);
}

fn install_path_source(evidence: &std::path::Path, name: &str, steps: Value) -> SourceRestore {
    let folder = evidence.join("native-cure-path").join(name);
    std::fs::create_dir_all(&folder).expect("create isolated path source");
    let mut document: Value = serde_json::from_str(COOK_PATH_JSON).expect("embedded Cook path");
    document["roles"][0]["sequences"][0]["steps"] = steps;
    std::fs::write(
        folder.join("index.json"),
        br#"{"schema":1,"paths":[{"id":"cook","file":"cook.json"}]}"#,
    )
    .expect("write isolated path index");
    std::fs::write(
        folder.join("cook.json"),
        serde_json::to_vec_pretty(&document).expect("serialize live Path"),
    )
    .expect("write isolated Cook Path");
    let previous = registry::source();
    registry::set_source(FolderSource {
        enabled: true,
        folder,
    });
    SourceRestore(previous)
}

fn add_poison_steps(
    scenario: &mut scenario::Scenario,
    witness: Arc<Mutex<Witness>>,
    before_start: bool,
    budget_ticks: u32,
) {
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, scenario::StepKind::StartScript))
        .expect("fixture starts its native script");
    let phase_witness = Arc::clone(&witness);
    let phase_arm = {
        let witness = witness.lock().expect("survival witness");
        if witness.poison_during_provisioning {
            scenario::Proof::BankOpen
        } else {
            scenario::Proof::Arrived {
                x: witness.stand.x,
                z: witness.stand.z,
                level: witness.stand.level,
            }
        }
    };
    let phase = scenario::Step {
        name: "wait for cleared login poison timer and authored activity readiness",
        kind: scenario::StepKind::Await {
            evidence: "31 observed ticks after login and authored activity readiness",
            ready: Box::new(move |snapshot| {
                if !snapshot.ingame()
                    || snapshot.scene_state() != 2
                    || snapshot.local_player().is_none()
                {
                    return false;
                }
                let mut witness = phase_witness.lock().expect("survival witness");
                if witness.baseline_chat.is_none() {
                    witness.baseline_chat = Some(latest_chat(snapshot));
                }
                let login_tick = *witness.login_tick.get_or_insert(snapshot.tick());
                let active = if witness.poison_during_provisioning {
                    witness.provisioning_tick.is_some()
                } else {
                    match witness.activity {
                        Activity::Woodcutting => {
                            stat_xp(snapshot, witness.skill_stat) > witness.baseline_xp
                        }
                        Activity::Mining => witness.work_animation.is_some_and(|animation| {
                            snapshot
                                .local_player()
                                .and_then(|player| player.animation_update)
                                .is_some_and(|current| {
                                    current.serial == animation.serial && current.sequence >= 0
                                })
                        }),
                        Activity::Dialogue => true,
                    }
                };
                if active
                    && witness.activity != Activity::Dialogue
                    && !witness.poison_during_provisioning
                {
                    witness.work_active_tick.get_or_insert(snapshot.tick());
                }
                let timer_cleared = snapshot.tick().saturating_sub(login_tick) >= TIMER_CLEAR_TICKS;
                if active && timer_cleared {
                    witness.timer_phase_ready_tick = Some(snapshot.tick());
                    true
                } else {
                    false
                }
            }),
            summary: None,
        },
        wait: scenario::Wait {
            arm: phase_arm,
            budget_ticks,
        },
    };
    let source = witness.lock().expect("survival witness").poison_source;
    let mut inserted = vec![phase];
    match source {
        PoisonSource::TwistedChest => {
            inserted.extend(twisted_chest_steps(Arc::clone(&witness), before_start));
        }
        PoisonSource::AccountStateDebugProc => {
            inserted.extend(account_state_poison_steps(Arc::clone(&witness)));
        }
    }
    let insertion = if before_start { start } else { start + 1 };
    scenario.steps.splice(insertion..insertion, inserted);
}

fn teleport_step(name: &'static str, tile: WorldTile) -> scenario::Step {
    let command = api::interact::tele_args(tile.level, tile.x, tile.z);
    scenario::Step {
        name,
        kind: scenario::StepKind::Perform {
            send: Box::new(move |client, _| api::interact::cheat(client, &command).is_sent()),
        },
        wait: scenario::Wait {
            arm: scenario::Proof::Arrived {
                x: tile.x,
                z: tile.z,
                level: tile.level,
            },
            budget_ticks: 10,
        },
    }
}

fn twisted_chest_steps(
    witness: Arc<Mutex<Witness>>,
    before_start: bool,
) -> Vec<scenario::Step> {
    let mut steps = vec![
        teleport_step("teleport to the real Yanille twisted chest", TWISTED_CHEST_TILE),
        scenario::Step {
            name: "observe the real Yanille twisted chest",
            kind: scenario::StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: scenario::Wait {
                arm: scenario::Proof::LocIdNear {
                    id: TWISTED_CHEST_LOC_ID,
                    x: TWISTED_CHEST_TILE.x,
                    z: TWISTED_CHEST_TILE.z,
                    level: TWISTED_CHEST_TILE.level,
                    radius: 2,
                },
                // The world map places the authored object on this tile;
                // agility_dungeon.rs2:122 only changes its inactive stage.
                budget_ticks: 10,
            },
        },
    ];
    let source_witness = Arc::clone(&witness);
    steps.push(scenario::Step {
        name: "use the account-owned sinister key on the real chest",
        kind: scenario::StepKind::Perform {
            send: Box::new(move |client, snapshot| {
                if snapshot.modals().chat != -1 || active_hitmark(snapshot, None) != Some(false) {
                    return false;
                }
                let Some(loc) = snapshot
                    .locs()
                    .iter()
                    .find(|loc| loc.id == TWISTED_CHEST_LOC_ID)
                else {
                    return false;
                };
                let Some(key) = snapshot
                    .inventory()
                    .iter()
                    .find(|item| item.def.id == SINISTER_KEY_ID && item.count > 0)
                else {
                    return false;
                };
                if !matches!(
                    Interactions::new(snapshot, client).use_item_on(key, OpTarget::Loc(loc)),
                    SendResult::Sent { .. }
                ) {
                    return false;
                }
                let mut witness = source_witness.lock().expect("survival witness");
                witness.poison_source_tick = Some(snapshot.tick());
                witness.poison_source_modal = Some(snapshot.modals().chat);
                true
            }),
        },
        wait: scenario::Wait {
            // agility_dungeon.rs2:105 resumes after p_delay(3); the poison
            // queue adds at most one tick (PlayerOps.ts:376-379, Player.ts:903-912).
            arm: scenario::Proof::FreshChat {
                needle: POISON_CHAT,
            },
            budget_ticks: TWISTED_CHEST_POISON_SIGNAL_TICKS,
        },
    });
    if !before_start {
        steps.push(scenario::Step {
            name: "observe the antipoison drink before returning to work",
            kind: scenario::StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: scenario::Wait {
                arm: scenario::Proof::FreshChat { needle: CURE_CHAT },
                budget_ticks: 10,
            },
        });
    }
    let stand = witness.lock().expect("survival witness").stand;
    steps.push(teleport_step(
        if before_start {
            "return to the Desert Captain before Start"
        } else {
            "return to the authored work site after cure"
        },
        stand,
    ));
    steps
}

fn account_state_poison_steps(witness: Arc<Mutex<Witness>>) -> Vec<scenario::Step> {
    let source_witness = Arc::clone(&witness);
    vec![scenario::Step {
        name: "seed account-owned poison state while the Quester bank is open",
        kind: scenario::StepKind::Perform {
            send: Box::new(move |client, snapshot| {
                let mut witness = source_witness.lock().expect("survival witness");
                let Some(login_tick) = witness.login_tick else {
                    return false;
                };
                if witness.timer_phase_ready_tick.is_none()
                    || !witness.poison_during_provisioning
                    || witness.provisioning_tick.is_none()
                    || snapshot.tick().saturating_sub(login_tick) < TIMER_CLEAR_TICKS
                    || snapshot.modals().chat != -1
                    || active_hitmark(snapshot, None) != Some(false)
                    || snapshot.bank_component_id() < 0
                    || !snapshot.bank_loaded()
                    || !witness
                        .tool_id
                        .is_some_and(|id| item_count(snapshot, id) == witness.baseline_tool)
                {
                    return false;
                }
                if !api::interact::cheat(client, POISON_DEBUG_COMMAND).is_sent() {
                    return false;
                }
                witness.poison_seed_bank_open =
                    Some(snapshot.bank_component_id() >= 0 && snapshot.bank_loaded());
                witness.poison_seed_tool_count =
                    witness.tool_id.map(|id| item_count(snapshot, id));
                witness.poison_source_tick = Some(snapshot.tick());
                witness.poison_source_modal = Some(snapshot.modals().chat);
                true
            }),
        },
        wait: scenario::Wait {
            // cheat_other.rs2:1-3 queues poison with delay 0; the signal is
            // emitted by poison.rs2:12-18 on the first eligible queue pass.
            arm: scenario::Proof::FreshChat {
                needle: POISON_CHAT,
            },
            budget_ticks: DEBUGPROC_POISON_SIGNAL_TICKS,
        },
    }]
}


fn capture_active_work(
    snapshot: &GameSnapshot,
    status: Option<&ScriptStatus>,
    witness: &mut Witness,
) {
    match witness.activity {
        Activity::Woodcutting => {
            if stat_xp(snapshot, witness.skill_stat) > witness.baseline_xp {
                witness.work_active_tick.get_or_insert(snapshot.tick());
            }
        }
        Activity::Mining => {
            if status.is_some_and(|status| status.phase == NativePhase::Working)
                && status_text(status, "step_id") == Some("survival-work")
            {
                if let Some(animation) = snapshot
                    .local_player()
                    .and_then(|player| player.animation_update)
                    .filter(|animation| {
                        animation.sequence >= 0
                            && Some(animation.serial) != witness.baseline_animation_serial
                    })
                {
                    witness.work_active_tick.get_or_insert(snapshot.tick());
                    if witness.work_animation.is_none() {
                        witness.work_animation_tick = Some(snapshot.tick());
                        witness.work_animation = Some(animation);
                    }
                }
            }
        }
        Activity::Dialogue => {}
    }
}

fn typed_lethal_need(slot: &host_play::SlotStatus) -> bool {
    slot.survival.decision == Decision::Needed
        && slot.survival.why == Some(CureWhy::Lethal)
        && slot.survival.owner == Owner::Step
        && matches!(
            slot.survival.need,
            Some(SurvivalNeed::Cure {
                why: NeedWhy::Lethal,
                ..
            })
        )
}

fn capture_need(
    snapshot: &GameSnapshot,
    status: Option<&ScriptStatus>,
    slot: &host_play::SlotStatus,
    witness: &mut Witness,
) -> bool {
    let Some(need_text) = status_text(status, "survival")
        .filter(|value| value.contains("poisoned: would die; needs antipoison"))
    else {
        return false;
    };
    if witness.poison_tick.is_none() || !typed_lethal_need(slot) {
        return false;
    }
    let report = slot.survival;
    let condition = slot.condition;
    let now = condition.tick as u16;
    let outlook =
        api::condition::poison_math::remaining(condition.poison, Exposure::Unknown, now, Q_UNKNOWN);
    let unlatched_poison = matches!(
        condition.poison,
        PoisonState::Onset {
            q: None,
            hit_since_line: false,
            ..
        }
    );
    let no_hit_at_edge = active_hitmark(snapshot, None).is_some_and(|active| !active);
    let is_combat = in_combat(snapshot);
    if witness.need_tick.is_none() {
        witness.need_tick = Some(snapshot.tick());
        witness.need_xp = Some(stat_xp(snapshot, witness.skill_stat));
        witness.need_resource = witness.resource_id.map(|id| item_count(snapshot, id));
        witness.need_status = Some(need_text.to_owned());
        witness.need_phase = status_text(status, "phase")
            .map(str::to_owned)
            .or_else(|| status.map(|status| format!("{:?}", status.phase).to_lowercase()));
        witness.need_step = status_text(status, "step_id").map(str::to_owned);
        witness.need_report_decision = Some(format!("{:?}", report.decision));
        witness.need_report_why = Some(format!("{:?}", report.why));
        witness.need_report = Some(format!("{:?}", report.need));
        witness.need_report_owner = Some(format!("{:?}", report.owner));
        witness.need_condition_tick = Some(condition.tick);
        witness.need_condition_poison = Some(format!("{:?}", condition.poison));
        witness.need_unlatched_poison = Some(unlatched_poison);
        witness.need_outlook_total = Some(outlook.total);
        witness.need_outlook_per_fire = Some(outlook.per_fire_now);
        witness.need_outlook_pending = Some(outlook.pending);
        witness.need_hp = Some(condition.hp);
        witness.need_hp_max = Some(condition.hp_max);
        witness.need_food = Some(condition.food.ordinary_count);
        witness.need_antipoison = Some(condition.cures.antipoison);
        witness.need_in_combat = is_combat;
        witness.need_no_hit_at_edge = Some(no_hit_at_edge);
    }
    if witness.activity == Activity::Dialogue
        && snapshot.modals().chat != -1
        && !snapshot.chat_options().is_empty()
        && witness.need_page_open_tick.is_none()
    {
        witness.dialogue_page_tick = Some(snapshot.tick());
        witness.need_page_open_tick = Some(snapshot.tick());
        witness.need_page_options = snapshot
            .chat_options()
            .iter()
            .map(|option| option.text.clone())
            .collect();
    }
    typed_lethal_need(slot)
}

fn observe_frame(
    snapshot: &GameSnapshot,
    status: Option<&ScriptStatus>,
    slot: Option<&host_play::SlotStatus>,
    witness: &mut Witness,
) -> bool {
    if witness.activity == Activity::Dialogue && witness.desert_stage_reply.is_none() {
        if let Some(line) = snapshot
            .chat_lines()
            .iter()
            .find(|line| line.text == DESERT_STAGE_REPLY)
        {
            witness.desert_stage_reply = Some(line.text.clone());
            witness.desert_stage_reply_tick = Some(snapshot.tick());
        }
    }
    if witness.baseline_chat.is_none() && snapshot.ingame() {
        witness.baseline_chat = Some(latest_chat(snapshot));
    }
    if witness.login_tick.is_none() && slot.is_some_and(|slot| slot.ingame && slot.scene_state == 2)
    {
        witness.login_tick = Some(snapshot.tick());
    }
    if witness.poison_during_provisioning
        && witness.provisioning_tick.is_none()
        && status_text(status, "provision") == Some("Withdrawing")
        && snapshot.bank_component_id() >= 0
        && snapshot.bank_loaded()
        && witness
            .tool_id
            .is_some_and(|id| item_count(snapshot, id) == witness.baseline_tool)
    {
        witness.provisioning_tick = Some(snapshot.tick());
    }
    capture_active_work(snapshot, status, witness);
    let baseline_chat = witness.baseline_chat.unwrap_or(0);
    if witness.poison_tick.is_none()
        && snapshot
            .chat_lines()
            .iter()
            .any(|line| line.sequence > baseline_chat && line.text == POISON_CHAT)
    {
        witness.poison_tick = Some(snapshot.tick());
    }
    if witness.poison_tick.is_some() {
        if witness.first_kind2_mark_tick.is_none()
            && snapshot.hitmarks().is_some_and(|hitmarks| {
                hitmarks
                    .marks
                    .iter()
                    .any(|mark| mark.kind == 2 && mark.cycle > hitmarks.loop_cycle)
            })
        {
            witness.first_kind2_mark_tick = Some(snapshot.tick());
        }
        if witness.cure_tick.is_none()
            && snapshot
                .chat_lines()
                .iter()
                .any(|line| line.sequence > baseline_chat && line.text == CURE_CHAT)
        {
            witness.cure_tick = Some(snapshot.tick());
        }
        if witness.dose_drop_tick.is_none()
            && item_count(snapshot, witness.antipoison_id) < witness.baseline_antipoison
        {
            witness.dose_drop_tick = Some(snapshot.tick());
        }
        if witness.drink_decision_tick.is_none()
            && slot.is_some_and(|slot| slot.survival.decision == Decision::Drinking)
        {
            witness.drink_decision_tick = Some(snapshot.tick());
            witness.drink_decision_modal = Some(snapshot.modals().chat);
        }
        if witness.immune_tick.is_none()
            && slot.is_some_and(|slot| matches!(slot.condition.poison, PoisonState::Immune { .. }))
        {
            witness.immune_tick = Some(snapshot.tick());
        }
    }
    let need_now = slot.is_some_and(|slot| capture_need(snapshot, status, slot, witness));
    if witness.activity == Activity::Dialogue
        && witness.profile_change_tick.is_some()
        && snapshot.tick() > witness.profile_change_tick.unwrap_or_default()
    {
        if snapshot.modals().chat == -1 && witness.dialogue_closed_tick.is_none() {
            witness.dialogue_closed_tick = Some(snapshot.tick());
        }
        if witness.settled_tick.is_some()
            && snapshot.tick() > witness.settled_tick.unwrap_or_default()
            && snapshot.modals().chat != -1
            && !snapshot.chat_options().is_empty()
            && status_text(status, "step_id") == Some("survival-talk")
            && witness.dialogue_reopened_tick.is_none()
        {
            witness.dialogue_reopened_tick = Some(snapshot.tick());
            witness.dialogue_reopened_options = snapshot
                .chat_options()
                .iter()
                .map(|option| option.text.clone())
                .collect();
            witness.dialogue_reopened_step = status_text(status, "step_id").map(str::to_owned);
        }
    }
    if witness.settled_tick.is_none()
        && witness.cure_tick.is_some()
        && witness.dose_drop_tick.is_some()
        && witness.immune_tick.is_some()
        && status_text(status, "survival") == Some("None")
    {
        witness.settled_tick = Some(snapshot.tick());
    }
    need_now
}

fn profile_control(initial: vault::CurePoison, witness: Arc<Mutex<Witness>>) -> LiveProfileControl {
    LiveProfileControl::new(initial, move |snapshot, status, slot| {
        let mut witness = witness.lock().expect("survival witness");
        let need_now = observe_frame(snapshot, status, slot, &mut witness);
        if initial == vault::CurePoison::Never
            && witness.activity == Activity::Dialogue
            && witness.profile_change_tick.is_none()
            && need_now
            && snapshot.modals().chat != -1
            && !snapshot.chat_options().is_empty()
            && status_text(status, "step_id") == Some("survival-talk")
        {
            witness.profile_change_tick = Some(snapshot.tick());
            witness.profile_change_modal = Some(snapshot.modals().chat);
            witness.profile_change_options = snapshot
                .chat_options()
                .iter()
                .map(|option| option.text.clone())
                .collect();
            Some(vault::CurePoison::WhenNeeded)
        } else {
            None
        }
    })
    .require_members_world()
}

fn check_mark(witness: &Witness) -> Result<(), String> {
    if let Some(mark) = witness.first_kind2_mark_tick {
        let poison = witness.poison_tick.unwrap_or_default();
        let classification = if witness.cure_tick.is_some_and(|cure| mark > cure) {
            "kind-2 mark after antipoison drink"
        } else {
            "timer-phase-uncontrolled: kind-2 mark before the cure line"
        };
        return Err(format!(
            "{classification}: first mark tick {mark}, poison line tick {poison}, cure tick {:?}",
            witness.cure_tick
        ));
    }
    Ok(())
}

fn check_deadline(witness: &Witness, tick: u32, deadline: u32) -> Result<u32, String> {
    let poison = witness
        .poison_tick
        .ok_or("real poison chat was not observed")?;
    let elapsed = tick.saturating_sub(poison);
    if elapsed > deadline {
        return Err(format!(
            "poison outcome exceeded {deadline} observed ticks: now={tick}, poison={poison}"
        ));
    }
    Ok(elapsed)
}

fn check_need(witness: &Witness) -> Result<(), String> {
    if witness.need_report_decision.as_deref() != Some("Needed")
        || witness.need_report_why.as_deref() != Some("Some(Lethal)")
        || !witness
            .need_report
            .as_deref()
            .is_some_and(|report| report.contains("why: Lethal"))
        || witness.need_report_owner.as_deref() != Some("Step")
        || witness.need_outlook_total != Some(LETHAL_OUTLOOK)
        || witness.need_outlook_per_fire != Some(Q_UNKNOWN)
        || witness.need_outlook_pending != Some(true)
        || witness.need_hp != Some(30)
        || witness.need_hp_max != Some(30)
        || witness.need_food != Some(0)
        || witness.need_antipoison != Some(1)
        || witness.need_in_combat != Some(false)
        || witness.need_no_hit_at_edge != Some(true)
        || witness.need_unlatched_poison != Some(true)
    {
        return Err(format!(
            "lethal poison need did not match the controlled edge: status={:?}, report={:?}/{:?}/{:?}/{:?}, condition={:?}, outlook={:?}/{:?}/{:?}, hp={:?}/{:?}, food={:?}, antipoison={:?}, combat={:?}, no_hit={:?}",
            witness.need_status,
            witness.need_report_decision,
            witness.need_report_why,
            witness.need_report,
            witness.need_report_owner,
            witness.need_condition_poison,
            witness.need_outlook_total,
            witness.need_outlook_per_fire,
            witness.need_outlook_pending,
            witness.need_hp,
            witness.need_hp_max,
            witness.need_food,
            witness.need_antipoison,
            witness.need_in_combat,
            witness.need_no_hit_at_edge,
        ));
    }
    Ok(())
}

fn common_receipt(
    snapshot: &GameSnapshot,
    witness: &Witness,
    deadline: u32,
) -> Result<Value, String> {
    check_mark(witness)?;
    let elapsed = check_deadline(witness, snapshot.tick(), deadline)?;
    let login_tick = witness.login_tick.ok_or("login tick was not observed")?;
    let phase_tick = witness
        .timer_phase_ready_tick
        .ok_or("controlled timer phase was not observed")?;
    let source_tick = witness
        .poison_source_tick
        .ok_or("poison source tick was not observed")?;
    let source_label = match witness.poison_source {
        PoisonSource::TwistedChest => "yanille-twisted-chest",
        PoisonSource::AccountStateDebugProc => "account-state-debugproc",
    };
    let source_signal_budget = match witness.poison_source {
        PoisonSource::TwistedChest => TWISTED_CHEST_POISON_SIGNAL_TICKS,
        PoisonSource::AccountStateDebugProc => DEBUGPROC_POISON_SIGNAL_TICKS,
    };
    let poison_tick = witness
        .poison_tick
        .ok_or("poison line tick was not observed")?;
    let need_tick = witness.need_tick.ok_or("lethal need was not observed")?;
    let drink_tick = witness
        .drink_decision_tick
        .ok_or("survival did not request the antipoison click")?;
    let cure_tick = witness
        .cure_tick
        .ok_or("antipoison drink line was not observed")?;
    let dose_drop_tick = witness
        .dose_drop_tick
        .ok_or("antipoison dose drop was not observed")?;
    let settled_tick = witness
        .settled_tick
        .ok_or("antipoison immunity did not settle")?;
    check_need(witness)?;
    let key_after = item_count(snapshot, witness.key_id);
    let antipoison_after = item_count(snapshot, witness.antipoison_id);
    let tool_after = witness.tool_id.map_or(0, |id| item_count(snapshot, id));
    let resource_after = witness
        .resource_id
        .map_or(0, |id| item_count(snapshot, id));
    let inventory_after = inventory_total(snapshot);
    let inventory_at_start = witness.baseline_antipoison
        + witness.baseline_tool
        + witness.baseline_key
        + witness.expected_inventory_extra;
    let chest_rewards_after_start =
        i32::from(witness.poison_source == PoisonSource::TwistedChest
            && witness.activity != Activity::Dialogue)
            * TWISTED_CHEST_REWARD_COUNT;
    let chest_key_consumed_after_start =
        i32::from(witness.poison_source == PoisonSource::TwistedChest
            && witness.activity != Activity::Dialogue)
            * witness.baseline_key;
    let expected_inventory_after = inventory_at_start + chest_rewards_after_start
        - chest_key_consumed_after_start
        - witness.baseline_antipoison.saturating_sub(antipoison_after)
        + tool_after.saturating_sub(witness.baseline_tool)
        + resource_after.saturating_sub(witness.baseline_resource);
    let inventory_matches = if witness.activity == Activity::Dialogue {
        inventory_after == expected_inventory_after
    } else {
        inventory_after >= expected_inventory_after
    };
    if phase_tick.saturating_sub(login_tick) < TIMER_CLEAR_TICKS
        || source_tick.saturating_sub(login_tick) < TIMER_CLEAR_TICKS
        || source_tick < phase_tick
        || poison_tick < source_tick
        || poison_tick.saturating_sub(source_tick) > source_signal_budget
        || need_tick < poison_tick
        || need_tick.saturating_sub(poison_tick) > 3
        || drink_tick < poison_tick
        || drink_tick.saturating_sub(poison_tick) > 3
        || cure_tick < poison_tick
        || cure_tick.saturating_sub(poison_tick) > 10
        || dose_drop_tick < poison_tick
        || dose_drop_tick.saturating_sub(poison_tick) > 10
        || settled_tick < cure_tick
        || snapshot.tick().saturating_sub(poison_tick) < TIMER_CLEAR_TICKS
        || witness.poison_source_modal != Some(-1)
        || key_after != 0
        || !inventory_matches
    {
        return Err(format!(
            "cure/source ticks or inventory outside the content and runner bounds: login={login_tick}, phase={phase_tick}, source={source_tick}, source_budget={source_signal_budget}, poison={poison_tick}, need={need_tick}, drink={drink_tick}, cure={cure_tick}, drop={dose_drop_tick}, settled={settled_tick}, key_after={key_after}, inventory={inventory_after}/{expected_inventory_after}, observed={}",
            snapshot.tick()
        ));
    }
    if witness.drink_decision_modal != Some(-1) {
        return Err(format!(
            "antipoison click was requested while a modal was open: {:?}",
            witness.drink_decision_modal
        ));
    }
    Ok(json!({
        "login_tick": login_tick,
        "timer_phase_ready_tick": phase_tick,
        "poison_source": source_label,
        "poison_source_tick": source_tick,
        "poison_source_chat_modal": witness.poison_source_modal,
        "poison_source_signal_budget_ticks": source_signal_budget,
        "ticks_after_login_at_source": source_tick.saturating_sub(login_tick),
        "poison_chat_tick": poison_tick,
        "lethal_need_tick": need_tick,
        "survival_report_at_need": {
            "decision": witness.need_report_decision,
            "why": witness.need_report_why,
            "need": witness.need_report,
            "owner": witness.need_report_owner,
            "status": witness.need_status,
            "phase": witness.need_phase,
            "step_id": witness.need_step,
        },
        "condition_at_need": {
            "tick": witness.need_condition_tick,
            "poison": witness.need_condition_poison,
            "unlatched_poison": witness.need_unlatched_poison,
            "outlook_total": witness.need_outlook_total,
            "outlook_per_fire": witness.need_outlook_per_fire,
            "outlook_pending": witness.need_outlook_pending,
            "hitpoints": witness.need_hp,
            "hitpoints_max": witness.need_hp_max,
            "food": witness.need_food,
            "antipoison": witness.need_antipoison,
            "in_combat": witness.need_in_combat,
            "no_hitmark_at_edge": witness.need_no_hit_at_edge,
        },
        "drink_decision_tick": drink_tick,
        "drink_decision_chat_modal": witness.drink_decision_modal,
        "antipoison_drink_chat_tick": cure_tick,
        "antipoison_drop_tick": dose_drop_tick,
        "immune_tick": witness.immune_tick,
        "antipoison_settled_tick": settled_tick,
        "sinister_key_at_start": witness.baseline_key,
        "sinister_key_after": key_after,
        "chest_reward_count_at_start": witness.expected_inventory_extra,
        "chest_rewards_after_start": chest_rewards_after_start,
        "inventory_total_at_start": inventory_at_start,
        "inventory_total_after": inventory_after,
        "inventory_total_minimum_after": expected_inventory_after,
        "ticks_from_poison_to_settled": settled_tick.saturating_sub(poison_tick),
        "first_kind2_mark_tick": witness.first_kind2_mark_tick,
        "first_kind2_mark": "none through the first scheduled fire",
        "observed_poison_ticks": elapsed,
        "first_scheduled_fire_covered": true,
        "first_scheduled_fire_covered_tick": snapshot.tick(),
    }))
}

fn first_poison_fire_observed(snapshot: &GameSnapshot, witness: &Witness) -> bool {
    witness
        .poison_tick
        .is_some_and(|poison_tick| snapshot.tick().saturating_sub(poison_tick) >= TIMER_CLEAR_TICKS)
}

fn work_receipt(
    snapshot: &GameSnapshot,
    status: Option<&ScriptStatus>,
    witness: &Witness,
    family: &str,
    deadline: u32,
) -> Result<Option<Value>, String> {
    check_mark(witness)?;
    check_deadline(witness, snapshot.tick(), deadline)?;
    let Some(source_tick) = witness.poison_source_tick else {
        return Ok(None);
    };
    let Some(work_active_tick) = witness.work_active_tick else {
        if witness.poison_during_provisioning {
            return Ok(None);
        }
        return Err("authored work did not start before poison source".to_owned());
    };
    let invalid_work_evidence = if witness.poison_during_provisioning {
        witness
            .provisioning_tick
            .is_none_or(|tick| tick > source_tick)
            || work_active_tick <= source_tick
            || witness.poison_seed_bank_open != Some(true)
            || witness.poison_seed_tool_count != Some(witness.baseline_tool)
    } else {
        work_active_tick > source_tick
            || (witness.activity == Activity::Mining
                && (witness.work_animation_tick.is_none() || witness.work_animation.is_none()))
    };
    if invalid_work_evidence {
        return Err(format!(
            "authored work evidence did not match poison-source timing: work={work_active_tick}, source={source_tick}, provision={:?}, bank_open={:?}, seed_tool_count={:?}, animation_tick={:?}, animation={:?}",
            witness.provisioning_tick,
            witness.poison_seed_bank_open,
            witness.poison_seed_tool_count,
            witness.work_animation_tick,
            witness.work_animation
        ));
    }
    if witness.poison_during_provisioning && witness.tool_id.is_none() {
        return Err("provisioning poison proof has no selected tool".to_owned());
    }
    let settled = witness.settled_tick;
    let Some(settled) = settled else {
        return Ok(None);
    };
    if snapshot.tick() <= settled
        || status.is_none_or(|status| status.phase != NativePhase::Working)
    {
        return Ok(None);
    }
    let xp = stat_xp(snapshot, witness.skill_stat);
    let resource = witness.resource_id.map_or(0, |id| item_count(snapshot, id));
    let (Some(need_xp), Some(need_resource)) = (witness.need_xp, witness.need_resource) else {
        return Ok(None);
    };
    if xp <= need_xp || (witness.resource_id.is_some() && resource <= need_resource) {
        return Ok(None);
    }
    if witness.activity == Activity::Mining
        && status_text(status, "step_id") != Some("survival-work")
    {
        return Ok(None);
    }
    if !first_poison_fire_observed(snapshot, witness) {
        return Ok(None);
    }
    let mut receipt = common_receipt(snapshot, witness, deadline)?;
    receipt["family"] = json!(family);
    receipt["stand"] = json!(witness.stand);
    receipt["skill"] = json!(witness.skill_name);
    receipt["skill_xp_at_need"] = json!(need_xp);
    receipt["skill_xp_after"] = json!(xp);
    receipt["resource_count_at_start"] = json!(witness.baseline_resource);
    receipt["resource_count_at_need"] = json!(need_resource);
    receipt["resource_count_after"] = json!(resource);
    receipt["tool_count_at_start"] = json!(witness.baseline_tool);
    receipt["tool_count_after"] = json!(witness.tool_id.map_or(0, |id| item_count(snapshot, id)));
    receipt["work_active_tick"] = json!(witness.work_active_tick);
    receipt["work_animation_tick"] = json!(witness.work_animation_tick);
    receipt["work_animation"] = json!(witness.work_animation);
    receipt["provisioning_tick"] = json!(witness.provisioning_tick);
    receipt["poison_seed_bank_open"] = json!(witness.poison_seed_bank_open);
    receipt["poison_seed_tool_count"] = json!(witness.poison_seed_tool_count);
    receipt["resumed_step_id"] = json!(status_text(status, "step_id"));
    receipt["resumed_work_tick"] = json!(snapshot.tick());
    receipt["resumed_work_ticks"] = json!(snapshot.tick().saturating_sub(settled));
    receipt["antipoison_remaining"] = json!(item_count(snapshot, witness.antipoison_id));
    Ok(Some(receipt))
}

fn matches_desert_captain_opening_options(options: &[String]) -> bool {
    options.len() == DESERT_CAPTAIN_OPENING_OPTIONS.len()
        && options
            .iter()
            .map(String::as_str)
            .eq(DESERT_CAPTAIN_OPENING_OPTIONS.iter().copied())
}

fn dialogue_receipt(
    snapshot: &GameSnapshot,
    _status: Option<&ScriptStatus>,
    witness: &Witness,
) -> Result<Option<Value>, String> {
    check_mark(witness)?;
    check_deadline(witness, snapshot.tick(), 150)?;
    let (Some(change), Some(closed), Some(reopened), Some(drink), Some(drink_modal)) = (
        witness.profile_change_tick,
        witness.dialogue_closed_tick,
        witness.dialogue_reopened_tick,
        witness.drink_decision_tick,
        witness.drink_decision_modal,
    ) else {
        return Ok(None);
    };
    let Some(cure) = witness.cure_tick else {
        return Ok(None);
    };
    let Some(drop) = witness.dose_drop_tick else {
        return Ok(None);
    };
    let Some(immune) = witness.immune_tick else {
        return Ok(None);
    };
    let Some(settled) = witness.settled_tick else {
        return Ok(None);
    };
    let Some(source_tick) = witness.poison_source_tick else {
        return Ok(None);
    };
    let Some(page_tick) = witness.dialogue_page_tick else {
        return Ok(None);
    };
    check_need(witness)?;
    if !first_poison_fire_observed(snapshot, witness) {
        return Ok(None);
    }
    if witness.desert_stage_reply.as_deref() != Some(DESERT_STAGE_REPLY)
        || witness.profile_change_modal == Some(-1)
        || !matches_desert_captain_opening_options(&witness.profile_change_options)
        || witness.need_page_open_tick != Some(change)
        || !matches_desert_captain_opening_options(&witness.need_page_options)
        || page_tick != change
        || source_tick >= page_tick
        || witness.need_step.as_deref() != Some("survival-talk")
        || closed > change.saturating_add(2)
        || drink < closed
        || drink > change.saturating_add(2)
        || drink_modal != -1
        || cure < change
        || cure.saturating_sub(change) > 10
        || drop < change
        || drop.saturating_sub(change) > 10
        || immune < cure
        || settled < immune
        || reopened <= settled
        || witness.first_kind2_mark_tick.is_some()
        || witness.dialogue_reopened_step.as_deref() != Some("survival-talk")
        || !matches_desert_captain_opening_options(&witness.dialogue_reopened_options)
    {
        return Err(format!(
            "dialogue cure/resume order failed: change={change}, stage={:?}, page_tick={page_tick}, source={source_tick}, modal={:?}, options={:?}, need_page={:?}, need_options={:?}, closed={closed}, drink={drink}, drink_modal={drink_modal}, cure={cure}, drop={drop}, immune={immune}, settled={settled}, reopened={reopened}, reopened_options={:?}, mark={:?}, step={:?}",
            witness.desert_stage_reply,
            witness.profile_change_modal,
            witness.profile_change_options,
            witness.need_page_open_tick,
            witness.need_page_options,
            witness.dialogue_reopened_options,
            witness.first_kind2_mark_tick,
            witness.dialogue_reopened_step,
        ));
    }
    let mut receipt = common_receipt(snapshot, witness, 150)?;
    receipt["family"] = json!("Quester");
    receipt["npc_config"] = json!("desertminingcaptain");
    receipt["npc_stage"] = json!(witness.desert_stage_reply);
    receipt["npc_stage_tick"] = json!(witness.desert_stage_reply_tick);
    receipt["dialogue_page_tick"] = json!(page_tick);
    receipt["dialogue_options_at_need"] = json!(witness.need_page_options);
    receipt["profile_change_tick"] = json!(change);
    receipt["profile_change_chat_modal"] = json!(witness.profile_change_modal);
    receipt["profile_change_options"] = json!(witness.profile_change_options);
    receipt["dialogue_closed_tick"] = json!(closed);
    receipt["drink_decision_tick"] = json!(drink);
    receipt["drink_decision_chat_modal"] = json!(drink_modal);
    receipt["dialogue_reopened_tick"] = json!(reopened);
    receipt["dialogue_options_after_resume"] = json!(witness.dialogue_reopened_options);
    receipt["resumed_step_id"] = json!(witness.dialogue_reopened_step);
    Ok(Some(receipt))
}

fn observe_start(snapshot: &GameSnapshot, witness: &mut Witness) -> Result<Value, String> {
    if witness.activity == Activity::Dialogue && witness.desert_stage_reply.is_none() {
        if let Some(line) = snapshot
            .chat_lines()
            .iter()
            .find(|line| line.text == DESERT_STAGE_REPLY)
        {
            witness.desert_stage_reply = Some(line.text.clone());
            witness.desert_stage_reply_tick = Some(snapshot.tick());
        }
    }
    witness
        .baseline_chat
        .get_or_insert_with(|| latest_chat(snapshot));
    witness.baseline_xp = stat_xp(snapshot, witness.skill_stat);
    witness.baseline_resource = witness.resource_id.map_or(0, |id| item_count(snapshot, id));
    witness.baseline_tool = witness.tool_id.map_or(0, |id| item_count(snapshot, id));
    witness.baseline_antipoison = item_count(snapshot, witness.antipoison_id);
    witness.baseline_key = item_count(snapshot, witness.key_id);
    witness.baseline_animation_serial = snapshot
        .local_player()
        .and_then(|player| player.animation_update)
        .map(|animation| animation.serial);
    let hp = stat_base(snapshot, HITPOINTS_STAT);
    let skill = stat_base(snapshot, witness.skill_stat);
    let inventory_expected = witness.baseline_antipoison
        + witness.baseline_tool
        + witness.baseline_key
        + witness.expected_inventory_extra;
    let inventory_observed = inventory_total(snapshot);
    let tile = snapshot.tile();
    if tile != Some((witness.stand.x, witness.stand.z, witness.stand.level))
        || hp != 30
        || skill != BASE40_SKILL_LEVEL
        || witness.baseline_antipoison != 1
        || witness.baseline_tool != witness.expected_tool_at_start
        || witness.baseline_key != witness.expected_key_at_start
        || witness.baseline_resource != 0
        || inventory_observed != inventory_expected
        || snapshot.modals().chat != -1
    {
        return Err(format!(
            "own-state fixture mismatch: tile={tile:?}, expected={:?}, HP={hp}, {}={skill}, antipoison={}, tool={}, key={}, resource={}, inventory_total={inventory_observed}, expected_total={inventory_expected}, chat_modal={}",
            witness.stand,
            witness.skill_name,
            witness.baseline_antipoison,
            witness.baseline_tool,
            witness.baseline_key,
            witness.baseline_resource,
            snapshot.modals().chat,
        ));
    }
    Ok(json!({
        "seed": match (witness.activity, witness.expected_tool_at_start) {
            (Activity::Dialogue, _) => "account-owned stats, one antipoison dose and nine herbs from the Yanille chest",
            (_, 0) => "account-owned stats, one antipoison dose and a banked work tool",
            _ => "account-owned stats, one antipoison dose, the work tool and a sinister key",
        },
        "start_tile": tile,
        "hitpoints_base": hp,
        "skill": witness.skill_name,
        "skill_base": skill,
        "skill_xp": witness.baseline_xp,
        "resource_count": witness.baseline_resource,
        "antipoison": witness.baseline_antipoison,
        "tool": witness.baseline_tool,
        "sinister_key": witness.baseline_key,
        "chest_reward_count": witness.expected_inventory_extra,
        "no_food": inventory_observed == inventory_expected,
        "inventory_total": inventory_observed,
        "chat_sequence": witness.baseline_chat,
        "login_tick": witness.login_tick,
        "poison_preseeded": witness.poison_tick.is_some(),
    }))
}

fn run_family(
    cell: Cell,
    script_name: &'static str,
    settings: Map<String, Value>,
    witness: Arc<Mutex<Witness>>,
    initial_cure_poison: vault::CurePoison,
    observe: impl FnMut(&GameSnapshot, Option<&ScriptStatus>, &Witness) -> Result<Option<Value>, String>
        + Send
        + 'static,
) -> Value {
    let mut cell = cell;
    cell.scenario.settings.deadline = Duration::from_secs(900);
    cell.start_settings = settings.clone();
    let witness_for_start = Arc::clone(&witness);
    cell.observe_start = Some(Box::new(move |snapshot| {
        let mut witness = witness_for_start
            .lock()
            .map_err(|_| "survival witness poisoned")?;
        observe_start(snapshot, &mut witness)
    }));
    let start: quester_live::StartFamily = Box::new(move |handle, account, _| {
        handle.start_compiled(account, script::CompiledId(script_name), settings.clone())
    });
    let observed_witness = Arc::clone(&witness);
    let mut observe = observe;
    let observe: quester_live::ObserveFamily = Box::new(move |snapshot, status, _| {
        let witness = observed_witness
            .lock()
            .map_err(|_| "survival witness poisoned")?;
        observe(snapshot, status, &witness)
    });
    quester_live::run_family_with_profile_control(
        cell,
        start,
        observe,
        profile_control(initial_cure_poison, witness),
    )
    .expect("native survival cure live proof")
}

fn assert_common_cure(receipt: &Value, resumed_ticks: u64) {
    assert_eq!(
        receipt["condition_at_need"]["outlook_total"],
        LETHAL_OUTLOOK
    );
    assert_eq!(receipt["condition_at_need"]["outlook_per_fire"], Q_UNKNOWN);
    assert_eq!(receipt["condition_at_need"]["outlook_pending"], true);
    assert_eq!(receipt["survival_report_at_need"]["why"], "Some(Lethal)");
    assert_eq!(receipt["survival_report_at_need"]["owner"], "Step");
    assert_eq!(receipt["condition_at_need"]["hitpoints"], 30);
    assert_eq!(receipt["condition_at_need"]["hitpoints_max"], 30);
    assert_eq!(receipt["condition_at_need"]["food"], 0);
    assert_eq!(receipt["condition_at_need"]["antipoison"], 1);
    assert_eq!(receipt["condition_at_need"]["in_combat"], false);
    assert_eq!(receipt["condition_at_need"]["no_hitmark_at_edge"], true);
    assert_eq!(receipt["drink_decision_chat_modal"], -1);
    assert_eq!(receipt["first_kind2_mark_tick"], Value::Null);
    assert!(receipt["first_scheduled_fire_covered"].as_bool().unwrap());
    assert!(receipt["observed_poison_ticks"].as_u64().unwrap() >= u64::from(TIMER_CLEAR_TICKS));
    assert!(receipt["ticks_from_poison_to_settled"].as_u64().unwrap() <= 10);
    assert!(
        receipt["resumed_work_ticks"]
            .as_u64()
            .unwrap_or(resumed_ticks)
            <= resumed_ticks
    );
    assert_eq!(receipt["antipoison_remaining"], 0);
}

#[test]
#[ignore = "requires LIVE=1, Engine L debug procedures enabled (debug: true), and isolated HOME"]
fn live_l_cure_when_needed_drains_willow_and_resumes_gatherer() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let mut cell = fixture(
        Activity::Woodcutting,
        "L-CURE-WHEN-NEEDED",
        GATHERER_STAND,
        &[
            (RUNE_AXE_ALIAS, 1),
            (ANTIPOISON_ALIAS, 1),
            (SINISTER_KEY_ALIAS, 1),
        ],
        &[("cookquest", 0)],
        vec![],
        Mode::Stage {
            expect: vec!["cook:0".into()],
        },
    );
    let witness = new_witness(
        Activity::Woodcutting,
        "Woodcutting",
        WOODCUTTING_STAT,
        Some(WILLOW_LOG_ID),
        Some(RUNE_AXE_ALIAS),
        PoisonSource::TwistedChest,
        GATHERER_STAND,
    );
    add_poison_steps(&mut cell.scenario, Arc::clone(&witness), false, 120);
    let settings: Map<String, Value> = serde_json::from_value(json!({
        "skill":"Woodcutting",
        "woodcuttingResources":["willow"],
        "miningResources":[],
        "targetPreference":"Nearest",
        "location":"Start",
        "radius":32,
        "disposition":"Power"
    }))
    .expect("Gatherer settings");
    let receipt = run_family(
        cell,
        "Gatherer",
        settings,
        Arc::clone(&witness),
        vault::CurePoison::WhenNeeded,
        move |snapshot, status, witness| work_receipt(snapshot, status, witness, "Gatherer", 120),
    );
    let proof = &receipt["family_receipt"];
    assert_eq!(proof["family"], "Gatherer");
    assert_eq!(proof["poison_source"], "yanille-twisted-chest");
    assert_eq!(proof["sinister_key_at_start"], 1);
    assert_eq!(proof["sinister_key_after"], 0);
    assert_eq!(proof["chest_rewards_after_start"], TWISTED_CHEST_REWARD_COUNT);
    assert_eq!(proof["stand"]["x"], GATHERER_STAND.x);
    assert_eq!(proof["stand"]["z"], GATHERER_STAND.z);
    assert!(
        proof["skill_xp_after"].as_i64().unwrap() > proof["skill_xp_at_need"].as_i64().unwrap()
    );
    assert_common_cure(proof, 120);
}

static PATH_SOURCE_LOCK: Mutex<()> = Mutex::new(());

#[test]
#[ignore = "requires LIVE=1, Engine L debug procedures enabled (debug: true), and isolated HOME"]
fn live_l_cure_when_needed_quester_mines_copper_and_resumes_gather() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let _path_source_guard = PATH_SOURCE_LOCK.lock().expect("Quester path source lock");
    let evidence = PathBuf::from(std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR"));
    let _restore = install_path_source(
        &evidence,
        "copper-mining",
        json!([{
            "id":"survival-work",
            "kind":"gather",
            "version":1,
            "advances":false,
            "args":{
                "skill":"mining",
                "resource":"copper",
                "until":{"obj":{"id":436},"qty":100}
            },
            "skip_if":{"Any":[]},
            "settle":{"All":[]}
        }]),
    );
    let mut cell = fixture(
        Activity::Mining,
        "L-CURE-WHEN-NEEDED-QUESTER",
        MINING_STAND,
        &[
            (RUNE_PICKAXE_ALIAS, 1),
            (ANTIPOISON_ALIAS, 1),
            (SINISTER_KEY_ALIAS, 1),
        ],
        &[("cookquest", 0)],
        vec![],
        Mode::Stage {
            expect: vec!["cook:0".into()],
        },
    );
    let witness = new_witness(
        Activity::Mining,
        "Mining",
        MINING_STAT,
        Some(COPPER_ORE_ID),
        Some(RUNE_PICKAXE_ALIAS),
        PoisonSource::TwistedChest,
        MINING_STAND,
    );
    add_poison_steps(&mut cell.scenario, Arc::clone(&witness), false, 120);
    let settings: Map<String, Value> =
        serde_json::from_value(json!({"quests":["cook"]})).expect("Quester settings");
    let receipt = run_family(
        cell,
        "Quester",
        settings,
        Arc::clone(&witness),
        vault::CurePoison::WhenNeeded,
        move |snapshot, status, witness| work_receipt(snapshot, status, witness, "Quester", 120),
    );
    let proof = &receipt["family_receipt"];
    assert_eq!(proof["family"], "Quester");
    assert_eq!(proof["poison_source"], "yanille-twisted-chest");
    assert_eq!(proof["sinister_key_at_start"], 1);
    assert_eq!(proof["sinister_key_after"], 0);
    assert_eq!(proof["chest_rewards_after_start"], TWISTED_CHEST_REWARD_COUNT);
    assert_eq!(proof["resumed_step_id"], "survival-work");
    assert_eq!(proof["stand"]["x"], MINING_STAND.x);
    assert_eq!(proof["stand"]["z"], MINING_STAND.z);
    assert!(
        proof["skill_xp_after"].as_i64().unwrap() > proof["skill_xp_at_need"].as_i64().unwrap()
    );
    assert!(
        proof["resource_count_after"].as_i64().unwrap()
            > proof["resource_count_at_start"].as_i64().unwrap()
    );
    assert!(proof["work_animation_tick"].as_u64().is_some());
    assert!(proof["work_animation"].is_object());
    assert_common_cure(proof, 120);
}

#[test]
#[ignore = "requires LIVE=1, Engine L debug procedures enabled (debug: true), and isolated HOME"]
fn live_l_cure_quester_interrupts_bank_provisioning_and_resumes_mining() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let _path_source_guard = PATH_SOURCE_LOCK.lock().expect("Quester path source lock");
    let evidence = PathBuf::from(std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR"));
    let _restore = install_path_source(
        &evidence,
        "copper-mining-provisioning",
        json!([{
            "id":"survival-work",
            "kind":"gather",
            "version":1,
            "advances":false,
            "args":{
                "skill":"mining",
                "resource":"copper",
                "until":{"obj":{"id":436},"qty":100}
            },
            "skip_if":{"Any":[]},
            "settle":{"All":[]}
        }]),
    );
    let mut cell = fixture(
        Activity::Mining,
        "L-CURE-QUESTER-PROVISIONING",
        MINING_STAND,
        &[(ANTIPOISON_ALIAS, 1)],
        &[("cookquest", 0)],
        vec![bank_seed_step(RUNE_PICKAXE_ALIAS)],
        Mode::Stage {
            expect: vec!["cook:0".into()],
        },
    );
    let witness = new_witness(
        Activity::Mining,
        "Mining",
        MINING_STAT,
        Some(COPPER_ORE_ID),
        Some(RUNE_PICKAXE_ALIAS),
        PoisonSource::AccountStateDebugProc,
        MINING_STAND,
    );
    {
        let mut witness = witness.lock().expect("survival witness");
        witness.expected_tool_at_start = 0;
        witness.poison_during_provisioning = true;
    }
    add_login_timer_wait_before_start(&mut cell.scenario, Arc::clone(&witness));
    add_poison_steps(&mut cell.scenario, Arc::clone(&witness), false, 120);
    let settings: Map<String, Value> =
        serde_json::from_value(json!({"quests":["cook"]})).expect("Quester settings");
    let receipt = run_family(
        cell,
        "Quester",
        settings,
        Arc::clone(&witness),
        vault::CurePoison::WhenNeeded,
        move |snapshot, status, witness| work_receipt(snapshot, status, witness, "Quester", 120),
    );
    let proof = &receipt["family_receipt"];
    assert_eq!(proof["family"], "Quester");
    assert_eq!(proof["poison_source"], "account-state-debugproc");
    assert_eq!(proof["poison_source_chat_modal"], -1);
    assert_eq!(proof["resumed_step_id"], "survival-work");
    assert_eq!(proof["tool_count_at_start"], 0);
    assert_eq!(proof["poison_seed_tool_count"], 0);
    assert_eq!(proof["poison_seed_bank_open"], true);
    assert_eq!(proof["tool_count_after"], 1);
    assert!(
        proof["provisioning_tick"].as_u64().unwrap()
            <= proof["poison_source_tick"].as_u64().unwrap()
    );
    assert!(
        proof["work_active_tick"].as_u64().unwrap()
            > proof["poison_source_tick"].as_u64().unwrap()
    );
    assert_common_cure(proof, 120);
}

#[test]
#[ignore = "requires LIVE=1, Engine L debug procedures enabled (debug: true), and isolated HOME"]
fn live_l_cure_quester_dialogue_updates_policy_on_desert_captain_page() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let _path_source_guard = PATH_SOURCE_LOCK.lock().expect("Quester path source lock");
    let evidence = PathBuf::from(std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR"));
    let _restore = install_path_source(
        &evidence,
        "desert-captain-dialogue",
        json!([{
            "id":"survival-talk",
            "kind":"talk",
            "version":1,
            "advances":false,
            "args":{
                "npc":"desertminingcaptain",
                "prefer":[
                    "Wow! A real captain!",
                    "I'd love to work for a tough guy like you!",
                    "Can't I do something for a strong Captain like you?",
                    "Sorry Sir, I don't think I can do that.",
                    "It's a funny captain who can't fight his own battles!"
                ]
            },
            "skip_if":{"Any":[]},
            "settle":{"All":[]}
        }]),
    );
    let mut cell = fixture(
        &[(ANTIPOISON_ALIAS, 1), (SINISTER_KEY_ALIAS, 1)],
        &[("cookquest", 0), ("desertrescue", 3)],
        vec![],
        Mode::Stage {
            expect: vec!["cook:0".into()],
        },
    );
    let witness = new_witness(
        Activity::Dialogue,
        "Mining",
        MINING_STAT,
        None,
        None,
        PoisonSource::TwistedChest,
        DESERT_CAPTAIN_STAND,
    );
    add_poison_steps(&mut cell.scenario, Arc::clone(&witness), true, 40);
    let settings: Map<String, Value> =
        serde_json::from_value(json!({"quests":["cook"]})).expect("Quester settings");
    let receipt = run_family(
        cell,
        "Quester",
        settings,
        Arc::clone(&witness),
        vault::CurePoison::Never,
        dialogue_receipt,
    );
    let proof = &receipt["family_receipt"];
    assert_eq!(proof["family"], "Quester");
    assert_eq!(proof["poison_source"], "yanille-twisted-chest");
    assert_eq!(proof["sinister_key_at_start"], 0);
    assert_eq!(proof["sinister_key_after"], 0);
    assert_eq!(proof["chest_reward_count_at_start"], TWISTED_CHEST_REWARD_COUNT);
    assert_eq!(proof["inventory_total_at_start"], 1 + TWISTED_CHEST_REWARD_COUNT);
    assert_eq!(proof["inventory_total_after"], TWISTED_CHEST_REWARD_COUNT);
    assert_eq!(proof["npc_config"], "desertminingcaptain");
    assert_eq!(proof["npc_stage"], DESERT_STAGE_REPLY);
    assert!(proof["npc_stage_tick"].as_u64().is_some());
    assert_eq!(
        proof["dialogue_options_at_need"],
        json!(DESERT_CAPTAIN_OPENING_OPTIONS)
    );
    assert_eq!(
        proof["profile_change_options"],
        json!(DESERT_CAPTAIN_OPENING_OPTIONS)
    );
    assert_eq!(
        proof["dialogue_options_after_resume"],
        json!(DESERT_CAPTAIN_OPENING_OPTIONS)
    );
    assert_eq!(proof["resumed_step_id"], "survival-talk");
    assert!(proof["profile_change_chat_modal"]
        .as_i64()
        .is_some_and(|modal| modal != -1));
    assert!(!proof["profile_change_options"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(
        proof["dialogue_closed_tick"].as_u64().unwrap()
            > proof["profile_change_tick"].as_u64().unwrap()
    );
    assert!(
        proof["dialogue_reopened_tick"].as_u64().unwrap()
            > proof["antipoison_settled_tick"].as_u64().unwrap()
    );
    assert_common_cure(proof, 150);
    let transitions = receipt["transitions"]
        .as_array()
        .expect("transition receipt");
    let change = transitions
        .iter()
        .find(|transition| {
            transition["event"] == "profile-command"
                && transition["command"] == "SetSurvival"
                && transition["setting"] == "cure_poison"
        })
        .expect("profile command transition");
    assert_eq!(change["before"], "never");
    assert_eq!(change["after"], "when_needed");
    assert_eq!(change["slot_mirror_policy"], "when_needed");
    assert_eq!(change["slot_mirror_tick"], change["tick"]);
    assert_eq!(change["slot_mirror_updated"], true);
    assert_eq!(change["tick"], proof["profile_change_tick"]);
    assert_eq!(change["chat_options"], proof["profile_change_options"]);
    assert!(change["chat_modal"]
        .as_i64()
        .is_some_and(|modal| modal != -1));
    assert!(!change["chat_options"].as_array().unwrap().is_empty());
}

struct SourceRestore(FolderSource);

impl Drop for SourceRestore {
    fn drop(&mut self) {
        registry::set_source(std::mem::replace(
            &mut self.0,
            FolderSource {
                enabled: false,
                folder: PathBuf::new(),
            },
        ));
    }
}
