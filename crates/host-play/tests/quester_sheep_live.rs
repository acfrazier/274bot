//! Live `quester_sheep` cell. Ignored unless LIVE=1.
#![cfg(feature = "live-harness")]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::snapshot::GameSnapshot;
use host::Pump;
use host_play::{ProfileOptions, ScriptStartHandle, SharedClientTemplate};
use scenario::{RunnerStatus, ScenarioRunner};
use serde_json::{json, Map, Value};
use vault::{Profile, ProfileSettings};

const POLL_INTERVAL: Duration = Duration::from_millis(20);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Result<Self, String> {
        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("clock: {error}"))?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("274bot-qs-{label}-{serial}"));
        std::fs::create_dir_all(&path)
            .map_err(|error| format!("create {}: {error}", path.display()))?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required for quester_sheep live"))
}

fn mint_profile(account: &str, password: &str, offset: i32) -> Result<Profile, String> {
    let uid = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("clock: {error}"))?
        .as_millis()
        .checked_rem(i32::MAX as u128)
        .ok_or("uid clock overflow")? as i32;
    Ok(Profile {
        username: account.to_owned(),
        password: password.to_owned().into(),
        uid: uid.saturating_add(offset),
        settings: ProfileSettings::default(),
    })
}

fn selected_profile(
    nav_pack: PathBuf,
    engine_dir: PathBuf,
    catalog_root: PathBuf,
    temp: &Path,
) -> Result<(Arc<host_play::ServerProfile>, Arc<SharedClientTemplate>), String> {
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(
            std::env::var("WORLD_GAME_PORT")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(45594),
        ),
        http_port: Some(
            std::env::var("WORLD_HTTP_PORT")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(2080),
        ),
        cache_dir: std::env::var_os("BOT_CACHE_DIR").map(PathBuf::from),
        nav_pack: Some(nav_pack),
        nav_flags: std::env::var_os("WORLD_NAV_FLAGS")
            .or_else(|| std::env::var_os("GATHERER_NAV_FLAGS"))
            .map(PathBuf::from),
        engine_dir: Some(engine_dir),
        vault_path: Some(temp.join("vault")),
        catalog_root: Some(catalog_root),
        ..ProfileOptions::default()
    };
    let profile = options.resolve(None)?.bind()?;
    if profile.profile_class() != host_play::ProfileClass::Local
        || profile.client().game_host() != "127.0.0.1"
    {
        return Err("quester_sheep live requires a loopback local profile".into());
    }
    let template = SharedClientTemplate::load(Arc::clone(&profile))?;
    if template.world().is_none() {
        return Err("selected template has no navigation world".into());
    }
    Ok((profile, template))
}

struct QuesterSheepState {
    runner: ScenarioRunner,
    account: String,
    snapshot: GameSnapshot,
    pump: Pump,
    start_handle: Option<ScriptStartHandle>,
    start_settings: Map<String, Value>,
    start_count: u32,
    error: Option<String>,
    trace_started: Instant,
    trace_last: Instant,
    last_wool: i32,
    last_balls: i32,
    total_sheared: i32,
}

impl QuesterSheepState {
    fn frame(&mut self, client: &mut client::client::Client, hold: bool) {
        let drain = self.pump.drain_client(client);
        host::publish_snapshot(&mut self.snapshot, client, drain);
        let count = |id| {
            self.snapshot
                .inventory()
                .iter()
                .filter(|item| item.def.id == id && item.count > 0)
                .map(|item| item.count)
                .sum::<i32>()
        };
        let wool = count(1737);
        let balls = count(1759);
        if wool > self.last_wool {
            self.total_sheared += wool - self.last_wool.max(0);
        }
        if wool != self.last_wool
            || balls != self.last_balls
            || self.trace_last.elapsed() >= Duration::from_secs(30)
        {
            println!(
                "{}",
                json!({
                    "phase": "wool-telemetry",
                    "account": self.account,
                    "elapsed_ms": self.trace_started.elapsed().as_millis(),
                    "wool": wool,
                    "balls_of_wool": balls,
                    "observed_successful_shears": self.total_sheared,
                    "tile": format!("{:?}", self.snapshot.tile()),
                })
            );
            self.trace_last = Instant::now();
        }
        self.last_wool = wool;
        self.last_balls = balls;
        if self.runner.on_start_script() && self.start_count == 0 {
            let Some(handle) = self.start_handle.as_ref() else {
                self.error = Some("Quester Start reached before ScriptStartHandle install".into());
                return;
            };
            match handle.start_compiled(
                &self.account,
                script::CompiledId("Quester"),
                self.start_settings.clone(),
            ) {
                Ok(()) => self.start_count = 1,
                Err(error) => {
                    self.error = Some(format!("Quester compiled Start failed: {error}"));
                }
            }
        }
        if !matches!(
            self.runner.status(),
            RunnerStatus::Passed | RunnerStatus::Failed(_)
        ) {
            self.runner.tick_with_hold(client, hold);
        }
    }
}

