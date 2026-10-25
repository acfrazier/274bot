//! Small typed projections of the operator session, shared by the panel
//! and the TUI: one [`FleetRow`] per fleet member (phase, queue place,
//! script state, walking, a retained error, the current login notice, the
//! newest operation) and a [`SlotDetail`] for the selected slot only.
//!
//! [`crate::OperatorSession::poll`] refreshes them from the status rows it
//! already polled and the front end's walk arms. A row whose facts did not
//! change is left alone (no label rebuilt, no allocation), and the
//! generations move only when something a front end shows changed, so a
//! front end copies or redraws only then. Rows hold identifiers and scalar
//! summaries, never chat, paint or world data. Colours, wrapping and layout
//! stay with each front end.

use std::collections::HashMap;
use std::fmt::{self, Write as _};
use std::sync::{Arc, Mutex, TryLockError};
use std::time::Instant;

use crate::scripts::{ScriptRunState as RunState, StatusView};
use host::login_queue::AdmitBlocker;
use host_play::{Play, SlotArm, SlotStatus, StartupPhase, WalkArm};

use api::hostlog::{Level, Source};

use crate::login_notice::{LoginFailure, LoginNotice};
use crate::operations::{write_op, ActionKind, OpChange, OperationId, OperationReport, Outcome};

/// Longest error or operation reason a row keeps, in bytes; longer text
/// keeps its head and ends in `…`.
pub const REASON_CAP: usize = 256;

/// Where a slot is in its lifecycle, from the host's published facts. One
/// derivation for both front ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Phase {
    /// No worker lifetime: not started yet, or its worker ended cleanly.
    #[default]
    Offline,
    /// Client asset startup.
    Preparing,
    /// Wants a login but holds no login-queue place (retry wait).
    Waiting,
    /// Holds a login-queue place ([`FleetRow::queue`]).
    Queued,
    /// Login handshake in flight.
    Connecting,
    /// Authenticated; the scene is still loading.
    Loading,
    /// Game-ready: scripts and walks may act.
    Ready,
    /// On the title screen without a login intent: an explicit Log out, or
    /// auto-login off (also when it was turned off while a failed login
    /// waited to retry: that error stays as the row's last error). Log in
    /// brings it back.
    LoggedOut,
    /// The last login attempt failed. The host retries it after its backoff
    /// while the slot still wants the login ([`FleetRow::retrying`]), or the
    /// error withdrew the login and holds it until the operator acts (a
    /// public world preference failure).
    LoginError,
    /// This worker lifetime ended (asset startup failure, exit or panic);
    /// only an explicit Log in recreates it.
    Failed,
}

impl Phase {
    /// Short label for tables and caps.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::Preparing => "preparing",
            Self::Waiting => "waiting",
            Self::Queued => "queued",
            Self::Connecting => "logging in",
            Self::Loading => "loading",
            Self::Ready => "ready",
            Self::LoggedOut => "logged out",
            Self::LoginError => "login error",
            Self::Failed => "failed",
        }
    }

    /// A current (not historical) failure.
    pub const fn is_error(self) -> bool {
        matches!(self, Self::LoginError | Self::Failed)
    }
}

/// A login-queue place: `position` of `total`, 1-based, and at the head of
/// the login queue what keeps it from starting its login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueuePlace {
    pub position: u32,
    pub total: u32,
    blocker: Option<AdmitBlocker>,
}

impl QueuePlace {
    /// A place with no admission reason (a start queue, or behind the head).
    pub const fn new(position: u32, total: u32) -> Self {
        Self {
            position,
            total,
            blocker: None,
        }
    }

    /// The place `status` publishes; `None` when it holds none (or the
    /// published pair is not a valid place).
    pub fn of(status: &SlotStatus) -> Option<Self> {
        let (position, total) = (status.queue_position, status.queue_total);
        (position >= 1 && total >= 1 && position <= total).then_some(Self {
            position: position as u32,
            total: total as u32,
            blocker: status.queue_blocker,
        })
    }

    /// Slots ahead of this one.
    pub fn ahead(self) -> u32 {
        self.position - 1
    }

    /// What keeps the login queue's head from starting its login.
    pub fn blocker(self) -> Option<AdmitBlocker> {
        self.blocker
    }

    /// The head's reason for waiting, with the time left when the host
    /// knows it: `another bot is logging in`, `login rate cooldown (29
    /// attempts), next try in 41 s`. Writes nothing and returns `false` when
    /// there is no such reason (behind the head, the bots ahead are it).
    pub fn write_wait(self, out: &mut String, now: Instant) -> bool {
        let Some(blocker) = self.blocker else {
            return false;
        };
        let _ = match blocker {
            AdmitBlocker::Handshake => write!(out, "another bot is logging in"),
            AdmitBlocker::Retiring => write!(out, "previous login is finishing"),
            AdmitBlocker::Spacing { .. } => write!(out, "pacing logins"),
            AdmitBlocker::AddressBudget {
                attempts,
                until: None,
            } => write!(
                out,
                "login rate cooldown ({attempts} attempts) after the current login"
            ),
            AdmitBlocker::AddressBudget { attempts, .. } => {
                write!(out, "login rate cooldown ({attempts} attempts)")
            }
            AdmitBlocker::DeviceBudget { .. } => write!(out, "device login limit"),
            AdmitBlocker::ServerHold { .. } => write!(out, "server: too many login attempts"),
        };
        if let Some(until) = blocker.until() {
            let left = until.saturating_duration_since(now).as_millis();
            let _ = write!(out, ", next try in {} s", left.div_ceil(1000));
        }
        true
    }
}

impl fmt::Display for QueuePlace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} of {}", self.position, self.total)
    }
}

/// Compact state for a status dot: error, offline, busy or idle. Which
/// colour each is stays with the front end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Light {
    /// Not connected (offline, starting, queued, logging in, logged out).
    Grey,
    /// A current login error or failed worker lifetime.
    Red,
    /// Connected, no script running and no walk queued.
    Yellow,
    /// Connected with a running script or a queued walk.
    Green,
}

/// The newest operation that targeted a slot, with this slot's outcome.
#[derive(Debug, PartialEq, Eq)]
pub struct OpBrief {
    id: OperationId,
    action: ActionKind,
    outcome: Outcome,
}

impl OpBrief {
    pub fn new(id: OperationId, action: ActionKind, outcome: Outcome) -> Self {
        Self {
            id,
            action,
            outcome,
        }
    }

    /// The operation identity.
    pub fn id(&self) -> OperationId {
        self.id
    }

    /// The action this brief is for.
    pub fn action(&self) -> ActionKind {
        self.action
    }

    /// This slot's outcome for that operation.
    pub fn outcome(&self) -> &Outcome {
        &self.outcome
    }

    pub(crate) fn duplicate(&self) -> Self {
        Self {
            id: self.id,
            action: self.action,
            outcome: self.outcome.duplicate(),
        }
    }
}

