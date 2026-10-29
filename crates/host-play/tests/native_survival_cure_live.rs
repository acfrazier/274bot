//! Live native-runner poison interruption and resume proofs using real content.
#![cfg(all(feature = "live-harness", feature = "test-support"))]

#[path = "common/quester_live.rs"]
mod quester_live;

use api::snapshot::{GameSnapshot, WorldTile};
use quester_live::{Cell, Mode, PathCell};
use script::native::{NativePhase, ScriptStatus, StatusValue};
use script::quester::registry::{self, FolderSource};
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const START: WorldTile = WorldTile {
    x: 2351,
    z: 4564,
    level: 0,
};
const YEW: WorldTile = WorldTile {
    x: 2345,
    z: 4551,
    level: 0,
};
// The real dungeon entrance uses 0_40_148_55_34 at
// content/scripts/areas/area_yanille/scripts/agility_dungeon.rs2:90. The
// chest is selected from the loaded scene rather than guessed by world tile.
const CHEST_REGION: WorldTile = WorldTile {
    x: 40 * 64 + 32,
    z: 148 * 64 + 32,
    level: 0,
};
const CHEST_LOC_ID: i32 = 377;
const WOODCUTTING_STAT: i32 = 8;
const HITPOINTS_STAT: i32 = 3;
const ANTIPOISON_ALIAS: &str = "1doseantipoison";
const RUNE_AXE_ALIAS: &str = "rune_axe";
const SINISTER_KEY_ALIAS: &str = "sinister_key";

#[derive(Default)]
struct Witness {
    baseline_chat: i32,
    baseline_wc_xp: i32,
    baseline_yew_logs: i32,
    yew_logs_id: i32,
    antipoison_id: i32,
    sinister_key_id: i32,
    cure_tick: Option<u32>,
    dose_drop_tick: Option<u32>,
    poison_tick: Option<u32>,
    need_tick: Option<u32>,
    need_text: Option<String>,
    need_phase: Option<String>,
    need_step: Option<String>,
    need_wc_xp: Option<i32>,
    need_yew_logs: Option<i32>,
    settled_tick: Option<u32>,
    resumed_step: Option<String>,
}

fn new_witness() -> Arc<Mutex<Witness>> {
    Arc::new(Mutex::new(Witness::default()))
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

fn selected_item_ids() -> (i32, i32, i32) {
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289)
        .expect("selected R289 data");
    let item_id = |alias: &str| {
        selected
            .item_by_alias(alias)
            .unwrap_or_else(|| panic!("missing selected item alias {alias}"))
            .id
    };
    (
        item_id(ANTIPOISON_ALIAS),
        item_id("yew_logs"),
        item_id(SINISTER_KEY_ALIAS),
    )
}

fn item_count(snapshot: &GameSnapshot, id: i32) -> i32 {
    snapshot
        .inv()
        .iter()
        .filter(|(item, _)| *item == id)
        .map(|(_, count)| *count)
        .sum()
}

fn fixture(label: &str, mode: Mode) -> Cell {
    let stat_seed = scenario::Step {
        name: "seed Woodcutting 99 and Hitpoints 30 before Start",
        kind: scenario::StepKind::Perform {
            send: Box::new(|client, _| {
                api::interact::cheat(client, "setstat woodcutting 99").is_sent()
                    && api::interact::cheat(client, "setstat hitpoints 30").is_sent()
            }),
        },
        wait: scenario::Wait {
            arm: scenario::Proof::Stat {
                id: WOODCUTTING_STAT,
                min: 99,
            },
            budget_ticks: 40,
        },
    };
    quester_live::path_cell(PathCell {
        quest: "cook",
        display: "Cook's Assistant",
        label: label.to_owned(),
        stage: Some("cook:0"),
        loadout: None,
        extra_items: &[
            (RUNE_AXE_ALIAS, 1),
            (ANTIPOISON_ALIAS, 1),
            (SINISTER_KEY_ALIAS, 1),
        ],
        seed_vars: &[("cookquest", 0)],
        stand: START,
        mode,
        before_relog: vec![stat_seed],
    })
    .expect("build account-owned live fixture")
}

