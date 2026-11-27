#![cfg(all(feature = "test-support", feature = "luau"))]
//! Live Engine A witnesses for Luau slice L4c: combat session, Gatherer session,
//! and the three quest reads (`questStatus`, `questPaths`, `questProgress`).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use api::interact;
use api::snapshot::{ChatLineView, GameSnapshot, QuestListStatus, WorldTile};
use client::client::Client;
use host_play::{
    mint_live_entries, mint_live_names, run_with_template, Play, ProfileOptions, SlotArm,
};
use script::native::{ScriptStatus, StatusValue};
use serde_json::{json, Value};
use vault::{Profile, ProfileSettings};

const COW_FIELD: WorldTile = WorldTile {
    x: 3255,
    z: 3278,
    level: 0,
};

const TREE_TILE: WorldTile = WorldTile {
    x: 3190,
    z: 3245,
    level: 0,
};

const BRONZE_AXE: i32 = 1351;

#[derive(Clone, Copy, PartialEq, Eq)]
enum CellMode {
    Combat,
    Gather,
    Quest,
}

#[derive(Default)]
struct Fixture {
    phase: u8,
    ready: bool,
    started: bool,
    error: Option<String>,
    tutorial_baseline: i32,
    tutorial_reseed_baseline: i32,
    tutorial_reseed: Option<scenario::tutorial::PostRelogTutorial>,
    observed_tile: Option<WorldTile>,
    observed_tick: u32,
    has_axe: bool,
    combat_xp: i32,
    woodcutting_xp: i32,
}

fn cheat(client: &mut Client, command: &str) -> Result<(), String> {
    if interact::cheat(client, command).is_sent() {
        Ok(())
    } else {
        Err(format!("cheat `{command}` refused"))
    }
}