impl fmt::Display for OpBrief {
    /// `op#41 Start completed`, `op#42 Log in failed: <reason>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_op(f, self.id, self.action, &self.outcome)
    }
}

/// One profile's projected row: identifiers and scalar summaries only.
/// A fleet member's row is [`Self::loaded`]; a vault profile that is not
/// loaded gets an unprojected row ([`Self::for_profile`]) in the fleet
/// model's universe.
#[derive(Debug, PartialEq)]
pub struct FleetRow {
    name: String,
    /// The profile is a fleet member (loaded), whatever its phase: an
    /// offline member is still loaded. Never inferred from the phase.
    loaded: bool,
    /// Active public world, `None` for a local profile.
    world: Option<u16>,
    phase: Phase,
    /// Login-queue place while queued (also while a login error waits for
    /// its retry in the queue).
    queue: Option<QueuePlace>,
    /// While [`Self::phase`] is [`Phase::LoginError`]: the slot still wants
    /// the login, so the host retries it after its backoff. `false` for an
    /// error that holds the login until the operator acts, and in every
    /// other phase.
    retrying: bool,
    /// The repeat guard parked the slot after repeated unexpected logouts:
    /// offline and unqueued, it may still hold a script's work, which a
    /// Log out ends.
    guard_parked: bool,
    /// Authenticated client session, independent of game readiness.
    connected: bool,
    script: RunState,
    /// The script run's published walk destination is live (tied to the run:
    /// only while the script lifecycle is live).
    script_walk: bool,
    /// A WalkTo walk is armed on the front end for this bot.
    manual_walk: bool,
    /// The lifetime's error. While [`Self::phase`] is an error phase this is
    /// the current error; otherwise it is the last login error, kept (as
    /// history) until the slot is ready, goes offline or a new worker
    /// lifetime starts. At most [`REASON_CAP`] bytes.
    error: Option<String>,
    /// The current login notice (`login {error}`, the words of the error
    /// state line): the newest failed login, kept through the retry's
    /// state changes until the bot logs in, a newer login fails, or
    /// [`crate::ERROR_MESSAGE_TIMEOUT`] passes ([`crate::login_notice`]).
    /// Boxed: a bot with no failed login pays one pointer, not a `String`.
    #[expect(
        clippy::box_collection,
        reason = "the row's one-pointer slot; a bare `String` is 24 bytes on every row"
    )]
    login_notice: Option<Box<String>>,
    /// Newest lifecycle operation on this slot (Load, Log in/out, Remove,
    /// Start/Pause/Resume/Stop, Apply to all) or failed profile write. Kept
    /// per slot, so it outlives the bounded operation history.
    last_op: Option<OpBrief>,
    /// Narrow status label (at most about 12 columns for a small queue):
    /// `queued 2/5`, `logging in`, `running`, `idle`, …
    brief: String,
}

impl Default for FleetRow {
    fn default() -> Self {
        Self {
            name: String::new(),
            loaded: false,
            world: None,
            phase: Phase::Offline,
            queue: None,
            retrying: false,
            guard_parked: false,
            connected: false,
            script: RunState::Idle,
            script_walk: false,
            manual_walk: false,
            error: None,
            login_notice: None,
            last_op: None,
            brief: String::from(Phase::Offline.label()),
        }
    }
}

impl FleetRow {
    /// Reuses the destination's buffers: a front end's retained copy costs
    /// no allocation when nothing grew. This is not [`Clone`]: seam rows
    /// do not implement it.
    pub fn copy_from(&mut self, source: &Self) {
        self.name.clone_from(&source.name);
        self.loaded = source.loaded;
        self.world = source.world;
        self.phase = source.phase;
        self.queue = source.queue;
        self.retrying = source.retrying;
        self.guard_parked = source.guard_parked;
        self.connected = source.connected;
        self.script = source.script;
        self.script_walk = source.script_walk;
        self.manual_walk = source.manual_walk;
        self.error.clone_from(&source.error);
        self.login_notice.clone_from(&source.login_notice);
        match (&mut self.last_op, source.last_op.as_ref()) {
            (Some(held), Some(brief)) => *held = brief.duplicate(),
            (_, Some(brief)) => self.last_op = Some(brief.duplicate()),
            (_, None) => self.last_op = None,
        }
        self.brief.clone_from(&source.brief);
    }

    /// A retained copy of this row, allocating only as `copy_from` does.
    pub fn snapshot(&self) -> Self {
        let mut row = Self {
            name: String::new(),
            loaded: false,
            world: None,
            phase: Phase::Offline,
            queue: None,
            retrying: false,
            guard_parked: false,
            connected: false,
            script: RunState::Idle,
            script_walk: false,
            manual_walk: false,
            error: None,
            login_notice: None,
            last_op: None,
            brief: String::new(),
        };
        row.copy_from(self);
        row
    }
}

impl FleetRow {
    /// A member's row before its first projection: loaded, offline.
    fn named(name: &str) -> Self {
        Self {
            name: name.to_string(),
            loaded: true,
            ..Self::default()
        }
    }

