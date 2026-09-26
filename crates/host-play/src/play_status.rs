use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use api::host_log;
use api::hostlog::{Category, Level};
use client::client::{Client, LoginError};

use super::RandomStatus;

/// Pollable per-slot view; the slot threads update it after each frame.
/// `clone_from` reuses the destination's string buffers, so a poll that
/// copies rows into a retained vector allocates nothing in steady state.
#[derive(Debug)]
pub struct SlotStatus {
    pub username: String,
    /// Active public world number, absent for local profiles.
    pub world: Option<u16>,
    pub startup_phase: StartupPhase,
    /// Monotonic instant at which `startup_phase` began.
    pub startup_phase_started: Instant,
    /// Client asset initialization progress; cleared when initialization ends.
    pub startup_progress_percent: Option<i32>,
    pub startup_progress_message: String,
    /// When the slot's first login handshake started (after its permit).
    pub login_started: Option<Instant>,
    /// Native client session state, independent of scene/player readiness.
    pub connected: bool,
    /// Producer readiness gate for game actions: connected, scene 2, and a
    /// current local-player observation.
    pub ingame: bool,
    pub scene_state: i32,
    /// Last login error (code + message); cleared after a successful login.
    pub error: Option<String>,
    /// Terminal outcome of this worker lifetime. Login errors leave this
    /// `None`; explicit restart replaces the whole status row.
    pub worker_terminal: Option<WorkerTerminal>,
    pub runenergy: i32,
    /// Accepted auto-run sends in the current connected session; reset on
    /// every session boundary.
    pub run_sends: u32,
    /// Local-player tile (filled from `local_player` in observe).
    pub tile_x: i32,
    pub tile_z: i32,
    /// Local-player plane (`Client::minusedlevel` in observe).
    pub tile_level: i32,
    /// Local-player name, empty until `PLAYER_INFO` lands.
    pub player: String,
    /// `Client.main_modal_id` (open modal interface, -1 when none).
    pub main_modal_id: i32,
    /// Host login-readiness: script work is held while the native welcome
    /// modal is open or a bounded dismiss failed.
    pub welcome_hold: bool,
    /// Visible bounded dismiss failure; `None` while idle/settled.
    pub welcome_failure: Option<String>,
    /// Last welcome phase line for panel/TUI logs.
    pub welcome_notice: Option<String>,
    /// Queued walk target tile, -1 when idle (mirrored from the slot's
    /// traveller's route dest by the pump's per-uid nav step each
    /// observe).
    pub walk_x: i32,
    pub walk_z: i32,
    pub walk_level: i32,
    /// Place in the login FIFO while waiting for a permit: 1-based
    /// `position` of `total`; both -1 when not queued (same sentinel as
    /// the `walk_*` fields).
    pub queue_position: i32,
    pub queue_total: i32,
    /// Intentional logout latch ([`SlotArm::login_latched`]): the slot is
    /// parked on the title until the operator explicitly arms login again.
    pub login_latched: bool,
    /// Payload bytes from the current `Client.stream`; reset to zero whenever
    /// the connected session ends.
    pub bytes_in: u64,
    pub bytes_out: u64,
    /// Newest `MESSAGE_GAME` / chat-ring head (`chat_text[0]`). Used to
    /// parse `getvar` replies (`get tutorial: 1000`).
    pub chat_head: String,
    /// The random-event guardian's published status (kind/name/ours/
    /// handling/hold/toggle/claim/cooldown), copied from `Host`'s
    /// `client_frame` return each observe. The chrome contract both the
    /// panel and the TUI bind.
    pub random: RandomStatus,
    /// The slot script's latest recorded paint frame (the Load isolate
    /// forwards it after every tick that painted). Disconnect and Pause retain
    /// this shared frame; Stop or slot unload clears it from the lifecycle
    /// owner even when no online observe runs. `None` means no current paint.
    pub script_paint: Option<std::sync::Arc<script::shim::ScriptPaint>>,
}

