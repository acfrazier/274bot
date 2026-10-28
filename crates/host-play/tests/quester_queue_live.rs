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
    quest_id: Option<String>,
    display: Option<String>,
}

#[derive(Default)]
struct QueueEvidence {
    latest: Option<NativeQueueReceipt>,
    active_order: Vec<String>,
    queue_states: Vec<String>,
    completed_states: Vec<i64>,
    deaths: Option<i64>,
    complete_observed: bool,
}

impl QueueEvidence {
    fn observe(&mut self, status: &ScriptStatus) -> Result<(), String> {
        if status.phase == NativePhase::Preparing {
            return Ok(());
        }
        let queue = text_field(status, "queue")?.ok_or("Quester status is missing queue text")?;
        let quest_id = text_field(status, "quest_id")?;
        let display = text_field(status, "display")?;
        let receipt = NativeQueueReceipt {
            phase: format!("{:?}", status.phase),
            queue,
            completed: integer_field(status, "completed")?,
            deaths: integer_field(status, "deaths")?,
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
                if receipt.completed != self.active_order.len() as i64 {
                    return Err(format!(
                        "{id} activated before the preceding Path completed: {receipt:?}"
                    ));
                }
                self.active_order.push(id.to_owned());
            }
        }

        if status.phase == NativePhase::Complete {
            if receipt.completed != EXPECTED_QUESTS.len() as i64
                || self
                    .active_order
                    .iter()
                    .map(String::as_str)
                    .ne(EXPECTED_QUESTS.iter().copied())
            {
                return Err(format!(
                    "native Complete arrived before the full queue: {receipt:?}, active_order={:?}",
                    self.active_order
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
            || self
                .active_order
                .iter()
                .map(String::as_str)
                .ne(EXPECTED_QUESTS.iter().copied())
        {
            return Err(format!(
                "native terminal receipt does not prove the required queue: {receipt:?}, active_order={:?}",
                self.active_order
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
            "active_quest_id": latest.and_then(|receipt| receipt.quest_id.as_deref()),
            "display": latest.and_then(|receipt| receipt.display.as_deref()),
            "native_phase": latest.map(|receipt| receipt.phase.as_str()),
        })
    }
}

#[cfg(feature = "test-support")]

const SKIP_CELL: &str = "qs_quester_skip";
#[cfg(feature = "test-support")]

const SKIP_QUESTS: &[&str] = &["cook", "sheep"];

#[cfg(feature = "test-support")]

#[derive(Debug, Clone)]
struct SkipActionReceipt {
    quest_id: String,
    step_id: String,
    action_state: String,
    child_step_id: Option<String>,
    phase: String,
}

#[cfg(feature = "test-support")]

#[derive(Debug, Clone)]
struct SkipStatusReceipt {
    phase: String,
    queue: String,
    completed: i64,
    quest_id: Option<String>,
    display: Option<String>,
    step_id: Option<String>,
    action_state: Option<String>,
    block_reason: Option<String>,
    failure: Option<(String, String)>,
}

#[cfg(feature = "test-support")]

#[derive(Debug, Clone)]
struct SkipRequestReceipt {
    quest_id: String,
    step_id: String,
    action: SkipActionReceipt,
}

#[cfg(feature = "test-support")]

#[derive(Debug, Clone, Copy)]
struct SkipCleanupOwnership {
    script_walk: Option<(i32, i32, i32)>,
    seat_held: bool,
    live_actions: usize,
    seat_results: usize,
    reserved_releases: usize,
}

#[cfg(feature = "test-support")]

#[derive(Default)]
struct SkipQueueEvidence {
    skip: Option<SkipRequestReceipt>,
    active_order: Vec<String>,
    queue_states: Vec<String>,
    completed_states: Vec<i64>,
    cook_parked: bool,
    cancellation_observed: bool,
    cleanup_ownership: Option<SkipCleanupOwnership>,
    sheep_completed: bool,
    latest: Option<SkipStatusReceipt>,
}


#[cfg(feature = "test-support")]

fn queue_row_letter(queue: &str, id: &str) -> Result<char, String> {
    let position = script::quester::registry::BUNDLED_INDEX
        .paths
        .iter()
        .position(|path| path.id == id)
        .ok_or_else(|| format!("bundled Quester index has no {id:?} row"))?;
    queue
        .chars()
        .nth(position)
        .ok_or_else(|| format!("Quester queue status is missing row {id:?}: {queue:?}"))
}

#[cfg(feature = "test-support")]

impl SkipQueueEvidence {
    fn active_cook_action(
        status: &ScriptStatus,
    ) -> Result<Option<SkipActionReceipt>, String> {
        if status.card != script::CompiledId("Quester")
            || !matches!(status.phase, NativePhase::Working | NativePhase::Waiting)
            || text_field(status, "quest_id")?.as_deref() != Some("cook")
        {
            return Ok(None);
        }
        let Some(step_id) = text_field(status, "step_id")?.filter(|step| !step.is_empty()) else {
            return Ok(None);
        };
        let Some(action_state) = text_field(status, "action_state")? else {
            return Ok(None);
        };
        let child_step_id = text_field(status, "child_step_id")?.filter(|step| !step.is_empty());
        let active_action = matches!(
            action_state.as_str(),
            "walking" | "talking" | "fighting" | "banking"
        ) || (action_state == "working" && child_step_id.is_some());
        if !active_action {
            return Ok(None);
        }
        Ok(Some(SkipActionReceipt {
            quest_id: "cook".into(),
            step_id,
            action_state,
            child_step_id,
            phase: format!("{:?}", status.phase),
        }))
    }

    #[cfg(feature = "test-support")]

    fn observe_cancel_ownership(
        &mut self,
        status: &ScriptStatus,
        play: &host_play::Play,
        name: &str,
    ) -> Result<(), String> {
        if self.skip.is_none() || self.cleanup_ownership.is_some() {
            return Ok(());
        }
        if status.phase == NativePhase::Preparing
            || status.card != script::CompiledId("Quester")
            || text_field(status, "block_reason")?.as_deref() != Some("skipped by operator")
        {
            return Ok(());
        }
        let queue = text_field(status, "queue")?.ok_or("parked Quester status is missing queue")?;
        if queue_row_letter(&queue, "cook")? != 'P'
            || text_field(status, "quest_id")?.is_some()
            || text_field(status, "step_id")?.is_some()
            || text_field(status, "action_state")?.is_some()
            || text_field(status, "child_step_id")?.is_some()
        {
            return Ok(());
        }
        let slot_status = play
            .statuses()
            .into_iter()
            .find(|status| status.username == name)
            .ok_or("live slot status disappeared before skip cancellation proof")?;
        let seat = play
            .script_seat_probe(name)
            .ok_or("Quester slot seat disappeared before skip cancellation proof")?;
        let ownership = SkipCleanupOwnership {
            script_walk: slot_status.script_walk(),
            seat_held: seat.held,
            live_actions: seat.live,
            seat_results: seat.results,
            reserved_releases: seat.reserved_releases,
        };
        if ownership.script_walk.is_none()
            && !ownership.seat_held
            && ownership.live_actions == 0
            && ownership.reserved_releases == 0
        {
            self.cleanup_ownership = Some(ownership);
            self.cancellation_observed = true;
        }
        Ok(())
    }

    fn accept_skip(
        &mut self,
        action: SkipActionReceipt,
        skipped: script::native::SkippedQuest,
    ) -> Result<(), String> {
        if self.skip.is_some() {
            return Err("quester skip cell accepted more than one request".into());
        }
        if skipped.quest_id.as_ref() != action.quest_id
            || skipped.step_id.as_ref() != action.step_id
        {
            return Err(format!(
                "Skip quest identity does not match the active Cook action: result={skipped:?} action={action:?}"
            ));
        }
        self.skip = Some(SkipRequestReceipt {
            quest_id: skipped.quest_id.to_string(),
            step_id: skipped.step_id.to_string(),
            action,
        });
        Ok(())
    }

    fn observe(&mut self, status: &ScriptStatus) -> Result<(), String> {
        if status.phase == NativePhase::Preparing {
            return Ok(());
        }
        if status.card != script::CompiledId("Quester") {
            return Err(format!(
                "quester skip status belongs to {:?}",
                status.card
            ));
        }
        let queue = text_field(status, "queue")?.ok_or("Quester status is missing queue text")?;
        let completed = integer_field(status, "completed")?;
        let quest_id = text_field(status, "quest_id")?;
        let display = text_field(status, "display")?;
        let step_id = text_field(status, "step_id")?;
        let action_state = text_field(status, "action_state")?;
        let block_reason = text_field(status, "block_reason")?;
        if quest_id.is_some() != display.is_some() {
            return Err(format!(
                "active Quester path id/display are not paired: id={quest_id:?} display={display:?}"
            ));
        }
        if let Some(id) = quest_id.as_deref() {
            let expected_display = match id {
                "cook" => "Cook's Assistant",
                "sheep" => "Sheep Shearer",
                _ => return Err(format!("unexpected active Quester path {id:?}")),
            };
            if display.as_deref() != Some(expected_display) {
                return Err(format!(
                    "active Quester path {id:?} published display {display:?}, expected {expected_display:?}"
                ));
            }
            if self.active_order.last().map(String::as_str) != Some(id) {
                let expected = SKIP_QUESTS.get(self.active_order.len()).copied();
                if expected != Some(id) {
                    return Err(format!(
                        "Quester skip path order is not Cook → Sheep: {:?} then {id:?}",
                        self.active_order
                    ));
                }
                if id == "sheep" && !self.cancellation_observed {
                    return Err("Sheep activated before the Cook action cancellation was observed".into());
                }
                self.active_order.push(id.to_owned());
            }
        }
        if completed < 0 || completed > 1 {
            return Err(format!(
                "Quester completed count is outside the Cook-skip/Sheep fixture: {completed}"
            ));
        }
        if self.completed_states.last() != Some(&completed) {
            self.completed_states.push(completed);
        }
        if self.queue_states.last() != Some(&queue) {
            self.queue_states.push(queue.clone());
        }

        let cook_state = queue_row_letter(&queue, "cook")?;
        let sheep_state = queue_row_letter(&queue, "sheep")?;
        if self.skip.is_some() && block_reason.as_deref() == Some("skipped by operator") {
            if cook_state != 'P' {
                return Err(format!(
                    "Cook has the operator skip reason but is not Parked: queue={queue:?}"
                ));
            }
            self.cook_parked = true;

        }
        if quest_id.as_deref() == Some("sheep") && cook_state != 'P' {
            return Err(format!(
                "Sheep activated before Cook was Parked: queue={queue:?} reason={block_reason:?}"
            ));
        }
        if completed == 1 {
            if cook_state != 'P' || sheep_state != 'D' {
                return Err(format!(
                    "one completion must be Sheep with Cook Parked: queue={queue:?}"
                ));
            }
            self.sheep_completed = true;
        }
        if status.phase == NativePhase::Blocked && !self.sheep_completed {
            return Err(format!(
                "Quester blocked before Sheep completed: status={status:?}"
            ));
        }
        self.latest = Some(SkipStatusReceipt {
            phase: format!("{:?}", status.phase),
            queue,
            completed,
            quest_id,
            display,
            step_id,
            action_state,
            block_reason,
            failure: status
                .failure
                .as_ref()
                .map(|failure| (failure.code.to_string(), failure.message.to_string())),
        });
        Ok(())
    }

    fn validate_terminal(
        &self,
        status: &ScriptStatus,
        lifecycle: &script::ScriptLifecycleReceipt,
        run_state: script::RunState,
        runner_passed: bool,
        start_count: u32,
    ) -> Result<(), String> {
        let skip = self
            .skip
            .as_ref()
            .ok_or("Skip quest was not accepted during an active Cook action")?;
        if skip.quest_id != "cook"
            || skip.action.step_id != skip.step_id
            || skip.action.quest_id != "cook"
            || skip.step_id.is_empty()
            || skip.action.action_state.is_empty()
            || skip.action.phase.is_empty()
        {
            return Err(format!("invalid active Cook skip receipt: {skip:?}"));
        }
        if self.active_order.iter().map(String::as_str).ne(SKIP_QUESTS.iter().copied()) {
            return Err(format!(
                "active Quester rows were not exactly Cook then Sheep: {:?}",
                self.active_order
            ));
        }
        if !self.cook_parked || !self.cancellation_observed || !self.sheep_completed {
            return Err(format!(
                "skip did not prove Cook Parked/cancelled and Sheep complete: {self:?}"
            ));
        }
        if start_count != 1 || !runner_passed {
            return Err(format!(
                "scenario did not pass after one compiled Start: passed={runner_passed} starts={start_count}"
            ));
        }
        if lifecycle.state != script::ScriptTerminalState::Failed
            || run_state != script::RunState::Error
            || status.phase != NativePhase::Blocked
        {
            return Err(format!(
                "the parked Cook row did not produce the normal queue-blocked terminal: state={run_state:?} lifecycle={lifecycle:?} status={status:?}"
            ));
        }
        let failure = status
            .failure
            .as_ref()
            .ok_or("blocked Quester status has no queue failure")?;
        if failure.code.as_ref() != "queue-blocked"
            || !failure.message.contains("skipped by operator")
        {
            return Err(format!(
                "the terminal queue failure does not retain the Cook skip reason: {failure:?}"
            ));
        }
        let reason = text_field(status, "block_reason")?;
        if reason.as_deref() != Some("skipped by operator") {
            return Err(format!(
                "terminal Quester status lost the Cook skip reason: {reason:?}"
            ));
        }
        let queue = text_field(status, "queue")?.ok_or("terminal status is missing queue")?;
        if queue_row_letter(&queue, "cook")? != 'P'
            || queue_row_letter(&queue, "sheep")? != 'D'
            || integer_field(status, "completed")? != 1
        {
            return Err(format!(
                "terminal queue is not Cook Parked and Sheep Done: queue={queue:?} status={status:?}"
            ));
        }
        Ok(())
    }

    fn json(&self) -> Value {
        let skip = self.skip.as_ref();
        let latest = self.latest.as_ref();
        json!({
            "skip_request": skip.map(|skip| json!({
                "quest_id": skip.quest_id,
                "step_id": skip.step_id,
                "action": {
                    "quest_id": skip.action.quest_id,
                    "step_id": skip.action.step_id,
                    "action_state": skip.action.action_state,
                    "child_step_id": skip.action.child_step_id,
                    "native_phase": skip.action.phase,
                },
            })),
            "active_order": self.active_order,
            "queue_states": self.queue_states,
            "completed_states": self.completed_states,
            "cook_parked": self.cook_parked,
            "active_action_cancelled": self.cancellation_observed,
            "cancellation_ownership": self.cleanup_ownership.map(|ownership| json!({
                "script_walk": ownership.script_walk,
                "seat_held": ownership.seat_held,
                "live_actions": ownership.live_actions,
                "seat_results": ownership.seat_results,
                "reserved_releases": ownership.reserved_releases,
            })),
            "sheep_completed": self.sheep_completed,

            "latest": latest.map(|status| json!({
                "native_phase": status.phase,
                "queue": status.queue,
                "completed": status.completed,
                "quest_id": status.quest_id,
                "display": status.display,
                "step_id": status.step_id,
                "action_state": status.action_state,
                "block_reason": status.block_reason,
                "failure": status.failure,
            })),
        })
    }
}

#[cfg(feature = "test-support")]

fn quester_skip_scenario() -> Result<(scenario::Scenario, Map<String, Value>), String> {
    let mut scenario =
        scenario::get("quester_queue").ok_or("scenario registry has no quester_queue")?;
    scenario.name = "quester_skip_queue";
    scenario.proof = scenario::Proof::QuestDone {
        name: "Sheep Shearer",
    };
    scenario.steps.retain(|step| {
        !matches!(
            step.name,
            "watch the queued Cook completion"
                | "watch the queued Romeo and Juliet completion"
                | "watch the queued Imp Catcher completion"
        )
    });
    if !scenario
        .steps
        .iter()
        .any(|step| step.name == "watch the queued Sheep completion")
    {
        return Err("quester_queue seed has no Sheep completion watcher".into());
    }
    let settings = Map::from_iter([
        ("quests".to_owned(), json!(SKIP_QUESTS)),
        ("order_override".to_owned(), json!(SKIP_QUESTS)),
        ("skip".to_owned(), json!([])),
    ]);
    scenario.settings.script_settings_inject = None;
    scenario.settings.script_settings_overrides = Some(settings.clone());
    Ok((scenario, settings))
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


#[cfg(feature = "test-support")]

fn run_quester_skip() -> Result<(), String> {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Ok(());
    }
    if std::env::var("BOT_LIVE_NAME_PREFIX").as_deref() != Ok("qs") {
        return Err("quester skip live requires BOT_LIVE_NAME_PREFIX=qs".into());
    }
    let _home = script::IsolatedEnv::enter(SKIP_CELL);
    api::hostlog::set_debug(true);
    let nav_pack = required_path("WORLD_NAV_PACK")?;
    let engine_dir = required_path("WORLD_ENGINE_DIR")?;
    let catalog_root = required_path("RS2B0T")?;
    let cache_dir = required_path("BOT_CACHE_DIR")?;

    let (scenario, start_settings) = quester_skip_scenario()?;
    if scenario.name != "quester_skip_queue"
        || scenario.settings.start_script != Some("Quester")
        || Value::Object(start_settings.clone())
            != json!({
                "quests": SKIP_QUESTS,
                "order_override": SKIP_QUESTS,
                "skip": [],
            })
    {
        return Err(format!(
            "quester skip scenario has an unexpected start: name={} card={:?} settings={start_settings:?}",
            scenario.name, scenario.settings.start_script
        ));
    }
    let deadline = scenario.settings.deadline;
    let mainland = scenario.seed.mainland;
    let temp = TempRoot::new(SKIP_CELL)?;
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
        .ok_or("failed to mint Quester skip live account")?;
    let password = host_play::mint_live_entries(&names)
        .first()
        .map(|(_, password)| password.clone())
        .ok_or("failed to mint Quester skip live credential")?;
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
        let mut slot = state.lock().map_err(|_| "Quester skip live state poisoned")?;
        slot.runner.set_obj_names(play.obj_names());
        slot.start_handle = Some(play.script_start_handle());
    }
    play.try_spawn_slot(mint_profile(&account, &password, 1)?, None, None, None)?;
    play.focus(&account);
    println!(
        "{}",
        json!({
            "phase": "identity",
            "live_case": SKIP_CELL,
            "profile": profile.label(),
            "game_port": profile.client().game_port(),
            "nav_pack": profile.nav_pack(),
            "engine_dir": required_path("WORLD_ENGINE_DIR")?,
            "catalog_root": profile.catalog_root(),
            "cache_dir": cache_dir,
            "account": account,
            "card": "Quester",
            "settings": start_settings,
            "scenario": "quester_skip_queue",
            "world_speed_change": null,
        })
    );
    let outer_deadline = Instant::now() + deadline + Duration::from_secs(15);
    let mut evidence = SkipQueueEvidence::default();
    let result = loop {
        let script_error = play.script_last_error(&account);
        let native_status = play.script_native_status(&account);
        let run_state = play.script_state(&account);
        let lifecycle = play.script_lifecycle_receipt(&account);
        if let Some(status) = native_status.as_ref() {
            if let Err(error) = evidence.observe(status) {
                break Err(error);
            }
            if let Err(error) = evidence.observe_cancel_ownership(status, &play, &account) {
                break Err(error);
            }
        }
        let (runner_status, start_count, error) = {
            let mut slot = state.lock().map_err(|_| "Quester skip live state poisoned")?;
            if run_state == script::RunState::Running {
                slot.runner.observe_script_running();
            }
            (slot.runner.status(), slot.start_count, slot.error.clone())
        };
        if let Some(error) = error {
            break Err(error);
        }
        if let Some(status) = native_status.as_ref() {
            if evidence.skip.is_none() {
                if let Some(action) = SkipQueueEvidence::active_cook_action(status)? {
                    match play.script_skip_quest(&account) {
                        Ok(skipped) => {
                            evidence.accept_skip(action, skipped.clone())?;
                            println!(
                                "{}",
                                json!({
                                    "phase": "skip-accepted",
                                    "live_case": SKIP_CELL,
                                    "account": account,
                                    "quest_id": skipped.quest_id,
                                    "step_id": skipped.step_id,
                                    "active_action": evidence.skip.as_ref().map(|receipt| json!({
                                        "action_state": receipt.action.action_state,
                                        "child_step_id": receipt.action.child_step_id,
                                        "native_phase": receipt.action.phase,
                                    })),
                                })
                            );
                        }
                        Err(failure) => {
                            break Err(format!(
                                "Skip quest refused while Cook action was active: {} ({})",
                                failure.message, failure.code
                            ));
                        }
                    }
                }
            }
        }
        if let Some(error) = script_error.as_deref() {
            if !evidence.sheep_completed || !error.contains("skipped by operator") {
                break Err(format!("Quester skip lifecycle error: {error}"));
            }
        }
        if let Some(receipt) = lifecycle.as_ref() {
            match receipt.state {
                script::ScriptTerminalState::Failed => {
                    if matches!(&runner_status, RunnerStatus::Passed) {
                        let Some(status) = native_status.as_ref() else {
                            break Err("failed Quester skip run has no native status".into());
                        };
                        if let Err(error) = evidence.validate_terminal(
                            status,
                            receipt,
                            run_state,
                            true,
                            start_count,
                        ) {
                            break Err(error);
                        }
                        println!(
                            "{}",
                            json!({
                                "phase": "witness",
                                "live_case": SKIP_CELL,
                                "scenario": "quester_skip_queue",
                                "native_evidence": evidence.json(),
                                "runner_status": format!("{runner_status:?}"),
                                "run_state": format!("{run_state:?}"),
                                "lifecycle": format!("{receipt:?}"),
                                "start_count": start_count,
                            })
                        );
                        break Ok(());
                    }
                }
                script::ScriptTerminalState::Completed
                | script::ScriptTerminalState::Cancelled
                | script::ScriptTerminalState::Stopped => {
                    break Err(format!(
                        "quester skip run ended unexpectedly: {receipt:?}"
                    ));
                }
            }
        }
        if let RunnerStatus::Failed(error) = &runner_status {
            break Err(format!("quester skip scenario failed: {error}"));
        }
        if Instant::now() >= outer_deadline {
            let tile = state
                .lock()
                .map_err(|_| "Quester skip live state poisoned")?
                .snapshot
                .tile();
            break Err(format!(
                "quester skip timed out: runner={runner_status:?} native_status={native_status:?} lifecycle={lifecycle:?} run_state={run_state:?} tile={tile:?} evidence={}",
                evidence.json()
            ));
        }
        std::thread::sleep(POLL_INTERVAL);
    };
    println!(
        "{}",
        json!({
            "phase": "comparison-terminal",
            "live_case": SKIP_CELL,
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
#[cfg(feature = "test-support")]

#[test]
#[ignore = "requires LIVE=1, BOT_LIVE_NAME_PREFIX=qs, explicit WORLD_NAV_PACK/WORLD_ENGINE_DIR/RS2B0T/BOT_CACHE_DIR and Engine L"]
fn quester_skip_active_cook_parks_then_sheep_completes() {
    run_quester_skip().unwrap();
}
#[test]
#[ignore = "requires LIVE=1, BOT_LIVE_NAME_PREFIX=qs, explicit WORLD_NAV_PACK/WORLD_ENGINE_DIR/RS2B0T/BOT_CACHE_DIR and local 289 engine"]
fn quester_queue() {
    run_quester_queue().unwrap();
}