    /// An unprojected row for `name`: a vault profile that is not loaded
    /// (offline, no host facts).
    pub fn for_profile(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// Make this row the unprojected row of `name` (see
    /// [`Self::for_profile`]), reusing its buffers.
    pub(crate) fn reset_unloaded(&mut self, name: &str) {
        self.name.clear();
        self.name.push_str(name);
        self.loaded = false;
        self.world = None;
        self.phase = Phase::Offline;
        self.queue = None;
        self.retrying = false;
        self.guard_parked = false;
        self.connected = false;
        self.script = RunState::Idle;
        self.script_walk = false;
        self.manual_walk = false;
        self.error = None;
        self.login_notice = None;
        self.last_op = None;
        self.brief.clear();
        self.brief.push_str(Phase::Offline.label());
    }

    /// Profile / slot name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The profile is a fleet member (loaded), whatever its phase.
    pub fn loaded(&self) -> bool {
        self.loaded
    }

    /// Active public world, `None` for a local profile.
    pub fn world(&self) -> Option<u16> {
        self.world
    }

    /// Where the slot is in its lifecycle.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Login-queue place while queued.
    pub fn queue(&self) -> Option<QueuePlace> {
        self.queue
    }

    /// While [`Self::phase`] is [`Phase::LoginError`]: the slot still wants
    /// the login.
    pub fn retrying(&self) -> bool {
        self.retrying
    }

    /// The repeat guard parked the slot (see [`crate::LogoutEligibility`]).
    pub fn guard_parked(&self) -> bool {
        self.guard_parked
    }

    /// Authenticated client session, independent of game readiness.
    pub fn connected(&self) -> bool {
        self.connected
    }

    /// Script run state on this slot.
    pub fn script(&self) -> RunState {
        self.script
    }

    /// The script run's published walk is live. It belongs to the run, so it
    /// is never true while the script is idle or failed.
    pub fn script_walk(&self) -> bool {
        self.script_walk
    }

    /// A WalkTo walk is armed on the front end for this bot. It belongs to
    /// no script run, so a script Stop leaves it visible.
    pub fn manual_walk(&self) -> bool {
        self.manual_walk
    }

    /// The lifetime's error, when one is retained.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The current login notice, whatever the phase: `login {error}`, the
    /// same words as the error state line. The newest failed login keeps it
    /// through the retry's state changes until the bot logs in, a newer
    /// login fails, or [`crate::ERROR_MESSAGE_TIMEOUT`] passes.
    pub fn login_notice(&self) -> Option<&str> {
        self.login_notice.as_deref().map(String::as_str)
    }

    /// The last error as history, for a `last error` row: only while the
    /// phase is not an error (the state line then says it) and no login
    /// notice stands (the notice then says it). Both front ends show this.
    pub fn last_error(&self) -> Option<&str> {
        if self.phase.is_error() || self.login_notice.is_some() {
            return None;
        }
        self.error.as_deref()
    }

    /// Newest lifecycle operation on this slot.
    pub fn last_op(&self) -> Option<&OpBrief> {
        self.last_op.as_ref()
    }

    /// Narrow status label (`queued 2/5`, `logging in`, `running`, `idle`, …).
    pub fn brief(&self) -> &str {
        &self.brief
    }

    /// The bot is doing something the label calls `running`: a script is
    /// starting or running, its published walk is live, or a WalkTo walk is
    /// armed. Each is a separate fact; none is inferred from another.
    pub fn busy(&self) -> bool {
        matches!(self.script, RunState::Starting | RunState::Running)
            || self.script_walk
            || self.manual_walk
    }

    pub fn light(&self) -> Light {
        if self.phase.is_error() {
            Light::Red
        } else if !self.connected {
            Light::Grey
        } else if self.busy() {
            Light::Green
        } else {
            Light::Yellow
        }
    }

    /// A current error, a script error or a failed newest operation.
    pub fn has_failure(&self) -> bool {
        self.phase.is_error()
            || self.script == RunState::Error
            || self
                .last_op
                .as_ref()
                .is_some_and(|op| op.outcome.is_failed())
    }

    /// World number (when on a public world) and login phase. This is not
    /// [`Self::brief`]: a logged-in bot with no script is `ready`, not `idle`.
    /// A profile that is not loaded reads `not loaded`.
    pub fn write_world_login(&self, out: &mut String) {
        if !self.loaded {
            out.push_str(crate::fleet::NOT_LOADED);
            return;
        }
        if let Some(world) = self.world {
            let _ = write!(out, "w{world} ");
        }
        out.push_str(self.phase.label());
    }

    fn write_brief(&mut self) {
        self.brief.clear();
        if self.script == RunState::Idle {
            if let Some(place) = self.queue {
                if !matches!(
                    self.phase,
                    Phase::LoginError | Phase::Connecting | Phase::Preparing | Phase::Loading
                ) {
                    let _ = write!(self.brief, "queued {}/{}", place.position, place.total);
                    return;
                }
            }
        }
        let text = match self.phase {
            Phase::Queued => {
                let place = self.queue.unwrap_or(QueuePlace::new(0, 0));
                let _ = write!(self.brief, "queued {}/{}", place.position, place.total);
                return;
            }
            Phase::Ready if self.busy() => "running",
            Phase::Ready => "idle",
            Phase::Preparing => "starting",
            Phase::LoginError => "error",
            phase => phase.label(),
        };
        self.brief.push_str(text);
    }
}

#[cfg(any(test, feature = "test-support"))]
impl FleetRow {
    /// Fixture seam for front-end view tests: `name` in `phase` on `world`
    /// (connected while loading or ready) at `queue`, with the label those
    /// facts derive.
    pub fn fixture(
        name: &str,
        phase: Phase,
        world: Option<u16>,
        queue: Option<QueuePlace>,
    ) -> Self {
        let mut row = Self {
            name: name.to_string(),
            loaded: true,
            world,
            phase,
            queue,
            connected: matches!(phase, Phase::Loading | Phase::Ready),
            ..Self::default()
        };
        row.write_brief();
        row
    }

    /// Set this row's lifecycle phase (test fixtures).
    pub fn set_phase(&mut self, phase: Phase) {
        self.phase = phase;
        self.write_brief();
    }

    /// Set this row's script run state (test fixtures).
    pub fn set_script(&mut self, script: RunState) {
        self.script = script;
        self.write_brief();
    }

    /// Mark this row loaded or not (test fixtures).
    pub fn set_loaded(&mut self, loaded: bool) {
        self.loaded = loaded;
    }

    /// Set the retained lifetime error (test fixtures).
    pub fn set_error(&mut self, error: Option<String>) {
        self.error = error;
    }

    /// Set the current login notice line (test fixtures).
    pub fn set_login_notice(&mut self, notice: Option<String>) {
        self.login_notice = notice.map(Box::new);
    }

    /// Set the newest operation brief (test fixtures).
    pub fn set_last_op(&mut self, last_op: Option<OpBrief>) {
        self.last_op = last_op;
    }
}

/// Fleet totals for a header line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FleetCounts {
    /// Fleet members.
    pub loaded: usize,
    pub ready: usize,
    /// Members waiting for a login: queued, waiting to connect, or a failed
    /// login waiting for its retry (which also counts as failed). An error
    /// that holds the login for the operator is not waiting for a login.
    pub queued: usize,
    /// Members with [`FleetRow::has_failure`].
    pub failed: usize,
}

/// The selected slot's detail. Only this slot is materialized.
#[derive(Debug, Default, PartialEq)]
pub struct SlotDetail {
    row: FleetRow,
    /// One status line for the phase: `ingame scene 2`, `waiting in login
    /// queue (2/5)`, `login code 3: …`, …
    state: String,
    /// When the in-flight phase began, for an elapsed timer; `None` when no
    /// timer applies (ready, logged out, errors, a server countdown).
    since: Option<Instant>,
    /// Local-player name, empty until the player is observed.
    player: String,
    /// Published tile `(x, z, level)`; `-1` fields when not observed.
    tile: (i32, i32, i32),
    /// The tile routing may start from: only while ready.
    ready_tile: Option<(i32, i32, i32)>,
    /// Open main modal interface, -1 when none.
    modal: i32,
    /// `holding` while the welcome modal holds script work, or the bounded
    /// dismiss failure.
    welcome: Option<String>,
    /// Random-event guardian status: `dialog: mysterious old man`, with
    /// `(hold)` while the slot holds on it and `(off)` when the profile
    /// toggle is off (detection still runs).
    random: Option<String>,
    /// The profile's last successful script assignment (display name).
    card: Option<String>,
    /// Only focused detail retains the richer native output; fleet rows stay scalar.
    native_status: Option<Arc<script::native::ScriptStatus>>,
    /// Retained native/v2 walk reason; only the focused detail projects it.
    walk_risk: Option<Arc<str>>,
    /// Current or last manual WalkTo reason, separate from script ownership.
    manual_walk_risk: Option<Arc<str>>,
}

