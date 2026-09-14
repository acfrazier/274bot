//! Bounded external raw TypeScript loader smoke witness.
//!
//! Proof infrastructure only. Ordinary Play/panel runs leave this disabled.
//! The panel headed adapter feeds scene/inventory/logs and drives existing
//! Session load/Start/Stop/reload seams. Catalog `bone_burier` is a different
//! full-cycle witness and is not this contract.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// `--live` token without the `script_` prefix. Not a scenario catalog row.
pub const LIVE_SCENARIO: &str = "external_loader";
pub const LIVE_NAME: &str = "script_external_loader";
pub const RECEIPT_PREFIX: &str = "EXTERNAL_LOADER";

pub const SCRIPT_NAME: &str = "BoneBurier";
pub const SCRIPT_VERSION: &str = "0.1.0";
pub const BONES_ID: i32 = 526;
pub const PRAYER_STAT_ID: i32 = 5;
pub const BONES_COUNT: i32 = 25;
pub const MIN_DISTINCT_BURIALS: usize = 10;

pub const START_DEADLINE: Duration = Duration::from_millis(180_000);
pub const STOP_DEADLINE: Duration = Duration::from_millis(10_000);
pub const PREP_DEADLINE: Duration = Duration::from_secs(180);

pub const BURIAL_LOG_PREFIX: &str = "buried bones";
pub const ONSTOP_LOG_NEEDLE: &str = "BoneBurier stopped";
pub const TERMINAL_SHOT: &str = "external_loader terminal";

/// Headed-qualified raw ExampleBot / BoneBurier source.
pub const FROZEN_SHA256: &str = "8e8e27cd9c2e57cd6478132e4bfaf910678664898db44915acc08ffc501c0973";

pub const NOTHING_CHANGED: &str = script::NOTHING_CHANGED_RELOAD;

/// Tracked exact raw fixture. Not the gitignored campaign copy.
pub fn default_frozen_source() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/ExampleBot.ts")
}

/// `--external-ts` must be an absolute existing file. Relative paths are refused
/// so a later suite child cannot bind a different cwd expansion.
pub fn resolve_source(explicit: Option<&Path>) -> Result<PathBuf, String> {
    let path = match explicit {
        Some(path) => {
            if !path.is_absolute() {
                return Err(format!(
                    "--external-ts {} is relative; pass an absolute path",
                    path.display()
                ));
            }
            path.to_path_buf()
        }
        None => default_frozen_source(),
    };
    if !path.is_file() {
        return Err(format!(
            "external loader source {} is not a file",
            path.display()
        ));
    }
    Ok(path)
}

pub fn source_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Copy frozen bytes to an owned temp path. Never writes the frozen file.
pub fn materialize_owned_source(frozen: &Path) -> Result<PathBuf, String> {
    let bytes = std::fs::read(frozen)
        .map_err(|e| format!("external loader source {}: {e}", frozen.display()))?;
    let sha = source_sha256(&bytes);
    if sha != FROZEN_SHA256 {
        return Err(format!(
            "external loader source sha {sha} does not match frozen {FROZEN_SHA256}"
        ));
    }
    let dest = std::env::temp_dir().join(format!(
        "274bot-external-{}-ExampleBot.ts",
        std::process::id()
    ));
    std::fs::write(&dest, &bytes)
        .map_err(|e| format!("external loader owned copy {}: {e}", dest.display()))?;
    Ok(dest)
}

