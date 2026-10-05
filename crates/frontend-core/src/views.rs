//! Small typed projections of the operator session, shared by the panel
//! and the TUI: one [`FleetRow`] per fleet member (phase, queue place,
//! script state, walking, a retained error, the newest operation) and a
//! [`SlotDetail`] for the selected slot only.
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

use host_play::{Play, SlotArm, SlotStatus, StartupPhase, WalkArm};
use script::RunState;

use api::hostlog::{Level, Source};

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

/// A login-queue place: `position` of `total`, 1-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueuePlace {
    pub position: u32,
    pub total: u32,
}

impl QueuePlace {
    /// The place `status` publishes; `None` when it holds none (or the
    /// published pair is not a valid place).
    pub fn of(status: &SlotStatus) -> Option<Self> {
        let (position, total) = (status.queue_position, status.queue_total);
        (position >= 1 && total >= 1 && position <= total).then_some(Self {
            position: position as u32,
            total: total as u32,
        })
    }

    /// Slots ahead of this one.
    pub fn ahead(self) -> u32 {
        self.position - 1
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpBrief {
    pub id: OperationId,
    pub action: ActionKind,
    pub outcome: Outcome,
}

impl fmt::Display for OpBrief {
    /// `op#41 Start completed`, `op#42 Log in failed: <reason>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_op(f, self.id, self.action, &self.outcome)
    }
}

/// One slot's projected row: identifiers and scalar summaries only.
#[derive(Debug, PartialEq)]
pub struct FleetRow {
    pub name: String,
    /// Active public world, `None` for a local profile.
    pub world: Option<u16>,
    pub phase: Phase,
    /// Login-queue place while queued (also while a login error waits for
    /// its retry in the queue).
    pub queue: Option<QueuePlace>,
    /// While [`Self::phase`] is [`Phase::LoginError`]: the slot still wants
    /// the login, so the host retries it after its backoff. `false` for an
    /// error that holds the login until the operator acts, and in every
    /// other phase.
    pub retrying: bool,
    /// Authenticated client session, independent of game readiness.
    pub connected: bool,
    pub script: RunState,
    /// A walk is queued on the slot.
    pub walking: bool,
    /// The lifetime's error. While [`Self::phase`] is an error phase this is
    /// the current error; otherwise it is the last login error, kept (as
    /// history) until the slot is ready, goes offline or a new worker
    /// lifetime starts. At most [`REASON_CAP`] bytes.
    pub error: Option<String>,
    /// Newest lifecycle operation on this slot (Load, Log in/out, Remove,
    /// Start/Pause/Resume/Stop, Apply to all) or failed profile write. Kept
    /// per slot, so it outlives the bounded operation history.
    pub last_op: Option<OpBrief>,
    /// Narrow status label (at most about 12 columns for a small queue):
    /// `queued 2/5`, `logging in`, `running`, `idle`, …
    pub brief: String,
}

impl Default for FleetRow {
    fn default() -> Self {
        Self {
            name: String::new(),
            world: None,
            phase: Phase::Offline,
            queue: None,
            retrying: false,
            connected: false,
            script: RunState::Idle,
            walking: false,
            error: None,
            last_op: None,
            brief: String::from(Phase::Offline.label()),
        }
    }
}

impl Clone for FleetRow {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            world: self.world,
            phase: self.phase,
            queue: self.queue,
            retrying: self.retrying,
            connected: self.connected,
            script: self.script,
            walking: self.walking,
            error: self.error.clone(),
            last_op: self.last_op.clone(),
            brief: self.brief.clone(),
        }
    }

    /// Reuses the destination's buffers: a front end's retained copy costs
    /// no allocation when nothing grew.
    fn clone_from(&mut self, source: &Self) {
        self.name.clone_from(&source.name);
        self.world = source.world;
        self.phase = source.phase;
        self.queue = source.queue;
        self.retrying = source.retrying;
        self.connected = source.connected;
        self.script = source.script;
        self.walking = source.walking;
        self.error.clone_from(&source.error);
        self.last_op.clone_from(&source.last_op);
        self.brief.clone_from(&source.brief);
    }
}

impl FleetRow {
    fn named(name: &str) -> Self {
        Self {
            name: name.to_string(),
            ..Self::default()
        }
    }

    /// A script is running or a walk is queued.
    pub fn busy(&self) -> bool {
        self.script == RunState::Running || self.walking
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
                .is_some_and(|op| matches!(op.outcome, Outcome::Failed(_)))
    }

    /// World number (when on a public world) and login phase. This is not
    /// [`Self::brief`]: a logged-in bot with no script is `ready`, not `idle`.
    pub fn write_world_login(&self, out: &mut String) {
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
                let place = self.queue.unwrap_or(QueuePlace {
                    position: 0,
                    total: 0,
                });
                let _ = write!(self.brief, "queued {}/{}", place.position, place.total);
                return;
            }
            Phase::Ready
                if self.walking
                    || matches!(self.script, RunState::Running | RunState::Starting) =>
            {
                "running"
            }
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
            world,
            phase,
            queue,
            connected: matches!(phase, Phase::Loading | Phase::Ready),
            ..Self::default()
        };
        row.write_brief();
        row
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
    pub row: FleetRow,
    /// One status line for the phase: `ingame scene 2`, `waiting in login
    /// queue (2/5)`, `login code 3: …`, …
    pub state: String,
    /// When the in-flight phase began, for an elapsed timer; `None` when no
    /// timer applies (ready, logged out, errors, a server countdown).
    pub since: Option<Instant>,
    /// Local-player name, empty until the player is observed.
    pub player: String,
    /// Published tile `(x, z, level)`; `-1` fields when not observed.
    pub tile: (i32, i32, i32),
    /// The tile routing may start from: only while ready.
    pub ready_tile: Option<(i32, i32, i32)>,
    /// Open main modal interface, -1 when none.
    pub modal: i32,
    /// `holding` while the welcome modal holds script work, or the bounded
    /// dismiss failure.
    pub welcome: Option<String>,
    /// Random-event guardian status: `dialog: mysterious old man`, with
    /// `(hold)` while the slot holds on it and `(off)` when the profile
    /// toggle is off (detection still runs).
    pub random: Option<String>,
    /// The profile's last successful script assignment (display name).
    pub card: Option<String>,
    /// Only focused detail retains the richer native output; fleet rows stay scalar.
    pub native_status: Option<Arc<script::native::ScriptStatus>>,
}

