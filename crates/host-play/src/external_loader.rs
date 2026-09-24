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
/// Prerequisite 25-bones fixture capture. Distinct from the post-run terminal.
pub const PREREQ_SHOT: &str = "external_loader prereq";
/// Post-Stop/reload (or failure) native capture of the owned actor at scene 2.
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
    // Native File cards take their display name from the file stem; defineBot metadata is
    // not evaluated at Load. Keep the fixture name stable inside the process-owned directory.
    let dir = std::env::temp_dir().join(format!("274bot-external-{}", std::process::id()));
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("external loader owned directory {}: {e}", dir.display()))?;
    let dest = dir.join(format!("{SCRIPT_NAME}.ts"));
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Counters {
    pub bones: i32,
    pub prayer_xp: i32,
    pub burial_logs: usize,
    pub distinct_burial_logs: usize,
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
    identity_key: String,
    compiled_sha: String,
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
    source_sha_after: Option<String>,
    compiled_sha_after: Option<String>,
    capture_requested: bool,
    failed_at: Option<Instant>,
    cleanup_invoked: bool,
    cleanup_outcome: Option<String>,
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
            identity_key: String::new(),
            compiled_sha: String::new(),
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
            source_sha_after: None,
            compiled_sha_after: None,
            capture_requested: false,
            failed_at: None,
            cleanup_invoked: false,
            cleanup_outcome: None,
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

    /// Last requested operation that the session pump may still dispatch.
    /// Failed/qualified/disabled watches keep the diagnostic request but
    /// are not eligible for further load/Start/Stop/reload work.
    pub fn dispatch_operation(&self) -> Option<Operation> {
        let state = self.inner.lock().unwrap();
        if matches!(
            state.stage,
            Stage::Failed | Stage::Qualified | Stage::Disabled
        ) {
            return None;
        }
        match state.requested {
            Some(Operation::Capture) => None,
            other => other,
        }
    }

    pub fn needs_terminal_hold(&self) -> bool {
        let state = self.inner.lock().unwrap();
        matches!(state.stage, Stage::Capture | Stage::Qualified)
            || (state.stage == Stage::Failed && state.cleanup_outcome.is_some())
    }

    pub fn cleanup_pending(&self) -> bool {
        let state = self.inner.lock().unwrap();
        state.stage == Stage::Failed && state.cleanup_outcome.is_none()
    }

    pub fn terminal_capture_due(&self) -> bool {
        let state = self.inner.lock().unwrap();
        (state.stage == Stage::Capture
            || (state.stage == Stage::Failed && state.cleanup_outcome.is_some()))
            && !state.capture_requested
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
        fail_locked(&mut state, reason);
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

    pub fn note_inventory(&self, now: Instant, account: &str, bones: i32, prayer_xp: i32) {
        let mut state = self.inner.lock().unwrap();
        if matches!(
            state.stage,
            Stage::Qualified | Stage::Failed | Stage::Disabled
        ) {
            return;
        }
        if account != state.account {
            return;
        }
        state.current.bones = bones;
        state.current.prayer_xp = prayer_xp;
        try_advance_run_locked(&mut state, now);
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
        state.distinct_burials.clear();
        state.saw_onstop = false;
        state.current.burial_logs = 0;
        state.current.distinct_burial_logs = 0;
    }

    pub fn note_logs(&self, now: Instant, account: &str, lines: &[String]) {
        let mut state = self.inner.lock().unwrap();
        if matches!(
            state.stage,
            Stage::Qualified | Stage::Failed | Stage::Disabled
        ) {
            return;
        }
        if account != state.account {
            return;
        }
        match state.stage {
            Stage::Run | Stage::Stop => {}
            _ => return,
        }
        for line in lines {
            let msg = line.strip_prefix("script: ").unwrap_or(line.as_str());
            if state.stage == Stage::Run && msg.starts_with(BURIAL_LOG_PREFIX) {
                state.distinct_burials.insert(msg.to_string());
            }
            if state.stage == Stage::Stop && msg.contains(ONSTOP_LOG_NEEDLE) {
                state.saw_onstop = true;
            }
        }
        state.current.burial_logs = state.distinct_burials.len();
        state.current.distinct_burial_logs = state.distinct_burials.len();
        try_advance_run_locked(&mut state, now);
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
    #[allow(clippy::too_many_arguments)] // loader note packs registration/path/identity/sha fields
    pub fn note_load(
        &self,
        registration_count: usize,
        name: &str,
        path: &Path,
        identity_key: &str,
        compiled_sha: &str,
        selected_file: bool,
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
        state.identity_key = identity_key.to_string();
        state.compiled_sha = compiled_sha.to_string();
        state.selected = selected_file;
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
        if !selected_file {
            fail_locked(
                &mut state,
                "external loader did not select the exact File card",
            );
            return;
        }
        if !script::paths_match(&state.source_path.display().to_string(), path)
            && path != state.source_path.as_path()
        {
            let expected = state.source_path.display().to_string();
            fail_locked(
                &mut state,
                format!(
                    "external loader loaded {} , expected {expected}",
                    path.display()
                ),
            );
            return;
        }
        if identity_key.is_empty() {
            fail_locked(&mut state, "external loader load missing card identity");
            return;
        }
        if compiled_sha.is_empty() {
            fail_locked(&mut state, "external loader load missing compiled hash");
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
        state.distinct_burials.clear();
        state.saw_onstop = false;
        state.current.burial_logs = 0;
        state.current.distinct_burial_logs = 0;
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
                    let reason = stop_missing_facts(&state, now);
                    fail_locked(&mut state, reason);
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
        if elapsed >= STOP_DEADLINE {
            let reason = stop_missing_facts(&state, now);
            fail_locked(&mut state, reason);
            return;
        }
        if idle && !paint_present && state.saw_onstop {
            state.completed = Some(Operation::Stop);
            state.requested = Some(Operation::ReloadUnchanged);
            state.stage = Stage::ReloadUnchanged;
        }
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

    #[allow(clippy::too_many_arguments)] // reload note packs before/after sha and path fields
    pub fn note_reload_changed(
        &self,
        registration_count: usize,
        applied: bool,
        nothing_changed: bool,
        path: &Path,
        identity_key: &str,
        source_sha_before: &str,
        source_sha_after: &str,
        compiled_sha_before: &str,
        compiled_sha_after: &str,
        selected_file: bool,
        running: bool,
    ) {
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
        state.source_sha_after = Some(source_sha_after.to_string());
        state.compiled_sha_after = Some(compiled_sha_after.to_string());
        if source_sha_before == source_sha_after {
            fail_locked(
                &mut state,
                "external loader changed reload source hash did not change",
            );
            return;
        }
        if nothing_changed {
            fail_locked(
                &mut state,
                "external loader changed reload returned NothingChanged after verified byte change",
            );
            return;
        }
        if !applied {
            fail_locked(
                &mut state,
                "external loader changed reload did not apply the replacement",
            );
            return;
        }
        if compiled_sha_before == compiled_sha_after {
            fail_locked(
                &mut state,
                "external loader changed reload compiled identity did not change",
            );
            return;
        }
        if registration_count != 1 {
            fail_locked(
                &mut state,
                format!(
                    "external loader changed reload duplicated registration ({registration_count})"
                ),
            );
            return;
        }
        if !selected_file {
            fail_locked(
                &mut state,
                "external loader changed reload did not keep the exact File card selected",
            );
            return;
        }
        if running {
            fail_locked(
                &mut state,
                "external loader changed reload left execution running",
            );
            return;
        }
        if !identity_key.is_empty() {
            state.identity_key = identity_key.to_string();
        }
        if path != state.source_path.as_path()
            && !script::paths_match(&state.source_path.display().to_string(), path)
        {
            let expected = state.source_path.display().to_string();
            fail_locked(
                &mut state,
                format!(
                    "external loader changed reload path {} , expected {expected}",
                    path.display()
                ),
            );
            return;
        }
        state.compiled_sha = compiled_sha_after.to_string();
        state.completed = Some(Operation::ReloadChanged);
        state.requested = Some(Operation::Capture);
        state.stage = Stage::Capture;
    }

    /// Record that a native capture request was actually issued.
    /// Failed watches keep the original failure and only flip the receipt bit.
    pub fn note_capture_requested(&self) {
        let mut state = self.inner.lock().unwrap();
        match state.stage {
            Stage::Failed => {
                state.capture_requested = true;
                state.terminal = Some(Arc::new(receipt_locked(&state)));
            }
            Stage::Capture => {
                state.capture_requested = true;
                state.completed = Some(Operation::Capture);
                state.stage = Stage::Qualified;
                state.terminal = Some(Arc::new(receipt_locked(&state)));
            }
            Stage::Qualified | Stage::Disabled => {}
            _ => {
                fail_locked(
                    &mut state,
                    "external loader capture requested outside Capture stage",
                );
            }
        }
    }

    pub fn note_cleanup_progress(&self, now: Instant, idle: bool, stop_invoked: bool) {
        let mut state = self.inner.lock().unwrap();
        if state.stage != Stage::Failed || state.cleanup_outcome.is_some() {
            return;
        }
        if stop_invoked {
            state.cleanup_invoked = true;
        }
        if idle {
            state.cleanup_outcome = Some(if state.cleanup_invoked {
                "stopped".into()
            } else {
                "already_idle".into()
            });
            state.terminal = Some(Arc::new(receipt_locked(&state)));
            return;
        }
        let Some(failed_at) = state.failed_at else {
            return;
        };
        if now.saturating_duration_since(failed_at) >= STOP_DEADLINE {
            state.cleanup_outcome = Some("stop_timeout".into());
            state.terminal = Some(Arc::new(receipt_locked(&state)));
        }
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

fn try_advance_run_locked(state: &mut WatchState, now: Instant) {
    if state.stage != Stage::Run {
        return;
    }
    if !(state.scene.ingame && state.scene.scene_state == 2) {
        return;
    }
    let Some(start) = state.start_at else {
        return;
    };
    if now.saturating_duration_since(start) >= START_DEADLINE {
        fail_locked(
            state,
            format!(
                "external loader did not observe {MIN_DISTINCT_BURIALS}+ distinct burials and inventory/Prayer progress within {}ms",
                START_DEADLINE.as_millis()
            ),
        );
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
            state.stop_at = Some(now);
        }
    }
}

fn fail_locked(state: &mut WatchState, reason: impl Into<String>) {
    if matches!(state.stage, Stage::Qualified | Stage::Failed) {
        return;
    }
    state.failure = Some(reason.into());
    state.stage = Stage::Failed;
    state.failed_at = Some(Instant::now());
    state.terminal = Some(Arc::new(receipt_locked(state)));
}

fn stop_missing_facts(state: &WatchState, now: Instant) -> String {
    let elapsed = state
        .stop_at
        .map(|t| now.saturating_duration_since(t).as_millis())
        .unwrap_or(0);
    let mut missing = Vec::new();
    if state.running {
        missing.push("Idle");
    }
    if state.paint_present {
        missing.push("Canvas clear");
    }
    if !state.saw_onstop {
        missing.push("onStop log");
    }
    if missing.is_empty() {
        format!(
            "external loader Stop did not finish inside {}ms (elapsed {elapsed}ms)",
            STOP_DEADLINE.as_millis()
        )
    } else {
        format!(
            "external loader Stop missing {} inside {}ms (elapsed {elapsed}ms)",
            missing.join(", "),
            STOP_DEADLINE.as_millis()
        )
    }
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
            "identity_key": state.identity_key,
            "compiled_sha": state.compiled_sha,
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
        "source_sha_after": state.source_sha_after,
        "compiled_sha_after": state.compiled_sha_after,
        "cleanup_outcome": state.cleanup_outcome,
        "auto_start": state.auto_start,
        "account": state.account,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn materialized_source_loads_with_the_native_fixture_identity() {
        let source = default_frozen_source();
        let original = std::fs::read(&source).unwrap();
        let owned = materialize_owned_source(&source).unwrap();
        let scratch =
            std::env::temp_dir().join(format!("external-fixture-library-{}", std::process::id()));
        let mut library =
            script::JsLibrary::with_cache(scratch.join("registry.json"), scratch.join("cache"));
        let card = library.load(&owned).unwrap();
        assert_eq!(
            card.name, SCRIPT_NAME,
            "use the real File card, not assumed defineBot metadata"
        );
        assert_eq!(card.sha256, FROZEN_SHA256);
        assert_eq!(card.source, script::ScriptSource::File);
        assert_eq!(std::fs::read(&source).unwrap(), original);
        let watch = ExternalWatch::default();
        watch.configure("alice", owned.clone(), FROZEN_SHA256.into());
        watch.note_scene(true, 2);
        watch.note_inventory(Instant::now(), "alice", BONES_COUNT, 0);
        watch.note_prereq_passed();
        watch.note_load(
            library.cards().len(),
            &card.name,
            &card.path,
            &card.identity_key(),
            &card.sha256,
            true,
            false,
        );
        assert!(watch.failure().is_none(), "{:?}", watch.failure());
        assert_eq!(watch.dispatch_operation(), Some(Operation::Start));
        std::fs::remove_file(&owned).unwrap();
        std::fs::remove_dir(owned.parent().unwrap()).unwrap();
        std::fs::remove_dir_all(scratch).unwrap();
    }

    #[test]
    fn failed_execution_holds_terminal_capture_until_cleanup_finishes_or_times_out() {
        for timeout in [false, true] {
            let watch = ExternalWatch::default();
            let now = Instant::now();
            drive_to_run(&watch, now);
            watch.fail("original proof failure");
            watch.note_cleanup_progress(Instant::now(), false, true);
            assert!(watch.cleanup_pending());
            assert!(!watch.terminal_capture_due());
            assert!(!watch.needs_terminal_hold());
            assert!(watch.evidence()["cleanup_outcome"].is_null());
            assert_eq!(watch.evidence()["requested_operation"], "observe_burials");

            let done = Instant::now()
                + if timeout {
                    STOP_DEADLINE
                } else {
                    Duration::ZERO
                };
            watch.note_cleanup_progress(done, !timeout, false);
            assert!(!watch.cleanup_pending());
            assert!(watch.terminal_capture_due());
            assert!(watch.needs_terminal_hold());
            assert_eq!(watch.failure().as_deref(), Some("original proof failure"));
            assert_eq!(
                watch.evidence()["cleanup_outcome"],
                if timeout { "stop_timeout" } else { "stopped" }
            );
            assert!(
                watch.qualify().is_err(),
                "cleanup never changes the failed proof"
            );
        }
    }

    fn ready(watch: &ExternalWatch) {
        let now = Instant::now();
        watch.configure(
            "alice",
            PathBuf::from("/tmp/ExampleBot.ts"),
            FROZEN_SHA256.into(),
        );
        watch.note_scene(true, 2);
        watch.note_inventory(now, "alice", BONES_COUNT, 0);
        watch.note_prereq_passed();
        watch.note_load(
            1,
            SCRIPT_NAME,
            Path::new("/tmp/ExampleBot.ts"),
            "file:/tmp/ExampleBot.ts",
            "compiled-a",
            true,
            false,
        );
    }

    fn burial_lines(n: usize) -> Vec<String> {
        (1..=n)
            .map(|i| format!("buried bones (#{i}, +{i} prayer xp total)"))
            .collect()
    }

    fn observe(watch: &ExternalWatch, now: Instant, n: usize, bones: i32, xp: i32) {
        watch.note_logs(now, "alice", &burial_lines(n));
        watch.note_inventory(now, "alice", bones, xp);
    }

    fn drive_to_run(watch: &ExternalWatch, now: Instant) {
        ready(watch);
        watch.begin_start(now).unwrap();
    }

    fn drive_to_reload_changed(watch: &ExternalWatch, t0: Instant) {
        drive_to_run(watch, t0);
        let now = t0 + Duration::from_millis(8);
        observe(watch, now, 10, 12, 45);
        watch.request_stop(now);
        watch.note_logs(
            now + Duration::from_millis(5),
            "alice",
            &["BoneBurier stopped — 10 buried, +45 prayer xp".into()],
        );
        watch.note_stop(now + Duration::from_millis(40), true, false);
        watch.note_reload_unchanged(NOTHING_CHANGED);
    }

    fn reload_changed_ok(watch: &ExternalWatch) {
        watch.note_reload_changed(
            1,
            true,
            false,
            Path::new("/tmp/ExampleBot.ts"),
            "file:/tmp/ExampleBot.ts",
            "sha-before",
            "sha-after",
            "compiled-a",
            "compiled-b",
            true,
            false,
        );
    }

    #[test]
    fn auto_start_at_load_fails_closed() {
        let watch = ExternalWatch::default();
        watch.configure("alice", PathBuf::from("/tmp/x.ts"), FROZEN_SHA256.into());
        watch.note_scene(true, 2);
        watch.note_inventory(Instant::now(), "alice", BONES_COUNT, 0);
        watch.note_prereq_passed();
        watch.note_load(
            1,
            SCRIPT_NAME,
            Path::new("/tmp/x.ts"),
            "file:/tmp/x.ts",
            "compiled-a",
            true,
            true,
        );
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
        watch.note_inventory(Instant::now(), "alice", BONES_COUNT, 0);
        watch.note_prereq_passed();
        watch.note_load(
            2,
            SCRIPT_NAME,
            Path::new("/tmp/x.ts"),
            "file:/tmp/x.ts",
            "compiled-a",
            true,
            false,
        );
        assert!(watch.failure().unwrap().contains("registration count 2"));
    }

    #[test]
    fn start_deadline_fails_without_real_progress() {
        let watch = ExternalWatch::default();
        let t0 = Instant::now();
        drive_to_run(&watch, t0);
        watch.note_logs(t0, "alice", &burial_lines(3));
        watch.note_inventory(t0, "alice", BONES_COUNT, 0);
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
        watch.note_inventory(t0, "alice", 0, 45);
        assert_eq!(watch.requested_operation(), Some(Operation::ObserveBurials));
        assert!(watch.qualify().is_err());
    }

    #[test]
    fn stop_deadline_fails_closed() {
        let watch = ExternalWatch::default();
        let t0 = Instant::now();
        drive_to_run(&watch, t0);
        observe(&watch, t0, 10, 15, 45);
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
        observe(&watch, t0, 10, 10, 45);
        let stop = t0 + Duration::from_millis(5);
        watch.request_stop(stop);
        watch.note_logs(
            stop,
            "alice",
            &["BoneBurier stopped — 10 buried, +45 prayer xp".into()],
        );
        watch.note_stop(stop + Duration::from_millis(20), true, false);
        watch.note_reload_unchanged("something else");
        assert!(watch.failure().unwrap().contains("Nothing changed"));
    }

    #[test]
    fn happy_path_receipt_serializes_required_fields() {
        let watch = ExternalWatch::default();
        let t0 = Instant::now();
        drive_to_reload_changed(&watch, t0);
        reload_changed_ok(&watch);
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
        assert_eq!(
            evidence["script"]["identity_key"],
            "file:/tmp/ExampleBot.ts"
        );
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
        assert_eq!(evidence["source_sha_after"], "sha-after");
        assert_eq!(evidence["compiled_sha_after"], "compiled-b");
        assert!(evidence["failure_reason"].is_null());
        let _parsed: Value = serde_json::from_str(&evidence.to_string()).unwrap();
    }

    #[test]
    fn changed_reload_duplicate_registration_fails() {
        let watch = ExternalWatch::default();
        let t0 = Instant::now();
        drive_to_reload_changed(&watch, t0);
        watch.note_reload_changed(
            2,
            true,
            false,
            Path::new("/tmp/ExampleBot.ts"),
            "file:/tmp/ExampleBot.ts",
            "sha-before",
            "sha-after",
            "compiled-a",
            "compiled-b",
            true,
            false,
        );
        assert!(watch.failure().unwrap().contains("duplicated registration"));
    }

    #[test]
    fn changed_reload_nothing_changed_after_byte_change_fails() {
        let watch = ExternalWatch::default();
        let t0 = Instant::now();
        drive_to_reload_changed(&watch, t0);
        watch.note_reload_changed(
            1,
            false,
            true,
            Path::new("/tmp/ExampleBot.ts"),
            "file:/tmp/ExampleBot.ts",
            "sha-before",
            "sha-after",
            "compiled-a",
            "compiled-b",
            true,
            false,
        );
        let reason = watch.failure().unwrap();
        assert!(reason.contains("NothingChanged"), "{reason}");
        assert_eq!(watch.requested_operation(), Some(Operation::ReloadChanged));
        assert!(watch.dispatch_operation().is_none());
    }

    #[test]
    fn late_final_log_at_deadline_fails_closed() {
        let watch = ExternalWatch::default();
        let t0 = Instant::now();
        drive_to_run(&watch, t0);
        observe(&watch, t0 + Duration::from_millis(10), 9, 12, 45);
        assert_eq!(watch.requested_operation(), Some(Operation::ObserveBurials));
        watch.note_logs(t0 + START_DEADLINE, "alice", &burial_lines(10));
        let reason = watch.failure().expect("deadline");
        assert!(
            reason.contains("180000") || reason.contains("distinct burials"),
            "{reason}"
        );
        assert_eq!(watch.requested_operation(), Some(Operation::ObserveBurials));
    }

    #[test]
    fn late_inventory_at_deadline_fails_closed() {
        let watch = ExternalWatch::default();
        let t0 = Instant::now();
        drive_to_run(&watch, t0);
        watch.note_logs(t0 + Duration::from_millis(10), "alice", &burial_lines(10));
        watch.note_inventory(t0 + START_DEADLINE, "alice", 12, 45);
        let reason = watch.failure().expect("deadline");
        assert!(
            reason.contains("180000") || reason.contains("distinct burials"),
            "{reason}"
        );
    }

    #[test]
    fn progress_before_deadline_still_advances() {
        let watch = ExternalWatch::default();
        let t0 = Instant::now();
        drive_to_run(&watch, t0);
        observe(
            &watch,
            t0 + START_DEADLINE - Duration::from_millis(1),
            10,
            12,
            45,
        );
        assert_eq!(watch.requested_operation(), Some(Operation::Stop));
        assert!(watch.failure().is_none());
    }

    #[test]
    fn pre_start_logs_cannot_qualify_current_start() {
        let watch = ExternalWatch::default();
        watch.configure(
            "alice",
            PathBuf::from("/tmp/ExampleBot.ts"),
            FROZEN_SHA256.into(),
        );
        watch.note_scene(true, 2);
        watch.note_inventory(Instant::now(), "alice", BONES_COUNT, 0);
        watch.note_prereq_passed();
        watch.note_logs(Instant::now(), "alice", &burial_lines(10));
        watch.note_logs(
            Instant::now(),
            "alice",
            &["BoneBurier stopped — stale".into()],
        );
        watch.note_load(
            1,
            SCRIPT_NAME,
            Path::new("/tmp/ExampleBot.ts"),
            "file:/tmp/ExampleBot.ts",
            "compiled-a",
            true,
            false,
        );
        let t0 = Instant::now();
        watch.begin_start(t0).unwrap();
        watch.note_inventory(t0, "alice", 12, 45);
        assert_eq!(watch.requested_operation(), Some(Operation::ObserveBurials));
        assert_eq!(watch.evidence()["final"]["distinct_burial_logs"], 0);
    }

    #[test]
    fn stop_waits_for_onstop_within_budget() {
        let watch = ExternalWatch::default();
        let t0 = Instant::now();
        drive_to_run(&watch, t0);
        observe(&watch, t0, 10, 12, 45);
        let stop = t0 + Duration::from_millis(5);
        watch.request_stop(stop);
        watch.note_stop(stop + Duration::from_millis(20), true, false);
        assert!(watch.failure().is_none());
        assert_eq!(watch.requested_operation(), Some(Operation::Stop));
        watch.note_logs(
            stop + Duration::from_millis(30),
            "alice",
            &["BoneBurier stopped — 10 buried, +45 prayer xp".into()],
        );
        watch.note_stop(stop + Duration::from_millis(40), true, false);
        assert_eq!(
            watch.requested_operation(),
            Some(Operation::ReloadUnchanged)
        );
    }

    #[test]
    fn stop_deadline_reports_missing_facts() {
        let watch = ExternalWatch::default();
        let t0 = Instant::now();
        drive_to_run(&watch, t0);
        observe(&watch, t0, 10, 12, 45);
        let stop = t0 + Duration::from_millis(5);
        watch.request_stop(stop);
        watch.note_stop(stop + Duration::from_millis(20), true, true);
        watch.poll_deadlines(stop + STOP_DEADLINE);
        let reason = watch.failure().unwrap();
        assert!(reason.contains("onStop"), "{reason}");
        assert!(reason.contains("Canvas"), "{reason}");
        assert_eq!(watch.requested_operation(), Some(Operation::Stop));
        assert!(watch.dispatch_operation().is_none());
    }

    #[test]
    fn fail_retains_last_request() {
        let watch = ExternalWatch::default();
        let t0 = Instant::now();
        drive_to_run(&watch, t0);
        watch.poll_deadlines(t0 + START_DEADLINE);
        assert_eq!(watch.requested_operation(), Some(Operation::ObserveBurials));
        assert!(watch.dispatch_operation().is_none());
        assert_eq!(watch.evidence()["requested_operation"], "observe_burials");
    }

    #[test]
    fn capture_is_not_completed_without_an_issued_request() {
        let watch = ExternalWatch::default();
        let t0 = Instant::now();
        drive_to_reload_changed(&watch, t0);
        reload_changed_ok(&watch);
        assert!(watch.terminal_capture_due());
        assert!(watch.qualify().is_err());
        assert_eq!(watch.evidence()["capture_requested"], false);
        assert_eq!(watch.evidence()["completed_operation"], "reload_changed");
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
