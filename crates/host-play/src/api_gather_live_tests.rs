#![cfg(feature = "memory-profile")]

use super::*;
use api::snapshot::WorldTile;
use script::api_gather::{GatherPage, GatherPhase};
use script::isolate_fb::{IsolateBuf, NativeFactsInput};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use vault::{Profile, ProfileSettings};

const LIVE_SOURCE: &str = r#"
const channel = new BroadcastChannel(__GATHER_CHANNEL__);
export const apiVersion = 2;
export function tick(api) {
  const page = api.snapshot.gather;
  globalThis.__page = page;
  const phase = page ? `${page.phase}:${page.status ? page.status.phase : 'no-status'}` : 'idle';
  if (phase !== globalThis.__lastPhase) {
    globalThis.__lastPhase = phase;
    (globalThis.__phases ||= []).push(phase);
  }
  if (globalThis.__start && !globalThis.__run) {
    globalThis.__run = api.gather.run({ skill: 'Woodcutting', woodcuttingResources: ['normal'], disposition: 'Power', location: 'Custom', customTile: globalThis.__tile });
    api.request({ op: 'held', name: 'Logs', action: 'Drop' });
    channel.postMessage({ proof: 'run' });
    globalThis.__run.then(value => { globalThis.__result = value; });
  }
  if (page && page.status && !globalThis.__liveHeldSent) {
    globalThis.__liveHeldSent = true;
    api.request({ op: 'held', name: 'Logs', action: 'Drop' });
    channel.postMessage({ proof: 'live' });
  }
  if (globalThis.__stop && api.snapshot.gather && !globalThis.__stopped) {
    globalThis.__stopped = true;
    globalThis.__stopResult = api.gather.stop();
    api.request({ op: 'held', name: 'Logs', action: 'Drop' });
    channel.postMessage({ proof: 'stop' });
  }
}
"#;

const CHANNEL_RECEIVER_SOURCE: &str = r#"
const channel = new BroadcastChannel(__GATHER_CHANNEL__);
globalThis.__receivedMessages = [];
channel.onmessage = event => globalThis.__receivedMessages.push(event.data);
export function tick(_api) {}
"#;

#[derive(Default)]
struct AdmissionLog {
    rows: Mutex<Vec<(String, String)>>,
}

impl api::hostlog::Sink for AdmissionLog {
    fn record(&self, record: &api::hostlog::Record<'_>) {
        if record.message.starts_with("gather foreground: dropped ") {
            self.rows.lock().expect("admission log lock").push((
                record.slot.unwrap_or("").to_owned(),
                record.message.to_owned(),
            ));
        }
    }
}