impl SlotDetail {
    /// Reuses the destination's buffers. Not [`Clone`]: seam views do not
    /// implement it.
    pub fn copy_from(&mut self, source: &Self) {
        self.row.copy_from(&source.row);
        self.state.clone_from(&source.state);
        self.since = source.since;
        self.player.clone_from(&source.player);
        self.tile = source.tile;
        self.ready_tile = source.ready_tile;
        self.modal = source.modal;
        self.welcome.clone_from(&source.welcome);
        self.random.clone_from(&source.random);
        self.card.clone_from(&source.card);
        self.native_status.clone_from(&source.native_status);
        self.walk_risk.clone_from(&source.walk_risk);
        self.manual_walk_risk.clone_from(&source.manual_walk_risk);
    }

    /// A retained copy of this detail.
    pub fn snapshot(&self) -> Self {
        let mut detail = Self::default();
        detail.copy_from(self);
        detail
    }

    /// The projected fleet row for this slot.
    pub fn row(&self) -> &FleetRow {
        &self.row
    }

    /// One status line for the phase.
    pub fn state(&self) -> &str {
        &self.state
    }

    /// The current login notice as a line of its own: `None` when no notice
    /// stands or when the state line already says it (an error phase that
    /// shows this same failure). Both front ends show it next to the state
    /// line, and in place of the `last error` row ([`FleetRow::last_error`]).
    pub fn login_notice_line(&self) -> Option<&str> {
        self.row
            .login_notice()
            .filter(|notice| *notice != self.state)
    }

    /// When the in-flight phase began, for an elapsed timer.
    pub fn since(&self) -> Option<Instant> {
        self.since
    }

    /// Local-player name, empty until the player is observed.
    pub fn player(&self) -> &str {
        &self.player
    }

    /// Published tile `(x, z, level)`; `-1` fields when not observed.
    pub fn tile(&self) -> (i32, i32, i32) {
        self.tile
    }

    /// The tile routing may start from: only while ready.
    pub fn ready_tile(&self) -> Option<(i32, i32, i32)> {
        self.ready_tile
    }

    /// Open main modal interface, -1 when none.
    pub fn modal(&self) -> i32 {
        self.modal
    }

    /// Welcome-modal hold or dismiss failure.
    pub fn welcome(&self) -> Option<&str> {
        self.welcome.as_deref()
    }

    /// Random-event guardian status line.
    pub fn random(&self) -> Option<&str> {
        self.random.as_deref()
    }

    /// The profile's last successful script assignment (display name).
    pub fn card(&self) -> Option<&str> {
        self.card.as_deref()
    }

    /// Retained native/v2 walk reason.
    pub fn walk_risk(&self) -> Option<&str> {
        self.walk_risk.as_deref()
    }

    /// Current or last manual WalkTo reason.
    pub fn manual_walk_risk(&self) -> Option<&str> {
        self.manual_walk_risk.as_deref()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl SlotDetail {
    /// Mutable fleet row, for test fixtures.
    pub fn row_mut(&mut self) -> &mut FleetRow {
        &mut self.row
    }

    /// Set the phase status line (test fixtures).
    pub fn set_state(&mut self, state: impl Into<String>) {
        self.state = state.into();
    }

    /// Set the open main modal interface (test fixtures).
    pub fn set_modal(&mut self, modal: i32) {
        self.modal = modal;
    }

    /// Replace the projected fleet row (test fixtures).
    pub fn set_row(&mut self, row: FleetRow) {
        self.row = row;
    }

    /// Set the published tile (test fixtures).
    pub fn set_tile(&mut self, tile: (i32, i32, i32)) {
        self.tile = tile;
    }

    /// Set the ready-tile (test fixtures).
    pub fn set_ready_tile(&mut self, tile: Option<(i32, i32, i32)>) {
        self.ready_tile = tile;
    }

    /// Set the local-player name (test fixtures).
    pub fn set_player(&mut self, player: impl Into<String>) {
        self.player = player.into();
    }

    /// Set the retained manual WalkTo reason (test fixtures).
    pub fn set_manual_walk_risk(&mut self, reason: Option<Arc<str>>) {
        self.manual_walk_risk = reason;
    }
}

impl SlotDetail {
    /// The retained native status of the focused slot; `None` for a slot
    /// that runs no native script.
    pub fn native_status(&self) -> Option<StatusView<'_>> {
        self.native_status.as_deref().map(StatusView::new)
    }

    /// Replaces the retained native status (fixtures; the field is private).
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_native_status(mut self, status: Option<Arc<script::native::ScriptStatus>>) -> Self {
        self.native_status = status;
        self
    }

    /// The phase and park reason the Status card shows for this slot's
    /// script. `None` when no script runs and no terminal failure remains.
    pub fn script_phase(&self) -> Option<ScriptPhase<'_>> {
        let view = self.native_status()?;
        let status = view.status();
        if self.row.script == RunState::Idle && status.failure.is_none() {
            return None;
        }
        Some(ScriptPhase {
            label: script_status_label(self.row.script, Some(view)),
            phase: if parked(status) {
                Some("blocked")
            } else {
                text_field(status, "action_state").or_else(|| text_field(status, "phase"))
            },
            reason: park_reason(status).or_else(|| text_field(status, "waiting_for")),
        })
    }
}

/// The script line the Status card shows: the lifecycle label, the phase
/// the script is in and why it waits or parked. Borrowed from the retained
/// status, so a front end renders it without allocating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptPhase<'a> {
    label: &'static str,
    phase: Option<&'a str>,
    reason: Option<&'a str>,
}

impl<'a> ScriptPhase<'a> {
    /// The lifecycle label: `working`, `blocked`, `stopped (blocked)`, ….
    pub fn label(&self) -> &'static str {
        self.label
    }

    /// The phase text: the quest's action (`walking`, `banking`), `blocked`
    /// while parked, or the script's own phase line.
    pub fn phase(&self) -> Option<&'a str> {
        self.phase
    }

    /// Why the script is parked or waiting.
    pub fn reason(&self) -> Option<&'a str> {
        self.reason
    }
}

/// Borrowed projections, from [`crate::OperatorSession::fleet_view`].
#[derive(Clone, Copy)]
pub struct FleetView<'a> {
    views: &'a Views,
    last: Option<&'a OperationReport>,
}

impl<'a> FleetView<'a> {
    /// Moves whenever anything here changed (rows, counts or the detail).
    pub fn generation(&self) -> u64 {
        self.views.generation
    }

    /// Moves only when the rows or counts changed.
    pub fn rows_generation(&self) -> u64 {
        self.views.rows_generation
    }