fn trigger_twisted_chest(
    client: &mut dyn api::interact::Driver,
    snapshot: &GameSnapshot,
) -> bool {
    let Some(loc) = snapshot.locs().iter().find(|loc| loc.id == CHEST_LOC_ID) else {
        return false;
    };
    let (_, _, key_id) = selected_item_ids();
    let Some(key) = snapshot
        .inventory()
        .iter()
        .find(|item| item.def.id == key_id && item.count > 0)
    else {
        return false;
    };
    matches!(
        api::interact::Interactions::new(snapshot, client)
            .use_item_on(key, api::interact::OpTarget::Loc(loc)),
        api::interact::SendResult::Sent { .. }
    )
}

fn add_poison_chest_steps(scenario: &mut scenario::Scenario) {
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, scenario::StepKind::StartScript))
        .expect("fixture starts its native script");
    let chest_teleport = api::interact::tele_args(CHEST_REGION.level, CHEST_REGION.x, CHEST_REGION.z);
    let yew_teleport = api::interact::tele_args(YEW.level, YEW.x, YEW.z);
    // Yew chopping attempts take three ticks and its respawn is 200 ticks
    // (woodcut.rs2:59-60; trees.dbrow:130-143). The 360-tick windows cover
    // one respawn plus 53 attempts. The chest queues severity-27 poison after
    // p_delay(3) (agility_dungeon.rs2:101-112); poison.rs2:12-18 emits the
    // exact chat signal used below.
    scenario.steps.splice(
        start + 1..start + 1,
        [
            scenario::Step {
                name: "observe active Yew work before the real poison source",
                kind: scenario::StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: scenario::Wait {
                    arm: scenario::Proof::FreshStatXpGain {
                        id: WOODCUTTING_STAT,
                        min: 1,
                    },
                    budget_ticks: 360,
                },
            },
            scenario::Step {
                name: "teleport to the Yanille dungeon region",
                kind: scenario::StepKind::Perform {
                    send: Box::new(move |client, _| {
                        api::interact::cheat(client, &chest_teleport).is_sent()
                    }),
                },
                wait: scenario::Wait {
                    arm: scenario::Proof::Arrived {
                        x: CHEST_REGION.x,
                        z: CHEST_REGION.z,
                        level: CHEST_REGION.level,
                    },
                    budget_ticks: 10,
                },
            },
            scenario::Step {
                name: "observe the real Yanille twisted chest",
                kind: scenario::StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: scenario::Wait {
                    arm: scenario::Proof::LocIdNear {
                        id: CHEST_LOC_ID,
                        x: CHEST_REGION.x,
                        z: CHEST_REGION.z,
                        level: CHEST_REGION.level,
                        radius: 32,
                    },
                    budget_ticks: 10,
                },
            },
            scenario::Step {
                name: "use the account-owned sinister key on the real chest",
                kind: scenario::StepKind::Perform {
                    send: Box::new(|client, snapshot| trigger_twisted_chest(client, snapshot)),
                },
                wait: scenario::Wait {
                    arm: scenario::Proof::FreshChat {
                        needle: "You have been poisoned!",
                    },
                    budget_ticks: 10,
                },
            },
            scenario::Step {
                name: "return to the Yew work site after the poison event",
                kind: scenario::StepKind::Perform {
                    send: Box::new(move |client, _| {
                        api::interact::cheat(client, &yew_teleport).is_sent()
                    }),
                },
                wait: scenario::Wait {
                    arm: scenario::Proof::Arrived {
                        x: YEW.x,
                        z: YEW.z,
                        level: YEW.level,
                    },
                    budget_ticks: 10,
                },
            },
            scenario::Step {
                name: "observe work resuming after the cure",
                kind: scenario::StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: scenario::Wait {
                    arm: scenario::Proof::FreshStatXpGain {
                        id: WOODCUTTING_STAT,
                        min: 1,
                    },
                    budget_ticks: 360,
                },
            },
        ],
    );
}

