#![cfg(all(feature = "test-support", feature = "luau"))]
//! Live Engine A witnesses for Luau promotion rows (ADR 0023 item 32):
//! - `host.luau-walk-route`: `walk-near` (`walk.near`) and `inspect-route` (`route.inspect`).
//! - `host.luau-bank-ops`: `walk-nearest-bank` (`walk.nearestBank`), `open-booth` (`bank.openBooth`),
//!   `close` (`bank.closeNow`), `open-stand` (`bank.openStand`), `deposit` (`bank.deposit`),
//!   `set-note-mode` (`bank.setNoteMode`), `withdraw` (`bank.withdraw`), `withdraw-x` (`bank.withdrawX`).
//! - `host.luau-held-puzzle`: `held` (`interact.heldByName`) and `puzzle-move` (`puzzle.move`).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use api::interact;
use api::snapshot::{ChatLineView, GameSnapshot, WorldTile};
use client::client::Client;
use host_play::{
    mint_live_entries, mint_live_names, run_with_template, Play, ProfileOptions, SlotArm,
};
use serde_json::{json, Value};
use vault::{Profile, ProfileSettings};

const LUMBRIDGE_START: WorldTile = WorldTile {
    x: 3222,
    z: 3222,
    level: 0,
};

const WALK_NEAR_TARGET: WorldTile = WorldTile {
    x: 3220,
    z: 3212,
    level: 0,
};

const DRAYNOR_ROAD: WorldTile = WorldTile {
    x: 3103,
    z: 3249,
    level: 0,
};

const DRAYNOR_BANK_CENTER: WorldTile = WorldTile {
    x: 3092,
    z: 3243,
    level: 0,
};

// Engine A obj.pack:1352-1353 (`bronze_axe`, `cert_bronze_axe`), varp.pack:116 (`bankcert`),
// obj.pack:2796 (`trail_clue_hard_riddle014_puzzlebox`).
const BRONZE_AXE: i32 = 1351;
const BRONZE_AXE_NOTED: i32 = 1352;
const VARP_BANKCERT: usize = 115;
const PUZZLE_BOX_ID: i32 = 2795;

#[derive(Clone, Copy, PartialEq, Eq)]
enum CellMode {
    WalkRoute,
    BankOps,
    HeldPuzzle,
}

#[derive(Clone, Copy, Debug)]
struct ObservedBooth {
    tile: WorldTile,
    id: i32,
    op: i32,
    distance: i32,
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
    bank_open: bool,
    bank_loaded: bool,
    bankcert_varp: i32,
    inv_counts: BTreeMap<i32, i32>,
    bank_counts: BTreeMap<i32, i32>,
    nearby_booth: Option<ObservedBooth>,
    puzzle_component: i32,
    puzzle_size: i32,
    puzzle_slots: BTreeMap<i32, i32>,
}