    /// One row per fleet member, in load order.
    pub fn rows(&self) -> &'a [FleetRow] {
        &self.views.rows
    }

    /// Copy the rows into a front end's retained vector, reusing its row
    /// buffers: no allocation when nothing grew.
    pub fn copy_rows_into(&self, out: &mut Vec<FleetRow>) {
        let rows = self.rows();
        if out.len() > rows.len() {
            out.truncate(rows.len());
        }
        for (dest, src) in out.iter_mut().zip(rows.iter()) {
            dest.copy_from(src);
        }
        if out.len() < rows.len() {
            out.extend(rows[out.len()..].iter().map(FleetRow::snapshot));
        }
    }

    /// Copy the selected slot's detail into a front end's retained copy,
    /// reusing its buffers.
    pub fn copy_detail_into(&self, out: &mut Option<SlotDetail>) {
        match (out.as_mut(), self.detail()) {
            (Some(held), Some(detail)) => held.copy_from(detail),
            (_, Some(detail)) => *out = Some(detail.snapshot()),
            (_, None) => *out = None,
        }
    }

    /// `name`'s row: a member's, else the selected slot's.
    pub fn row(&self, name: &str) -> Option<&'a FleetRow> {
        self.views
            .rows
            .iter()
            .find(|row| row.name == name)
            .or_else(|| {
                self.views
                    .detail
                    .as_ref()
                    .map(|detail| &detail.row)
                    .filter(|row| row.name == name)
            })
    }

    pub fn counts(&self) -> FleetCounts {
        self.views.counts
    }

    pub fn detail(&self) -> Option<&'a SlotDetail> {
        self.views.detail.as_ref()
    }

    /// The newest operation of any kind.
    pub fn last_operation(&self) -> Option<&'a OperationReport> {
        self.last
    }
}

/// Per-row derivation state that is not part of the projection.
#[derive(Debug, Default)]
struct RowState {
    /// Index of the row's status in the last poll (checked first).
    hint: usize,
    /// [`SlotArm::lifetime_id`] of the worker the row last saw (0: none).
    lifetime: u64,
    /// The clock of the row's login notice ([`FleetRow::login_notice`]).
    /// Boxed and created by the bot's first failed login: a bot that never
    /// failed one pays one pointer.
    notice: Option<Box<LoginNotice>>,
}

/// The front end's walk arms (routes its WalkTo armed), keyed by slot.
pub(crate) type WalkRoutes = HashMap<String, Arc<Mutex<WalkArm>>>;

/// What a refresh reads, borrowed from the session.
pub(crate) struct Inputs<'a> {
    pub members: &'a [String],
    pub selected: Option<&'a str>,
    pub statuses: &'a [SlotStatus],
    pub play: Option<&'a Play>,
    /// The front end's walk arms, locked for this refresh.
    pub walks: Option<&'a WalkRoutes>,
    /// The selected profile's assignment display name.
    pub card: Option<&'a str>,
    /// Start-all / marked-Start places. Login-queue phases ignore this.
    pub start_places: &'a HashMap<String, QueuePlace>,
    /// The session's poll time: the login notices' clock.
    pub now: Instant,
}

/// The cached projections owned by the session.
#[derive(Debug, Default)]
pub(crate) struct Views {
    generation: u64,
    rows_generation: u64,
    rows: Vec<FleetRow>,
    states: Vec<RowState>,
    counts: FleetCounts,
    detail: Option<SlotDetail>,
    detail_state: RowState,
    /// Reused buffer for detail text built before comparing.
    scratch: String,
    /// Rows rebuilt (their facts changed) since creation: test evidence
    /// that an unchanged fleet is not re-derived.
    #[cfg(test)]
    pub(crate) rebuilt: u64,
}

impl Views {
    pub(crate) fn view<'a>(&'a self, last: Option<&'a OperationReport>) -> FleetView<'a> {
        FleetView { views: self, last }
    }

    /// Bring every projection up to date. `changes` are the operation
    /// outcomes recorded since the last refresh, in order.
    pub(crate) fn refresh(&mut self, input: &Inputs<'_>, changes: &[OpChange]) {
        let mut rows_changed = self.sync_members(input.members);
        for change in changes.iter().filter(|c| tracked(c)) {
            if let Some(row) = self.rows.iter_mut().find(|row| row.name == change.slot) {
                rows_changed |= set_last_op(row, change);
            }
        }
        for (row, state) in self.rows.iter_mut().zip(self.states.iter_mut()) {
            if update_row(row, state, input) {
                row.write_brief();
                rows_changed = true;
                #[cfg(test)]
                {
                    self.rebuilt += 1;
                }
            }
        }
        let counts = count(&self.rows);
        if counts != self.counts {
            self.counts = counts;
            rows_changed = true;
        }
        let detail_changed = self.refresh_detail(input, changes);
        if rows_changed {
            self.rows_generation += 1;
        }
        if rows_changed || detail_changed {
            self.generation += 1;
        }
    }

    /// Make the rows exactly the members, in order, keeping each surviving
    /// member's row and derivation state. Returns whether anything moved.
    fn sync_members(&mut self, members: &[String]) -> bool {
        if self.rows.len() == members.len()
            && self.rows.iter().zip(members).all(|(row, m)| row.name == *m)
        {
            return false;
        }
        let mut old_rows = std::mem::take(&mut self.rows);
        let mut old_states = std::mem::take(&mut self.states);
        for member in members {
            match old_rows.iter().position(|row| row.name == *member) {
                Some(i) => {
                    self.rows.push(old_rows.swap_remove(i));
                    self.states.push(old_states.swap_remove(i));
                }
                None => {
                    self.rows.push(FleetRow::named(member));
                    self.states.push(RowState::default());
                }
            }
        }
        true
    }

    fn refresh_detail(&mut self, input: &Inputs<'_>, changes: &[OpChange]) -> bool {
        let Some(selected) = input.selected else {
            return self.detail.take().is_some();
        };
        let mut changed = false;
        let reselected = self
            .detail
            .as_ref()
            .is_none_or(|detail| detail.row.name != selected);
        if reselected {
            self.detail = Some(SlotDetail {
                row: FleetRow::named(selected),
                ..SlotDetail::default()
            });
            self.detail_state = RowState::default();
            changed = true;
        }
        let Views {
            rows,
            detail,
            detail_state,
            scratch,
            ..
        } = self;
        let detail = detail.as_mut().expect("detail set above");
        // A member's row is already derived: share it (and its retained
        // error and operation); another slot is derived here.
        match rows.iter().find(|row| row.name == selected) {
            Some(member) => {
                if detail.row != *member {
                    detail.row.copy_from(member);
                    changed = true;
                }
            }
            None => {
                for change in changes.iter().filter(|c| tracked(c) && c.slot == selected) {
                    changed |= set_last_op(&mut detail.row, change);
                }
                if update_row(&mut detail.row, detail_state, input) {
                    detail.row.write_brief();
                    changed = true;
                }
            }
        }
        let status = find_status(input.statuses, selected, &mut detail_state.hint);
        let manual_walk_cancelled = input
            .play
            .is_some_and(|play| play.script_walk_cancelled_by_user(selected));
        changed |= update_detail(detail, status, manual_walk_cancelled, input.card, scratch);
        let native = input
            .play
            .and_then(|play| play.script_native_status(selected));
        if detail.native_status != native {
            detail.native_status = native;
            changed = true;
        }
        let risk = input.play.and_then(|play| play.script_walk_risk(selected));
        changed |= set_shared_text(&mut detail.walk_risk, risk);
        match input.walks.and_then(|walks| walks.get(selected)) {
            Some(arm) => match arm.try_lock() {
                Ok(arm) => {
                    changed |=
                        set_shared_text(&mut detail.manual_walk_risk, arm.risk_reason().cloned());
                }
                // Preserve the last answer while its slot is stepping.
                Err(TryLockError::WouldBlock) => {}
                Err(TryLockError::Poisoned(_)) => {
                    changed |= set_shared_text(&mut detail.manual_walk_risk, None);
                }
            },
            None => changed |= set_shared_text(&mut detail.manual_walk_risk, None),
        }
        changed
    }
}

