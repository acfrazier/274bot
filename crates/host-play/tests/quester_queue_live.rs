//! Unattended Cook → Sheep → Romeo & Juliet → Imp Quester queue. Ignored unless LIVE=1.
#![cfg(feature = "live-harness")]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::snapshot::GameSnapshot;
use host::Pump;
use host_play::{ProfileOptions, ScriptStartHandle, SharedClientTemplate};
use scenario::{RunnerStatus, ScenarioRunner};
use script::native::{NativePhase, ScriptStatus, StatusValue};
use serde_json::{json, Map, Value};
use vault::{Profile, ProfileSettings};

const CELL: &str = "qs_quester_queue";
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const EXPECTED_QUESTS: &[&str] = &["cook", "sheep", "romeojuliet", "imp"];

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
    std::env::var(name).map_err(|_| format!("{name} is required for {CELL} live"))
}

fn required_path(name: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(required(name)?);
    if !path.is_absolute() {
        return Err(format!(
            "{name} must be an absolute path: {}",
            path.display()
        ));
    }
    Ok(path)
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
    cache_dir: PathBuf,
    temp: &Path,
) -> Result<(Arc<host_play::ServerProfile>, Arc<SharedClientTemplate>), String> {
    for (name, path, is_file) in [
        ("WORLD_NAV_PACK", &nav_pack, true),
        ("WORLD_ENGINE_DIR", &engine_dir, false),
        ("RS2B0T", &catalog_root, false),
        ("BOT_CACHE_DIR", &cache_dir, false),
    ] {
        if is_file && !path.is_file() {
            return Err(format!("{name} is not a file: {}", path.display()));
        }
        if !is_file && !path.is_dir() {
            return Err(format!("{name} is not a directory: {}", path.display()));
        }
    }
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
        cache_dir: Some(cache_dir),
        nav_pack: Some(nav_pack),
        nav_flags: std::env::var_os("WORLD_NAV_FLAGS")
            .or_else(|| std::env::var_os("GATHERER_NAV_FLAGS"))
            .map(PathBuf::from),
        engine_dir: Some(engine_dir),
        vault_path: Some(temp.join("vault")),
        catalog_root: Some(catalog_root),
        ..ProfileOptions::default()
    };
    let profile = options.resolve(None)?.bind_runtime()?;
    if profile.profile_class() != host_play::ProfileClass::Local
        || profile.client().game_host() != "127.0.0.1"
    {
        return Err(format!("{CELL} live requires a loopback local profile"));
    }
    let template = SharedClientTemplate::load(Arc::clone(&profile))?;
    if template.world().is_none() {
        return Err("selected template has no navigation world".into());
    }
    Ok((profile, template))
}

struct QuesterQueueState {
    runner: ScenarioRunner,
    account: String,
    snapshot: GameSnapshot,
    pump: Pump,
    start_handle: Option<ScriptStartHandle>,
    start_settings: Map<String, Value>,
    start_count: u32,
    error: Option<String>,
}

impl QuesterQueueState {
    fn frame(&mut self, client: &mut client::client::Client, hold: bool) {
        let drain = self.pump.drain_client(client);
        host::publish_snapshot(&mut self.snapshot, client, drain);
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
                    return;
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

fn status_field<'a>(status: &'a ScriptStatus, key: &str) -> Option<&'a StatusValue> {
    status
        .fields
        .iter()
        .find(|field| field.key == key)
        .map(|field| &field.value)
}

fn text_field(status: &ScriptStatus, key: &str) -> Result<Option<String>, String> {
    match status_field(status, key) {
        None => Ok(None),
        Some(StatusValue::Text(value)) => Ok(Some(value.to_string())),
        Some(value) => Err(format!(
            "Quester status field {key} must be text, got {value:?}"
        )),
    }
}