impl Clone for SlotStatus {
    fn clone(&self) -> Self {
        Self {
            username: self.username.clone(),
            world: self.world,
            startup_phase: self.startup_phase,
            startup_phase_started: self.startup_phase_started,
            startup_progress_percent: self.startup_progress_percent,
            startup_progress_message: self.startup_progress_message.clone(),
            login_started: self.login_started,
            connected: self.connected,
            ingame: self.ingame,
            scene_state: self.scene_state,
            error: self.error.clone(),
            worker_terminal: self.worker_terminal,
            runenergy: self.runenergy,
            run_sends: self.run_sends,
            tile_x: self.tile_x,
            tile_z: self.tile_z,
            tile_level: self.tile_level,
            player: self.player.clone(),
            main_modal_id: self.main_modal_id,
            welcome_hold: self.welcome_hold,
            welcome_failure: self.welcome_failure.clone(),
            welcome_notice: self.welcome_notice.clone(),
            walk_x: self.walk_x,
            walk_z: self.walk_z,
            walk_level: self.walk_level,
            queue_position: self.queue_position,
            queue_total: self.queue_total,
            login_latched: self.login_latched,
            bytes_in: self.bytes_in,
            bytes_out: self.bytes_out,
            chat_head: self.chat_head.clone(),
            random: self.random.clone(),
            script_paint: self.script_paint.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.username.clone_from(&source.username);
        self.world.clone_from(&source.world);
        self.startup_phase.clone_from(&source.startup_phase);
        self.startup_phase_started
            .clone_from(&source.startup_phase_started);
        self.startup_progress_percent
            .clone_from(&source.startup_progress_percent);
        self.startup_progress_message
            .clone_from(&source.startup_progress_message);
        self.login_started.clone_from(&source.login_started);
        self.connected.clone_from(&source.connected);
        self.ingame.clone_from(&source.ingame);
        self.scene_state.clone_from(&source.scene_state);
        self.error.clone_from(&source.error);
        self.worker_terminal.clone_from(&source.worker_terminal);
        self.runenergy.clone_from(&source.runenergy);
        self.run_sends.clone_from(&source.run_sends);
        self.tile_x.clone_from(&source.tile_x);
        self.tile_z.clone_from(&source.tile_z);
        self.tile_level.clone_from(&source.tile_level);
        self.player.clone_from(&source.player);
        self.main_modal_id.clone_from(&source.main_modal_id);
        self.welcome_hold.clone_from(&source.welcome_hold);
        self.welcome_failure.clone_from(&source.welcome_failure);
        self.welcome_notice.clone_from(&source.welcome_notice);
        self.walk_x.clone_from(&source.walk_x);
        self.walk_z.clone_from(&source.walk_z);
        self.walk_level.clone_from(&source.walk_level);
        self.queue_position.clone_from(&source.queue_position);
        self.queue_total.clone_from(&source.queue_total);
        self.login_latched.clone_from(&source.login_latched);
        self.bytes_in.clone_from(&source.bytes_in);
        self.bytes_out.clone_from(&source.bytes_out);
        self.chat_head.clone_from(&source.chat_head);
        self.script_paint.clone_from(&source.script_paint);
        let (random, from) = (&mut self.random, &source.random);
        random.kind.clone_from(&from.kind);
        random.name.clone_from(&from.name);
        random.ours = from.ours;
        random.handling = from.handling;
        random.hold = from.hold;
        random.toggle = from.toggle;
        random.claim.clone_from(&from.claim);
        random.cooldown = from.cooldown;
    }
}

/// Status rows are display observations, not a transactional data structure:
/// each update is a sequence of independent field assignments. If a worker
/// unwinds while holding the mutex, the first reader may therefore recover
/// the rows and clear poison while still holding the guard. This prevents one
/// failed slot from cascading through the UI and unrelated workers before its
/// terminal publisher runs.
pub(super) fn lock_statuses(statuses: &Mutex<Vec<SlotStatus>>) -> MutexGuard<'_, Vec<SlotStatus>> {
    match statuses.lock() {
        Ok(rows) => rows,
        Err(poisoned) => {
            let rows = poisoned.into_inner();
            statuses.clear_poison();
            rows
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupPhase {
    Preparing,
    Queueing,
    Connecting,
    LoadingScene,
    Ready,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerTerminal {
    Failed,
    Panicked,
}

impl SlotStatus {
    /// Wall member is online: every slot is a full `Client` now (no lean
    /// special case), so a slot is up when the scene is built.
    pub fn is_up(&self) -> bool {
        self.ingame && self.scene_state == 2
    }

    /// Current producer-authorized routing tile. Disconnected/loading rows,
    /// reset sentinels, and invalid client planes have no usable origin.
    pub fn ready_tile(&self) -> Option<(i32, i32, i32)> {
        (self.ingame
            && self.tile_x >= 0
            && self.tile_z >= 0
            && (self.tile_x != 0 || self.tile_z != 0)
            && (0..=3).contains(&self.tile_level))
        .then_some((self.tile_x, self.tile_z, self.tile_level))
    }

    /// A preparation failure that is terminal for the current slot lifetime.
    /// Login errors also use `StartupPhase::Error`, but are retryable and do
    /// not carry this producer-owned asset-initialization fact.
    pub fn terminal_startup_error(&self) -> Option<&str> {
        if self.startup_phase != StartupPhase::Error {
            return None;
        }
        self.error.as_deref().filter(|error| {
            self.worker_terminal.is_some()
                || error.starts_with("profile asset initialization failed:")
        })
    }
}

/// Find a terminal asset-init failure among the slots owned by one run.
/// Ownership is explicit so unrelated user slots cannot fail a live watch.
pub fn owned_terminal_startup_error(
    statuses: &[SlotStatus],
    owned_names: &[String],
) -> Option<String> {
    statuses
        .iter()
        .filter(|slot| owned_names.iter().any(|name| name == &slot.username))
        .find_map(|slot| {
            slot.terminal_startup_error()
                .map(|error| format!("{}: {error}", slot.username))
        })
}

/// Absolute world tile from the scene origin plus the local-player route
/// head (`route_x[0]` / `route_z[0]`). Scene pixels (`lp.x` / `lp.z`) are
/// 128× these; WalkTo and the picker need world tiles.
pub fn player_world_tile(
    map_build_base_x: i32,
    map_build_base_z: i32,
    route_x: i32,
    route_z: i32,
) -> (i32, i32) {
    (map_build_base_x + route_x, map_build_base_z + route_z)
}

/// The local player's world tile `(x, z, level)` from the scene origin,
/// route head, and [`Client::minusedlevel`] — the same level
/// [`GameSnapshot::tile`] and [`nav::traveller::Traveller::follow`] use.
pub fn player_here_tile(c: &Client) -> Option<(i32, i32, i32)> {
    c.local_player.as_ref().map(|lp| {
        let (tx, tz) = player_world_tile(
            c.map_build_base_x,
            c.map_build_base_z,
            lp.route_x[0],
            lp.route_z[0],
        );
        (tx, tz, c.minusedlevel)
    })
}

/// Defaults match the derived `Default` for every field except the queued
/// walk tile, which starts `-1` (none) instead of `0`.
impl Default for SlotStatus {
    fn default() -> Self {
        Self {
            username: String::new(),
            world: None,
            startup_phase: StartupPhase::Preparing,
            startup_phase_started: Instant::now(),
            startup_progress_percent: None,
            startup_progress_message: String::new(),
            login_started: None,
            connected: false,
            ingame: false,
            scene_state: 0,
            error: None,
            worker_terminal: None,
            runenergy: 0,
            run_sends: 0,
            tile_x: 0,
            tile_z: 0,
            tile_level: 0,
            player: String::new(),
            main_modal_id: 0,
            welcome_hold: false,
            welcome_failure: None,
            welcome_notice: None,
            walk_x: -1,
            walk_z: -1,
            walk_level: -1,
            queue_position: -1,
            queue_total: -1,
            login_latched: false,
            bytes_in: 0,
            bytes_out: 0,
            chat_head: String::new(),
            random: RandomStatus::default(),
            script_paint: None,
        }
    }
}

/// Copy the stream byte counters from `Client` onto a `SlotStatus` row. No
/// stream → bytes stay 0.
///
/// The old draw-entry counters and frame timings are gone from `Client`
/// (M2 Task 1): `game_draw_enters`/`title_screen_draw_enters` are
/// unmaintainable through the opaque `Renderer::mainredraw`, and the
/// loop/raster/paint/skip timings are slot-local in `host`'s private
/// `SlotLoop`. The status row keeps only what `Client.stream` still
/// exposes.
pub fn copy_stream_bytes(c: &Client, s: &mut SlotStatus) {
    let (bi, bo) = c
        .stream
        .as_ref()
        .map(|st| (st.bytes_in(), st.bytes_out()))
        .unwrap_or((0, 0));
    s.bytes_in = bi;
    s.bytes_out = bo;
}

pub(super) fn mark_login_started(statuses: &Arc<Mutex<Vec<SlotStatus>>>, name: &str) {
    let mut all = lock_statuses(statuses);
    if let Some(s) = all.iter_mut().find(|s| s.username == name) {
        if s.login_started.is_none() {
            s.login_started = Some(Instant::now());
        }
        s.startup_phase = StartupPhase::Connecting;
        s.startup_phase_started = Instant::now();
        s.error = None;
    }
}

pub(super) fn publish_startup_phase(
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    name: &str,
    message: &str,
) {
    let mut all = lock_statuses(statuses);
    if let Some(s) = all.iter_mut().find(|s| s.username == name) {
        s.startup_phase = StartupPhase::Preparing;
        s.startup_phase_started = Instant::now();
        s.startup_progress_percent = None;
        s.startup_progress_message.clear();
        s.startup_progress_message.push_str(message);
    }
}

pub(super) fn publish_startup_progress(
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    name: &str,
    message: &str,
    percent: i32,
) {
    let mut all = lock_statuses(statuses);
    if let Some(s) = all.iter_mut().find(|s| s.username == name) {
        s.startup_progress_percent = Some(percent.clamp(0, 100));
        s.startup_progress_message.clear();
        s.startup_progress_message.push_str(message);
    }
}

pub(super) fn clear_startup_progress(statuses: &Arc<Mutex<Vec<SlotStatus>>>, name: &str) {
    let mut all = lock_statuses(statuses);
    if let Some(s) = all.iter_mut().find(|s| s.username == name) {
        s.startup_progress_percent = None;
        s.startup_progress_message.clear();
    }
}

pub(super) fn set_startup_phase(
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    name: &str,
    phase: StartupPhase,
) {
    let mut all = lock_statuses(statuses);
    if let Some(s) = all.iter_mut().find(|s| s.username == name) {
        s.connected = matches!(phase, StartupPhase::LoadingScene | StartupPhase::Ready);
        if s.startup_phase != phase {
            s.startup_phase = phase;
            s.startup_phase_started = Instant::now();
            host_log!(
                Category::Login,
                phase_level(phase),
                slot = name,
                "startup phase {phase:?}"
            );
        }
    }
}

/// Scene loads flip LoadingScene/Ready on every region change, so those two
/// phases log at Debug; the login path (Preparing/Queueing/Connecting/Error)
/// stays visible at the default Info filter.
fn phase_level(phase: StartupPhase) -> Level {
    match phase {
        StartupPhase::LoadingScene | StartupPhase::Ready => Level::Debug,
        StartupPhase::Preparing
        | StartupPhase::Queueing
        | StartupPhase::Connecting
        | StartupPhase::Error => Level::Info,
    }
}

pub(super) fn startup_phase_after_observation(
    current: StartupPhase,
    ready: bool,
    client_ingame: bool,
) -> Option<StartupPhase> {
    if matches!(current, StartupPhase::Preparing | StartupPhase::Error) {
        return None;
    }
    if !client_ingame {
        return Some(StartupPhase::Queueing);
    }
    Some(if ready {
        StartupPhase::Ready
    } else {
        StartupPhase::LoadingScene
    })
}

pub(super) fn apply_startup_phase(
    s: &mut SlotStatus,
    name: &str,
    ready: bool,
    client_ingame: bool,
) {
    s.connected = client_ingame;
    if let Some(next_phase) = startup_phase_after_observation(s.startup_phase, ready, client_ingame)
    {
        if s.startup_phase != next_phase {
            s.startup_phase = next_phase;
            s.startup_phase_started = Instant::now();
            host_log!(
                Category::Login,
                phase_level(next_phase),
                slot = name,
                "startup phase {next_phase:?}"
            );
        }
    }
}

pub(super) fn record_login_error(
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    name: &str,
    e: &LoginError,
) {
    let msg = format!("code {}: {}", e.code, e.mes2);
    // Code 1 is a transient retry the host owns and never publishes; any
    // other code reaches the slot log as the status row's login error.
    if e.code == 1 {
        host_log!(
            Category::Login,
            Level::Info,
            slot = name,
            "login {msg}: retrying"
        );
    } else {
        host_log!(Category::Echo, Level::Warn, slot = name, "login {msg}");
    }
    // Response 1 is a transient protocol retry owned by this host. Keep the
    // slot in Connecting rather than publishing a terminal-looking Error.
    if e.code == 1 {
        return;
    }
    let mut all = lock_statuses(statuses);
    if let Some(s) = all.iter_mut().find(|s| s.username == name) {
        s.startup_phase = StartupPhase::Error;
        s.startup_phase_started = Instant::now();
        s.error = Some(msg);
    }
}

/// Drop producer-gated observation without changing the display phase.
/// A successful login/reconnect (`Pump` session gen) must reset snapshot
/// fields without mislabeling the new session as a queue wait.
pub(super) fn reset_slot_observation(s: &mut SlotStatus) {
    s.connected = false;
    s.ingame = false;
    s.scene_state = 0;
    s.runenergy = 0;
    s.run_sends = 0;
    s.bytes_in = 0;
    s.bytes_out = 0;
    s.main_modal_id = -1;
    s.welcome_hold = false;
    s.welcome_failure = None;
    s.welcome_notice = None;
    s.tile_x = -1;
    s.tile_z = -1;
    s.tile_level = -1;
    s.player.clear();
    s.chat_head.clear();
    s.walk_x = -1;
    s.walk_z = -1;
    s.walk_level = -1;
    s.random = RandomStatus::default();
}

pub(super) fn publish_slot_observation_reset(statuses: &Arc<Mutex<Vec<SlotStatus>>>, name: &str) {
    if let Some(s) = lock_statuses(statuses)
        .iter_mut()
        .find(|s| s.username == name)
    {
        reset_slot_observation(s);
        s.connected = true;
    }
}

/// Close the producer gate before clearing any queued work. Never hold this
/// lock while locking a script: script -> statuses is the established order.
pub(super) fn publish_slot_disconnected(statuses: &Arc<Mutex<Vec<SlotStatus>>>, name: &str) {
    if let Some(s) = lock_statuses(statuses)
        .iter_mut()
        .find(|s| s.username == name)
    {
        reset_slot_observation(s);
        s.login_started = None;
        if s.startup_phase != StartupPhase::Error && s.startup_phase != StartupPhase::Queueing {
            s.startup_phase = StartupPhase::Queueing;
            s.startup_phase_started = Instant::now();
            host_log!(
                Category::Login,
                Level::Info,
                slot = name,
                "startup phase {:?}",
                StartupPhase::Queueing
            );
        }
    }
}

/// Publish a terminal worker outcome after every normal early return or
/// caught unwind. The shared guard recovers poison before mutating this
/// lifetime's row, so UI snapshots and unrelated workers remain live even
/// before terminal publication reaches this boundary.
pub(super) fn publish_worker_terminal(
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    name: &str,
    terminal: WorkerTerminal,
    detail: Option<String>,
) {
    let mut rows = lock_statuses(statuses);
    if let Some(status) = rows.iter_mut().find(|status| status.username == name) {
        reset_slot_observation(status);
        status.script_paint = None;
        status.login_started = None;
        status.queue_position = -1;
        status.queue_total = -1;
        status.startup_phase = StartupPhase::Error;
        status.startup_phase_started = Instant::now();
        status.startup_progress_percent = None;
        status.startup_progress_message.clear();
        status.worker_terminal = Some(terminal);
        if let Some(detail) = detail {
            status.error = Some(detail);
        } else if status.error.is_none() {
            status.error = Some("slot worker exited unexpectedly".to_string());
        }
    }
}

/// `session_changed` is a successful login/reconnect, not a drop. Close the
/// producer gate in both cases; only a native logout returns the banner to
/// queue/connect wait.
pub(super) fn publish_session_boundary_status(
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    name: &str,
    session_changed: bool,
    client_ingame: bool,
    snapshot_ingame: bool,
) -> bool {
    let session_boundary = session_changed || (!client_ingame && snapshot_ingame);
    if session_boundary {
        if client_ingame {
            publish_slot_observation_reset(statuses, name);
        } else {
            publish_slot_disconnected(statuses, name);
        }
    }
    session_boundary
}