/// Operations the log and the rows record: lifecycle commands, and profile
/// writes only when they failed (a parameter edit saves per keystroke).
pub(crate) fn tracked(change: &OpChange) -> bool {
    match change.action {
        ActionKind::Select => false,
        ActionKind::SaveProfile | ActionKind::DeleteProfile => change.outcome.is_failed(),
        _ => true,
    }
}

/// Where an operation line lands in the log, and how loud it is.
pub(crate) fn log_class(change: &OpChange) -> (Source, Level) {
    let source = match change.action {
        ActionKind::ScriptStart
        | ActionKind::ScriptPause
        | ActionKind::ScriptResume
        | ActionKind::ScriptStop
        | ActionKind::SyncSettings => Source::Script,
        ActionKind::Login | ActionKind::Logout => Source::Login,
        ActionKind::Select
        | ActionKind::Load
        | ActionKind::Remove
        | ActionKind::SaveProfile
        | ActionKind::DeleteProfile => Source::Host,
    };
    let level = if change.outcome.is_failed() {
        Level::Error
    } else {
        Level::Info
    };
    (source, level)
}

fn set_last_op(row: &mut FleetRow, change: &OpChange) -> bool {
    let outcome = if change.outcome.is_failed() {
        Outcome::failed(capped(change.outcome.reason().unwrap_or("")))
    } else if change.outcome.is_skipped() {
        Outcome::skipped(capped(change.outcome.reason().unwrap_or("")))
    } else {
        change.outcome.duplicate()
    };
    let brief = OpBrief::new(change.id, change.action, outcome);
    // A late settlement of an older operation never replaces a newer one.
    if row.last_op.as_ref().is_some_and(|op| op.id > brief.id) {
        return false;
    }
    if row.last_op.as_ref() == Some(&brief) {
        return false;
    }
    row.last_op = Some(brief);
    true
}

fn find_status<'a>(
    statuses: &'a [SlotStatus],
    name: &str,
    hint: &mut usize,
) -> Option<&'a SlotStatus> {
    if let Some(status) = statuses.get(*hint).filter(|s| s.username == name) {
        return Some(status);
    }
    let index = statuses.iter().position(|s| s.username == name)?;
    *hint = index;
    Some(&statuses[index])
}

/// Derive `row`'s facts from the host and the front end's walk arms.
/// Returns whether any changed (the caller then rebuilds the label).
fn update_row(row: &mut FleetRow, state: &mut RowState, input: &Inputs<'_>) -> bool {
    let status = find_status(input.statuses, &row.name, &mut state.hint);
    let arm = input.play.and_then(|play| play.arm(&row.name));
    let lifetime = arm.as_ref().map_or(0, |arm| arm.lifetime_id());
    let new_lifetime = std::mem::replace(&mut state.lifetime, lifetime) != lifetime;
    let (phase, retrying) = phase_of(status, arm.as_deref());
    let queue = match phase {
        Phase::Queued | Phase::LoginError => status.and_then(QueuePlace::of),
        _ => input.start_places.get(row.name.as_str()).copied(),
    };
    let script = match (input.play, arm.is_some()) {
        (Some(play), true) => RunState::from_script(play.script_state(&row.name)),
        _ => RunState::Idle,
    };
    let mut changed = false;
    let world = status.and_then(|s| s.world);
    let connected = status.is_some_and(|s| s.connected);
    let guard_parked = status.is_some_and(|s| {
        matches!(
            s.login_latch_reason,
            Some(host_play::LoginLatchReason::RepeatedUnexpectedLogouts { .. })
        )
    });
    // The script walk is published on the host row and belongs to the script
    // run: a destination left on the row after the run ended is not a walk.
    // A WalkTo walk lives on the front end's arm and belongs to no run.
    let script_walk = script.is_live() && status.is_some_and(|s| s.script_walk().is_some());
    let manual_walk = match input.walks.and_then(|walks| walks.get(&row.name)) {
        None => false,
        Some(walk) => match walk.try_lock() {
            Ok(walk) => walk.route.is_some(),
            // Its slot thread is stepping the walk right now: keep the
            // last answer rather than wait on the slot.
            Err(TryLockError::WouldBlock) => row.manual_walk,
            Err(TryLockError::Poisoned(_)) => false,
        },
    };
    if (
        row.world,
        row.phase,
        row.queue,
        row.retrying,
        row.guard_parked,
        row.connected,
        row.script,
        row.script_walk,
        row.manual_walk,
    ) != (
        world,
        phase,
        queue,
        retrying,
        guard_parked,
        connected,
        script,
        script_walk,
        manual_walk,
    ) {
        row.world = world;
        row.phase = phase;
        row.queue = queue;
        row.retrying = retrying;
        row.guard_parked = guard_parked;
        row.connected = connected;
        row.script = script;
        row.script_walk = script_walk;
        row.manual_walk = manual_walk;
        changed = true;
    }
    match status.and_then(|s| s.error.as_deref()) {
        Some(error) => {
            if !same_capped(row.error.as_deref(), error) {
                row.error = Some(capped(error));
                changed = true;
            }
        }
        None if new_lifetime || matches!(phase, Phase::Ready | Phase::Offline) => {
            changed |= row.error.take().is_some();
        }
        // The host cleared it for a retry: keep it as the last error.
        None => {}
    }
    // A failed login of this worker (the host's Error phase with its text)
    // is the row's login notice, apart from the phase: a retry's Connecting
    // or a parked logged out does not clear it.
    let failure = status
        .filter(|s| {
            arm.is_some()
                && phase != Phase::Failed
                && s.startup_phase == StartupPhase::Error
                && s.error.is_some()
        })
        .and_then(|s| {
            Some(LoginFailure {
                lifetime,
                at: s.startup_phase_started,
                text: row.error.as_deref()?,
            })
        });
    let logged_in = matches!(phase, Phase::Loading | Phase::Ready);
    changed |= crate::login_notice::update_slot(
        &mut state.notice,
        &mut row.login_notice,
        failure,
        logged_in,
        input.now,
    );
    changed
}

