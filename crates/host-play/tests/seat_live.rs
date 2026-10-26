#![cfg(feature = "test-support")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use api::interact::{self, Interactions};
use api::snapshot::{GameSnapshot, QuestListStatus, WorldTile};
use client::client::Client;
use host_play::{mint_live_entries, mint_live_names, run_with_template, Play, ProfileOptions};
use script::native::{ScriptStatus, StatusValue};
use serde_json::{json, Map, Value};
use vault::{Profile, ProfileSettings};

const TREE_TILE: WorldTile = WorldTile {
    x: 3190,
    z: 3245,
    level: 0,
};
const AXE: i32 = 1351;
static NEXT_UID: AtomicI32 = AtomicI32::new(274_279_104);

#[derive(Clone, Copy)]
enum FixtureTrigger {
    Maze,
    LostConnection,
}

#[derive(Default)]
struct Fixture {
    phase: u8,
    ready: bool,
    error: Option<String>,
    tutorial_reseed: Option<scenario::tutorial::PostRelogTutorial>,
    trigger: Option<FixtureTrigger>,
    trigger_sent: bool,
    disconnect_offline: bool,
    reconnected: bool,
}

struct LiveCell {
    play: Play,
    account: String,
    fixture: Arc<Mutex<Fixture>>,
}

fn evidence_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("BOT_EVIDENCE_DIR").expect("BOT_EVIDENCE_DIR"))
}

fn cheat(client: &mut Client, command: &str) -> Result<(), String> {
    if interact::cheat(client, command).is_sent() {
        Ok(())
    } else {
        Err(format!("fixture cheat refused: {command}"))
    }
}

fn logout(client: &mut Client) -> Result<(), String> {
    let ifaces = Arc::clone(&client.ifaces);
    if interact::logout(client, &ifaces) {
        Ok(())
    } else {
        Err("fixture logout unavailable".into())
    }
}

fn chat_has(snapshot: &GameSnapshot, needle: &str) -> bool {
    snapshot
        .chat_lines()
        .iter()
        .any(|line| line.text.contains(needle))
        || snapshot
            .chat_modal_texts()
            .iter()
            .any(|line| line.contains(needle))
}