static ADMISSION_LOG: AdmissionLog = AdmissionLog {
    rows: Mutex::new(Vec::new()),
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SeedPhase {
    WaitIngame,
    SkipTutorial,
    QueryTutorial,
    WaitTutorial,
    Logout,
    WaitRelog,
    SeedWoodcutting,
    SeedAttack,
    SeedDefence,
    SeedAxe,
    SeedHitpoints,
    SeedTeleport,
    WaitSeed,
    Ready,
}

struct LiveSetup {
    phase: SeedPhase,
    error: Option<String>,
    disconnect_requested: bool,
    disconnected: bool,
    reconnected: bool,
}

impl Default for LiveSetup {
    fn default() -> Self {
        Self {
            phase: SeedPhase::WaitIngame,
            error: None,
            disconnect_requested: false,
            disconnected: false,
            reconnected: false,
        }
    }
}

struct ThrowawayHome {
    path: PathBuf,
    previous: Option<std::ffi::OsString>,
}

impl ThrowawayHome {
    fn enter(label: &str, evidence_dir: &Path) -> Self {
        let previous = std::env::var_os("HOME");
        let path = evidence_dir.join(format!(
            "274bot-api-gather-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |value| value.as_nanos())
        ));
        std::fs::create_dir_all(&path).expect("create throwaway HOME");
        std::env::set_var("HOME", &path);
        Self { path, previous }
    }
}

impl Drop for ThrowawayHome {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            std::env::set_var("HOME", previous);
        } else {
            std::env::remove_var("HOME");
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn live_options(home: &Path) -> ProfileOptions {
    let port = std::env::var("BOT_GAME_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(45594);
    let http_port = std::env::var("BOT_HTTP_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(2080);
    let nav_pack = PathBuf::from(
        std::env::var_os("GATHERER_NAV_PACK")
            .expect("Gather API live proof requires GATHERER_NAV_PACK"),
    );
    let engine_dir = PathBuf::from(
        std::env::var_os("GATHERER_ENGINE_DIR")
            .expect("Gather API live proof requires GATHERER_ENGINE_DIR"),
    );
    assert!(
        nav_pack.is_absolute() && nav_pack.is_file(),
        "GATHERER_NAV_PACK must be an absolute file path: {}",
        nav_pack.display()
    );
    assert!(
        engine_dir.is_absolute() && engine_dir.is_dir(),
        "GATHERER_ENGINE_DIR must be an absolute directory path: {}",
        engine_dir.display()
    );
    ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(port),
        http_port: Some(http_port),
        vault_path: Some(home.join("vault")),
        cache_dir: Some(PathBuf::from(
            std::env::var_os("BOT_CACHE_DIR").expect("live proof requires copied BOT_CACHE_DIR"),
        )),
        unpack_dir: Some(home.join("unpack")),
        nav_pack: Some(nav_pack),
        engine_dir: Some(engine_dir),
        ..ProfileOptions::default()
    }
}

fn tile_from_env() -> WorldTile {
    let value = std::env::var("GATHERER_WC_TILE")
        .expect("Gather API live proof requires GATHERER_WC_TILE=x,z,level");
    let mut parts = value.split(',');
    let parse = |part: Option<&str>| {
        part.and_then(|part| part.parse::<i32>().ok())
            .expect("GATHERER_WC_TILE must be x,z,level")
    };
    let tile = WorldTile {
        x: parse(parts.next()),
        z: parse(parts.next()),
        level: parse(parts.next()),
    };
    assert!(
        parts.next().is_none(),
        "GATHERER_WC_TILE must have exactly three values"
    );
    tile
}

fn send_cheat(client: &mut client::client::Client, command: &str) -> Result<(), String> {
    if api::interact::cheat(client, command).is_sent() {
        Ok(())
    } else {
        Err(format!("fixture command refused: {command}"))
    }
}

fn live_frame(
    client: &mut client::client::Client,
    username: &str,
    account: &str,
    tile: WorldTile,
    shared: &Arc<Mutex<LiveSetup>>,
) {
    if username != account {
        return;
    }
    let Ok(mut state) = shared.lock() else {
        return;
    };
    if state.disconnect_requested && !state.disconnected {
        client.lost_con();
        state.disconnected = true;
        return;
    }
    if state.disconnected && client.ingame && client.scene_state == 2 {
        state.reconnected = true;
    }
    if state.phase == SeedPhase::Ready || state.error.is_some() {
        return;
    }
    let mut snapshot = api::snapshot::GameSnapshot::new();
    snapshot.rebuild(client);
    let result = (|| -> Result<(), String> {
        match state.phase {
            SeedPhase::WaitIngame => {
                if snapshot.ingame() && snapshot.scene_state() == 2 {
                    state.phase = SeedPhase::SkipTutorial;
                }
                Ok(())
            }
            SeedPhase::SkipTutorial => {
                send_cheat(client, "setvar tutorial 1000")?;
                state.phase = SeedPhase::QueryTutorial;
                Ok(())
            }
            SeedPhase::QueryTutorial => {
                send_cheat(client, "getvar tutorial")?;
                state.phase = SeedPhase::WaitTutorial;
                Ok(())
            }
            SeedPhase::WaitTutorial => {
                if snapshot.chat_lines().iter().any(|line| {
                    line.text
                        .to_ascii_lowercase()
                        .contains("get tutorial: 1000")
                }) || snapshot
                    .chat_modal_texts()
                    .iter()
                    .any(|text| text.to_ascii_lowercase().contains("get tutorial: 1000"))
                {
                    state.phase = SeedPhase::Logout;
                }
                Ok(())
            }
            SeedPhase::Logout => {
                let ifaces = Arc::clone(&client.ifaces);
                if api::interact::logout(client, &ifaces) {
                    state.phase = SeedPhase::WaitRelog;
                    Ok(())
                } else {
                    Err("logout interface unavailable during Gather API setup".into())
                }
            }
            SeedPhase::WaitRelog => {
                if snapshot.ingame()
                    && snapshot.scene_state() == 2
                    && snapshot
                        .side_tabs()
                        .iter()
                        .any(|tab| tab.index == 3 && tab.available)
                {
                    state.phase = SeedPhase::SeedWoodcutting;
                }
                Ok(())
            }
            SeedPhase::SeedWoodcutting => {
                send_cheat(client, "setstat woodcutting 1")?;
                state.phase = SeedPhase::SeedAttack;
                Ok(())
            }
            SeedPhase::SeedAttack => {
                send_cheat(client, "setstat attack 5")?;
                state.phase = SeedPhase::SeedDefence;
                Ok(())
            }
            SeedPhase::SeedDefence => {
                send_cheat(client, "setstat defence 99")?;
                state.phase = SeedPhase::SeedAxe;
                Ok(())
            }
            SeedPhase::SeedAxe => {
                send_cheat(client, "give bronze_axe 1")?;
                state.phase = SeedPhase::SeedHitpoints;
                Ok(())
            }
            SeedPhase::SeedHitpoints => {
                send_cheat(client, "setstat hitpoints 99")?;
                state.phase = SeedPhase::SeedTeleport;
                Ok(())
            }
            SeedPhase::SeedTeleport => {
                send_cheat(
                    client,
                    &api::interact::tele_args(tile.level, tile.x, tile.z),
                )?;
                state.phase = SeedPhase::WaitSeed;
                Ok(())
            }
            SeedPhase::WaitSeed => {
                let on_tile = snapshot.tile().is_some_and(|(x, z, level)| {
                    level == tile.level && x.abs_diff(tile.x) <= 6 && z.abs_diff(tile.z) <= 6
                });
                let has_axe = snapshot
                    .inventory()
                    .iter()
                    .any(|item| item.def.id == 1351 && item.count > 0);
                let has_skill = snapshot
                    .stats()
                    .iter()
                    .any(|stat| stat.name.eq_ignore_ascii_case("woodcutting") && stat.base >= 1);
                if snapshot.ingame()
                    && snapshot.scene_state() == 2
                    && on_tile
                    && has_axe
                    && has_skill
                {
                    state.phase = SeedPhase::Ready;
                }
                Ok(())
            }
            SeedPhase::Ready => Ok(()),
        }
    })();
    if let Err(error) = result {
        state.error = Some(error);
    }
}

fn wait_until(label: &str, timeout: Duration, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    loop {
        if ready() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{label} timed out after {timeout:?}"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

fn slot_probe(play: &Play, name: &str, expression: &str) -> serde_json::Value {
    script_slot(&play.scripts, name)
        .expect("live script slot")
        .lock()
        .expect("live script slot lock")
        .probe(expression)
        .expect("live Load probe")
}

fn run_session(session: &serde_json::Value) -> u64 {
    session["token"].as_u64().expect("Gather API token")
}

fn run_session_epoch(play: &Play, name: &str) -> u64 {
    script_slot(&play.scripts, name)
        .expect("live script slot")
        .lock()
        .expect("live script slot lock")
        .native_status()
        .expect("Gatherer status during live API session")
        .run
        .session
}

fn wire_shadow_lengths(
    token: u64,
    status: Arc<script::native::ScriptStatus>,
    tile: WorldTile,
) -> (usize, usize) {
    let page = GatherPage {
        token,
        phase: GatherPhase::Running,
        status: Some(status),
    };
    let mut encoder = IsolateBuf::new();
    let (changed_bytes, fingerprint) = with_script_snapshot_input(
        1,
        Some((tile.x, tile.z, tile.level)),
        true,
        None,
        None,
        None,
        None,
        None,
        false,
        false,
        false,
        0,
        false,
        0,
        false,
        0,
        false,
        None,
        PostedWalkOutcome::default(),
        PostedInspect::default(),
        |input, native| {
            let native = NativeFactsInput {
                api_gather: Some(&page),
                ..native
            };
            let (bytes, fingerprint) =
                encoder.encode_snapshot_delta_with_native(None, input, native, false);
            (bytes.len(), fingerprint)
        },
    );
    let unchanged_bytes = with_script_snapshot_input(
        2,
        Some((tile.x, tile.z, tile.level)),
        true,
        None,
        None,
        None,
        None,
        None,
        false,
        false,
        false,
        0,
        false,
        0,
        false,
        0,
        false,
        None,
        PostedWalkOutcome::default(),
        PostedInspect::default(),
        |input, native| {
            let native = NativeFactsInput {
                api_gather: Some(&page),
                ..native
            };
            let (bytes, _) =
                encoder.encode_snapshot_delta_with_native(Some(&fingerprint), input, native, false);
            bytes.len()
        },
    );
    (changed_bytes, unchanged_bytes)
}

#[test]
#[ignore = "requires LIVE=1 and local R289 Gatherer fixtures"]
fn live_gather_api_receipt_tracks_memory_reconnect_and_wire_shadow() {
    use script::shim::InteractReq;
    use script_channels::BrokerWorld;
    assert!(
        std::env::var("LIVE").is_ok_and(|value| value == "1"),
        "run this ignored receipt only with LIVE=1"
    );
    let tile = tile_from_env();
    let evidence_dir = PathBuf::from(
        std::env::var_os("BOT_EVIDENCE_DIR").expect("LIVE receipt requires BOT_EVIDENCE_DIR"),
    );
    assert!(
        evidence_dir.is_absolute(),
        "BOT_EVIDENCE_DIR must be absolute"
    );
    std::fs::create_dir_all(&evidence_dir).expect("create receipt directory");
    assert!(
        api::hostlog::install_sink(&ADMISSION_LOG),
        "run this ignored live receipt in its own test process"
    );
    api::hostlog::set_debug(true);
    let home = ThrowawayHome::enter("receipt", &evidence_dir);
    let options = live_options(&home.path);
    let template = options
        .resolve(None)
        .expect("resolve local-289 profile")
        .prepare_template()
        .expect("prepare cold local-289 template");
    let names = play_bootstrap::mint_live_names(2);
    let credentials = mint_live_entries(&names);
    let account = names.first().expect("minted Gather account").clone();
    let receiver_account = names.get(1).expect("minted channel receiver").clone();
    let channel_name = format!("rs2b0t:kq:v1:{account},{receiver_account},carol,dave");
    let channel_literal =
        serde_json::to_string(&channel_name).expect("encode BroadcastChannel name");
    let live_source = LIVE_SOURCE.replace("__GATHER_CHANNEL__", &channel_literal);
    let receiver_source = CHANNEL_RECEIVER_SOURCE.replace("__GATHER_CHANNEL__", &channel_literal);
    let setup = Arc::new(Mutex::new(LiveSetup::default()));
    let frame_setup = Arc::clone(&setup);
    let frame_account = account.clone();
    let mut play = run_with_template(
        template,
        true,
        vec![],
        |_| (None, None),
        move |client, username, _hold| {
            live_frame(client, username, &frame_account, tile, &frame_setup);
        },
    )
    .expect("start live Play");
    for (index, peer_name) in ["carol", "dave"].into_iter().enumerate() {
        let peer = play.channels.slot(peer_name);
        let _ = peer.pump(
            index as u64 + 1,
            BrokerWorld::Local,
            true,
            vec![InteractReq::ChannelOpen {
                channel_id: index as u64 + 1,
                name: channel_name.clone(),
            }],
        );
    }
    for (index, (username, password)) in credentials.iter().enumerate() {
        play.try_spawn_slot(
            Profile {
                username: username.clone(),
                password: password.clone().into(),
                uid: 274_279_003 + index as i32,
                settings: ProfileSettings::default(),
            },
            None,
            None,
            None,
        )
        .expect("spawn minted live account");
    }

    wait_until(
        "Gather API fixture readiness",
        Duration::from_secs(240),
        || {
            let (error, ready) = {
                let state = setup.lock().expect("live setup lock");
                (state.error.clone(), state.phase == SeedPhase::Ready)
            };
            assert!(
                error.is_none(),
                "Gather API fixture setup failed: {error:?}"
            );
            let statuses = play.statuses();
            ready
                && [&account, &receiver_account].into_iter().all(|name| {
                    statuses.iter().any(|status| {
                        status.username.as_str() == name.as_str()
                            && status.ingame
                            && status.scene_state == 2
                    })
                })
        },
    );

    play.script_start_load(
        &receiver_account,
        receiver_source,
        script::LoadShape::NativeTick,
        None,
        vec![],
    )
    .expect("start real BroadcastChannel receiver Load script");
    wait_until(
        "BroadcastChannel receiver Load script Running",
        Duration::from_secs(30),
        || play.script_state(&receiver_account) == script::RunState::Running,
    );
    play.script_start_load(
        &account,
        live_source,
        script::LoadShape::NativeTick,
        None,
        vec![],
    )
    .expect("start public API v2 Load script");
    wait_until("Load script Running", Duration::from_secs(30), || {
        play.script_state(&account) == script::RunState::Running
    });
    script_slot(&play.scripts, &account)
        .expect("live script slot")
        .lock()
        .expect("live script slot lock")
        .probe(&format!(
            "globalThis.__tile = {{x:{}, z:{}, level:{}}}",
            tile.x, tile.z, tile.level
        ))
        .expect("configure selected tree tile");
    let metrics_before = play
        .memory_script_metrics(&account)
        .expect("real Play memory metrics before API session");
    slot_probe(&play, &account, "globalThis.__start = true");
    wait_until(
        "Gather API installed with a published status",
        Duration::from_secs(180),
        || {
            let page = slot_probe(&play, &account, "globalThis.__page");
            page["phase"] == "running"
                && page["status"]["phase"] == "gathering"
                && page["status"]["yielded"].as_i64().is_some_and(|n| n > 0)
        },
    );
    let live_page = slot_probe(&play, &account, "globalThis.__page");
    let token = run_session(&live_page);
    let session_before = run_session_epoch(&play, &account);
    assert_ne!(token, 0, "live API session publishes a nonzero token");
    let metrics_during = play
        .memory_script_metrics(&account)
        .expect("real Play memory metrics during API session");
    let status = script_slot(&play.scripts, &account)
        .expect("live script slot")
        .lock()
        .expect("live script slot lock")
        .native_status()
        .expect("live Gatherer status")
        .clone();
    let (changed_bytes, unchanged_bytes) = wire_shadow_lengths(token, status, tile);
    let expected_during_seat = serde_json::json!([
        {"proof": "run"},
        {"proof": "live"}
    ]);
    wait_until(
        "run and live BroadcastChannel posts reach the real receiver while Gather owns foreground",
        Duration::from_secs(30),
        || {
            slot_probe(&play, &receiver_account, "globalThis.__receivedMessages")
                == expected_during_seat
        },
    );
    let received_during_seat =
        slot_probe(&play, &receiver_account, "globalThis.__receivedMessages");
    let seat_owned_when_posts_arrived = script_slot(&play.scripts, &account)
        .expect("Gather sender script slot")
        .lock()
        .expect("Gather sender slot lock")
        .api_owns_foreground();
    assert!(
        seat_owned_when_posts_arrived,
        "the real receiver saw run/live posts while Gather owned the sender slot"
    );
    assert_eq!(
        received_during_seat, expected_during_seat,
        "the real receiver got exactly the run/live posts during the owned seat"
    );

    {
        let mut state = setup.lock().expect("live setup lock");
        state.disconnect_requested = true;
    }
    wait_until(
        "unexpected reconnect keeps the Gather API token",
        Duration::from_secs(300),
        || {
            if !setup.lock().expect("live setup lock").reconnected {
                return false;
            }
            let page = slot_probe(&play, &account, "globalThis.__page");
            page["token"].as_u64() == Some(token)
                && page["phase"] == "running"
                && run_session_epoch(&play, &account) != session_before
        },
    );
    let reconnect_page = slot_probe(&play, &account, "globalThis.__page");
    let session_after = run_session_epoch(&play, &account);
    let metrics_reconnected = play
        .memory_script_metrics(&account)
        .expect("real Play memory metrics after reconnect");

    slot_probe(&play, &account, "globalThis.__stop = true");
    wait_until("Gather API Stop terminal", Duration::from_secs(30), || {
        let page = slot_probe(&play, &account, "globalThis.__page");
        page.is_null()
            && slot_probe(&play, &account, "globalThis.__result")["value"]["end"] == "stopped"
    });
    let metrics_after = play
        .memory_script_metrics(&account)
        .expect("real Play memory metrics after API Stop");
    let expected_channel_messages = serde_json::json!([
        {"proof": "run"},
        {"proof": "live"},
        {"proof": "stop"}
    ]);
    wait_until(
        "run/live/stop BroadcastChannel posts reach the real receiver",
        Duration::from_secs(30),
        || {
            slot_probe(&play, &receiver_account, "globalThis.__receivedMessages")
                == expected_channel_messages
        },
    );
    let received_after_stop = slot_probe(&play, &receiver_account, "globalThis.__receivedMessages");
    assert_eq!(
        received_after_stop, expected_channel_messages,
        "real Load receiver observed exactly the BroadcastChannel posts from run/live/stop"
    );
    let terminal = slot_probe(&play, &account, "globalThis.__result");
    assert_eq!(terminal["kind"], "done");
    assert_eq!(terminal["value"]["token"].as_u64(), Some(token));
    assert!(
        terminal["value"]["counts"]["yielded"]
            .as_i64()
            .zip(live_page["status"]["yielded"].as_i64())
            .is_some_and(|(terminal_count, live_count)| terminal_count >= live_count),
        "Stop preserves pre-reconnect published counts: {terminal:?}"
    );
    assert_eq!(
        slot_probe(&play, &account, "globalThis.__stopResult"),
        serde_json::json!({"ok": true, "value": null})
    );
    let phases = slot_probe(&play, &account, "globalThis.__phases");
    assert!(
        phases.as_array().is_some_and(|rows| {
            rows.iter().any(|row| row == "idle")
                && rows.iter().any(|row| row == "running:gathering")
        }),
        "actual snapshot.gather status transitions: {phases:?}"
    );
    let dropped_rows: Vec<_> = ADMISSION_LOG
        .rows
        .lock()
        .expect("admission log lock")
        .iter()
        .filter(|(slot, _)| slot == &account)
        .map(|(_, message)| message.clone())
        .collect();
    assert_eq!(
        dropped_rows,
        vec![
            "gather foreground: dropped 1 script game rows",
            "gather foreground: dropped 3 script game rows total",
        ],
        "only the three emitted game rows count as dropped; broker posts are preserved"
    );
    let receipt = serde_json::json!({
        "proof": "live Gather API seat memory and reconnect receipt",
        "profile": "local-289",
        "account": account,
        "tile": {"x": tile.x, "z": tile.z, "level": tile.level},
        "settings": {"skill": "Woodcutting", "woodcuttingResources": ["normal"], "disposition": "Power"},
        "snapshot_phases": phases,
        "foreground_drop_trace": dropped_rows,
        "broadcast_channel": {
            "name": channel_name,
            "sender": account,
            "receiver": receiver_account,
            "seat_token": token,
            "received_while_seat_owned": received_during_seat,
            "received_run_live_stop": received_after_stop,
            "seat_owned_when_run_live_arrived": seat_owned_when_posts_arrived,
        },
        "driver_trace": "native-packet account/run/tick lines in the full live command log; InteractTrace is stderr-only",
        "token_before_reconnect": token,
        "token_after_reconnect": reconnect_page["token"],
        "run_session_before_reconnect": session_before,
        "run_session_after_reconnect": session_after,
        "metrics_before": metrics_before,
        "metrics_during": metrics_during,
        "metrics_after_reconnect": metrics_reconnected,
        "metrics_after_stop": metrics_after,
        "terminal": terminal,
        "wire_shadow": {
            "changed_bytes": changed_bytes,
            "unchanged_bytes": unchanged_bytes,
            "source": "IsolateBuf GatherPage diagnostic shadow; not an instrumented production packet",
            "unchanged_note": "Snapshot.tick is always encoded; the GatherPage delta is elided",
        },
    });
    let receipt_path = evidence_dir.join("api-gather-live-receipt.json");
    std::fs::write(
        &receipt_path,
        serde_json::to_vec_pretty(&receipt).expect("serialize live receipt"),
    )
    .expect("write live receipt");
    println!("api-gather-live-receipt={}", receipt_path.display());
    println!(
        "{}",
        serde_json::to_string_pretty(&receipt).expect("format receipt")
    );
}

#[test]
#[ignore = "requires LIVE=1 and the shared local R289 engine"]
fn live_quest_progress_complete_cook_through_load_api() {
    assert!(std::env::var("LIVE").is_ok_and(|value| value == "1"));
    let evidence_dir = PathBuf::from(
        std::env::var_os("BOT_EVIDENCE_DIR").expect("progress live evidence directory"),
    );
    let home = ThrowawayHome::enter("progress", &evidence_dir);
    let template = live_options(&home.path)
        .resolve(None)
        .expect("resolve local profile")
        .prepare_template()
        .expect("prepare local cached template");
    let names = play_bootstrap::mint_live_names(1);
    let account = names[0].clone();
    let credentials = mint_live_entries(&names);
    let setup = Arc::new(Mutex::new((0u8, None::<String>)));
    let frame_setup = Arc::clone(&setup);
    let frame_account = account.clone();
    let mut play = run_with_template(
        template,
        false,
        vec![],
        |_| (None, None),
        move |client, username, frame_input| {
            if username != frame_account {
                return;
            }
            let mut state = frame_setup.lock().expect("progress setup lock");
            if state.1.is_some() || state.0 == 7 {
                return;
            }
            let mut snapshot = api::snapshot::GameSnapshot::new();
            snapshot.rebuild(client);
            if frame_input.hold {
                return;
            }
            let result = (|| -> Result<(), String> {
                match state.0 {
                    0 if client.ingame && client.scene_state == 2 => {
                        // Own tutorial setup so the automatic mainland hop cannot race logout.
                        api::interact::mainland_hop(client);
                        state.0 = 1;
                    }
                    1 => {
                        send_cheat(client, "getvar tutorial")?;
                        state.0 = 2;
                    }
                    2 if snapshot.chat_lines().iter().any(|line| {
                        line.text
                            .to_ascii_lowercase()
                            .contains("get tutorial: 1000")
                    }) || snapshot
                        .chat_modal_texts()
                        .iter()
                        .any(|line| line.to_ascii_lowercase().contains("get tutorial: 1000")) =>
                    {
                        let ifaces = Arc::clone(&client.ifaces);
                        if !api::interact::logout(client, &ifaces) {
                            return Err("tutorial logout interface unavailable".into());
                        }
                        state.0 = 3;
                    }
                    3 if client.ingame
                        && client.scene_state == 2
                        && snapshot
                            .side_tabs()
                            .iter()
                            .any(|tab| tab.index == 3 && tab.available) =>
                    {
                        // The local 289 engine's variable is cookquest, not cook.
                        send_cheat(client, "setvar cookquest 2")?;
                        state.0 = 4;
                    }
                    4 => {
                        send_cheat(client, "getvar cookquest")?;
                        state.0 = 5;
                    }
                    5 if snapshot.chat_lines().iter().any(|line| {
                        line.text.to_ascii_lowercase().contains("get cookquest: 2")
                    }) =>
                    {
                        // setvar changes the varp, not the quest-list colour. Login's
                        // update_questlist hydrates the actual server-side tab state.
                        let ifaces = Arc::clone(&client.ifaces);
                        if !api::interact::logout(client, &ifaces) {
                            return Err("Cook seed logout interface unavailable".into());
                        }
                        state.0 = 6;
                    }
                    6 if snapshot.quest_statuses().iter().any(|row| {
                        row.name == "Cook's Assistant"
                            && row.status() == api::snapshot::QuestListStatus::Complete
                    }) && client.ingame
                        && client.scene_state == 2 =>
                    {
                        state.0 = 7;
                    }
                    _ => {}
                }
                Ok(())
            })();
            if let Err(error) = result {
                state.1 = Some(error);
            }
        },
    )
    .expect("start progress live Play");
    let (username, password) = &credentials[0];
    play.try_spawn_slot(
        Profile {
            username: username.clone(),
            password: password.clone().into(),
            uid: 274_279_104,
            settings: ProfileSettings::default(),
        },
        None,
        None,
        None,
    )
    .expect("spawn fresh progress account");
    wait_until("Cook complete fixture", Duration::from_secs(240), || {
        let state = setup.lock().expect("progress setup lock");
        assert!(state.1.is_none(), "fixture error: {:?}", state.1);
        state.0 == 7
    });
    play.script_start_load(
        &account,
        r#"export const apiVersion = 2;
export async function tick(api) {
  if (globalThis.__started) return;
  globalThis.__started = true;
  globalThis.__paths = api.questPaths();
  globalThis.__progress = await api.questProgress({quest:'cook'});
}"#
        .into(),
        script::LoadShape::NativeTick,
        None,
        vec![],
    )
    .expect("start progress public Load consumer");
    wait_until("progress Load Running", Duration::from_secs(30), || {
        play.script_state(&account) == script::RunState::Running
    });
    wait_until("questProgress terminal", Duration::from_secs(30), || {
        !slot_probe(&play, &account, "globalThis.__progress || null").is_null()
    });
    let paths = slot_probe(&play, &account, "globalThis.__paths");
    let terminal = slot_probe(&play, &account, "globalThis.__progress");
    assert_eq!(paths["ok"], true);
    assert_eq!(
        paths["value"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["cook", "sheep", "runemysteries", "romeojuliet"]
    );
    assert_eq!(terminal["kind"], "done");
    assert_eq!(terminal["value"]["end"], "done");
    assert_eq!(terminal["value"]["row"]["stage"]["state"], "known");
    assert_eq!(terminal["value"]["row"]["stage"]["value"], "cook:2");
    assert_eq!(terminal["value"]["row"]["complete"], "true");
    assert_eq!(terminal["value"]["row"]["journal_read"], false);
    let receipt = serde_json::json!({
        "proof": "fresh local-289 Cook complete through public Load API",
        "account": account,
        "seed": "setvar cookquest 2",
        "paths": paths,
        "terminal": terminal,
        "journal_rule_witness": "design slice D owns the released Romeo & Juliet stage-30/stage-40 live quiet-lease witness",
    });
    let path = evidence_dir.join("api-progress-live-receipt.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
    println!("api-progress-live-receipt={}\n{receipt}", path.display());
}