/// The single phase derivation, with whether a login error is a retry
/// wait ([`FleetRow::retrying`]). `arm` is the slot's live worker lifetime:
/// its intent (not the row's last published latch) decides whether a
/// disconnected worker is parked, and whether a failed login is retried or
/// held for the operator; both answers come from the same intent read.
fn phase_of(status: Option<&SlotStatus>, arm: Option<&SlotArm>) -> (Phase, bool) {
    let Some(s) = status else {
        return (Phase::Offline, false);
    };
    if s.worker_terminal.is_some() || s.terminal_startup_error().is_some() {
        return (Phase::Failed, false);
    }
    let Some(arm) = arm else {
        return (Phase::Offline, false);
    };
    if s.ingame {
        return (Phase::Ready, false);
    }
    if s.connected {
        return (Phase::Loading, false);
    }
    if arm.login_latched() {
        return (Phase::LoggedOut, false);
    }
    let phase = match s.startup_phase {
        StartupPhase::Preparing => Phase::Preparing,
        StartupPhase::Connecting => Phase::Connecting,
        StartupPhase::LoadingScene | StartupPhase::Ready => Phase::Loading,
        StartupPhase::Queueing | StartupPhase::Error => {
            let failed = s.error.is_some() || s.startup_phase == StartupPhase::Error;
            if !arm.login_wanted() {
                // Parked on the title. An error that withdrew the login
                // still needs the operator; any other failure is history
                // once nothing wants the login any more.
                if failed && arm.login_held_by_error() {
                    Phase::LoginError
                } else {
                    Phase::LoggedOut
                }
            } else if failed {
                // Still wanted: the host retries it after its backoff.
                return (Phase::LoginError, true);
            } else if QueuePlace::of(s).is_some() {
                Phase::Queued
            } else {
                Phase::Waiting
            }
        }
    };
    (phase, false)
}

/// Refresh the selected slot's detail fields. Text is built into `scratch`
/// and swapped in only when it differs, so a steady detail allocates
/// nothing.
fn update_detail(
    detail: &mut SlotDetail,
    status: Option<&SlotStatus>,
    manual_walk_cancelled: bool,
    card: Option<&str>,
    scratch: &mut String,
) -> bool {
    let mut changed = false;
    scratch.clear();
    write_state(scratch, &detail.row, status, manual_walk_cancelled);
    changed |= swap_if_different(&mut detail.state, scratch);
    let since = status.and_then(|s| elapsed_from(detail.row.phase, s));
    let (player, tile, ready_tile, modal) = match status {
        Some(s) => (
            s.player.as_str(),
            (s.tile_x, s.tile_z, s.tile_level),
            s.ready_tile(),
            s.main_modal_id,
        ),
        None => ("", (-1, -1, -1), None, -1),
    };
    if (detail.since, detail.tile, detail.ready_tile, detail.modal)
        != (since, tile, ready_tile, modal)
    {
        detail.since = since;
        detail.tile = tile;
        detail.ready_tile = ready_tile;
        detail.modal = modal;
        changed = true;
    }
    if detail.player != player {
        detail.player.clear();
        detail.player.push_str(player);
        changed = true;
    }
    let welcome = status.and_then(|s| match s.welcome_failure.as_deref() {
        Some(failure) => Some(failure),
        None => s.welcome_hold.then_some("holding"),
    });
    changed |= set_text(&mut detail.welcome, welcome);
    scratch.clear();
    let random = status.is_some_and(|s| write_random(scratch, &s.random));
    changed |= set_text(&mut detail.random, random.then_some(scratch.as_str()));
    changed |= set_text(&mut detail.card, card.filter(|c| !c.is_empty()));
    changed
}

fn write_state(
    out: &mut String,
    row: &FleetRow,
    status: Option<&SlotStatus>,
    manual_walk_cancelled: bool,
) {
    let error = row.error.as_deref();
    match (row.phase, status) {
        (Phase::Offline, _) | (_, None) => out.push_str("offline"),
        (Phase::Failed, _) => match error {
            Some(error) => {
                let _ = write!(out, "failed: {error}");
            }
            None => out.push_str("failed"),
        },
        (Phase::Ready, Some(_)) if manual_walk_cancelled => {
            out.push_str("cancelled by user input");
        }
        (Phase::Ready, Some(s)) => {
            let _ = write!(out, "ingame scene {}", s.scene_state);
        }
        (Phase::LoggedOut, Some(s)) => match s.login_latch_reason {
            Some(host_play::LoginLatchReason::RepeatedUnexpectedLogouts {
                count,
                window_seconds,
            }) => {
                let _ = write!(
                    out,
                    "logged out (repeat guard: {count} unexpected logouts in {window_seconds}s; Log in to retry)"
                );
            }
            _ => out.push_str("logged out (Log in to connect)"),
        },
        (Phase::LoginError, _) => match error {
            Some(error) => {
                let _ = write!(out, "login {error}");
            }
            None => out.push_str("login error"),
        },
        (Phase::Preparing, Some(s)) => {
            let message = if s.startup_progress_message.is_empty() {
                "preparing client"
            } else {
                s.startup_progress_message.as_str()
            };
            out.push_str(message);
            if let Some(percent) = s.startup_progress_percent {
                let _ = write!(out, " ({percent}%)");
            }
        }
        (Phase::Waiting, _) => out.push_str("waiting to connect"),
        (Phase::Queued, _) => match row.queue {
            Some(place) => {
                let _ = write!(
                    out,
                    "waiting in login queue ({}/{})",
                    place.position, place.total
                );
                let head = out.len();
                out.push_str(": next in line, ");
                if !place.write_wait(out, Instant::now()) {
                    out.truncate(head);
                }
            }
            None => out.push_str("waiting in login queue"),
        },
        // The host's own message here is the server's transfer countdown.
        (Phase::Connecting, Some(s)) if !s.startup_progress_message.is_empty() => {
            out.push_str(&s.startup_progress_message)
        }
        (Phase::Connecting, _) => out.push_str("logging in…"),
        (Phase::Loading, _) => out.push_str("loading scene…"),
    }
}

/// When the current in-flight phase began. A server countdown carries its
/// own time, so a rising timer beside it would contradict it.
fn elapsed_from(phase: Phase, s: &SlotStatus) -> Option<Instant> {
    match phase {
        Phase::Preparing | Phase::Waiting | Phase::Queued | Phase::Loading => {
            Some(s.startup_phase_started)
        }
        Phase::Connecting if s.startup_progress_message.is_empty() => Some(s.startup_phase_started),
        _ => None,
    }
}

/// `dialog: mysterious old man (hold) (off)`. Returns false when nothing
/// is detected.
fn write_random(out: &mut String, random: &host_play::RandomStatus) -> bool {
    let Some(kind) = random.kind else {
        return false;
    };
    let _ = write!(
        out,
        "{}: {}",
        random_kind_name(kind),
        random.name.as_deref().unwrap_or("?")
    );
    if random.hold {
        out.push_str(" (hold)");
    }
    if !random.toggle {
        out.push_str(" (off)");
    }
    true
}