fn cheat(client: &mut Client, command: &str) -> Result<(), String> {
    if interact::cheat(client, command).is_sent() {
        Ok(())
    } else {
        Err(format!("cheat `{command}` refused"))
    }
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

fn chebyshev(a: WorldTile, b: WorldTile) -> i32 {
    (a.x - b.x).abs().max((a.z - b.z).abs())
}

fn fixture_frame(
    client: &mut Client,
    held: bool,
    mode: CellMode,
    shared: &Mutex<Fixture>,
    snapshot: &Mutex<GameSnapshot>,
    arm: &SlotArm,
) {
    let mut state = shared.lock().unwrap();
    if state.error.is_some() {
        return;
    }
    let mut snapshot = snapshot.lock().unwrap();
    snapshot.rebuild(client);
    state.observed_tick = snapshot.tick();
    state.observed_tile = snapshot
        .tile()
        .map(|(x, z, level)| WorldTile { x, z, level });
    state.bank_open = snapshot.bank_component_id() != -1;
    state.bank_loaded = snapshot.bank_loaded();
    state.bankcert_varp = snapshot
        .varps()
        .iter()
        .find(|varp| varp.index == VARP_BANKCERT as i32)
        .map_or(0, |varp| varp.value);
    state.inv_counts.clear();
    for item in snapshot.inventory() {
        *state.inv_counts.entry(item.def.id).or_default() += item.count;
    }
    if state.bank_open && state.bank_loaded {
        state.bank_counts.clear();
        for item in snapshot.bank() {
            *state.bank_counts.entry(item.def.id).or_default() += item.count;
        }
    }
    state.nearby_booth = state.observed_tile.and_then(|here| {
        snapshot
            .locs()
            .iter()
            .filter(|loc| loc.name.as_deref() == Some("Bank booth"))
            .filter(|loc| {
                api::query::loc_approach::can_operate_from(loc, snapshot.scene(), here)
                    == Some(true)
            })
            .filter_map(|loc| {
                let op = loc
                    .actions
                    .iter()
                    .position(|action| action.as_deref() == Some("Use-quickly"))
                    .map(|index| (index + 1) as i32)?;
                Some(ObservedBooth {
                    tile: loc.tile,
                    id: loc.id,
                    op,
                    distance: loc.distance,
                })
            })
            .min_by_key(|booth| booth.distance)
    });

    let board = snapshot.puzzle_board();
    state.puzzle_component = board.component_id;
    state.puzzle_size = board.size;
    state.puzzle_slots.clear();
    for item in board.items {
        state.puzzle_slots.insert(item.slot, item.def.id);
    }

    if state.started {
        return;
    }

    if !client.ingame && state.phase == 3 {
        state.phase = 4;
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
                state.tutorial_reseed = Some(scenario::tutorial::PostRelogTutorial::new(baseline));
                state.phase = 5;
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
                    match mode {
                        CellMode::WalkRoute => {
                            cheat(
                                client,
                                &interact::tele_args(
                                    LUMBRIDGE_START.level,
                                    LUMBRIDGE_START.x,
                                    LUMBRIDGE_START.z,
                                ),
                            )?;
                        }
                        CellMode::BankOps => {
                            cheat(client, "give bronze_axe 5")?;
                            cheat(
                                client,
                                &interact::tele_args(
                                    DRAYNOR_ROAD.level,
                                    DRAYNOR_ROAD.x,
                                    DRAYNOR_ROAD.z,
                                ),
                            )?;
                        }
                        CellMode::HeldPuzzle => {
                            cheat(client, "give trail_clue_hard_riddle014_puzzlebox 1")?;
                            cheat(
                                client,
                                &interact::tele_args(
                                    LUMBRIDGE_START.level,
                                    LUMBRIDGE_START.x,
                                    LUMBRIDGE_START.z,
                                ),
                            )?;
                        }
                    }
                    state.phase = 6;
                }
            }
            6 => {
                let (target_tile, items_ready) = match mode {
                    CellMode::WalkRoute => (LUMBRIDGE_START, true),
                    CellMode::BankOps => (
                        DRAYNOR_ROAD,
                        state.inv_counts.get(&BRONZE_AXE).copied().unwrap_or(0) >= 5,
                    ),
                    CellMode::HeldPuzzle => (
                        LUMBRIDGE_START,
                        state.inv_counts.get(&PUZZLE_BOX_ID).copied().unwrap_or(0) >= 1,
                    ),
                };
                if client.ingame
                    && client.scene_state == 2
                    && snapshot.tile() == Some((target_tile.x, target_tile.z, target_tile.level))
                    && items_ready
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
            "R289 cache snapshot for Luau promotion live cell",
        )),
        unpack_dir: Some(home.join(".274bot/unpack-289")),
        vault_path: Some(home.join("vault")),
        nav_pack: Some(asset_path(
            "WORLD_NAV_PACK",
            "BOT_NAV_PACK",
            "R289 navigation pack for Luau promotion live cell",
        )),
        engine_dir: Some(asset_path(
            "WORLD_ENGINE_DIR",
            "BOT_ENGINE_DIR",
            "R289 engine content for Luau promotion live cell",
        )),
        ..ProfileOptions::default()
    };
    let template = options
        .resolve(None)
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
    let frame_snapshot = Mutex::new(GameSnapshot::new());
    let mut play = run_with_template(
        template,
        false,
        vec![],
        |_| (None, None),
        move |client, name, input| {
            if name == frame_account {
                fixture_frame(
                    client,
                    input.hold,
                    mode,
                    &frame_state,
                    &frame_snapshot,
                    &frame_arm,
                );
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

fn run_luau_step(
    play: &Play,
    account: &str,
    fixture: &Mutex<Fixture>,
    label: &str,
    body: &str,
    logs: &mut Vec<String>,
) -> u64 {
    let source = format!(
        r#"
local started = false
return {{
    tick = function(api)
        if started then return end
        started = true
        api.log("{label}:start", api.status.tick())
        {body}
        api.wait()
        api.log("{label}:end", api.status.tick())
        api.stop("{label} complete")
    end,
}}
"#
    );
    std::fs::write(evidence_dir().join(format!("{label}.luau")), &source).unwrap();
    play.script_start_luau(account, source, None)
        .expect("start Luau promotion step");
    wait_for(play, account, fixture, Duration::from_secs(120), |play| {
        take_logs(play, account, logs);
        play.script_state(account) == script::RunState::Idle
            && play
                .script_lifecycle_receipt(account)
                .is_some_and(|receipt| {
                    receipt.state == script::slot::ScriptTerminalState::Stopped
                        && receipt.reason == format!("{label} complete")
                })
    });
    take_logs(play, account, logs);
    let start = logged_fields(logs, &format!("{label}:start"));
    let end = logged_fields(logs, &format!("{label}:end"));
    let tick = start[0].parse::<u64>().unwrap();
    assert!(end[0].parse::<u64>().unwrap() > tick);
    tick
}

fn save_receipt(name: &str, value: &Value) {
    let path = evidence_dir().join(name);
    std::fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    println!("receipt={}\n{value}", path.display());
}

#[test]
#[ignore = "requires LIVE=1, disposable HOME/cache, Engine A and real R289 nav pack"]
fn luau_walk_near_and_route_inspect() {
    let (play, account, fixture) = start_fixture(CellMode::WalkRoute, 274_289_501);
    fixture.lock().unwrap().started = true;
    let source = r#"
local started = false
return {
    tick = function(api)
        if started then return end
        started = true
        local targets = {
            { x = 3093, z = 3243, level = 0, avoid = { { minX = 3250, maxX = 3260, minZ = 3250, maxZ = 3260, level = 0 } } },
            { x = 3103, z = 3249, level = 0 },
            { x = 3232, z = 3222, level = 0 },
            { x = 3220, z = 3212, level = 0 },
        }
        local prior = api.route.result()
        local last_seq = prior and prior.seq or 0
        for i, target in ipairs(targets) do
            local opts = {
                allow_teleports = false,
                allow_wilderness = false,
                allow_bank_fetch = false,
                request_id = 0,
            }
            if target.avoid ~= nil then
                opts.avoid = target.avoid
            end
            api.route.inspect(
                { x = 3222, z = 3222, level = 0 },
                { x = target.x, z = target.z, level = target.level },
                opts
            )
            local inspected = nil
            for _ = 1, 60 do
                api.wait()
                local res = api.route.result()
                if res ~= nil and res.seq > last_seq then
                    inspected = res
                    last_seq = res.seq
                    break
                end
            end
            assert(inspected ~= nil, "route.inspect #" .. tostring(i) .. " produced no result")
            api.log("inspect_" .. tostring(i), inspected.seq, tostring(inspected.ok), inspected.ticks, inspected.request_id, #inspected.hops, inspected.reason)
        end
        local walk = api.walk.near(3220, 3212, 0, 3)
        local x, z, level = api.status.tile()
        api.log("walk_near", tostring(walk.ok), tostring(walk.failed), walk.radius, x, z, level)
        api.stop("luau walk-route complete")
    end,
}
"#;
    std::fs::write(evidence_dir().join("luau-walk-route.luau"), source).unwrap();
    play.script_start_luau(&account, source.to_owned(), None)
        .expect("start Luau walk-near + route-inspect cell");
    let mut logs = Vec::new();
    wait_for(
        &play,
        &account,
        &fixture,
        Duration::from_secs(120),
        |play| {
            take_logs(play, &account, &mut logs);
            play.script_state(&account) == script::RunState::Idle
        },
    );
    take_logs(&play, &account, &mut logs);
    let receipt = play
        .script_lifecycle_receipt(&account)
        .expect("walk-route lifecycle receipt");
    assert_eq!(
        receipt.state,
        script::slot::ScriptTerminalState::Stopped,
        "logs={logs:?}, error={:?}",
        play.script_last_error(&account)
    );
    assert_eq!(receipt.reason, "luau walk-route complete");

    let mut inspect_receipts = Vec::new();
    let mut prev_seq = 0u64;
    for idx in 1..=4 {
        let inspect = logged_fields(&logs, &format!("inspect_{idx}"));
        let inspect_seq: u64 = inspect[0].parse().unwrap();
        let inspect_ticks: f64 = inspect[2].parse().unwrap();
        assert!(
            inspect_seq > prev_seq,
            "inspect_{idx} seq {inspect_seq} did not advance past {prev_seq}: {inspect:?}"
        );
        assert_eq!(inspect[1], "true", "inspect_{idx}={inspect:?}");
        assert!(inspect_ticks > 0.0, "inspect_{idx}={inspect:?}");
        assert_eq!(inspect[3], "0", "inspect_{idx}={inspect:?}");
        prev_seq = inspect_seq;
        inspect_receipts.push(json!({
            "index": idx,
            "seq": inspect_seq,
            "ok": true,
            "ticks": inspect_ticks,
            "hops": inspect[4].parse::<usize>().unwrap(),
        }));
    }
    let walk = logged_fields(&logs, "walk_near");
    assert_eq!(walk[0], "true", "walk={walk:?}");
    assert_eq!(walk[1], "false", "walk={walk:?}");
    assert_eq!(walk[2], "3", "walk={walk:?}");
    let final_tile = WorldTile {
        x: walk[3].parse().unwrap(),
        z: walk[4].parse().unwrap(),
        level: walk[5].parse().unwrap(),
    };
    assert_eq!(final_tile.level, WALK_NEAR_TARGET.level);
    let dist = chebyshev(final_tile, WALK_NEAR_TARGET);
    assert!(
        dist <= 3,
        "expected walk.near within radius 3 of {WALK_NEAR_TARGET:?}, got {final_tile:?} (dist={dist})"
    );
    let observed = fixture
        .lock()
        .unwrap()
        .observed_tile
        .expect("observed tile");
    assert!(
        chebyshev(observed, WALK_NEAR_TARGET) <= 3,
        "observed Engine A tile {observed:?} outside radius 3 of {WALK_NEAR_TARGET:?}"
    );

    save_receipt(
        "luau-walk-route.json",
        &json!({
            "cell": "host.luau-walk-route",
            "account": account,
            "inspects": inspect_receipts,
            "walk_near": {
                "target": [WALK_NEAR_TARGET.x, WALK_NEAR_TARGET.z, WALK_NEAR_TARGET.level],
                "radius": 3,
                "final_tile": [final_tile.x, final_tile.z, final_tile.level],
                "chebyshev_distance": dist,
            },
            "receipt": receipt,
        }),
    );
}

#[test]
#[ignore = "requires LIVE=1, disposable HOME/cache, Engine A and real R289 nav pack"]
fn luau_bank_ops_and_walk_nearest_bank() {
    let (play, account, fixture) = start_fixture(CellMode::BankOps, 274_289_502);
    fixture.lock().unwrap().started = true;
    let mut logs = Vec::new();

    // 1. walk-nearest-bank: from Draynor road (10 tiles out) to Draynor Bank.
    let walk_tick = run_luau_step(
        &play,
        &account,
        &fixture,
        "walk-nearest-bank",
        "api.walk.nearestBank()",
        &mut logs,
    );
    wait_for(&play, &account, &fixture, Duration::from_secs(60), |_| {
        let state = fixture.lock().unwrap();
        state
            .observed_tile
            .is_some_and(|tile| chebyshev(tile, DRAYNOR_BANK_CENTER) <= 4)
            && state.nearby_booth.is_some()
    });
    let booth = fixture
        .lock()
        .unwrap()
        .nearby_booth
        .expect("Draynor Bank booth");

    // 2. open-booth: open exact booth loc by tile, id, name and action.
    let open_booth_tick = run_luau_step(
        &play,
        &account,
        &fixture,
        "open-booth",
        &format!(
            "api.bank.openBooth({}, {}, {}, {}, {{ name = \"Bank booth\", action = \"Use-quickly\" }})",
            booth.tile.x, booth.tile.z, booth.tile.level, booth.id
        ),
        &mut logs,
    );
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |_| {
        let state = fixture.lock().unwrap();
        state.bank_open && state.bank_loaded
    });

    // 3. close: fire-and-forget NativeOp:close closes the bank modal.
    let close_tick = run_luau_step(
        &play,
        &account,
        &fixture,
        "close-now",
        "api.bank.closeNow()",
        &mut logs,
    );
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |_| {
        !fixture.lock().unwrap().bank_open
    });

    // 4. open-stand: reopen the packed booth stand.
    let open_stand_tick = run_luau_step(
        &play,
        &account,
        &fixture,
        "open-stand",
        &format!(
            "api.bank.openStand({}, {}, {}, \"booth\", {{ name = \"Bank booth\", stand_op = {} }})",
            booth.tile.x, booth.tile.z, booth.tile.level, booth.op
        ),
        &mut logs,
    );
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |_| {
        let state = fixture.lock().unwrap();
        state.bank_open && state.bank_loaded
    });
    let bank_axes_before = fixture
        .lock()
        .unwrap()
        .bank_counts
        .get(&BRONZE_AXE)
        .copied()
        .unwrap_or(0);
    let seeded_axes = fixture
        .lock()
        .unwrap()
        .inv_counts
        .get(&BRONZE_AXE)
        .copied()
        .unwrap_or(0);
    assert!(seeded_axes >= 5, "seeded_axes={seeded_axes}");

    // 5. deposit: deposit all Bronze axes by name.
    let deposit_tick = run_luau_step(
        &play,
        &account,
        &fixture,
        "deposit",
        "api.bank.deposit(\"Bronze axe\")",
        &mut logs,
    );
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |_| {
        let state = fixture.lock().unwrap();
        state.inv_counts.get(&BRONZE_AXE).copied().unwrap_or(0) == 0
            && state.bank_counts.get(&BRONZE_AXE).copied().unwrap_or(0)
                == bank_axes_before + seeded_axes
    });

    // 6. set-note-mode(true): toggles %bankcert = 1 (bank.rs2:13).
    let note_on_tick = run_luau_step(
        &play,
        &account,
        &fixture,
        "note-on",
        "api.bank.setNoteMode(true)",
        &mut logs,
    );
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |_| {
        fixture.lock().unwrap().bankcert_varp == 1
    });

    // 7. withdraw: withdraw 1 Bronze axe while noted mode is on -> lands as cert_bronze_axe (1352).
    let withdraw_tick = run_luau_step(
        &play,
        &account,
        &fixture,
        "withdraw-noted",
        "api.bank.withdraw(\"Bronze axe\", \"Withdraw 1\")",
        &mut logs,
    );
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |_| {
        let state = fixture.lock().unwrap();
        state
            .inv_counts
            .get(&BRONZE_AXE_NOTED)
            .copied()
            .unwrap_or(0)
            == 1
            && state.bank_counts.get(&BRONZE_AXE).copied().unwrap_or(0)
                == bank_axes_before + seeded_axes - 1
    });

    // 8. set-note-mode(false): toggles %bankcert = 0 (bank.rs2:12).
    let note_off_tick = run_luau_step(
        &play,
        &account,
        &fixture,
        "note-off",
        "api.bank.setNoteMode(false)",
        &mut logs,
    );
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |_| {
        fixture.lock().unwrap().bankcert_varp == 0
    });

    // 9. withdraw-x: read live bank.generation() in Luau and withdraw exactly 3 unnoted axes.
    let withdraw_x_tick = run_luau_step(
        &play,
        &account,
        &fixture,
        "withdraw-x",
        &format!(
            "local gen = api.bank.generation()\nassert(gen > 0, \"expected positive bank generation\")\napi.log(\"withdraw_x_gen\", gen)\napi.bank.withdrawX({{ name = \"Bronze axe\", count = 3, bank_item_id = {BRONZE_AXE}, lands_as_id = {BRONZE_AXE}, action = \"Withdraw X\", bank_generation = gen }})"
        ),
        &mut logs,
    );
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |_| {
        let state = fixture.lock().unwrap();
        state.inv_counts.get(&BRONZE_AXE).copied().unwrap_or(0) == 3
            && state.bank_counts.get(&BRONZE_AXE).copied().unwrap_or(0)
                == bank_axes_before + seeded_axes - 4
    });

    // Close bank cleanly.
    let final_close_tick = run_luau_step(
        &play,
        &account,
        &fixture,
        "close-final",
        "api.bank.closeNow()",
        &mut logs,
    );
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |_| {
        !fixture.lock().unwrap().bank_open
    });

    let gen_fields = logged_fields(&logs, "withdraw_x_gen");
    let withdraw_x_gen: u64 = gen_fields[0].parse().unwrap();
    assert!(withdraw_x_gen > 0);

    save_receipt(
        "luau-bank-ops.json",
        &json!({
            "cell": "host.luau-bank-ops",
            "account": account,
            "booth": {
                "tile": [booth.tile.x, booth.tile.z, booth.tile.level],
                "id": booth.id,
                "op": booth.op,
            },
            "steps": {
                "walk_nearest_bank_tick": walk_tick,
                "open_booth_tick": open_booth_tick,
                "close_now_tick": close_tick,
                "open_stand_tick": open_stand_tick,
                "deposit_tick": deposit_tick,
                "note_on_tick": note_on_tick,
                "withdraw_noted_tick": withdraw_tick,
                "note_off_tick": note_off_tick,
                "withdraw_x_tick": withdraw_x_tick,
                "withdraw_x_generation": withdraw_x_gen,
                "final_close_tick": final_close_tick,
            },
            "content_evidence": {
                "bank_rs2": "content/scripts/interface_bank/scripts/bank.rs2:1-5,12-28,53-107",
                "obj_pack": "content/pack/obj.pack:1352-1353",
                "varp_pack": "content/pack/varp.pack:116",
            },
        }),
    );
}