fn fixture_frame(client: &mut Client, hold: bool, shared: &Mutex<Fixture>) {
    let mut state = shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(client);

    if state.ready {
        if let Some(trigger) = state.trigger {
            if !hold && client.ingame && client.scene_state == 2 {
                match trigger {
                    FixtureTrigger::Maze => {
                        if interact::cheat(client, "~maze").is_sent() {
                            state.trigger = None;
                            state.trigger_sent = true;
                        }
                    }
                    FixtureTrigger::LostConnection => {
                        // This closes the real Engine A stream; run_with_template
                        // retains external reconnect ownership and credentials.
                        client.lost_con();
                        state.trigger = None;
                        state.trigger_sent = true;
                        state.disconnect_offline = !client.ingame;
                    }
                }
            }
        }
        if state.trigger_sent
            && state.disconnect_offline
            && client.ingame
            && client.scene_state == 2
        {
            state.reconnected = true;
        }
        return;
    }

    // Match the existing local-289 script API fixture: complete the tutorial
    // through a real relog before equipping the axe and entering the tree site.
    if !client.ingame && state.phase == 3 {
        state.phase = 4;
        return;
    }
    if hold || state.ready {
        return;
    }

    let result = (|| -> Result<(), String> {
        match state.phase {
            0 if client.ingame && client.scene_state == 2 => {
                interact::mainland_hop(client);
                state.phase = 1;
            }
            1 => {
                cheat(client, "getvar tutorial")?;
                state.phase = 2;
            }
            2 if chat_has(&snapshot, "get tutorial: 1000") => {
                logout(client)?;
                state.phase = 3;
            }
            4 if client.ingame
                && client.scene_state == 2
                && snapshot
                    .side_tabs()
                    .iter()
                    .any(|tab| tab.index == 3 && tab.available) =>
            {
                let baseline = scenario::tutorial::chat_baseline(&snapshot);
                state.tutorial_reseed = Some(scenario::tutorial::PostRelogTutorial::new(baseline));
                cheat(client, "setvar tutorial 1000")?;
                cheat(client, "setstat woodcutting 1")?;
                cheat(client, "setstat attack 1")?;
                cheat(client, "setstat strength 1")?;
                cheat(client, "setstat defence 98")?;
                cheat(client, "setstat hitpoints 99")?;
                state.phase = 5;
            }
            5 => {
                cheat(client, "getvar tutorial")?;
                state.phase = 6;
            }
            6 => {
                let confirmed = match state.tutorial_reseed.as_ref() {
                    Some(reseed) => reseed.check(&snapshot)?,
                    None => false,
                };
                if confirmed {
                    cheat(client, "give bronze_axe 1")?;
                    state.phase = 7;
                }
            }
            7 if snapshot
                .inventory()
                .iter()
                .any(|item| item.def.id == AXE && item.count > 0) =>
            {
                if !matches!(
                    Interactions::new(&snapshot, client).wear(AXE),
                    api::interact::SendResult::Sent { .. }
                ) {
                    return Err("fixture bronze axe Wield refused".into());
                }
                state.phase = 8;
            }
            8 if snapshot
                .equipment()
                .iter()
                .any(|item| item.def.id == AXE && item.count == 1) =>
            {
                cheat(
                    client,
                    &interact::tele_args(TREE_TILE.level, TREE_TILE.x, TREE_TILE.z),
                )?;
                state.phase = 9;
            }
            9 => {
                let wielded = snapshot
                    .equipment()
                    .iter()
                    .any(|item| item.def.id == AXE && item.count == 1);
                let level_one = snapshot
                    .stats()
                    .iter()
                    .any(|stat| stat.name.eq_ignore_ascii_case("woodcutting") && stat.base == 1);
                let here = snapshot.tile() == Some((TREE_TILE.x, TREE_TILE.z, TREE_TILE.level));
                let cook_red = snapshot.quest_statuses().iter().any(|row| {
                    row.name == "Cook's Assistant" && row.status() == QuestListStatus::NotStarted
                });
                if client.ingame
                    && client.scene_state == 2
                    && wielded
                    && level_one
                    && here
                    && cook_red
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

fn start_fixture(random_events: bool) -> LiveCell {
    assert_eq!(
        std::env::var("LIVE").as_deref(),
        Ok("1"),
        "ignored live cells require LIVE=1"
    );
    let evidence = evidence_dir();
    std::fs::create_dir_all(&evidence).expect("create BOT_EVIDENCE_DIR");
    let home = PathBuf::from(std::env::var_os("HOME").expect("throwaway HOME"));
    assert!(
        home.starts_with(&evidence),
        "HOME must be under BOT_EVIDENCE_DIR"
    );

    let http_port = std::env::var("BOT_HTTP_PORT")
        .expect("BOT_HTTP_PORT")
        .parse()
        .expect("numeric BOT_HTTP_PORT");
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(44594),
        http_port: Some(http_port),
        cache_dir: Some(PathBuf::from(
            std::env::var_os("BOT_CACHE_DIR").expect("copied BOT_CACHE_DIR"),
        )),
        unpack_dir: Some(home.join(".274bot/unpack-289")),
        vault_path: Some(home.join("vault")),
        nav_pack: Some(PathBuf::from(
            std::env::var_os("WORLD_NAV_PACK").expect("WORLD_NAV_PACK"),
        )),
        engine_dir: Some(PathBuf::from(
            std::env::var_os("WORLD_ENGINE_DIR").expect("WORLD_ENGINE_DIR"),
        )),
        ..ProfileOptions::default()
    };
    let template = options
        .resolve(None)
        .expect("local 289 profile")
        .prepare_template()
        .expect("cached client template");
    let names = mint_live_names(1);
    let entries = mint_live_entries(&names);
    let account = entries[0].0.clone();
    let fixture = Arc::new(Mutex::new(Fixture::default()));
    let frame_fixture = Arc::clone(&fixture);
    let frame_account = account.clone();
    let mut play = run_with_template(
        template,
        false,
        vec![],
        |_| (None, None),
        move |client, name, input| {
            if name == frame_account {
                fixture_frame(client, input.hold, &frame_fixture);
            }
        },
    )
    .expect("start public Play");
    play.try_spawn_slot(
        Profile {
            username: account.clone(),
            password: entries[0].1.clone().into(),
            uid: NEXT_UID.fetch_add(1, Ordering::Relaxed),
            settings: ProfileSettings {
                random_events,
                ..ProfileSettings::default()
            },
        },
        None,
        None,
        None,
    )
    .expect("spawn fresh Engine A fixture account");
    let live = LiveCell {
        play,
        account,
        fixture,
    };
    wait_until(&live, "fixture ready", Duration::from_secs(240), |live| {
        live.fixture
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .ready
    });
    live
}

fn wait_until(
    live: &LiveCell,
    label: &str,
    timeout: Duration,
    mut ready: impl FnMut(&LiveCell) -> bool,
) {
    let deadline = Instant::now() + timeout;
    loop {
        let state = live
            .fixture
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(
            state.error.is_none(),
            "{label}: fixture error={:?}, phase={}",
            state.error,
            state.phase
        );
        drop(state);
        if ready(live) {
            return;
        }
        let phase = live
            .fixture
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .phase;
        assert!(
            Instant::now() < deadline,
            "timeout waiting for {label}: account={}, phase={phase}, script={:?}, last_error={:?}, native={:?}, guardian={:?}, seat={:?}",
            live.account,
            live.play.script_state(&live.account),
            live.play.script_last_error(&live.account),
            live.play.script_native_status(&live.account),
            live.play
                .statuses()
                .into_iter()
                .find(|status| status.username == live.account)
                .map(|status| (status.random, status.tile_x, status.tile_z, status.tile_level)),
            live.play.script_seat_probe(&live.account)
        );
        std::thread::sleep(Duration::from_millis(40));
    }
}

fn take_logs(live: &LiveCell, logs: &mut Vec<String>) {
    logs.extend(live.play.script_take_pending_logs(&live.account));
}

fn logged(logs: &[String], prefix: &str) -> Value {
    let rows = logs
        .iter()
        .filter_map(|line| line.strip_prefix(prefix))
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 1, "expected one {prefix:?}, logs={logs:?}");
    serde_json::from_str(rows[0]).expect("public API JSON log")
}

fn status_field(status: &ScriptStatus, key: &str) -> i64 {
    status
        .fields
        .iter()
        .find_map(|field| match (&field.value, field.key == key) {
            (StatusValue::Integer(value), true) => Some(*value),
            _ => None,
        })
        .unwrap_or(0)
}

fn status_text(status: &ScriptStatus, key: &str) -> Option<String> {
    status
        .fields
        .iter()
        .find_map(|field| match (&field.value, field.key == key) {
            (StatusValue::Text(value), true) => Some(value.to_string()),
            _ => None,
        })
}

fn api_probe(live: &LiveCell) -> Value {
    live.play.script_api_live_probe(&live.account)
}

fn seat_probe(live: &LiveCell) -> script::slot::SeatProbe {
    live.play
        .script_seat_probe(&live.account)
        .expect("real slot-owned gameplay seat")
}

fn seat_json(probe: script::slot::SeatProbe) -> Value {
    json!({
        "held": probe.held,
        "dropped": probe.dropped,
        "late_controls": probe.late_controls,
        "live": probe.live,
        "results": probe.results,
        "reserved_releases": probe.reserved_releases,
        "mirror_revokes_consumed": probe.mirror_revokes_consumed,
    })
}

fn save_receipt(name: &str, receipt: &Value) {
    let path = evidence_dir().join(name);
    std::fs::write(&path, serde_json::to_vec_pretty(receipt).unwrap())
        .unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
    println!("seat-live-receipt={}\n{receipt}", path.display());
}

const GATHER_SETTINGS_JSON: &str = r#"{"skill":"Woodcutting","woodcuttingResources":["normal"],"location":"Custom","customTile":{"x":3190,"z":3245,"level":0},"disposition":"Power","radius":12}"#;

const PROMISE_SOURCE: &str = r#"
export const apiVersion = 2;
const settings = { skill: 'Woodcutting', woodcuttingResources: ['normal'], location: 'Custom', customTile: { x: 3190, z: 3245, level: 0 }, disposition: 'Power', radius: 12 };
let run = null;
let stopRequested = false;
export function tick(api) {
  const page = api.snapshot.gather;
  if (page) globalThis.__seatGatherPage = page;
  if (!run && api.snapshot.ingame && api.snapshot.here) {
    run = api.gather.run(settings);
    api.log('seat-promise-start:' + JSON.stringify({ tick: api.tick, here: api.snapshot.here }));
    run.then(value => {
      globalThis.__seatGatherOutcome = value;
      api.log('seat-promise-result:' + JSON.stringify(value));
      api.stop('seat-promise-complete');
    });
  }
  if (run && !stopRequested && page && page.status && page.status.phase === 'gathering' && page.status.yielded >= 1) {
    stopRequested = true;
    globalThis.__seatGatherStop = api.gather.stop();
    api.log('seat-promise-stop:' + JSON.stringify(globalThis.__seatGatherStop));
  }
}
"#;

const HOLD_SOURCE: &str = r#"
export const apiVersion = 2;
let started = false;
function chooseDest(api, here) {
  const offsets = [];
  for (let distance = 6; distance >= 2; distance--) {
    offsets.push([distance, 0], [-distance, 0], [0, distance], [0, -distance],
                 [distance, distance], [distance, -distance], [-distance, distance], [-distance, -distance]);
  }
  for (const [dx, dz] of offsets) {
    const tile = { x: here.x + dx, z: here.z + dz, level: here.level };
    const walkable = api.walkable({ tile });
    const reachable = api.canReach({ tile, adjacentOk: false, maxSteps: 24 });
    if (walkable.ok && walkable.value && reachable.ok && reachable.value) return tile;
  }
  return null;
}
export async function tick(api) {
  if (started || !api.snapshot.ingame || !api.snapshot.here) return;
  started = true;
  const here = { x: api.snapshot.here.x, z: api.snapshot.here.z, level: api.snapshot.here.level };
  const dest = chooseDest(api, here);
  if (!dest) throw new Error('no reachable hold destination in the real scene');
  const minX = here.x - 10, maxX = here.x + 10, minZ = here.z - 10, maxZ = here.z + 10;
  const box = { minX, maxX, minZ, maxZ, level: here.level };
  const calls = [];
  let hooks;
  const note = (name, receiver, payload) => calls.push({ name, receiver: receiver === hooks, payload });
  hooks = {
    inArea: function(tile) {
      const inside = tile.level === box.level && tile.x >= box.minX && tile.x <= box.maxX && tile.z >= box.minZ && tile.z <= box.maxZ;
      note('inArea', this, { tile, inside });
      return inside;
    },
    setStatus: function(message) { note('setStatus', this, { message }); },
    died: function() { note('died', this, {}); return false; },
    hpFraction: function() { note('hpFraction', this, {}); return 1; },
    panicHp: function() { note('panicHp', this, {}); return 0.1; },
    retreatHp: function() { note('retreatHp', this, {}); return 0.2; },
    hasFood: function() { note('hasFood', this, {}); return true; },
    needEat: function() { note('needEat', this, {}); return false; },
    style: function() { note('style', this, {}); return 'range'; },
    safespotIndex: function() { note('safespotIndex', this, {}); return 0; },
    buryBones: function() { note('buryBones', this, {}); return false; },
    boneName: function() { note('boneName', this, {}); return 'Bones'; },
    sustain: async function() { note('sustain', this, {}); }
  };
  const site = {
    key: 'seat-live-hold', target: 'hold', alsoHunt: [], safespots: [dest],
    meleeAnchor: here, boxes: [box], fireAtRange: false, rangedThreat: false
  };
  const begun = api.holdBegin(site);
  if (!begun.ok) throw new Error(begun.error);
  const token = begun.value.token;
  const valid = api.holdValidate({ token }, hooks);
  if (!valid.ok || !valid.value) throw new Error(valid.ok ? 'hold site validation returned false' : valid.error);
  api.log('seat-hold-start:' + JSON.stringify({ tick: api.tick, token, from: here, dest }));
  const outcome = await api.holdRun({ token }, hooks);
  const after = api.snapshot.here;
  const receipt = { token, from: here, dest, after, outcome, callbacks: calls };
  api.log('seat-hold-result:' + JSON.stringify(receipt));
}
"#;

fn start_load(live: &LiveCell, source: &str) {
    live.play
        .script_start_load(
            &live.account,
            source.into(),
            script_js::LoadShape::NativeTick,
            None,
            vec![],
        )
        .expect("start public Load v2 script");
}

fn run_promise_cell() {
    let live = start_fixture(false);
    start_load(&live, PROMISE_SOURCE);
    let mut logs = Vec::new();
    let mut live_page = Value::Null;
    let mut active_seat = None;
    let mut active_metrics = None;
    wait_until(
        &live,
        "JS Gather Promise settlement",
        Duration::from_secs(300),
        |live| {
            take_logs(live, &mut logs);
            let probe = api_probe(live);
            if probe["gather"]["phase"] == "running"
                && probe["gather"]["status"]["phase"] == "gathering"
                && probe["gather"]["status"]["yielded"].as_i64().unwrap_or(0) >= 1
            {
                live_page = probe["gather"].clone();
                active_seat = live.play.script_seat_probe(&live.account);
                active_metrics = live.play.script_slot_metrics(&live.account);
            }
            live.play.script_state(&live.account) == script::RunState::Idle
                && logs
                    .iter()
                    .any(|line| line.starts_with("seat-promise-result:"))
        },
    );
    take_logs(&live, &mut logs);
    let started = logged(&logs, "seat-promise-start:");
    let stopped = logged(&logs, "seat-promise-stop:");
    let outcome = logged(&logs, "seat-promise-result:");
    let stopped_seat = api_probe(&live);
    assert!(
        started["tick"].as_u64().unwrap_or(0) > 0,
        "Promise must start on an observed host tick"
    );
    assert_eq!(outcome["kind"], "done");
    assert_eq!(
        outcome["value"]["end"],
        "stopped",
        "a real Gather Promise must retain its product terminal after seat Done: outcome={outcome}, stop={stopped}, page={live_page}"
    );
    assert!(outcome["value"]["counts"]["yielded"].as_u64().unwrap_or(0) >= 1);
    assert_eq!(stopped, json!({"ok":true,"value":null}));
    assert!(live_page["token"].as_u64().unwrap_or(0) > 0);
    assert!(
        active_seat.is_some(),
        "the live Promise page must coexist with its slot-owned seat"
    );
    assert_eq!(
        stopped_seat["seat_present"], false,
        "Promise completion Stop must release the seat"
    );
    let metrics =
        active_metrics.expect("the host exposes production SlotMetrics for the armed seat");
    assert!(metrics.dropped >= active_seat.unwrap().dropped);
    assert!(metrics.late_controls >= active_seat.unwrap().late_controls);
    save_receipt(
        "seat-live-js-promise.json",
        &json!({
            "cell":"seat_live_js_promise",
            "account":live.account,
            "host":"127.0.0.1:44594",
            "start":started,
            "active_gather_page":live_page,
            "active_seat":active_seat.map(seat_json),
            "active_slot_metrics":{"dropped":metrics.dropped,"late_controls":metrics.late_controls},
            "stop":stopped,
            "result":outcome,
            "after_stop":stopped_seat,
        }),
    );
}

fn run_hold_cell(random_events: bool, trigger_maze: bool, reconnect: bool) {
    let live = start_fixture(random_events);
    start_load(&live, HOLD_SOURCE);
    let mut logs = Vec::new();
    wait_until(&live, "holdRun start", Duration::from_secs(120), |live| {
        take_logs(live, &mut logs);
        logs.iter().any(|line| line.starts_with("seat-hold-start:"))
    });
    let started = logged(&logs, "seat-hold-start:");
    let initial_seat = seat_probe(&live);
    assert_ne!(
        live.play.script_state(&live.account),
        script::RunState::Error
    );

    if trigger_maze {
        live.fixture
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .trigger = Some(FixtureTrigger::Maze);
        wait_until(
            &live,
            "real Maze trigger send",
            Duration::from_secs(30),
            |live| {
                live.fixture
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .trigger_sent
            },
        );
    }
    if reconnect {
        live.fixture
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .trigger = Some(FixtureTrigger::LostConnection);
        wait_until(
            &live,
            "real lost_con edge",
            Duration::from_secs(30),
            |live| {
                live.fixture
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .trigger_sent
            },
        );
    }

    let mut held_seat = None;
    let mut held_state = None;
    let mut maze_status = Value::Null;
    if trigger_maze {
        // Sample the seat only while the guardian is the one hold reason
        // and the scene is ready: the Maze teleport also reloads the scene,
        // and since round 9 a scene load is a held tick that does not hold
        // the seat (`SlotScript::seat_host_hold`).
        wait_until(
            &live,
            "Maze SessionHeld seat hold",
            Duration::from_secs(90),
            |live| {
                let status = live
                    .play
                    .statuses()
                    .into_iter()
                    .find(|status| status.username == live.account)
                    .expect("slot status");
                if status.random.kind == Some(api::random::RandomKind::Maze) && status.random.hold {
                    let inside_maze =
                        api::random::trapped_area(status.tile_x, status.tile_z, status.tile_level)
                            == Some(api::random::RandomKind::Maze);
                    let reasons = live
                        .play
                        .arm(&live.account)
                        .map(|arm| arm.session_hold_reasons())
                        .unwrap_or_default();
                    let guardian_only = reasons == [api::lifecycle::HoldReason::Guardian];
                    let probe = live.play.script_seat_probe(&live.account);
                    if let Some(probe) = probe {
                        maze_status = json!({
                            "kind": format!("{:?}", status.random.kind),
                            "name": status.random.name,
                            "ours": status.random.ours,
                            "handling": status.random.handling,
                            "hold": status.random.hold,
                            "claim": format!("{:?}", status.random.claim),
                            "tile": [status.tile_x, status.tile_z, status.tile_level],
                            "inside_maze": inside_maze,
                            "client_up": status.is_up(),
                            "ingame": status.ingame,
                            "scene_state": status.scene_state,
                            "hold_reasons": format!("{reasons:?}"),
                            "seat": seat_json(probe),
                        });
                        if inside_maze && guardian_only && status.is_up() && probe.held {
                            held_seat = Some(probe);
                            held_state = Some(live.play.script_state(&live.account));
                        }
                    }
                }
                held_seat.is_some()
            },
        );
        // A guardian hold changes seat custody, not the script lifecycle
        // (`SlotScript::seat_guardian_hold`), and the host ticks a held
        // Running isolate for paint. The sample is taken with the scene
        // ready, so the script is Running.
        let state = held_state.expect("lifecycle sampled with the held seat");
        maze_status["run_state"] = json!(format!("{state:?}"));
        assert_eq!(
            state,
            script::RunState::Running,
            "a guardian hold with the scene ready keeps the script Running"
        );
        assert_eq!(maze_status["scene_state"], 2, "the scene is ready");
        assert_eq!(maze_status["hold_reasons"], "[Guardian]");
        let held = held_seat.expect("observed held seat");
        assert!(held.held, "the guardian holds the seat");
        assert_eq!(
            held.live, initial_seat.live,
            "the held operation keeps its live seat entry"
        );
        assert_eq!(
            held.results, initial_seat.results,
            "the hold publishes no terminal result"
        );
        assert_eq!(maze_status["inside_maze"], true);
        live.play
            .script_api_live_queue_held_gameplay(&live.account)
            .expect("queue one real isolate gameplay row during guardian hold");
        wait_until(
            &live,
            "held gameplay drop",
            Duration::from_secs(60),
            |live| {
                let probe = seat_probe(live);
                probe.held && probe.dropped > initial_seat.dropped
            },
        );
        let dropped_seat = seat_probe(&live);
        assert!(dropped_seat.held && dropped_seat.dropped > initial_seat.dropped);
        maze_status["seat"] = seat_json(dropped_seat);
        held_seat = Some(dropped_seat);
    }

    if reconnect {
        wait_until(
            &live,
            "Engine A reconnect",
            Duration::from_secs(420),
            |live| {
                live.fixture
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .reconnected
            },
        );
    }

    let timeout = if trigger_maze {
        Duration::from_secs(420)
    } else {
        Duration::from_secs(240)
    };
    wait_until(&live, "holdRun post-resume result", timeout, |live| {
        take_logs(live, &mut logs);
        logs.iter()
            .any(|line| line.starts_with("seat-hold-result:"))
    });
    take_logs(&live, &mut logs);
    let result = logged(&logs, "seat-hold-result:");
    let after_control = live.play.script_seat_probe(&live.account);
    assert_eq!(result["outcome"]["kind"], "done");
    assert_eq!(
        result["after"], result["dest"],
        "the real seat-held operation must produce its destination after resume"
    );
    assert!(
        result["callbacks"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty() && rows.iter().all(|row| row["receiver"] == true)),
        "Rust-invoked callbacks must retain their hooks receiver: {result}"
    );
    assert_eq!(
        live.play.script_state(&live.account),
        script::RunState::Running
    );
    assert!(
        after_control.is_some(),
        "seat must remain live until the explicit non-kept Stop"
    );
    let api = api_probe(&live);
    live.play.script_stop(&live.account);
    wait_until(
        &live,
        "explicit non-kept Stop",
        Duration::from_secs(30),
        |live| live.play.script_state(&live.account) == script::RunState::Idle,
    );
    assert!(live.play.script_seat_probe(&live.account).is_none());
    assert_eq!(api_probe(&live)["seat_present"], false);

    let receipt = json!({
        "cell": if trigger_maze { "seat_live_guardian_hold" } else if reconnect { "seat_live_kept_reconnect" } else { "seat_live_hook_callbacks" },
        "account": live.account,
        "host":"127.0.0.1:44594",
        "start": started,
        "initial_seat": seat_json(initial_seat),
        "guardian_during_hold": if trigger_maze { maze_status } else { Value::Null },
        "seat_during_hold": held_seat.map(seat_json),
        "result": result,
        "seat_after_control": after_control.map(seat_json),
        "api_before_stop": api,
        "seat_after_stop": api_probe(&live),
        "reconnected": reconnect && live.fixture.lock().unwrap().reconnected,
        "non_kept_stop":"explicit Play::script_stop removed Load and gameplay seats",
    });
    let name = if trigger_maze {
        "seat-live-guardian-hold.json"
    } else if reconnect {
        "seat-live-kept-reconnect.json"
    } else {
        "seat-live-hook-callbacks.json"
    };
    save_receipt(name, &receipt);
}

fn run_native_cell() {
    let live = start_fixture(false);
    let settings: Map<String, Value> =
        serde_json::from_str(GATHER_SETTINGS_JSON).expect("Gatherer settings");
    live.play
        .script_start(&live.account, script::CompiledId("Gatherer"), settings)
        .expect("start compiled Gatherer card");
    let mut yielded = 0;
    let mut phases = Vec::new();
    let mut active_seat = None;
    wait_until(
        &live,
        "compiled Gatherer seat action",
        Duration::from_secs(300),
        |live| {
            if let Some(status) = live.play.script_native_status(&live.account) {
                yielded = yielded.max(status_field(&status, "yielded"));
                if let Some(phase) = status_text(&status, "phase") {
                    if phases.last() != Some(&phase) {
                        phases.push(phase);
                    }
                }
                if yielded > 0 {
                    active_seat = live.play.script_seat_probe(&live.account);
                }
            }
            yielded > 0 && active_seat.is_some()
        },
    );
    let status = live
        .play
        .script_native_status(&live.account)
        .expect("compiled card status");
    let seat = active_seat.expect("compiled card gameplay seat");
    assert_eq!(status.card, script::CompiledId("Gatherer"));
    assert!(
        yielded > 0,
        "native card must produce a real gathering result"
    );
    assert!(
        seat.live > 0 || seat.results > 0,
        "compiled action must be visible in the seat"
    );
    assert!(
        seat.reserved_releases <= 1,
        "the first completed native walk must free its seat release slot before the return walk"
    );
    let before_stop = seat_json(seat);
    live.play.script_stop(&live.account);
    wait_until(
        &live,
        "compiled card Stop",
        Duration::from_secs(30),
        |live| live.play.script_state(&live.account) == script::RunState::Idle,
    );
    assert!(live.play.script_seat_probe(&live.account).is_none());
    save_receipt(
        "seat-live-native-card.json",
        &json!({
            "cell":"seat_live_native_card",
            "account":live.account,
            "host":"127.0.0.1:44594",
            "card":format!("{:?}", status.card),
            "phase":status_text(&status, "phase"),
            "phases":phases,
            "yielded":yielded,
            "seat_during_run":before_stop,
            "seat_after_stop":null,
        }),
    );
}

fn run_world_switch_restart_cell() {
    let live = start_fixture(false);
    start_load(&live, HOLD_SOURCE);
    let mut logs = Vec::new();
    wait_until(
        &live,
        "pre-switch admitted hold Session",
        Duration::from_secs(120),
        |live| {
            take_logs(live, &mut logs);
            logs.iter().any(|line| line.starts_with("seat-hold-start:"))
                && live
                    .play
                    .script_seat_probe(&live.account)
                    .is_some_and(|seat| seat.live > 0)
        },
    );
    let before = seat_probe(&live);
    let first_generation = live
        .play
        .script_runtime_generation(&live.account)
        .expect("active first Load generation");
    let arm = live.play.arm(&live.account).expect("real owner arm");
    let first_session = arm.session_key().expect("first real session");

    // Local Engine A has one fixed endpoint. SwitchWorld(1) exercises the
    // deliberate owner turnover and the seat's non-kept session reset.
    arm.switch_world(1);
    live.play.wake(&live.account);
    wait_until(
        &live,
        "world-switch turnover",
        Duration::from_secs(90),
        |live| {
            arm.session_key()
                .is_some_and(|session| session != first_session)
                && live.play.statuses().into_iter().any(|status| {
                    status.username == live.account
                        && status.connected
                        && status.ingame
                        && status.scene_state == 2
                })
        },
    );
    let second_session = arm.session_key().expect("replacement session");
    let reset = seat_probe(&live);
    assert_eq!(
        reset.live, 0,
        "old Session survived deliberate world turnover"
    );
    assert_eq!(reset.results, 0, "old result survived non-kept reset");
    assert_eq!(
        reset.reserved_releases, 0,
        "old release crossed the session fence"
    );

    live.play.script_stop(&live.account);
    wait_until(
        &live,
        "first instance stopped",
        Duration::from_secs(30),
        |live| live.play.script_state(&live.account) == script::RunState::Idle,
    );
    assert!(live.play.script_seat_probe(&live.account).is_none());
    // Idle is the product's signal that the first isolate retired: the slot
    // stages that isolate's final log lines, including the reset terminal of
    // its hold operation, before it reports Idle (`SlotScript::
    // observe_load_lifecycle`). The log read does not block, though: while
    // the slot pump holds the slot lock it returns nothing and keeps the
    // lines for a later read (`Play::script_take_pending_logs`). Old lines
    // can therefore arrive together with the fresh instance's lines, but
    // always before them. The fresh instance logs exactly one start line;
    // its result is the result after that line.
    let mut old_run_logs = Vec::new();
    take_logs(&live, &mut old_run_logs);
    let old_lines_at_idle = old_run_logs.len();

    start_load(&live, HOLD_SOURCE);
    let second_generation = live
        .play
        .script_runtime_generation(&live.account)
        .expect("fresh Load generation");
    assert_ne!(first_generation, second_generation);
    let mut restarted_logs = Vec::new();
    let fresh_start = |logs: &[String]| {
        logs.iter()
            .position(|line| line.starts_with("seat-hold-start:"))
    };
    wait_until(
        &live,
        "post-switch fresh hold result",
        Duration::from_secs(180),
        |live| {
            take_logs(live, &mut restarted_logs);
            fresh_start(&restarted_logs).is_some_and(|start| {
                restarted_logs[start..]
                    .iter()
                    .any(|line| line.starts_with("seat-hold-result:"))
            })
        },
    );
    let start = fresh_start(&restarted_logs).expect("fresh start line");
    let old_lines_after_idle: Vec<String> = restarted_logs.drain(..start).collect();
    old_run_logs.extend(old_lines_after_idle.iter().cloned());
    let restarted = logged(&restarted_logs, "seat-hold-result:");
    assert_eq!(restarted["outcome"]["kind"], "done");
    assert_eq!(restarted["after"], restarted["dest"]);
    let fresh = seat_probe(&live);
    live.play.script_stop(&live.account);
    wait_until(
        &live,
        "second instance stopped",
        Duration::from_secs(30),
        |live| live.play.script_state(&live.account) == script::RunState::Idle,
    );
    assert!(live.play.script_seat_probe(&live.account).is_none());
    save_receipt(
        "seat-live-world-switch-restart.json",
        &json!({
            "cell":"seat_live_world_switch_restart",
            "account":live.account,
            "host":"127.0.0.1:44594",
            "sessions":[format!("{first_session:?}"),format!("{second_session:?}")],
            "generations":[first_generation, second_generation],
            "seat_before_world_switch":seat_json(before),
            "seat_after_nonkept_reset":seat_json(reset),
            "fresh_result":restarted,
            "fresh_seat":seat_json(fresh),
            "seat_after_second_stop":null,
            "old_generation_lines_at_idle":old_lines_at_idle,
            "old_generation_lines_after_idle":old_lines_after_idle,
            "old_generation_logs":old_run_logs,
        }),
    );
}

#[test]
#[ignore = "requires LIVE=1, Engine A 127.0.0.1:44594, disposable HOME/cache, and real R289 assets"]
fn seat_live_js_promise() {
    run_promise_cell();
}

#[test]
#[ignore = "requires LIVE=1, Engine A 127.0.0.1:44594, disposable HOME/cache, and real R289 assets"]
fn seat_live_hook_callbacks() {
    run_hold_cell(false, false, false);
}

#[test]
#[ignore = "requires LIVE=1, Engine A 127.0.0.1:44594, disposable HOME/cache, and real R289 assets"]
fn seat_live_native_card() {
    run_native_cell();
}

#[test]
#[ignore = "requires LIVE=1, Engine A 127.0.0.1:44594, disposable HOME/cache, and real R289 assets"]
fn seat_live_guardian_hold() {
    run_hold_cell(true, true, false);
}

#[test]
#[ignore = "requires LIVE=1, Engine A 127.0.0.1:44594, disposable HOME/cache, and real R289 assets"]
fn seat_live_kept_reconnect_and_stop() {
    run_hold_cell(false, false, true);
}

#[test]
#[ignore = "requires LIVE=1, Engine A 127.0.0.1:44594, disposable HOME/cache, and real R289 assets"]
fn seat_live_world_switch_restart() {
    run_world_switch_restart_cell();
}