fn run_quester_sheep() -> Result<(), String> {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Ok(());
    }
    const CELL: &str = "quester_sheep";
    let _home = script::IsolatedEnv::enter(CELL);
    api::hostlog::set_debug(true);
    let nav_pack = PathBuf::from(required("WORLD_NAV_PACK")?);
    let engine_dir = PathBuf::from(required("WORLD_ENGINE_DIR")?);
    let catalog_root = PathBuf::from(
        std::env::var("RS2B0T")
            .or_else(|_| std::env::var("GATHERER_CATALOG_ROOT"))
            .map_err(|_| "RS2B0T or GATHERER_CATALOG_ROOT is required")?,
    );
    for (name, path, is_file) in [
        ("WORLD_NAV_PACK", &nav_pack, true),
        ("WORLD_ENGINE_DIR", &engine_dir, false),
        ("catalog", &catalog_root, false),
    ] {
        if !path.is_absolute() {
            return Err(format!(
                "{name} must be an absolute path: {}",
                path.display()
            ));
        }
        if is_file && !path.is_file() {
            return Err(format!("{name} is not a file: {}", path.display()));
        }
        if !is_file && !path.is_dir() {
            return Err(format!("{name} is not a directory: {}", path.display()));
        }
    }
    let mut scenario =
        scenario::get("quester_sheep").ok_or("scenario registry has no quester_sheep")?;
    if scenario.settings.start_script != Some("Quester") {
        return Err(format!(
            "quester_sheep starts {:?}, not Quester",
            scenario.settings.start_script
        ));
    }
    let start_settings = scenario::settings_inject_map(scenario.settings.script_settings_inject)
        .ok_or("quester_sheep has no compiled Start settings")?;
    if start_settings.len() != 1 || start_settings.get("quests") != Some(&json!(["sheep"])) {
        return Err(format!(
            "quester_sheep must Start Quester with exactly {{\"quests\":[\"sheep\"]}}, got {start_settings:?}"
        ));
    }
    let deadline = scenario.settings.deadline;
    let mainland = scenario.seed.mainland;
    scenario.settings.nav.engine_speed_ms = None;
    let temp = TempRoot::new(CELL)?;
    let (profile, template) = selected_profile(nav_pack, engine_dir, catalog_root, temp.path())?;
    let names = host_play::mint_live_names(1);
    let account = names
        .first()
        .cloned()
        .ok_or("failed to mint Quester live account")?;
    let password = host_play::mint_live_entries(&names)
        .first()
        .map(|(_, password)| password.clone())
        .ok_or("failed to mint Quester live credential")?;
    let mut runner = ScenarioRunner::with_world(scenario, template.world());
    runner.set_map_members(profile.map_members());
    runner.set_live_names(&names);
    runner.set_shot_sink(Box::new(|_, _| {}));
    let state = Arc::new(Mutex::new(QuesterSheepState {
        runner,
        account: account.clone(),
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
        start_handle: None,
        start_settings: start_settings.clone(),
        start_count: 0,
        error: None,
        trace_started: Instant::now(),
        trace_last: Instant::now(),
        last_wool: -1,
        last_balls: -1,
        total_sheared: 0,
    }));
    let frame_state = Arc::clone(&state);
    let frame_account = account.clone();
    let mut play = host_play::run_with_template(
        Arc::clone(&template),
        mainland,
        vec![],
        |_| (None, None),
        move |client, username, hold| {
            if username == frame_account {
                frame_state.lock().unwrap().frame(client, hold.hold);
            }
        },
    )?;
    {
        let mut slot = state.lock().map_err(|_| "Quester live state poisoned")?;
        slot.runner.set_obj_names(play.obj_names());
        slot.start_handle = Some(play.script_start_handle());
    }
    play.try_spawn_slot(mint_profile(&account, &password, 1)?, None, None, None)?;
    play.focus(&account);
    println!(
        "{}",
        json!({
            "phase": "identity",
            "live_case": CELL,
            "profile": profile.label(),
            "game_port": profile.client().game_port(),
            "nav_pack": profile.nav_pack(),
            "account": account,
            "card": "Quester",
            "settings": start_settings,
            "scenario": "quester_sheep",
        })
    );
    let outer_deadline = Instant::now() + deadline + Duration::from_secs(15);
    let result = loop {
        let script_error = play.script_last_error(&account);
        let native_status = play.script_native_status(&account);
        let run_state = play.script_state(&account);
        let lifecycle = play.script_lifecycle_receipt(&account);
        let (runner_status, start_count, error) = {
            let mut slot = state.lock().map_err(|_| "Quester live state poisoned")?;
            if run_state == script::RunState::Running {
                slot.runner.observe_script_running();
            }
            (slot.runner.status(), slot.start_count, slot.error.clone())
        };
        if let Some(error) = error {
            break Err(error);
        }
        if let Some(error) = script_error {
            break Err(format!("Quester lifecycle error: {error}"));
        }
        if let Some(receipt) = lifecycle.as_ref() {
            match receipt.state {
                script::ScriptTerminalState::Failed
                | script::ScriptTerminalState::Cancelled
                | script::ScriptTerminalState::Stopped => {
                    break Err(format!(
                        "quester_sheep native run ended {:?}: {receipt:?}",
                        receipt.state
                    ));
                }
                script::ScriptTerminalState::Completed => {}
            }
        }
        match runner_status {
            RunnerStatus::Failed(error) => {
                break Err(format!("quester_sheep scenario failed: {error}"));
            }
            RunnerStatus::Passed => {
                if start_count != 1 {
                    break Err(format!(
                        "quester_sheep passed after {start_count} compiled Starts, expected one"
                    ));
                }
                if let Some(status) = native_status.as_ref() {
                    if status.card != script::CompiledId("Quester") {
                        break Err(format!(
                            "quester_sheep terminal status belongs to {:?}",
                            status.card
                        ));
                    }
                }
                let completed = lifecycle
                    .as_ref()
                    .is_some_and(|receipt| receipt.state == script::ScriptTerminalState::Completed);
                if completed && run_state == script::RunState::Idle {
                    println!(
                        "{}",
                        json!({
                            "phase": "witness",
                            "live_case": CELL,
                            "scenario": "quester_sheep",
                            "native_phase": native_status
                                .as_ref()
                                .map(|status| format!("{:?}", status.phase)),
                            "run_state": format!("{run_state:?}"),
                            "lifecycle": format!("{lifecycle:?}"),
                            "start_count": start_count,
                        })
                    );
                    break Ok(());
                }
            }
            RunnerStatus::Seeding | RunnerStatus::Running { .. } => {}
        }
        if Instant::now() >= outer_deadline {
            let tile = state
                .lock()
                .map_err(|_| "Quester live state poisoned")?
                .snapshot
                .tile();
            break Err(format!(
                "quester_sheep timed out: runner={runner_status:?} native_status={native_status:?} lifecycle={lifecycle:?} run_state={run_state:?} tile={tile:?}"
            ));
        }
        std::thread::sleep(POLL_INTERVAL);
    };
    println!(
        "{}",
        json!({
            "phase": "comparison-terminal",
            "account": account,
            "result": format!("{:?}", result),
            "native_status": format!("{:?}", play.script_native_status(&account)),
            "lifecycle": format!("{:?}", play.script_lifecycle_receipt(&account)),
            "run_state": format!("{:?}", play.script_state(&account)),
        })
    );
    play.stop_slot(&account);
    result
}

#[test]
#[ignore = "requires LIVE=1, WORLD_NAV_PACK, WORLD_ENGINE_DIR and local 289 engine"]
fn quester_sheep() {
    run_quester_sheep().unwrap();
}