fn observe_start(snapshot: &GameSnapshot, witness: &mut Witness) -> Result<Value, String> {
    witness.baseline_chat = latest_chat(snapshot);
    witness.baseline_wc_xp = stat_xp(snapshot, WOODCUTTING_STAT);
    let (antipoison_id, yew_logs_id, sinister_key_id) = selected_item_ids();
    witness.antipoison_id = antipoison_id;
    witness.yew_logs_id = yew_logs_id;
    witness.sinister_key_id = sinister_key_id;
    witness.baseline_yew_logs = item_count(snapshot, yew_logs_id);
    let hp = stat_base(snapshot, HITPOINTS_STAT);
    let woodcutting = stat_base(snapshot, WOODCUTTING_STAT);
    let antipoison = item_count(snapshot, antipoison_id);
    let sinister_key = item_count(snapshot, sinister_key_id);
    if antipoison != 1 || sinister_key != 1 || hp != 30 || woodcutting != 99 {
        return Err(format!(
            "own-state fixture mismatch: antipoison={antipoison}, key={sinister_key}, HP={hp}, Woodcutting={woodcutting}"
        ));
    }
    Ok(json!({
        "seed":"account-owned stats, antipoison, rune axe and sinister key",
        "start_tile":snapshot.tile(),
        "hitpoints_base":hp,
        "woodcutting_base":woodcutting,
        "woodcutting_xp":witness.baseline_wc_xp,
        "yew_logs":witness.baseline_yew_logs,
        "antipoison":antipoison,
        "sinister_key":sinister_key,
        "chat_sequence":witness.baseline_chat,
    }))
}

fn observe_poison_and_cure(
    snapshot: &GameSnapshot,
    status: Option<&ScriptStatus>,
    witness: &mut Witness,
) {
    let tick = snapshot.tick();
    if witness.poison_tick.is_none()
        && snapshot.chat_lines().iter().any(|line| {
            line.sequence > witness.baseline_chat && line.text == "You have been poisoned!"
        })
    {
        witness.poison_tick = Some(tick);
    }
    if witness.cure_tick.is_none()
        && snapshot.chat_lines().iter().any(|line| {
            line.sequence > witness.baseline_chat
                && line.text == "You drink some of your antipoison potion."
        })
    {
        witness.cure_tick = Some(tick);
    }
    if witness.dose_drop_tick.is_none()
        && item_count(snapshot, witness.antipoison_id) == 0
    {
        witness.dose_drop_tick = Some(tick);
    }
    if witness.need_tick.is_none() {
        if let Some(need) = status_text(status, "survival")
            .filter(|value| value.contains("poisoned: would die; needs antipoison"))
        {
            witness.need_tick = Some(tick);
            witness.need_wc_xp = Some(stat_xp(snapshot, WOODCUTTING_STAT));
            witness.need_yew_logs = Some(item_count(snapshot, witness.yew_logs_id));
            witness.need_text = Some(need.to_owned());
            witness.need_phase = status_text(status, "phase")
                .map(str::to_owned)
                .or_else(|| status.map(|status| format!("{:?}", status.phase).to_lowercase()));
            witness.need_step = status_text(status, "step_id").map(str::to_owned);
        }
    }
    if witness.settled_tick.is_none()
        && witness.cure_tick.is_some()
        && witness.dose_drop_tick.is_some()
        && status_text(status, "survival") == Some("None")
    {
        witness.settled_tick = Some(tick);
    }
}

fn check_window(witness: &Witness, snapshot: &GameSnapshot) -> Result<(u32, u32, u32, u32), String> {
    let poison = witness.poison_tick.ok_or("real poison chat was not observed")?;
    let need = witness.need_tick.ok_or("lethal poison need was not observed")?;
    let cure = witness.cure_tick.ok_or("antipoison drink line was not observed")?;
    let dose_drop = witness
        .dose_drop_tick
        .ok_or("antipoison dose drop was not observed")?;
    let settled = witness.settled_tick.ok_or("antipoison did not settle")?;
    let resumed = snapshot.tick().saturating_sub(settled);
    if need < poison
        || need.saturating_sub(poison) > 3
        || cure < need
        || cure.saturating_sub(need) > 3
        || dose_drop < need
        || dose_drop.saturating_sub(need) > 3
        || settled <= need
        || resumed > 360
    {
        return Err(format!(
            "poison/cure/work ticks outside source and runner bounds: poison={poison}, need={need}, cure={cure}, dose_drop={dose_drop}, settled={settled}, resumed={resumed}"
        ));
    }
    Ok((poison, need, cure, dose_drop))
}