/// Harmless trailing newline on the owned copy only.
pub fn apply_harmless_whitespace(path: &Path) -> Result<(), String> {
    let mut text = std::fs::read_to_string(path)
        .map_err(|e| format!("external loader read {}: {e}", path.display()))?;
    text.push('\n');
    std::fs::write(path, text)
        .map_err(|e| format!("external loader write {}: {e}", path.display()))?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Disabled,
    Prepare,
    Load,
    Start,
    Run,
    Stop,
    ReloadUnchanged,
    ReloadChanged,
    Capture,
    Qualified,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    PrepareFixture,
    WaitScene,
    LoadRawTs,
    Start,
    ObserveBurials,
    Stop,
    ReloadUnchanged,
    ReloadChanged,
    Capture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalWatchStatus {
    Disabled,
    Ready,
    Running,
    Qualified,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SceneGate {
    pub ingame: bool,
    pub scene_state: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Counters {
    pub bones: i32,
    pub prayer_xp: i32,
    pub burial_logs: usize,
    pub distinct_burial_logs: usize,
}

impl Default for Counters {
    fn default() -> Self {
        Self {
            bones: 0,
            prayer_xp: 0,
            burial_logs: 0,
            distinct_burial_logs: 0,
        }
    }
}

#[derive(Clone, Default)]
pub struct ExternalWatch {
    active: Arc<AtomicBool>,
    inner: Arc<Mutex<WatchState>>,
}

#[derive(Debug)]
struct WatchState {
    stage: Stage,
    requested: Option<Operation>,
    completed: Option<Operation>,
    failure: Option<String>,
    account: String,
    source_path: PathBuf,
    source_sha: String,
    script_name: String,
    script_version: String,
    registration_count: usize,
    registration_after_reload: Option<usize>,
    selected: bool,
    running: bool,
    auto_start: bool,
    scene: SceneGate,
    initial: Option<Counters>,
    current: Counters,
    distinct_burials: BTreeSet<String>,
    saw_onstop: bool,
    paint_present: bool,
    start_at: Option<Instant>,
    stop_at: Option<Instant>,
    stop_elapsed_ms: Option<u128>,
    reload_unchanged: Option<String>,
    capture_requested: bool,
    terminal: Option<Arc<Value>>,
}

impl Default for WatchState {
    fn default() -> Self {
        Self {
            stage: Stage::Disabled,
            requested: None,
            completed: None,
            failure: None,
            account: String::new(),
            source_path: PathBuf::new(),
            source_sha: String::new(),
            script_name: String::new(),
            script_version: String::new(),
            registration_count: 0,
            registration_after_reload: None,
            selected: false,
            running: false,
            auto_start: false,
            scene: SceneGate {
                ingame: false,
                scene_state: 0,
            },
            initial: None,
            current: Counters::default(),
            distinct_burials: BTreeSet::new(),
            saw_onstop: false,
            paint_present: false,
            start_at: None,
            stop_at: None,
            stop_elapsed_ms: None,
            reload_unchanged: None,
            capture_requested: false,
            terminal: None,
        }
    }
}

impl ExternalWatch {
    pub fn configure(&self, account: impl Into<String>, source_path: PathBuf, source_sha: String) {
        let mut state = self.inner.lock().unwrap();
        *state = WatchState {
            stage: Stage::Prepare,
            requested: Some(Operation::PrepareFixture),
            account: account.into(),
            source_path,
            source_sha,
            script_name: SCRIPT_NAME.into(),
            script_version: SCRIPT_VERSION.into(),
            ..WatchState::default()
        };
        state.requested = Some(Operation::PrepareFixture);
        state.stage = Stage::Prepare;
        self.active.store(true, Ordering::Release);
    }

    pub fn clear(&self) {
        self.active.store(false, Ordering::Release);
        *self.inner.lock().unwrap() = WatchState::default();
    }

    pub fn configured(&self) -> bool {
        self.active.load(Ordering::Acquire) && self.inner.lock().unwrap().stage != Stage::Disabled
    }

    pub fn status(&self) -> ExternalWatchStatus {
        if !self.active.load(Ordering::Acquire) {
            return ExternalWatchStatus::Disabled;
        }
        match self.inner.lock().unwrap().stage {
            Stage::Disabled => ExternalWatchStatus::Disabled,
            Stage::Qualified => ExternalWatchStatus::Qualified,
            Stage::Failed => ExternalWatchStatus::Failed,
            Stage::Run | Stage::Stop => ExternalWatchStatus::Running,
            _ => ExternalWatchStatus::Ready,
        }
    }

    pub fn requested_operation(&self) -> Option<Operation> {
        self.inner.lock().unwrap().requested
    }

    pub fn source_path(&self) -> PathBuf {
        self.inner.lock().unwrap().source_path.clone()
    }

    pub fn account(&self) -> String {
        self.inner.lock().unwrap().account.clone()
    }

    pub fn failure(&self) -> Option<String> {
        self.inner.lock().unwrap().failure.clone()
    }

    pub fn fail(&self, reason: impl Into<String>) {
        let mut state = self.inner.lock().unwrap();
        if matches!(state.stage, Stage::Qualified | Stage::Failed) {
            return;
        }
        state.failure = Some(reason.into());
        state.stage = Stage::Failed;
        state.requested = None;
        state.terminal = Some(Arc::new(receipt_locked(&state)));
    }

    pub fn note_prereq_failed(&self, reason: impl Into<String>) {
        self.fail(format!("prerequisite: {}", reason.into()));
    }

    pub fn note_scene(&self, ingame: bool, scene_state: i32) {
        let mut state = self.inner.lock().unwrap();
        if matches!(
            state.stage,
            Stage::Qualified | Stage::Failed | Stage::Disabled
        ) {
            return;
        }
        state.scene = SceneGate {
            ingame,
            scene_state,
        };
        if state.stage == Stage::Prepare && ingame && scene_state == 2 {
            state.completed = Some(Operation::WaitScene);
            if state.requested == Some(Operation::PrepareFixture)
                || state.requested == Some(Operation::WaitScene)
            {
                state.requested = Some(Operation::WaitScene);
            }
        }
    }

    pub fn note_inventory(&self, bones: i32, prayer_xp: i32) {
        let mut state = self.inner.lock().unwrap();
        if matches!(
            state.stage,
            Stage::Qualified | Stage::Failed | Stage::Disabled
        ) {
            return;
        }
        state.current.bones = bones;
        state.current.prayer_xp = prayer_xp;
        try_advance_run_locked(&mut state);
    }

    pub fn note_prereq_passed(&self) {
        let mut state = self.inner.lock().unwrap();
        if matches!(
            state.stage,
            Stage::Qualified | Stage::Failed | Stage::Disabled
        ) {
            return;
        }
        if state.stage != Stage::Prepare {
            return;
        }
        if !(state.scene.ingame && state.scene.scene_state == 2) {
            fail_locked(
                &mut state,
                "prerequisite passed without ingame && scene_state == 2",
            );
            return;
        }
        let bones = state.current.bones;
        if bones < BONES_COUNT {
            fail_locked(
                &mut state,
                format!("prerequisite passed with {bones} bones, expected {BONES_COUNT}"),
            );
            return;
        }
        state.completed = Some(Operation::PrepareFixture);
        state.requested = Some(Operation::LoadRawTs);
        state.stage = Stage::Load;
    }

    pub fn note_logs(&self, lines: &[String]) {
        let mut state = self.inner.lock().unwrap();
        if matches!(
            state.stage,
            Stage::Qualified | Stage::Failed | Stage::Disabled
        ) {
            return;
        }
        for line in lines {
            let msg = line.strip_prefix("script: ").unwrap_or(line.as_str());
            if msg.starts_with(BURIAL_LOG_PREFIX) {
                state.distinct_burials.insert(msg.to_string());
            }
            if msg.contains(ONSTOP_LOG_NEEDLE) {
                state.saw_onstop = true;
            }
        }
        state.current.burial_logs = state.distinct_burials.len();
        state.current.distinct_burial_logs = state.distinct_burials.len();
        try_advance_run_locked(&mut state);
    }

    pub fn note_paint(&self, present: bool) {
        let mut state = self.inner.lock().unwrap();
        if matches!(
            state.stage,
            Stage::Qualified | Stage::Failed | Stage::Disabled
        ) {
            return;
        }
        state.paint_present = present;
    }

    /// Record production load/select without Start.
    pub fn note_load(
        &self,
        registration_count: usize,
        name: &str,
        version: &str,
        selected: bool,
        running: bool,
    ) {
        let mut state = self.inner.lock().unwrap();
        if matches!(
            state.stage,
            Stage::Qualified | Stage::Failed | Stage::Disabled
        ) {
            return;
        }
        state.registration_count = registration_count;
        state.script_name = name.to_string();
        if !version.is_empty() {
            state.script_version = version.to_string();
        }
        state.selected = selected;
        state.running = running;
        if running {
            state.auto_start = true;
            fail_locked(
                &mut state,
                "external loader auto-started; load must select without Start",
            );
            return;
        }
        if registration_count != 1 {
            fail_locked(
                &mut state,
                format!("external loader registration count {registration_count}, expected 1"),
            );
            return;
        }
        if name != SCRIPT_NAME {
            fail_locked(
                &mut state,
                format!("external loader registered {name:?}, expected {SCRIPT_NAME}"),
            );
            return;
        }
        if !selected {
            fail_locked(
                &mut state,
                "external loader did not select the profile script",
            );
            return;
        }
        if !(state.scene.ingame && state.scene.scene_state == 2) {
            fail_locked(
                &mut state,
                "external loader load before ingame && scene_state == 2",
            );
            return;
        }
        state.completed = Some(Operation::LoadRawTs);
        state.requested = Some(Operation::Start);
        state.stage = Stage::Start;
    }

    /// Freeze the last inventory/prayer observation immediately before Start.
    pub fn begin_start(&self, now: Instant) -> Result<(), String> {
        let mut state = self.inner.lock().unwrap();
        match state.stage {
            Stage::Start => {}
            Stage::Disabled => return Ok(()),
            Stage::Failed => {
                return Err(state
                    .failure
                    .clone()
                    .unwrap_or_else(|| "external loader already failed".into()))
            }
            other => {
                return Err(format!(
                    "external loader Start from stage {other:?}; load must complete first"
                ))
            }
        }
        if state.auto_start || state.running {
            let error = "external loader Start refused: already running".to_string();
            fail_locked(&mut state, error.clone());
            return Err(error);
        }
        if !(state.scene.ingame && state.scene.scene_state == 2) {
            let error = "external loader Start without ingame && scene_state == 2".to_string();
            fail_locked(&mut state, error.clone());
            return Err(error);
        }
        state.initial = Some(state.current);
        state.start_at = Some(now);
        state.running = true;
        state.completed = Some(Operation::Start);
        state.requested = Some(Operation::ObserveBurials);
        state.stage = Stage::Run;
        Ok(())
    }

    pub fn fail_start(&self, error: impl Into<String>) {
        self.fail(error);
    }

    pub fn poll_deadlines(&self, now: Instant) {
        let mut state = self.inner.lock().unwrap();
        if matches!(
            state.stage,
            Stage::Qualified | Stage::Failed | Stage::Disabled
        ) {
            return;
        }
        if state.stage == Stage::Run {
            if let Some(start) = state.start_at {
                if now.saturating_duration_since(start) >= START_DEADLINE {
                    fail_locked(
                        &mut state,
                        format!(
                            "external loader did not observe {MIN_DISTINCT_BURIALS}+ distinct burials and inventory/Prayer progress within {}ms",
                            START_DEADLINE.as_millis()
                        ),
                    );
                    return;
                }
            }
        }
        if state.stage == Stage::Stop {
            if let Some(stop) = state.stop_at {
                if now.saturating_duration_since(stop) >= STOP_DEADLINE {
                    fail_locked(
                        &mut state,
                        format!(
                            "external loader Stop did not finish inside {}ms",
                            STOP_DEADLINE.as_millis()
                        ),
                    );
                }
            }
        }
    }

    pub fn request_stop(&self, now: Instant) {
        let mut state = self.inner.lock().unwrap();
        if state.stage != Stage::Run {
            return;
        }
        state.stop_at = Some(now);
        state.requested = Some(Operation::Stop);
        state.stage = Stage::Stop;
    }

    pub fn note_stop(&self, now: Instant, idle: bool, paint_present: bool) {
        let mut state = self.inner.lock().unwrap();
        if matches!(
            state.stage,
            Stage::Qualified | Stage::Failed | Stage::Disabled
        ) {
            return;
        }
        if state.stage != Stage::Stop {
            fail_locked(&mut state, "external loader Stop noted outside Stop stage");
            return;
        }
        let elapsed = state
            .stop_at
            .map(|t| now.saturating_duration_since(t))
            .unwrap_or_default();
        state.stop_elapsed_ms = Some(elapsed.as_millis());
        state.running = !idle;
        state.paint_present = paint_present;
        if elapsed > STOP_DEADLINE {
            fail_locked(
                &mut state,
                format!(
                    "external loader Stop took {}ms, deadline {}ms",
                    elapsed.as_millis(),
                    STOP_DEADLINE.as_millis()
                ),
            );
            return;
        }
        if !idle {
            fail_locked(&mut state, "external loader Stop did not reach Idle");
            return;
        }
        if paint_present {
            fail_locked(&mut state, "external loader Stop left Canvas paint live");
            return;
        }
        if !state.saw_onstop {
            fail_locked(
                &mut state,
                "external loader Stop did not deliver onStop log",
            );
            return;
        }
        state.completed = Some(Operation::Stop);
        state.requested = Some(Operation::ReloadUnchanged);
        state.stage = Stage::ReloadUnchanged;
    }

    pub fn note_reload_unchanged(&self, message: &str) {
        let mut state = self.inner.lock().unwrap();
        if matches!(
            state.stage,
            Stage::Qualified | Stage::Failed | Stage::Disabled
        ) {
            return;
        }
        if state.stage != Stage::ReloadUnchanged {
            fail_locked(
                &mut state,
                "external loader unchanged reload outside ReloadUnchanged stage",
            );
            return;
        }
        if message != NOTHING_CHANGED {
            fail_locked(
                &mut state,
                format!("external loader unchanged reload returned {message:?}, expected {NOTHING_CHANGED:?}"),
            );
            return;
        }
        state.reload_unchanged = Some(message.to_string());
        state.completed = Some(Operation::ReloadUnchanged);
        state.requested = Some(Operation::ReloadChanged);
        state.stage = Stage::ReloadChanged;
    }

    pub fn note_reload_changed(&self, registration_count: usize) {
        let mut state = self.inner.lock().unwrap();
        if matches!(
            state.stage,
            Stage::Qualified | Stage::Failed | Stage::Disabled
        ) {
            return;
        }
        if state.stage != Stage::ReloadChanged {
            fail_locked(
                &mut state,
                "external loader changed reload outside ReloadChanged stage",
            );
            return;
        }
        state.registration_after_reload = Some(registration_count);
        if registration_count != 1 {
            fail_locked(
                &mut state,
                format!(
                    "external loader changed reload duplicated registration ({registration_count})"
                ),
            );
            return;
        }
        state.completed = Some(Operation::ReloadChanged);
        state.requested = Some(Operation::Capture);
        state.stage = Stage::Capture;
    }

    pub fn note_capture_requested(&self) {
        let mut state = self.inner.lock().unwrap();
        if matches!(
            state.stage,
            Stage::Qualified | Stage::Failed | Stage::Disabled
        ) {
            return;
        }
        if state.stage != Stage::Capture {
            fail_locked(
                &mut state,
                "external loader capture requested outside Capture stage",
            );
            return;
        }
        state.capture_requested = true;
        state.completed = Some(Operation::Capture);
        state.requested = None;
        state.stage = Stage::Qualified;
        state.terminal = Some(Arc::new(receipt_locked(&state)));
    }

    pub fn qualify(&self) -> Result<Arc<Value>, String> {
        let mut state = self.inner.lock().unwrap();
        if let Some(existing) = &state.terminal {
            if state.stage == Stage::Qualified {
                return Ok(Arc::clone(existing));
            }
            if state.stage == Stage::Failed {
                return Err(state
                    .failure
                    .clone()
                    .unwrap_or_else(|| "external loader failed".into()));
            }
        }
        if state.stage != Stage::Qualified {
            let error = match &state.failure {
                Some(reason) => reason.clone(),
                None => format!("external loader stage {:?} is not qualified", state.stage),
            };
            if state.stage != Stage::Failed {
                fail_locked(&mut state, error.clone());
            }
            return Err(error);
        }
        Ok(state
            .terminal
            .clone()
            .expect("qualified watch caches a receipt"))
    }

    pub fn evidence(&self) -> Value {
        let state = self.inner.lock().unwrap();
        state
            .terminal
            .as_ref()
            .map(|v| (**v).clone())
            .unwrap_or_else(|| receipt_locked(&state))
    }
}

fn try_advance_run_locked(state: &mut WatchState) {
    if state.stage != Stage::Run {
        return;
    }
    let Some(initial) = state.initial else {
        return;
    };
    let burials = state.distinct_burials.len() >= MIN_DISTINCT_BURIALS;
    let inventory = state.current.bones < initial.bones;
    let prayer = state.current.prayer_xp > initial.prayer_xp;
    if burials && inventory && prayer {
        state.completed = Some(Operation::ObserveBurials);
        state.requested = Some(Operation::Stop);
        state.stage = Stage::Stop;
        if state.stop_at.is_none() {
            state.stop_at = Some(Instant::now());
        }
    }
}

fn fail_locked(state: &mut WatchState, reason: impl Into<String>) {
    if matches!(state.stage, Stage::Qualified | Stage::Failed) {
        return;
    }
    state.failure = Some(reason.into());
    state.stage = Stage::Failed;
    state.requested = None;
    state.terminal = Some(Arc::new(receipt_locked(state)));
}

fn receipt_locked(state: &WatchState) -> Value {
    json!({
        "stage": state.stage,
        "requested_operation": state.requested,
        "completed_operation": state.completed,
        "failure_reason": state.failure,
        "script": {
            "name": state.script_name,
            "version": state.script_version,
            "sha256": state.source_sha,
            "path": state.source_path.display().to_string(),
        },
        "registration_count": state.registration_count,
        "registration_count_after_reload": state.registration_after_reload,
        "scene_gate": state.scene,
        "initial": state.initial,
        "final": state.current,
        "start_deadline_ms": START_DEADLINE.as_millis() as u64,
        "stop_deadline_ms": STOP_DEADLINE.as_millis() as u64,
        "stop_elapsed_ms": state.stop_elapsed_ms,
        "capture_requested": state.capture_requested,
        "reload_unchanged": state.reload_unchanged,
        "auto_start": state.auto_start,
        "account": state.account,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn ready(watch: &ExternalWatch) {
        watch.configure(
            "alice",
            PathBuf::from("/tmp/ExampleBot.ts"),
            FROZEN_SHA256.into(),
        );
        watch.note_scene(true, 2);
        watch.note_inventory(BONES_COUNT, 0);
        watch.note_prereq_passed();
        watch.note_load(1, SCRIPT_NAME, SCRIPT_VERSION, true, false);
    }

    fn burial_lines(n: usize) -> Vec<String> {
        (1..=n)
            .map(|i| format!("buried bones (#{i}, +{i} prayer xp total)"))
            .collect()
    }

    fn drive_to_run(watch: &ExternalWatch, now: Instant) {
        ready(watch);
        watch.begin_start(now).unwrap();
    }

    #[test]
    fn auto_start_at_load_fails_closed() {
        let watch = ExternalWatch::default();
        watch.configure("alice", PathBuf::from("/tmp/x.ts"), FROZEN_SHA256.into());
        watch.note_scene(true, 2);
        watch.note_inventory(BONES_COUNT, 0);
        watch.note_prereq_passed();
        watch.note_load(1, SCRIPT_NAME, SCRIPT_VERSION, true, true);
        assert_eq!(watch.status(), ExternalWatchStatus::Failed);
        let reason = watch.failure().unwrap();
        assert!(reason.contains("auto-start"), "{reason}");
        assert_eq!(watch.evidence()["auto_start"], true);
    }

    #[test]
    fn registration_must_be_exactly_one() {
        let watch = ExternalWatch::default();
        watch.configure("alice", PathBuf::from("/tmp/x.ts"), FROZEN_SHA256.into());
        watch.note_scene(true, 2);
        watch.note_inventory(BONES_COUNT, 0);
        watch.note_prereq_passed();
        watch.note_load(2, SCRIPT_NAME, SCRIPT_VERSION, true, false);
        assert!(watch.failure().unwrap().contains("registration count 2"));
    }

    #[test]
    fn start_deadline_fails_without_real_progress() {
        let watch = ExternalWatch::default();
        let t0 = Instant::now();
        drive_to_run(&watch, t0);
        watch.note_logs(&burial_lines(3));
        watch.note_inventory(BONES_COUNT, 0);
        watch.poll_deadlines(t0 + START_DEADLINE);
        let reason = watch.failure().expect("deadline");
        assert!(
            reason.contains("180000") || reason.contains("distinct burials"),
            "{reason}"
        );
        assert_ne!(watch.status(), ExternalWatchStatus::Qualified);
    }

    #[test]
    fn seeded_inventory_without_logs_is_not_pass() {
        let watch = ExternalWatch::default();
        let t0 = Instant::now();
        drive_to_run(&watch, t0);
        watch.note_inventory(0, 45);
        assert_eq!(watch.requested_operation(), Some(Operation::ObserveBurials));
        assert!(watch.qualify().is_err());
    }

    #[test]
    fn stop_deadline_fails_closed() {
        let watch = ExternalWatch::default();
        let t0 = Instant::now();
        drive_to_run(&watch, t0);
        watch.note_logs(&burial_lines(10));
        watch.note_inventory(15, 45);
        assert_eq!(watch.requested_operation(), Some(Operation::Stop));
        let stop_at = t0 + Duration::from_millis(1);
        watch.request_stop(stop_at);
        watch.poll_deadlines(stop_at + STOP_DEADLINE);
        let reason = watch.failure().unwrap();
        assert!(
            reason.contains("10000") || reason.contains("Stop"),
            "{reason}"
        );
    }

    #[test]
    fn unchanged_reload_and_duplicate_registration_are_fail_closed() {
        let watch = ExternalWatch::default();
        let t0 = Instant::now();
        drive_to_run(&watch, t0);
        watch.note_logs(&burial_lines(10));
        watch.note_inventory(10, 45);
        let stop = t0 + Duration::from_millis(5);
        watch.request_stop(stop);
        watch.note_logs(&["BoneBurier stopped — 10 buried, +45 prayer xp".into()]);
        watch.note_stop(stop + Duration::from_millis(20), true, false);
        watch.note_reload_unchanged("something else");
        assert!(watch.failure().unwrap().contains("Nothing changed"));
    }

    #[test]
    fn happy_path_receipt_serializes_required_fields() {
        let watch = ExternalWatch::default();
        let t0 = Instant::now();
        drive_to_run(&watch, t0);
        watch.note_logs(&burial_lines(10));
        watch.note_inventory(12, 45);
        let stop = t0 + Duration::from_millis(8);
        watch.request_stop(stop);
        watch.note_logs(&["BoneBurier stopped — 10 buried, +45 prayer xp".into()]);
        watch.note_stop(stop + Duration::from_millis(40), true, false);
        watch.note_reload_unchanged(NOTHING_CHANGED);
        watch.note_reload_changed(1);
        watch.note_capture_requested();
        let evidence = watch.qualify().expect("qualified");
        let cached = watch.qualify().expect("cached");
        assert!(Arc::ptr_eq(&evidence, &cached));
        for key in [
            "stage",
            "requested_operation",
            "completed_operation",
            "failure_reason",
            "script",
            "registration_count",
            "registration_count_after_reload",
            "scene_gate",
            "initial",
            "final",
            "start_deadline_ms",
            "stop_deadline_ms",
            "stop_elapsed_ms",
            "capture_requested",
            "reload_unchanged",
            "auto_start",
        ] {
            assert!(evidence.get(key).is_some(), "missing {key} in {evidence}");
        }
        assert_eq!(evidence["stage"], "qualified");
        assert_eq!(evidence["script"]["name"], SCRIPT_NAME);
        assert_eq!(evidence["script"]["version"], SCRIPT_VERSION);
        assert_eq!(evidence["registration_count"], 1);
        assert_eq!(evidence["registration_count_after_reload"], 1);
        assert_eq!(evidence["scene_gate"]["ingame"], true);
        assert_eq!(evidence["scene_gate"]["scene_state"], 2);
        assert_eq!(evidence["initial"]["bones"], BONES_COUNT);
        assert!(evidence["final"]["distinct_burial_logs"].as_u64().unwrap() >= 10);
        assert_eq!(evidence["start_deadline_ms"], 180_000);
        assert_eq!(evidence["stop_deadline_ms"], 10_000);
        assert_eq!(evidence["capture_requested"], true);
        assert_eq!(evidence["auto_start"], false);
        assert_eq!(evidence["reload_unchanged"], NOTHING_CHANGED);
        assert!(evidence["failure_reason"].is_null());
        let _parsed: Value = serde_json::from_str(&evidence.to_string()).unwrap();
    }

    #[test]
    fn changed_reload_duplicate_registration_fails() {
        let watch = ExternalWatch::default();
        let t0 = Instant::now();
        drive_to_run(&watch, t0);
        watch.note_logs(&burial_lines(10));
        watch.note_inventory(12, 45);
        let stop = t0 + Duration::from_millis(8);
        watch.request_stop(stop);
        watch.note_logs(&["BoneBurier stopped — 10 buried, +45 prayer xp".into()]);
        watch.note_stop(stop + Duration::from_millis(40), true, false);
        watch.note_reload_unchanged(NOTHING_CHANGED);
        watch.note_reload_changed(2);
        assert!(watch.failure().unwrap().contains("duplicated registration"));
    }

    #[test]
    fn source_sha256_matches_frozen_constant_format() {
        assert_eq!(source_sha256(b"not-the-bot").len(), 64);
        assert_ne!(source_sha256(b"a"), FROZEN_SHA256);
    }

    #[test]
    fn tracked_fixture_matches_frozen_sha() {
        let path = default_frozen_source();
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(source_sha256(&bytes), FROZEN_SHA256);
    }

    #[test]
    fn resolve_source_refuses_relative_paths() {
        let err = resolve_source(Some(Path::new("ExampleBot.ts"))).unwrap_err();
        assert!(err.contains("relative"), "{err}");
    }
}