impl Clone for SlotDetail {
    fn clone(&self) -> Self {
        Self {
            row: self.row.clone(),
            state: self.state.clone(),
            since: self.since,
            player: self.player.clone(),
            tile: self.tile,
            ready_tile: self.ready_tile,
            modal: self.modal,
            welcome: self.welcome.clone(),
            random: self.random.clone(),
            card: self.card.clone(),
            native_status: self.native_status.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.row.clone_from(&source.row);
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
        out.truncate(rows.len());
        let kept = out.len();
        out.clone_from_slice(&rows[..kept]);
        out.extend_from_slice(&rows[kept..]);
    }

    /// Copy the selected slot's detail into a front end's retained copy,
    /// reusing its buffers.
    pub fn copy_detail_into(&self, out: &mut Option<SlotDetail>) {
        match (out.as_mut(), self.detail()) {
            (Some(held), Some(detail)) => held.clone_from(detail),
            (_, detail) => *out = detail.cloned(),
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
#[derive(Debug, Default, Clone, Copy)]
struct RowState {
    /// Index of the row's status in the last poll (checked first).
    hint: usize,
    /// [`SlotArm::lifetime_id`] of the worker the row last saw (0: none).
    lifetime: u64,
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
                    detail.row.clone_from(member);
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
        changed
    }
}

/// Operations the log and the rows record: lifecycle commands, and profile
/// writes only when they failed (a parameter edit saves per keystroke).
pub(crate) fn tracked(change: &OpChange) -> bool {
    match change.action {
        ActionKind::Select => false,
        ActionKind::SaveProfile | ActionKind::DeleteProfile => {
            matches!(change.outcome, Outcome::Failed(_))
        }
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
    let level = match change.outcome {
        Outcome::Failed(_) => Level::Error,
        _ => Level::Info,
    };
    (source, level)
}

fn set_last_op(row: &mut FleetRow, change: &OpChange) -> bool {
    let outcome = match &change.outcome {
        Outcome::Failed(reason) => Outcome::Failed(capped(reason)),
        Outcome::Skipped(reason) => Outcome::Skipped(capped(reason)),
        other => other.clone(),
    };
    let brief = OpBrief {
        id: change.id,
        action: change.action,
        outcome,
    };
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
        (Some(play), true) => play.script_state(&row.name),
        _ => RunState::Idle,
    };
    let mut changed = false;
    let world = status.and_then(|s| s.world);
    let connected = status.is_some_and(|s| s.connected);
    // A script walk is published on the host row; a WalkTo walk lives on
    // the front end's arm.
    let walking = status.is_some_and(|s| s.walk_x != -1)
        || match input.walks.and_then(|walks| walks.get(&row.name)) {
            None => false,
            Some(walk) => match walk.try_lock() {
                Ok(walk) => walk.route.is_some(),
                // Its slot thread is stepping the walk right now: keep the
                // last answer rather than wait on the slot.
                Err(TryLockError::WouldBlock) => row.walking,
                Err(TryLockError::Poisoned(_)) => false,
            },
        };
    if (
        row.world,
        row.phase,
        row.queue,
        row.retrying,
        row.connected,
        row.script,
        row.walking,
    ) != (world, phase, queue, retrying, connected, script, walking)
    {
        row.world = world;
        row.phase = phase;
        row.queue = queue;
        row.retrying = retrying;
        row.connected = connected;
        row.script = script;
        row.walking = walking;
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
pub fn script_status_label(
    state: RunState,
    status: Option<&script::native::ScriptStatus>,
) -> &'static str {
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
    match status.map(|status| status.phase) {
        Some(script::native::NativePhase::Preparing) => "preparing",
        Some(script::native::NativePhase::Working) => "working",
        Some(script::native::NativePhase::Waiting) => "waiting",
        Some(script::native::NativePhase::Blocked) => "blocked",
        Some(script::native::NativePhase::Complete) => "complete",
        None => "running",
    }
}

/// The current refusal or wait takes priority over a normal action caption.
pub fn script_status_reason(status: &script::native::ScriptStatus) -> Option<&str> {
    status
        .failure
        .as_ref()
        .map(|failure| failure.message.as_ref())
        .or_else(|| {
            ["waiting_for", "action_state"].into_iter().find_map(|key| {
                status.fields.iter().find_map(|field| {
                    (field.key == key)
                        .then_some(&field.value)
                        .and_then(|value| match value {
                            script::native::StatusValue::Text(text) if !text.is_empty() => {
                                Some(text.as_ref())
                            }
                            _ => None,
                        })
                })
            })
        })
}

/// The short text rows both front ends show under the status line, as
/// `(label, value)`. Folder Path comments are shown only with a source marker.
pub fn script_status_rows(
    status: &script::native::ScriptStatus,
) -> impl Iterator<Item = (&str, &str)> {
    status.fields.iter().filter_map(|field| match &field.value {
        script::native::StatusValue::Text(value)
            if matches!(field.key, "display" | "queue" | "path_source")
                || (field.key == "step_comment"
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