#[test]
#[ignore = "requires LIVE=1, disposable HOME/cache, Engine A and real R289 nav pack"]
fn luau_held_by_name_and_puzzle_move() {
    let (play, account, fixture) = start_fixture(CellMode::HeldPuzzle, 274_289_503);
    fixture.lock().unwrap().started = true;
    let mut logs = Vec::new();

    // 1. held (NativeOp:held): open the seeded Puzzle box by name and action only.
    let held_tick = run_luau_step(
        &play,
        &account,
        &fixture,
        "held-open-puzzle",
        "api.interact.heldByName(\"Puzzle box\", \"Open\")",
        &mut logs,
    );
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |_| {
        let state = fixture.lock().unwrap();
        state.puzzle_component >= 0 && state.puzzle_size == 25 && state.puzzle_slots.len() == 24
    });

    // 2. puzzle-move (NativeOp:puzzle-move): read the live 5x5 board in Luau,
    // pick an orthogonal neighbor of the empty slot, and slide it.
    let move_body = r#"
local board = api.puzzle.board()
assert(board ~= nil and board.component_id >= 0 and board.size == 25 and board.generation > 0, "expected open 5x5 puzzle board")
local occupied = {}
for i = 1, #board.items do
    occupied[board.items[i].slot] = board.items[i]