/// Guardian kind names (kebab-case).
pub fn random_kind_name(kind: api::RandomKind) -> &'static str {
    use api::RandomKind::*;
    match kind {
        Dialog => "dialog",
        Pick => "pick",
        Evade => "evade",
        Maze => "maze",
        Mime => "mime",
        Box => "box",
        Lamp => "lamp",
        Hazard => "hazard",
        LostTool => "lost-tool",
        LostGear => "lost-gear",
    }
}

/// The script lifecycle label for a state.
pub const fn run_state_label(state: RunState) -> &'static str {
    match state {
        RunState::Idle => "idle",
        RunState::Starting => "starting",
        RunState::Running => "running",
        RunState::Paused => "paused",
        RunState::Stopping => "stopping",
        RunState::Error => "error",
    }
}

/// Terminal native failures remain visible after the lifecycle stops.
/// Waiting is a live phase; it does not retire the run.
pub fn script_status_label(state: RunState, status: Option<StatusView<'_>>) -> &'static str {
    let status = status.map(|status| status.status());
    if state == RunState::Idle
        && status.is_some_and(|status| {
            status.phase == script::native::NativePhase::Blocked && status.failure.is_some()
        })
    {
        return "stopped (blocked)";
    }
    if state != RunState::Running {
        return run_state_label(state);
    }
    let Some(status) = status else {
        return "running";
    };
    if parked(status) {
        return "blocked";
    }
    match status.phase {
        script::native::NativePhase::Preparing => "preparing",
        script::native::NativePhase::Working => "working",
        script::native::NativePhase::Waiting => "waiting",
        script::native::NativePhase::Blocked => "blocked",
        script::native::NativePhase::Complete => "complete",
    }
}

/// The current refusal or wait takes priority over a normal action caption.
pub fn script_status_reason(status: StatusView<'_>) -> Option<&str> {
    let status = status.status();
    park_reason(status)
        .or_else(|| text_field(status, "waiting_for"))
        .or_else(|| text_field(status, "action_state"))
}

/// The park reason: the terminal failure, or while parked the script's own
/// `block_reason`. A queue wait parks without a failure, so it needs the field.
///
/// A status can carry several `block_reason` fields. The queue copies a reason
/// from whichever quest it picked, earlier quests included, and the runner
/// appends the current quest's own park line after those, so the last one is
/// this park's reason. The first one can belong to an earlier quest.
fn park_reason(status: &script::native::ScriptStatus) -> Option<&str> {
    if let Some(failure) = status.failure.as_ref() {
        return Some(failure.message.as_ref());
    }
    if parked(status) {
        return last_text_field(status, "block_reason");
    }
    None
}

/// Whether the script is parked. A Quester queue wait parks while its phase
/// stays `Working`, so the published `action_state` is checked as well.
fn parked(status: &script::native::ScriptStatus) -> bool {
    status.phase == script::native::NativePhase::Blocked
        || text_field(status, "action_state") == Some("blocked")
}

/// The non-empty text of a status field, borrowed from the status.
fn text_field<'a>(status: &'a script::native::ScriptStatus, key: &str) -> Option<&'a str> {
    status
        .fields
        .iter()
        .find_map(|field| field_text(field, key))
}

/// The last non-empty text of a status field. The runner appends the current
/// park's own reason after any reason the queue copied from another quest.
fn last_text_field<'a>(status: &'a script::native::ScriptStatus, key: &str) -> Option<&'a str> {
    status
        .fields
        .iter()
        .rev()
        .find_map(|field| field_text(field, key))
}

/// The text of one status field, when it has the key and is non-empty.
fn field_text<'a>(field: &'a script::native::StatusField, key: &str) -> Option<&'a str> {
    match &field.value {
        script::native::StatusValue::Text(text) if field.key == key && !text.is_empty() => {
            Some(text.as_ref())
        }
        _ => None,
    }
}

/// The short text rows both front ends show under the status line, as
/// `(label, value)`. Folder Path comments are shown only with a source marker.
pub fn script_status_rows(status: StatusView<'_>) -> impl Iterator<Item = (&str, &str)> {
    let status = status.status();
    status.fields.iter().filter_map(|field| match &field.value {
        script::native::StatusValue::Text(value)
            if matches!(
                field.key,
                "display" | "queue" | "path_source" | "draft_status"
            ) || (field.key == "step_comment"
                && !value.is_empty()
                && status.fields.iter().any(|field| field.key == "path_source")) =>
        {
            Some((field.label, value.as_ref()))
        }
        _ => None,
    })
}

fn count(rows: &[FleetRow]) -> FleetCounts {
    let mut counts = FleetCounts {
        loaded: rows.len(),
        ..FleetCounts::default()
    };
    for row in rows {
        match row.phase {
            Phase::Ready => counts.ready += 1,
            Phase::Queued | Phase::Waiting => counts.queued += 1,
            // A failed login waiting for its retry; an error that holds
            // the login waits for the operator instead.
            Phase::LoginError if row.retrying => counts.queued += 1,
            _ => {}
        }
        if row.has_failure() {
            counts.failed += 1;
        }
    }
    counts
}

fn swap_if_different(current: &mut String, candidate: &mut String) -> bool {
    if current == candidate {
        return false;
    }
    std::mem::swap(current, candidate);
    true
}

fn set_shared_text(current: &mut Option<Arc<str>>, next: Option<Arc<str>>) -> bool {
    if match (current.as_ref(), next.as_ref()) {
        (Some(current), Some(next)) => Arc::ptr_eq(current, next),
        (None, None) => true,
        _ => false,
    } {
        return false;
    }
    *current = next;
    true
}

fn set_text(current: &mut Option<String>, text: Option<&str>) -> bool {
    match (current.as_mut(), text) {
        (Some(held), Some(text)) if held == text => false,
        (Some(held), Some(text)) => {
            held.clear();
            held.push_str(text);
            true
        }
        (None, Some(text)) => {
            *current = Some(text.to_string());
            true
        }
        (Some(_), None) => {
            *current = None;
            true
        }
        (None, None) => false,
    }
}

/// Where [`capped`] cuts `text` (a char boundary), or `None` when it fits.
fn cap_end(text: &str) -> Option<usize> {
    if text.len() <= REASON_CAP {
        return None;
    }
    let mut end = REASON_CAP - '…'.len_utf8();
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Some(end)
}

/// `text` cut to [`REASON_CAP`] bytes on a char boundary, ending in `…`.
fn capped(text: &str) -> String {
    let Some(end) = cap_end(text) else {
        return text.to_string();
    };
    let mut out = String::with_capacity(end + '…'.len_utf8());
    out.push_str(&text[..end]);
    out.push('…');
    out
}

/// Whether `held` is what [`capped`] makes of `text`, without allocating.
fn same_capped(held: Option<&str>, text: &str) -> bool {
    let Some(held) = held else {
        return false;
    };
    match cap_end(text) {
        None => held == text,
        Some(end) => held
            .strip_suffix('…')
            .is_some_and(|head| head == &text[..end]),
    }
}

#[cfg(test)]
#[path = "views_tests.rs"]
mod tests;