fn integer_field(status: &ScriptStatus, key: &str) -> Result<i64, String> {
    match status_field(status, key) {
        Some(StatusValue::Integer(value)) => Ok(*value),
        None => Err(format!("Quester status is missing integer field {key}")),
        Some(value) => Err(format!(
            "Quester status field {key} must be an integer, got {value:?}"
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeQueueReceipt {
    phase: String,
    queue: String,
    completed: i64,
    deaths: i64,
    retreat_count: i64,
    last_retreat: Option<String>,
    quest_id: Option<String>,
    display: Option<String>,
}

#[derive(Debug, Clone)]
struct RetreatReceipt {
    count: i64,
    quest_id: String,
    completed: i64,
    active_quest: Option<String>,
    queue: String,
}

#[derive(Default)]
struct QueueEvidence {
    latest: Option<NativeQueueReceipt>,
    active_order: Vec<String>,
    queue_states: Vec<String>,
    completed_states: Vec<i64>,
    retreat_receipts: Vec<RetreatReceipt>,
    deaths: Option<i64>,
    complete_observed: bool,
}

impl QueueEvidence {
    fn observe(&mut self, status: &ScriptStatus) -> Result<(), String> {
        if status.phase == NativePhase::Preparing {
            return Ok(());
        }
        let queue = text_field(status, "queue")?.ok_or("Quester status is missing queue text")?;
        let last_retreat = text_field(status, "last_retreat")?;
        let quest_id = text_field(status, "quest_id")?;
        let display = text_field(status, "display")?;
        let receipt = NativeQueueReceipt {
            phase: format!("{:?}", status.phase),
            queue,
            completed: integer_field(status, "completed")?,
            deaths: integer_field(status, "deaths")?,
            retreat_count: integer_field(status, "retreat_count")?,
            last_retreat,
            quest_id,
            display,
        };
        if receipt.completed < 0 || receipt.completed > EXPECTED_QUESTS.len() as i64 {
            return Err(format!(
                "Quester completed count is outside this four-quest fixture: {receipt:?}"
            ));
        }
        if receipt.deaths < 0 {
            return Err(format!("Quester deaths count is negative: {receipt:?}"));
        }
        if receipt.retreat_count < 0 || receipt.retreat_count > 2 {
            return Err(format!(
                "Quester retreat count is outside the two non-owning Paths: {receipt:?}"
            ));
        }
        if receipt.quest_id.is_some() != receipt.display.is_some() {
            return Err(format!(
                "Quester active quest id/display are not paired: {receipt:?}"
            ));
        }
        if let Some(id) = receipt.quest_id.as_deref() {
            let expected_display = match id {
                "cook" => "Cook's Assistant",
                "sheep" => "Sheep Shearer",
                "romeojuliet" => "Romeo & Juliet",
                "imp" => "Imp Catcher",
                _ => return Err(format!("unexpected active Quester path {id:?}")),
            };
            if receipt.display.as_deref() != Some(expected_display) {
                return Err(format!(
                    "active Quester path {id:?} published display {:?}, expected {expected_display:?}",
                    receipt.display
                ));
            }
        }

        let previous = self.latest.as_ref();
        if let Some(previous) = previous {
            if receipt.completed < previous.completed {
                return Err(format!(
                    "Quester completed count regressed: {previous:?} → {receipt:?}"
                ));
            }
            if receipt.deaths < previous.deaths {
                return Err(format!(
                    "Quester deaths count regressed: {previous:?} → {receipt:?}"
                ));
            }
            if receipt.retreat_count < previous.retreat_count {
                return Err(format!(
                    "Quester retreat count regressed: {previous:?} → {receipt:?}"
                ));
            }
            if receipt.retreat_count == previous.retreat_count
                && receipt.last_retreat != previous.last_retreat
            {
                return Err(format!(
                    "Quester last-retreat id changed without a count: {previous:?} → {receipt:?}"
                ));
            }
            if receipt.retreat_count > previous.retreat_count {
                if receipt.retreat_count != previous.retreat_count + 1 {
                    return Err(format!(
                        "Quester retreat count skipped a receipt: {previous:?} → {receipt:?}"
                    ));
                }
                let expected = EXPECTED_QUESTS
                    .get(self.retreat_receipts.len())
                    .filter(|id| matches!(**id, "cook" | "sheep"))
                    .copied()
                    .ok_or_else(|| format!("unexpected extra retreat: {receipt:?}"))?;
                let next = EXPECTED_QUESTS
                    .get(self.retreat_receipts.len() + 1)
                    .copied();
                if receipt.last_retreat.as_deref() != Some(expected)
                    || receipt.completed < receipt.retreat_count
                    || receipt.quest_id.as_deref() == next
                {
                    return Err(format!(
                        "retreat for {expected} was not witnessed before its next activation: {receipt:?}"
                    ));
                }
                self.retreat_receipts.push(RetreatReceipt {
                    count: receipt.retreat_count,
                    quest_id: expected.to_owned(),
                    completed: receipt.completed,
                    active_quest: receipt.quest_id.clone(),
                    queue: receipt.queue.clone(),
                });
            }
        } else if receipt.retreat_count != 0 || receipt.last_retreat.is_some() {
            return Err(format!(
                "first observed native receipt already contains a retreat, so ordering cannot be proven: {receipt:?}"
            ));
        }

        if self.queue_states.last() != Some(&receipt.queue) {
            self.queue_states.push(receipt.queue.clone());
        }
        if self.completed_states.last() != Some(&receipt.completed) {
            self.completed_states.push(receipt.completed);
        }
        self.deaths = Some(receipt.deaths);

        if let Some(id) = receipt.quest_id.as_deref() {
            if self.active_order.last().map(String::as_str) != Some(id) {
                let expected = EXPECTED_QUESTS.get(self.active_order.len()).copied();
                if expected != Some(id) {
                    return Err(format!(
                        "active Quester order is not Cook → Sheep → Romeo & Juliet → Imp: {:?} then {id:?}",
                        self.active_order
                    ));
                }
                let expected_retreat = match id {
                    "cook" => {
                        if receipt.completed != 0 || receipt.retreat_count != 0 {
                            return Err(format!(
                                "Cook did not activate from the clean queue start: {receipt:?}"
                            ));
                        }
                        None
                    }
                    "sheep" => Some(("cook", 1, 1)),
                    "romeojuliet" => Some(("sheep", 2, 2)),
                    "imp" => Some(("sheep", 2, 3)),
                    _ => unreachable!(),
                };
                if let Some((retreated, count, minimum_completed)) = expected_retreat {
                    if receipt.retreat_count != count
                        || receipt.last_retreat.as_deref() != Some(retreated)
                        || receipt.completed < minimum_completed
                    {
                        return Err(format!(
                            "{id} activated before the preceding generic retreat: {receipt:?}"
                        ));
                    }
                }
                self.active_order.push(id.to_owned());
            }
        }

        if status.phase == NativePhase::Complete {
            if receipt.completed != EXPECTED_QUESTS.len() as i64
                || receipt.retreat_count != 2
                || receipt.last_retreat.as_deref() != Some("sheep")
                || self
                    .active_order
                    .iter()
                    .map(String::as_str)
                    .ne(EXPECTED_QUESTS.iter().copied())
                || self.retreat_receipts.len() != 2
            {
                return Err(format!(
                    "native Complete arrived before the full queue and both generic retreats: {receipt:?}, active_order={:?}, retreats={:?}",
                    self.active_order, self.retreat_receipts
                ));
            }
            self.complete_observed = true;
        }
        if self.latest.as_ref() != Some(&receipt) {
            self.latest = Some(receipt);
        }
        Ok(())
    }

    fn validate_terminal(&self) -> Result<(), String> {
        let Some(receipt) = self.latest.as_ref() else {
            return Err("Quester ended without publishing native queue evidence".into());
        };
        if !self.complete_observed
            || receipt.phase != "Complete"
            || receipt.completed != EXPECTED_QUESTS.len() as i64
            || receipt.retreat_count != 2
            || receipt.last_retreat.as_deref() != Some("sheep")
            || self
                .active_order
                .iter()
                .map(String::as_str)
                .ne(EXPECTED_QUESTS.iter().copied())
            || self.retreat_receipts.len() != 2
        {
            return Err(format!(
                "native terminal receipt does not prove the required queue: {receipt:?}, active_order={:?}, retreats={:?}",
                self.active_order, self.retreat_receipts
            ));
        }
        Ok(())
    }

    fn json(&self) -> Value {
        let latest = self.latest.as_ref();
        json!({
            "queue": latest.map(|receipt| receipt.queue.as_str()),
            "queue_states": self.queue_states,
            "active_order": self.active_order,
            "completed": latest.map(|receipt| receipt.completed),
            "completed_states": self.completed_states,
            "deaths": self.deaths,
            "retreat_count": latest.map(|receipt| receipt.retreat_count),
            "last_retreat": latest.and_then(|receipt| receipt.last_retreat.as_deref()),
            "retreat_receipts": self.retreat_receipts.iter().map(|receipt| json!({
                "count": receipt.count,
                "quest_id": receipt.quest_id,
                "completed": receipt.completed,
                "active_quest": receipt.active_quest,
                "queue": receipt.queue,
            })).collect::<Vec<_>>(),
            "active_quest_id": latest.and_then(|receipt| receipt.quest_id.as_deref()),
            "display": latest.and_then(|receipt| receipt.display.as_deref()),
            "native_phase": latest.map(|receipt| receipt.phase.as_str()),
        })
    }
}

fn run_quester_queue() -> Result<(), String> {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Ok(());
    }
    if std::env::var("BOT_LIVE_NAME_PREFIX").as_deref() != Ok("qs") {
        return Err("quester_queue live requires BOT_LIVE_NAME_PREFIX=qs".into());
    }
    let _home = script::IsolatedEnv::enter(CELL);
    api::hostlog::set_debug(true);
    let nav_pack = required_path("WORLD_NAV_PACK")?;
    let engine_dir = required_path("WORLD_ENGINE_DIR")?;
    let catalog_root = required_path("RS2B0T")?;
    let cache_dir = required_path("BOT_CACHE_DIR")?;

    let mut scenario =
        scenario::get("quester_queue").ok_or("scenario registry has no quester_queue")?;
    if scenario.name != "quester_queue" || scenario.settings.start_script != Some("Quester") {
        return Err(format!(
            "quester_queue must start Quester, got name={} start={:?}",
            scenario.name, scenario.settings.start_script
        ));
    }
    let start_settings = scenario::settings_inject_map(scenario.settings.script_settings_inject)
        .ok_or("quester_queue has no compiled Start settings")?;
    let expected_settings = json!({
        "quests": EXPECTED_QUESTS,
        "order_override": EXPECTED_QUESTS,
        "skip": [],
    });
    if Value::Object(start_settings.clone()) != expected_settings {
        return Err(format!(
            "quester_queue must Start with exactly {expected_settings}, got {start_settings:?}"
        ));
    }
    let deadline = scenario.settings.deadline;
    let mainland = scenario.seed.mainland;
    scenario.settings.nav.engine_speed_ms = None;
    let temp = TempRoot::new(CELL)?;
    let (profile, template) = selected_profile(
        nav_pack,
        engine_dir,
        catalog_root,
        cache_dir.clone(),
        temp.path(),
    )?;
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
    let state = Arc::new(Mutex::new(QuesterQueueState {
        runner,
        account: account.clone(),
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
        start_handle: None,
        start_settings: start_settings.clone(),
        start_count: 0,
        error: None,
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
            "engine_dir": required_path("WORLD_ENGINE_DIR")?,
            "catalog_root": profile.catalog_root(),
            "cache_dir": cache_dir,
            "account": account,
            "card": "Quester",
            "settings": start_settings,
            "scenario": "quester_queue",
        })
    );
    let outer_deadline = Instant::now() + deadline + Duration::from_secs(15);
    let mut evidence = QueueEvidence::default();
    let result = loop {
        let script_error = play.script_last_error(&account);
        let native_status = play.script_native_status(&account);
        let run_state = play.script_state(&account);
        let lifecycle = play.script_lifecycle_receipt(&account);
        if let Some(status) = native_status.as_ref() {
            if status.card != script::CompiledId("Quester") {
                break Err(format!(
                    "quester_queue native status belongs to {:?}",
                    status.card
                ));
            }
            if let Err(error) = evidence.observe(status) {
                break Err(error);
            }
            if status.phase == NativePhase::Blocked {
                break Err(format!(
                    "quester_queue Quester blocked: {:?}",
                    status.failure
                ));
            }
        }
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
                        "quester_queue native run ended {:?}: {receipt:?}",
                        receipt.state
                    ));
                }
                script::ScriptTerminalState::Completed => {}
            }
        }
        match runner_status {
            RunnerStatus::Failed(error) => {
                break Err(format!("quester_queue scenario failed: {error}"));
            }
            RunnerStatus::Passed => {
                if start_count != 1 {
                    break Err(format!(
                        "quester_queue passed after {start_count} compiled Starts, expected one"
                    ));
                }
                let completed = lifecycle
                    .as_ref()
                    .is_some_and(|receipt| receipt.state == script::ScriptTerminalState::Completed);
                if completed && run_state == script::RunState::Idle {
                    if let Err(error) = evidence.validate_terminal() {
                        break Err(error);
                    }
                    println!(
                        "{}",
                        json!({
                            "phase": "witness",
                            "live_case": CELL,
                            "scenario": "quester_queue",
                            "native_evidence": evidence.json(),
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
                "quester_queue timed out: runner={runner_status:?} native_status={native_status:?} lifecycle={lifecycle:?} run_state={run_state:?} tile={tile:?} evidence={}",
                evidence.json()
            ));
        }
        std::thread::sleep(POLL_INTERVAL);
    };
    println!(
        "{}",
        json!({
            "phase": "comparison-terminal",
            "account": account,
            "result": format!("{result:?}"),
            "native_evidence": evidence.json(),
            "native_status": format!("{:?}", play.script_native_status(&account)),
            "lifecycle": format!("{:?}", play.script_lifecycle_receipt(&account)),
            "run_state": format!("{:?}", play.script_state(&account)),
        })
    );
    play.stop_slot(&account);
    result
}

#[test]
#[ignore = "requires LIVE=1, BOT_LIVE_NAME_PREFIX=qs, explicit WORLD_NAV_PACK/WORLD_ENGINE_DIR/RS2B0T/BOT_CACHE_DIR and local 289 engine"]
fn quester_queue() {
    run_quester_queue().unwrap();
}