end
local empty_slot = nil
for s = 0, 24 do
    if occupied[s] == nil then
        empty_slot = s
        break
    end
end
assert(empty_slot ~= nil, "expected one empty slot")
local er = math.floor(empty_slot / 5)
local ec = empty_slot % 5
local chosen = nil
for i = 1, #board.items do
    local item = board.items[i]
    local r = math.floor(item.slot / 5)
    local c = item.slot % 5
    if math.abs(r - er) + math.abs(c - ec) == 1 then
        chosen = item
        break
    end
end
assert(chosen ~= nil, "expected an adjacent piece")
api.log("puzzle_move", chosen.id, chosen.slot, empty_slot, board.component_id, board.generation)
api.puzzle.move(chosen.id, chosen.slot, board.component_id, board.generation)
"#;
    let move_tick = run_luau_step(
        &play,
        &account,
        &fixture,
        "puzzle-move",
        move_body,
        &mut logs,
    );
    let moved = logged_fields(&logs, "puzzle_move");
    let piece_id: i32 = moved[0].parse().unwrap();
    let from_slot: i32 = moved[1].parse().unwrap();
    let to_slot: i32 = moved[2].parse().unwrap();
    let component_id: i32 = moved[3].parse().unwrap();
    let generation: u64 = moved[4].parse().unwrap();

    // Engine A [opheld5,_trail_puzzle_piece] (trail_puzzle.rs2:328-351) moves the piece into empty_slot.
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |_| {
        let state = fixture.lock().unwrap();
        state.puzzle_slots.get(&to_slot) == Some(&piece_id)
            && !state.puzzle_slots.contains_key(&from_slot)
    });

    save_receipt(
        "luau-held-puzzle.json",
        &json!({
            "cell": "host.luau-held-puzzle",
            "account": account,
            "held_tick": held_tick,
            "puzzle_move": {
                "tick": move_tick,
                "piece_id": piece_id,
                "from_slot": from_slot,
                "to_slot": to_slot,
                "component_id": component_id,
                "generation": generation,
            },
            "content_evidence": {
                "puzzle_box_obj": "content/scripts/minigames/game_trail/configs/trail.obj:748-762",
                "puzzle_script": "content/scripts/minigames/game_trail/scripts/trail_puzzle.rs2:1-10,328-351",
            },
        }),
    );
}