fn gathered_receipt(
    snapshot: &GameSnapshot,
    status: Option<&ScriptStatus>,
    witness: &mut Witness,
) -> Result<Option<Value>, String> {
    observe_poison_and_cure(snapshot, status, witness);
    let xp = stat_xp(snapshot, WOODCUTTING_STAT);
    let need_xp = witness.need_wc_xp;
    if witness.settled_tick.is_some_and(|settled| snapshot.tick() > settled)
        && need_xp.is_some_and(|before| xp > before)
        && status.is_some_and(|status| status.phase == NativePhase::Working)
    {
        let need_phase = witness.need_phase.as_deref().ok_or("Gatherer phase absent at need")?;
        if !matches!(need_phase, "walking" | "tending") {
            return Err(format!("Gatherer need was not observed during work: {need_phase}"));
        }
        let (poison, need, cure, dose_drop) = check_window(witness, snapshot)?;
        return Ok(Some(json!({
            "family":"Gatherer",
            "source":"Yanille Shattered Twisted Chest",
            "poison_chat_tick":poison,
            "lethal_need_tick":need,
            "antipoison_drink_chat_tick":cure,
            "antipoison_drop_tick":dose_drop,
            "need":witness.need_text,
            "need_phase":need_phase,
            "antipoison_settled_tick":witness.settled_tick,
            "work_resumed_tick":snapshot.tick(),
            "resumed_work_ticks":snapshot.tick().saturating_sub(witness.settled_tick.unwrap()),
            "woodcutting_xp_at_need":need_xp,
            "woodcutting_xp_after":xp,
            "phase":format!("{:?}", status.expect("status checked").phase),
            "antipoison_remaining":item_count(snapshot, witness.antipoison_id),
        })));
    }
    Ok(None)
}

fn quester_receipt(
    snapshot: &GameSnapshot,
    status: Option<&ScriptStatus>,
    witness: &mut Witness,
) -> Result<Option<Value>, String> {
    observe_poison_and_cure(snapshot, status, witness);
    let xp = stat_xp(snapshot, WOODCUTTING_STAT);
    let logs = item_count(snapshot, witness.yew_logs_id);
    let need_xp = witness.need_wc_xp;
    let need_logs = witness.need_yew_logs;
    if witness.settled_tick.is_some_and(|settled| snapshot.tick() > settled)
        && status_text(status, "step_id") == Some("survival-work")
        && need_xp.is_some_and(|before| xp > before)
        && need_logs.is_some_and(|before| logs > before)
        && status.is_some_and(|status| status.phase == NativePhase::Working)
    {
        witness.resumed_step = status_text(status, "step_id").map(str::to_owned);
        let need_phase = witness.need_phase.as_deref().ok_or("Quester phase absent at need")?;
        if need_phase != "working" || witness.need_step.as_deref() != Some("survival-work") {
            return Err(format!(
                "Quester need was not observed in the gather step: phase={need_phase}, step={:?}",
                witness.need_step
            ));
        }
        let (poison, need, cure, dose_drop) = check_window(witness, snapshot)?;
        return Ok(Some(json!({
            "family":"Quester",
            "source":"Yanille Shattered Twisted Chest",
            "poison_chat_tick":poison,
            "lethal_need_tick":need,
            "antipoison_drink_chat_tick":cure,
            "antipoison_drop_tick":dose_drop,
            "need":witness.need_text,
            "need_phase":need_phase,
            "antipoison_settled_tick":witness.settled_tick,
            "work_resumed_tick":snapshot.tick(),
            "resumed_work_ticks":snapshot.tick().saturating_sub(witness.settled_tick.unwrap()),
            "resumed_step":witness.resumed_step,
            "woodcutting_xp_at_need":need_xp,
            "woodcutting_xp_after":xp,
            "yew_logs_at_need":need_logs,
            "yew_logs_after":logs,
            "antipoison_remaining":item_count(snapshot, witness.antipoison_id),
        })));
    }
    Ok(None)
}