fn chat_has(snapshot: &GameSnapshot, needle: &str) -> bool {
    let needle = needle.to_ascii_lowercase();
    snapshot
        .chat_lines()
        .iter()
        .any(|line| line.text.to_ascii_lowercase().contains(&needle))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreRelogPhase {
    Initial,
    AfterReseed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreRelogAction {
    Logout,
    Reseed,
}

fn fresh_tutorial_reply(chat: &[ChatLineView], baseline: i32) -> Option<i32> {
    chat.iter()
        .filter(|line| line.sequence > baseline)
        .filter_map(|line| scenario::tutorial::parse_tutorial_reply(&line.text))
        .next_back()
}

fn pre_relog_step(
    phase: PreRelogPhase,
    baseline: i32,
    chat: &[ChatLineView],
) -> Result<Option<PreRelogAction>, String> {
    let Some(value) = fresh_tutorial_reply(chat, baseline) else {
        return Ok(None);
    };
    match (phase, value) {
        (_, scenario::tutorial::TUTORIAL_COMPLETE) => Ok(Some(PreRelogAction::Logout)),
        (PreRelogPhase::Initial, 1) => Ok(Some(PreRelogAction::Reseed)),
        (PreRelogPhase::AfterReseed, 1) => {
            Err("fixture tutorial gate: tutorial=1 persisted after reseed".into())
        }
        (_, other) => Err(format!(
            "fixture tutorial gate: unexpected tutorial={other}"
        )),
    }
}

fn logout_then_login(client: &mut Client, arm: &SlotArm) -> Result<(), String> {
    let ifaces = Arc::clone(&client.ifaces);
    if !interact::logout(client, &ifaces) {
        return Err("logout interface unavailable".into());
    }
    arm.arm_explicit_login();
    Ok(())
}

fn stat_base(snapshot: &GameSnapshot, name: &str) -> i32 {
    snapshot
        .stats()
        .iter()
        .find(|stat| stat.name.eq_ignore_ascii_case(name))
        .map_or(0, |stat| stat.base)
}

fn stat_xp(snapshot: &GameSnapshot, name: &str) -> i32 {
    snapshot
        .stats()
        .iter()
        .find(|stat| stat.name.eq_ignore_ascii_case(name))
        .map_or(0, |stat| stat.xp)
}

fn fixture_frame(
    client: &mut Client,
    held: bool,
    mode: CellMode,
    shared: &Mutex<Fixture>,
    arm: &SlotArm,
) {
    let mut state = shared.lock().unwrap();
    if state.error.is_some() {
        return;
    }
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(client);
    state.observed_tick = snapshot.tick();
    state.observed_tile = snapshot
        .tile()
        .map(|(x, z, level)| WorldTile { x, z, level });
    state.has_axe = snapshot
        .inventory()
        .iter()
        .any(|item| item.def.id == BRONZE_AXE && item.count > 0)
        || snapshot
            .equipment()
            .iter()
            .any(|item| item.def.id == BRONZE_AXE && item.count > 0);
    state.combat_xp = stat_xp(&snapshot, "attack")
        + stat_xp(&snapshot, "strength")
        + stat_xp(&snapshot, "defence")
        + stat_xp(&snapshot, "hitpoints");
    state.woodcutting_xp = stat_xp(&snapshot, "woodcutting");

    if state.started {
        return;
    }

    if !client.ingame && (state.phase == 3 || (mode == CellMode::Quest && state.phase == 7)) {
        state.phase += 1;
        return;
    }
    if held || state.ready {
        return;
    }

    let result = (|| -> Result<(), String> {
        match state.phase {
            0 if client.ingame && client.scene_state == 2 => {
                interact::mainland_hop(client);
                state.phase = 1;
            }
            1 => {
                state.tutorial_baseline = scenario::tutorial::chat_baseline(&snapshot);
                cheat(client, scenario::tutorial::TUTORIAL_GETVAR)?;
                state.phase = 2;
            }
            2 => match pre_relog_step(
                PreRelogPhase::Initial,
                state.tutorial_baseline,
                snapshot.chat_lines(),
            )? {
                None => {}
                Some(PreRelogAction::Logout) => {
                    logout_then_login(client, arm)?;
                    state.phase = 3;
                }
                Some(PreRelogAction::Reseed) => {
                    state.tutorial_reseed_baseline = scenario::tutorial::chat_baseline(&snapshot);
                    cheat(client, scenario::tutorial::TUTORIAL_SETVAR)?;
                    cheat(client, scenario::tutorial::TUTORIAL_GETVAR)?;
                    state.phase = 10;
                }
            },
            10 => match pre_relog_step(
                PreRelogPhase::AfterReseed,
                state.tutorial_reseed_baseline,
                snapshot.chat_lines(),
            )? {
                None => {}
                Some(PreRelogAction::Logout) => {
                    logout_then_login(client, arm)?;
                    state.phase = 3;
                }
                Some(PreRelogAction::Reseed) => {
                    return Err("fixture tutorial gate: second pre-relog reseed refused".into());
                }
            },
            4 if client.ingame
                && client.scene_state == 2
                && snapshot
                    .side_tabs()
                    .iter()
                    .any(|tab| tab.index == 3 && tab.available) =>
            {
                let baseline = scenario::tutorial::chat_baseline(&snapshot);
                cheat(client, scenario::tutorial::TUTORIAL_SETVAR)?;
                match mode {
                    CellMode::Combat => {
                        state.tutorial_reseed =
                            Some(scenario::tutorial::PostRelogTutorial::new(baseline));
                        cheat(client, "setstat attack 40")?;
                        cheat(client, "setstat strength 40")?;
                        cheat(client, "setstat defence 40")?;
                        cheat(client, "setstat hitpoints 40")?;
                        state.phase = 5;
                    }
                    CellMode::Gather => {
                        state.tutorial_reseed =
                            Some(scenario::tutorial::PostRelogTutorial::new(baseline));
                        cheat(client, "setstat woodcutting 1")?;
                        cheat(client, "setstat strength 20")?;
                        cheat(client, "setstat defence 20")?;
                        cheat(client, "setstat hitpoints 99")?;
                        state.phase = 5;
                    }
                    CellMode::Quest => {
                        cheat(client, "setvar cookquest 2")?;
                        cheat(client, "setvar rjquest 30")?;
                        cheat(client, "getvar cookquest")?;
                        cheat(client, "getvar rjquest")?;
                        state.phase = 5;
                    }
                }
            }
            5 if mode == CellMode::Quest => {
                if chat_has(&snapshot, "get cookquest: 2") && chat_has(&snapshot, "get rjquest: 30")
                {
                    logout_then_login(client, arm)?;
                    state.phase = 7;
                }
            }
            8 if mode == CellMode::Quest => {
                let cook_green = snapshot.quest_statuses().iter().any(|row| {
                    row.name == "Cook's Assistant" && row.status() == QuestListStatus::Complete
                });
                let rj_yellow = snapshot.quest_statuses().iter().any(|row| {
                    row.name == "Romeo & Juliet" && row.status() == QuestListStatus::InProgress
                });
                if client.ingame && client.scene_state == 2 && cook_green && rj_yellow {
                    state.ready = true;
                }
            }
            5 => {
                cheat(client, scenario::tutorial::TUTORIAL_GETVAR)?;
                state.phase = 9;
            }
            9 => {
                let confirmed = state
                    .tutorial_reseed
                    .as_ref()
                    .map(|reseed| reseed.check(&snapshot))
                    .transpose()?
                    .unwrap_or(false);
                if confirmed {
                    println!("{}", scenario::tutorial::confirmation_log());
                    if mode == CellMode::Gather {
                        cheat(client, "give bronze_axe 1")?;
                    }
                    let tile = match mode {
                        CellMode::Combat => COW_FIELD,
                        CellMode::Gather => TREE_TILE,
                        CellMode::Quest => unreachable!(),
                    };
                    cheat(client, &interact::tele_args(tile.level, tile.x, tile.z))?;
                    state.phase = 6;
                }
            }
            6 => {
                let (target_tile, stats_ready) = match mode {
                    CellMode::Combat => (
                        COW_FIELD,
                        stat_base(&snapshot, "attack") >= 40
                            && stat_base(&snapshot, "strength") >= 40
                            && stat_base(&snapshot, "hitpoints") >= 40,
                    ),
                    CellMode::Gather => (
                        TREE_TILE,
                        stat_base(&snapshot, "hitpoints") >= 99
                            && stat_base(&snapshot, "woodcutting") == 1
                            && state.has_axe,
                    ),
                    CellMode::Quest => unreachable!(),
                };
                if client.ingame
                    && client.scene_state == 2
                    && snapshot.tile() == Some((target_tile.x, target_tile.z, target_tile.level))
                    && stats_ready
                {
                    state.ready = true;
                }
            }
            _ => {}
        }
        Ok(())
    })();
    if let Err(error) = result {
        state.error = Some(error);
    }
}

fn evidence_dir() -> PathBuf {
    let dir = std::env::var_os("BOT_EVIDENCE_DIR")
        .or_else(|| std::env::var_os("LIVE_EVIDENCE_DIR"))
        .expect("BOT_EVIDENCE_DIR or LIVE_EVIDENCE_DIR");
    let path = PathBuf::from(dir);
    std::fs::create_dir_all(&path).expect("create evidence dir");
    path
}

fn asset_path(primary: &str, fallback: &str, description: &str) -> PathBuf {
    api::test_support::local_asset_path(primary, "", description)
        .or_else(|| api::test_support::local_asset_path(fallback, "", description))
        .unwrap_or_else(|| panic!("missing {primary}/{fallback}: {description}"))
}

fn start_fixture(mode: CellMode, uid: i32) -> (Play, String, Arc<Mutex<Fixture>>) {
    assert_eq!(
        std::env::var("LIVE").as_deref(),
        Ok("1"),
        "ignored live cells require LIVE=1"
    );
    let home = PathBuf::from(std::env::var_os("HOME").expect("throwaway HOME"));
    let port = std::env::var("BOT_GAME_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(44_594);
    let http_port = std::env::var("BOT_HTTP_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1_080);
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(port),
        http_port: Some(http_port),
        cache_dir: Some(asset_path(
            "BOT_CACHE_DIR",
            "BOT_CACHE_DIR",
            "R289 cache snapshot for Luau L4c live cell",
        )),
        unpack_dir: Some(home.join(".274bot/unpack-289")),
        vault_path: Some(home.join("vault")),
        nav_pack: Some(asset_path(
            "WORLD_NAV_PACK",
            "BOT_NAV_PACK",
            "R289 navigation pack for Luau L4c live cell",
        )),
        engine_dir: Some(asset_path(
            "WORLD_ENGINE_DIR",
            "BOT_ENGINE_DIR",
            "R289 engine content for Luau L4c live cell",
        )),
        ..ProfileOptions::default()
    };
    let template = options
        .resolve_preset(None)
        .expect("local 289 profile")
        .prepare_template()
        .expect("cached client template");
    let entries = mint_live_entries(&mint_live_names(1));
    let account = entries[0].0.clone();
    let state = Arc::new(Mutex::new(Fixture::default()));
    let frame_state = Arc::clone(&state);
    let frame_account = account.clone();
    let arm = SlotArm::new(uid, true);
    let frame_arm = Arc::clone(&arm);
    let mut play = run_with_template(
        template,
        false,
        vec![],
        |_| (None, None),
        move |client, name, input| {
            if name == frame_account {
                fixture_frame(client, input.hold, mode, &frame_state, &frame_arm);
            }
        },
    )
    .expect("start public Play");
    play.try_spawn_slot(
        Profile {
            username: account.clone(),
            password: entries[0].1.clone().into(),
            uid,
            settings: ProfileSettings::default(),
        },
        None,
        None,
        Some(arm),
    )
    .expect("spawn fresh local fixture account");
    wait_for(&play, &account, &state, Duration::from_secs(240), |_| {
        state.lock().unwrap().ready
    });
    (play, account, state)
}

fn wait_for(
    play: &Play,
    account: &str,
    fixture: &Mutex<Fixture>,
    timeout: Duration,
    mut ready: impl FnMut(&Play) -> bool,
) {
    let deadline = Instant::now() + timeout;
    loop {
        let error = fixture.lock().unwrap().error.clone();
        assert!(error.is_none(), "fixture error: {error:?}");
        if ready(play) {
            return;
        }
        let phase = fixture.lock().unwrap().phase;
        assert!(
            Instant::now() < deadline,
            "live cell timeout: account={account}, phase={phase}, state={:?}, error={:?}, native={:?}",
            play.script_state(account),
            play.script_last_error(account),
            play.script_native_status(account)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn take_logs(play: &Play, account: &str, logs: &mut Vec<String>) {
    logs.extend(play.script_take_pending_logs(account));
}

fn logged_fields(logs: &[String], label: &str) -> Vec<String> {
    let prefix = format!("{label}\t");
    let rows = logs
        .iter()
        .filter_map(|line| line.strip_prefix(&prefix))
        .collect::<Vec<_>>();
    assert_eq!(
        rows.len(),
        1,
        "expected one {label:?} log line; logs={logs:?}"
    );
    rows[0].split('\t').map(str::to_owned).collect()
}

fn status_int(status: &ScriptStatus, key: &str) -> i64 {
    status
        .fields
        .iter()
        .find_map(|field| match (&field.value, field.key == key) {
            (StatusValue::Integer(value), true) => Some(*value),
            _ => None,
        })
        .unwrap_or(0)
}

fn save_receipt(name: &str, value: &Value) {
    let path = evidence_dir().join(name);
    std::fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    println!("receipt={}\n{value}", path.display());
}

#[test]
#[ignore = "requires LIVE=1, disposable HOME/cache, Engine A and real R289 nav pack"]
fn luau_combat_session() {
    let (play, account, fixture) = start_fixture(CellMode::Combat, 274_289_401);
    fixture.lock().unwrap().started = true;
    let xp_before = fixture.lock().unwrap().combat_xp;
    let source = r#"
local started = false
return {
    tick = function(api)
        if started then return end
        started = true
        local out = api.combat.fight({
            target = { npc = "cow" },
            area = { min_x = 3240, min_z = 3254, max_x = 3266, max_z = 3299, level = 0 },
            style = "melee",
            prayer = false,
            food = false,
            potions = false,
            budgetTicks = 200,
        })
        local value = out.value or {}
        local report = value.report or {}
        api.log(
            "l4c:combat",
            out.kind,
            value["end"],
            report["end"],
            report.npcType,
            report.ticks,
            report.swings
        )
        api.stop("l4c combat complete")
    end,
}
"#;
    let mut logs = Vec::new();
    play.script_start_luau(&account, source.to_owned(), None)
        .expect("start Luau combat session");
    wait_for(
        &play,
        &account,
        &fixture,
        Duration::from_secs(180),
        |play| {
            take_logs(play, &account, &mut logs);
            assert_ne!(
                play.script_state(&account),
                script::RunState::Error,
                "Luau combat errored: {:?}; logs={logs:?}",
                play.script_last_error(&account)
            );
            play.script_state(&account) == script::RunState::Idle
                && play
                    .script_lifecycle_receipt(&account)
                    .is_some_and(|receipt| {
                        receipt.state == script::slot::ScriptTerminalState::Stopped
                    })
        },
    );
    take_logs(&play, &account, &mut logs);
    let receipt = play
        .script_lifecycle_receipt(&account)
        .expect("combat lifecycle receipt");
    assert_eq!(receipt.reason, "l4c combat complete", "logs={logs:?}");
    let fields = logged_fields(&logs, "l4c:combat");
    assert_eq!(fields[0], "done", "logs={logs:?}");
    assert_eq!(fields[1], "fought", "logs={logs:?}");
    assert_eq!(fields[2], "killed", "logs={logs:?}");
    let npc_type: i32 = fields[3].parse().expect("npcType integer");
    let ticks: i64 = fields[4].parse().expect("ticks integer");
    let swings: i64 = fields[5].parse().expect("swings integer");
    assert!(npc_type >= 0, "engaged real cow npc type: {fields:?}");
    assert!(
        ticks > 0 && swings > 0,
        "combat fought on real content: {fields:?}"
    );
    let xp_after = fixture.lock().unwrap().combat_xp;
    assert!(
        xp_after > xp_before,
        "combat XP rose on Engine A: before={xp_before}, after={xp_after}"
    );
    save_receipt(
        "luau-combat-session-receipt.json",
        &json!({
            "cell": "luau_combat_session",
            "account": account,
            "combat_xp_before": xp_before,
            "combat_xp_after": xp_after,
            "npc_type": npc_type,
            "ticks": ticks,
            "swings": swings,
            "logs": logs,
            "host_commit": build_stamp::COMMIT,
            "host_dirty": build_stamp::DIRTY,
            "receipt": receipt,
        }),
    );
}

#[test]
#[ignore = "requires LIVE=1, disposable HOME/cache, Engine A and real R289 nav pack"]
fn luau_gather_session() {
    let (play, account, fixture) = start_fixture(CellMode::Gather, 274_289_402);
    fixture.lock().unwrap().started = true;
    let wc_xp_before = fixture.lock().unwrap().woodcutting_xp;
    let source = r#"
local started = false
return {
    tick = function(api)
        if started then return end
        started = true
        api.log("l4c:gather-start", api.status.tick())
        local out = api.gather.run({
            skill = "Woodcutting",
            woodcuttingResources = { "normal" },
            location = "Start",
            radius = 12,
            disposition = "Power",
            buyTools = false,
            allowTeleports = false,
        })
        api.log("l4c:gather-ended", out.kind)
    end,
    onStop = function()
        api.log("l4c:gather-onstop")
    end,
}
"#;
    let mut logs = Vec::new();
    play.script_start_luau(&account, source.to_owned(), None)
        .expect("start Luau gather live woodcutting");
    let mut yielded = 0;
    let mut status_xp = 0;
    let mut saw_gathering = false;
    wait_for(
        &play,
        &account,
        &fixture,
        Duration::from_secs(180),
        |play| {
            take_logs(play, &account, &mut logs);
            assert_ne!(
                play.script_state(&account),
                script::RunState::Error,
                "Luau gather errored: {:?}; logs={logs:?}",
                play.script_last_error(&account)
            );
            if let Some(status) = play.script_native_status(&account) {
                yielded = yielded.max(status_int(&status, "yielded"));
                status_xp = status_xp.max(status_int(&status, "xp"));
                if status.fields.iter().any(|field| {
                    field.key == "phase"
                        && matches!(&field.value, StatusValue::Text(phase) if phase.as_ref() == "gathering")
                }) {
                    saw_gathering = true;
                }
            }
            yielded >= 1 && status_xp > 0 && fixture.lock().unwrap().woodcutting_xp > wc_xp_before
        },
    );
    play.script_stop(&account);
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |play| {
        take_logs(play, &account, &mut logs);
        play.script_state(&account) == script::RunState::Idle
    });
    take_logs(&play, &account, &mut logs);
    assert!(
        logs.iter()
            .any(|line| line.starts_with("l4c:gather-start\t")),
        "gather start logged: logs={logs:?}"
    );
    assert!(
        logs.iter().any(|line| line == "l4c:gather-onstop"),
        "onStop ran on stop: logs={logs:?}"
    );
    assert!(
        play.script_native_status(&account).is_none(),
        "Gatherer seat retired on script stop"
    );
    let wc_xp_after = fixture.lock().unwrap().woodcutting_xp;
    save_receipt(
        "luau-gather-session-receipt.json",
        &json!({
            "cell": "luau_gather_session",
            "account": account,
            "saw_gathering": saw_gathering,
            "yielded": yielded,
            "status_xp": status_xp,
            "woodcutting_xp_before": wc_xp_before,
            "woodcutting_xp_after": wc_xp_after,
            "logs": logs,
            "host_commit": build_stamp::COMMIT,
            "host_dirty": build_stamp::DIRTY,
        }),
    );
}

#[test]
#[ignore = "requires LIVE=1, disposable HOME/cache, Engine A and real R289 nav pack"]
fn luau_quest_reads() {
    let (play, account, fixture) = start_fixture(CellMode::Quest, 274_289_403);
    fixture.lock().unwrap().started = true;
    let source = r#"
local started = false
return {
    tick = function(api)
        if started then return end
        started = true
        local cookStatus = api.questStatus("Cook's Assistant")
        local rjStatus = api.questStatus("Romeo & Juliet")
        local missingStatus = api.questStatus("No Such Quest")
        local paths = api.questPaths()
        local ids = {}
        for i = 1, #paths do
            ids[i] = paths[i].id
        end
        local cook = api.questProgress("cook")
        local cookVal = cook.value or {}
        local cookRow = cookVal.row or {}
        local cookStage = cookRow.stage or {}
        local rj = api.questProgress("romeojuliet")
        local rjVal = rj.value or {}
        local rjRow = rjVal.row or {}
        local rjStage = rjRow.stage or {}
        local unknown = api.questProgress("nonexistent_quest_path")
        local unknownVal = unknown.value or {}
        api.log("l4c:status-cook", cookStatus.ok, cookStatus.status, cookStatus.as_of_sequence)
        api.log("l4c:status-rj", rjStatus.ok, rjStatus.status, rjStatus.as_of_sequence)
        api.log("l4c:status-missing", missingStatus.ok, missingStatus.error)
        api.log("l4c:paths", table.concat(ids, ","))
        api.log(
            "l4c:progress-cook",
            cook.kind,
            cookVal["end"],
            cookRow.quest,
            cookRow.colour,
            cookStage.state,
            cookStage.value,
            cookRow.complete,
            cookRow.journal_read
        )
        api.log(
            "l4c:progress-rj",
            rj.kind,
            rjVal["end"],
            rjRow.quest,
            rjRow.colour,
            rjStage.state,
            rjStage.value,
            rjRow.complete,
            rjRow.journal_read
        )
        api.log("l4c:progress-unknown", unknown.kind, unknownVal["end"], unknownVal.reason)
        api.stop("l4c quest reads complete")
    end,
}
"#;
    let mut logs = Vec::new();
    play.script_start_luau(&account, source.to_owned(), None)
        .expect("start Luau quest reads");
    wait_for(
        &play,
        &account,
        &fixture,
        Duration::from_secs(120),
        |play| {
            take_logs(play, &account, &mut logs);
            assert_ne!(
                play.script_state(&account),
                script::RunState::Error,
                "Luau quest reads errored: {:?}; logs={logs:?}",
                play.script_last_error(&account)
            );
            play.script_state(&account) == script::RunState::Idle
                && play
                    .script_lifecycle_receipt(&account)
                    .is_some_and(|receipt| {
                        receipt.state == script::slot::ScriptTerminalState::Stopped
                    })
        },
    );
    take_logs(&play, &account, &mut logs);
    let receipt = play
        .script_lifecycle_receipt(&account)
        .expect("quest reads lifecycle receipt");
    assert_eq!(receipt.reason, "l4c quest reads complete", "logs={logs:?}");

    let status_cook = logged_fields(&logs, "l4c:status-cook");
    assert_eq!(status_cook[0], "true");
    assert_eq!(status_cook[1], "complete");
    assert!(status_cook[2].parse::<u64>().unwrap_or(0) > 0);

    let status_rj = logged_fields(&logs, "l4c:status-rj");
    assert_eq!(status_rj[0], "true");
    assert_eq!(status_rj[1], "inProgress");

    let status_missing = logged_fields(&logs, "l4c:status-missing");
    assert_eq!(status_missing, ["false", "not-on-tab"]);

    let paths = logged_fields(&logs, "l4c:paths");
    let expected_paths = script::quester::card::released_paths()
        .iter()
        .map(|row| row.id.as_ref())
        .collect::<Vec<_>>()
        .join(",");
    assert_eq!(paths[0], expected_paths);

    let progress_cook = logged_fields(&logs, "l4c:progress-cook");
    assert_eq!(
        progress_cook,
        ["done", "done", "cook", "complete", "known", "cook:2", "true", "false"]
    );

    let progress_rj = logged_fields(&logs, "l4c:progress-rj");
    assert_eq!(progress_rj[0], "done");
    assert_eq!(progress_rj[1], "done");
    assert_eq!(progress_rj[2], "romeojuliet");
    assert_eq!(progress_rj[3], "inProgress");
    assert_eq!(progress_rj[4], "known");
    assert_eq!(progress_rj[6], "false");
    assert_eq!(progress_rj[7], "true");

    let progress_unknown = logged_fields(&logs, "l4c:progress-unknown");
    assert_eq!(progress_unknown, ["done", "refused", "unknown-path"]);

    save_receipt(
        "luau-quest-reads-receipt.json",
        &json!({
            "cell": "luau_quest_reads",
            "account": account,
            "status_cook": status_cook,
            "status_rj": status_rj,
            "status_missing": status_missing,
            "paths": paths[0],
            "progress_cook": progress_cook,
            "progress_rj": progress_rj,
            "progress_unknown": progress_unknown,
            "logs": logs,
            "host_commit": build_stamp::COMMIT,
            "host_dirty": build_stamp::DIRTY,
            "receipt": receipt,
        }),
    );
}