#[test]
#[ignore = "requires LIVE=1, Engine L and isolated HOME"]
fn live_gatherer_cures_yanille_chest_poison_and_resumes_yew_work() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let witness = new_witness();
    let mut cell = fixture("native-cure-gatherer", Mode::Stage { expect: vec!["cook:0".into()] });
    cell.scenario.settings.deadline = Duration::from_secs(900);
    add_poison_chest_steps(&mut cell.scenario);
    let settings: Map<String, Value> = serde_json::from_value(json!({
        "skill":"Woodcutting",
        "woodcuttingResources":["yew"],
        "miningResources":[],
        "targetPreference":"Nearest",
        "location":"Start",
        "radius":32,
        "disposition":"Power"
    }))
    .expect("Gatherer settings");
    cell.start_settings = settings.clone();
    let witness_for_start = Arc::clone(&witness);
    cell.observe_start = Some(Box::new(move |snapshot| {
        let mut witness = witness_for_start
            .lock()
            .map_err(|_| "survival witness poisoned")?;
        observe_start(snapshot, &mut witness)
    }));
    let start_settings = settings;
    let start = Box::new(move |handle: &host_play::ScriptStartHandle, account: &str, _| {
        handle.start_compiled(account, script::CompiledId("Gatherer"), start_settings.clone())
    });
    let observe = Box::new(move |snapshot: &GameSnapshot, status: Option<&ScriptStatus>, _| {
        let mut witness = witness.lock().map_err(|_| "survival witness poisoned")?;
        gathered_receipt(snapshot, status, &mut witness)
    });
    let receipt = quester_live::run_family(cell, start, observe).expect("Gatherer cure live proof");
    assert!(receipt["woodcutting_xp_after"].as_i64().unwrap() > receipt["woodcutting_xp_at_need"].as_i64().unwrap());
    assert!(receipt["resumed_work_ticks"].as_u64().unwrap() <= 360);
    assert_eq!(receipt["antipoison_remaining"], 0);
}

#[test]
#[ignore = "requires LIVE=1, Engine L and isolated HOME"]
fn live_quester_cures_yanille_chest_poison_and_resumes_mid_step() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let evidence = PathBuf::from(std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR"));
    let folder = evidence.join("native-cure-path").join("289");
    std::fs::create_dir_all(&folder).expect("create isolated path source");
    let mut document: Value =
        serde_json::from_slice(script::quester::compile::cook_bytes()).expect("embedded Cook path");
    document["roles"][0]["sequences"][0]["steps"] = json!([
        {
            "id":"survival-work",
            "kind":"gather",
            "version":1,
            "advances":false,
            "args":{
                "skill":"woodcutting",
                "resource":"yew",
                "until":{"obj":{"id":1511},"qty":100}
            },
            "skip_if":{"Any":[]},
            "settle":{"All":[]}
        }
    ]);
    std::fs::write(
        folder.join("cook.json"),
        serde_json::to_vec_pretty(&document).expect("serialize live Path"),
    )
    .expect("write isolated Cook Path");
    let previous = registry::source();
    registry::set_source(FolderSource {
        enabled: true,
        folder: folder.clone(),
    });
    let restore = SourceRestore(previous);

    let witness = new_witness();
    let mut cell = fixture(
        "native-cure-quester",
        Mode::Stage {
            expect: vec!["cook:0".into()],
        },
    );
    cell.scenario.settings.deadline = Duration::from_secs(900);
    add_poison_chest_steps(&mut cell.scenario);
    let settings: Map<String, Value> = serde_json::from_value(json!({"quests":["cook"]}))
        .expect("Quester settings");
    cell.start_settings = settings.clone();
    let witness_for_start = Arc::clone(&witness);
    cell.observe_start = Some(Box::new(move |snapshot| {
        let mut witness = witness_for_start
            .lock()
            .map_err(|_| "survival witness poisoned")?;
        observe_start(snapshot, &mut witness)
    }));
    let start_settings = settings;
    let start = Box::new(move |handle: &host_play::ScriptStartHandle, account: &str, _| {
        handle.start_compiled(account, script::CompiledId("Quester"), start_settings.clone())
    });
    let observe = Box::new(move |snapshot: &GameSnapshot, status: Option<&ScriptStatus>, _| {
        let mut witness = witness.lock().map_err(|_| "survival witness poisoned")?;
        quester_receipt(snapshot, status, &mut witness)
    });
    let receipt = quester_live::run_family(cell, start, observe).expect("Quester cure live proof");
    drop(restore);
    assert_eq!(receipt["resumed_step"], "survival-work");
    assert!(receipt["woodcutting_xp_after"].as_i64().unwrap() > receipt["woodcutting_xp_at_need"].as_i64().unwrap());
    assert!(receipt["yew_logs_after"].as_i64().unwrap() > receipt["yew_logs_at_need"].as_i64().unwrap());
    assert!(receipt["resumed_work_ticks"].as_u64().unwrap() <= 360);
    assert_eq!(receipt["antipoison_remaining"], 0);
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
