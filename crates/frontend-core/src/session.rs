//! [`OperatorSession`]: the production operator lifecycle shared by the
//! panel and the TUI. It owns the unlocked vault, the [`Play`], fleet
//! membership and the logout latch, the selected bot, the per-slot surface
//! IO, pending removals, the polled status rows and operation results.
//!
//! Host-play stays authoritative for worker existence, connection and
//! readiness, the login queue, script execution and cancellation; this
//! module only decides *when* to ask it for something and reports what the
//! host then observably did. Nothing here blocks: removal is a polled clean
//! logout bounded by [`SLOT_REMOVE_TIMEOUT`], and workers are joined only
//! once finished.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use host_play::{InstancePermit, Play, SlotArm, SlotStatus};
use vault::Vault;

use crate::fleet::Fleet;
use crate::operations::{ActionKind, OperationBook, OperationId, OperationReport, Outcome};
use crate::surface::SlotSurface;

/// Clean-logout window a connected member gets on removal before its worker
/// is stopped regardless.
pub const SLOT_REMOVE_TIMEOUT: Duration = Duration::from_secs(10);

/// A removal is owned by the exact arm that received its clean-logout
/// request. `io` stays here while that worker drains so re-adding the member
/// reattaches the same surface devices without a second client lifetime.
struct PendingRemoval<Io> {
    started: Instant,
    arm: Arc<SlotArm>,
    io: Option<Io>,
    op: OperationId,
}

/// One observed status change since the previous poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transition {
    SlotUp,
    LoginError(String),
    Ingame,
    Scene(i32),
    Welcome(String),
    WelcomeFailure(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotTransition {
    pub slot: String,
    pub transition: Transition,
}

/// Selection change result. `previous` is the bot that lost selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    Unchanged,
    Changed { previous: Option<String> },
}

/// What a removal did to the selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    pub op: OperationId,
    /// The removed member was selected and its neighbour is now selected.
    pub reselected: Option<String>,
    /// The removed member was selected and no member remains to select.
    pub selection_cleared: bool,
}

/// A prepared script Start. Preparation (card, parameters, sibling modules)
/// stays with the caller; the host still owns the isolate.
pub enum ScriptStart {
    Compiled(script::CompiledId),
    Load {
        js: String,
        shape: script::LoadShape,
        bag: Option<serde_json::Map<String, serde_json::Value>>,
        siblings: Vec<(String, String)>,
    },
}

/// A Load Start whose isolate setup settled (or was cancelled by removal:
/// `outcome == None`). Front ends consume these to commit assignments and
/// record load diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartSettled {
    pub slot: String,
    pub op: OperationId,
    pub outcome: Option<script::StartOutcome>,
}

pub struct OperatorSession<Io> {
    vault: Option<Vault>,
    play: Option<Play>,
    fleet: Fleet,
    selected: Option<String>,
    slots: HashMap<String, Io>,
    removals: HashMap<String, PendingRemoval<Io>>,
    statuses: Vec<SlotStatus>,
    transitions: Vec<SlotTransition>,
    operations: OperationBook,
    /// Load Starts whose setup has not settled, by slot.
    starts: HashMap<String, OperationId>,
    settled_starts: Vec<StartSettled>,
    spawn_workers: bool,
    /// Process-lifetime single-instance lock (or an explicit skip).
    _instance: InstancePermit,
}

impl<Io> OperatorSession<Io> {
    pub fn new(instance: InstancePermit) -> Self {
        Self {
            vault: None,
            play: None,
            fleet: Fleet::default(),
            selected: None,
            slots: HashMap::new(),
            removals: HashMap::new(),
            statuses: Vec::new(),
            transitions: Vec::new(),
            operations: OperationBook::default(),
            starts: HashMap::new(),
            settled_starts: Vec::new(),
            spawn_workers: true,
            _instance: instance,
        }
    }

    /// Adopt an unlocked vault and its freshly built [`Play`]. No slot is
    /// spawned here.
    pub fn start(&mut self, vault: Vault, play: Play) {
        self.statuses = play.statuses();
        self.play = Some(play);
        self.vault = Some(vault);
    }

    pub fn vault(&self) -> Option<&Vault> {
        self.vault.as_ref()
    }

    pub fn vault_mut(&mut self) -> Option<&mut Vault> {
        self.vault.as_mut()
    }

    pub fn play(&self) -> Option<&Play> {
        self.play.as_ref()
    }

    pub fn play_mut(&mut self) -> Option<&mut Play> {
        self.play.as_mut()
    }

    /// Drop the play (joins its workers). Final teardown only.
    pub fn close_play(&mut self) {
        self.play = None;
    }

    pub fn fleet(&self) -> &Fleet {
        &self.fleet
    }

    pub fn members(&self) -> &[String] {
        self.fleet.members()
    }

    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// Surface IO of every slot this session spawned (or retains for a
    /// terminal lifetime).
    pub fn slots(&self) -> &HashMap<String, Io> {
        &self.slots
    }

    pub fn slot_io(&self, name: &str) -> Option<&Io> {
        self.slots.get(name)
    }

    pub fn removal_pending(&self, name: &str) -> bool {
        self.removals.contains_key(name)
    }

    /// Rows from the last [`Self::poll`] (or [`Self::start`]).
    pub fn statuses(&self) -> &[SlotStatus] {
        &self.statuses
    }

    pub fn status(&self, name: &str) -> Option<&SlotStatus> {
        self.statuses.iter().find(|s| s.username == name)
    }

    /// Status changes observed by the last poll.
    pub fn transitions(&self) -> &[SlotTransition] {
        &self.transitions
    }

    pub fn operation(&self, id: OperationId) -> Option<&OperationReport> {
        self.operations.get(id)
    }

    pub fn last_operation(&self) -> Option<&OperationReport> {
        self.operations.last()
    }

    /// Failure text of `id`, for a front end's error line.
    pub fn failure(&self, id: OperationId) -> Option<String> {
        self.operations
            .get(id)
            .and_then(OperationReport::first_failure)
    }

    /// Load Starts settled since the last take.
    pub fn take_settled_starts(&mut self) -> Vec<StartSettled> {
        std::mem::take(&mut self.settled_starts)
    }

    /// Vault usernames plus any running slot outside the vault.
    pub fn profile_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .vault
            .as_ref()
            .map(|v| v.profiles().map(|p| p.username.clone()).collect())
            .unwrap_or_default();
        if let Some(play) = &self.play {
            for s in play.statuses() {
                if !names.contains(&s.username) {
                    names.push(s.username);
                }
            }
        }
        names
    }

    /// Live slots other than the selected one.
    pub fn background_bot_count(&self) -> usize {
        self.play
            .as_ref()
            .map(|play| play.background_bot_count(self.selected()))
            .unwrap_or(0)
    }

    /// Make `name` the selected bot: pure bookkeeping, never a spawn or a
    /// login. The host's focus follows (the preferred login owner), and both
    /// the outgoing and incoming slots are woken so a parked worker re-reads
    /// its draw state within a frame.
    pub fn select(&mut self, name: &str) -> Selection {
        if self.selected.as_deref() == Some(name) {
            return Selection::Unchanged;
        }
        let previous = self.selected.replace(name.to_string());
        if let Some(play) = self.play.as_mut() {
            play.focus(name);
            if let Some(old) = previous.as_deref() {
                play.wake(old);
            }
        }
        Selection::Changed { previous }
    }

    pub fn clear_selection(&mut self) {
        self.selected = None;
    }

    /// Control arm for a vault profile: auto-login remains a saved policy,
    /// while a fleet logout latch starts the worker in an explicit
    /// logged-out hold.
    pub fn arm_for_profile(&self, name: &str) -> Option<Arc<SlotArm>> {
        let profile = self.vault.as_ref().and_then(|v| v.get(name))?;
        let arm = SlotArm::new(profile.uid, profile.settings.auto_login);
        arm.random_events
            .store(profile.settings.random_events, Ordering::Relaxed);
        arm.lamp_auto
            .store(profile.settings.lamp_auto, Ordering::Relaxed);
        *arm.lamp_skill.lock().unwrap() = profile.settings.lamp_skill.clone();
        if self.fleet.latched(name) {
            arm.hold_logged_out();
        }
        Some(arm)
    }

    /// Spawn `name`'s worker when it has none. `arm` carries the spawn's
    /// login intent. Existing IO with no arm is a preserved terminal
    /// lifetime and restarts only when `restart_terminal` is set (explicit
    /// Log in). Without a play (pre-unlock) only the IO is registered.
    pub fn ensure_slot<S: SlotSurface<Io = Io>>(
        &mut self,
        name: &str,
        arm: Option<Arc<SlotArm>>,
        restart_terminal: bool,
        surface: &mut S,
    ) -> Result<(), String> {
        if self
            .play
            .as_ref()
            .is_some_and(|play| play.arm(name).is_some())
        {
            return Ok(());
        }
        if self.slots.contains_key(name) && (self.play.is_none() || !restart_terminal) {
            return Ok(());
        }
        let Some(mut profile) = self.vault.as_ref().and_then(|v| v.get(name)).cloned() else {
            return Ok(());
        };
        surface.lifetime_reset(name);
        let retained = self.slots.remove(name);
        let attach = surface.attach(name, &mut profile, retained);
        if !self.spawn_workers {
            if let (Some(play), Some(arm)) = (self.play.as_mut(), arm.as_ref()) {
                play.attach_arm(name, Arc::clone(arm));
            }
            self.slots.insert(name.to_string(), attach.io);
            return Ok(());
        }
        if let Some(play) = self.play.as_mut() {
            if let Err(error) = play.try_spawn_slot(profile, attach.input, attach.mailbox, arm) {
                self.slots.insert(name.to_string(), attach.io);
                return Err(error);
            }
        }
        self.slots.insert(name.to_string(), attach.io);
        Ok(())
    }

    /// Cancel a pending removal in response to an operator action. The
    /// retained IO is restored only when no replacement arm exists or when
    /// the current arm is the exact lifetime that owned the removal. Returns
    /// true only when the current arm also owns the cancelled removal, so
    /// callers may undo that removal's clean-logout request.
    fn cancel_removal(&mut self, name: &str) -> bool {
        let Some(mut pending) = self.removals.remove(name) else {
            return false;
        };
        self.operations.set(pending.op, name, Outcome::Cancelled);
        let current = self.play.as_ref().and_then(|play| play.arm(name));
        let owns_cancelled_removal = current
            .as_ref()
            .is_some_and(|arm| Arc::ptr_eq(arm, &pending.arm));
        if current.is_none() || owns_cancelled_removal {
            if let Some(io) = pending.io.take() {
                self.slots.entry(name.to_string()).or_insert(io);
            }
        }
        owns_cancelled_removal
    }

    /// Bring `name`'s worker up without changing login intent: cancel a
    /// pending removal and spawn with the profile's saved arm. A terminal
    /// lifetime is kept (its failure stays visible) until explicit Log in.
    pub fn open_slot<S: SlotSurface<Io = Io>>(
        &mut self,
        name: &str,
        surface: &mut S,
    ) -> Result<(), String> {
        self.cancel_removal(name);
        let arm = self.arm_for_profile(name);
        self.ensure_slot(name, arm, false, surface)
    }

    /// Add `name` to the fleet and bring its worker up. Login intent follows
    /// the profile's auto-login unless the logout latch blocks it. Returns
    /// whether the name was newly added.
    pub fn load<S: SlotSurface<Io = Io>>(
        &mut self,
        name: &str,
        surface: &mut S,
    ) -> (OperationId, bool) {
        let op = self.operations.open(ActionKind::Load);
        let added = self.load_member(op, name, surface);
        (op, added)
    }

    /// Load every profile (vault plus running slots) that is not a member
    /// yet. Returns the operation and how many were newly added.
    pub fn load_all<S: SlotSurface<Io = Io>>(&mut self, surface: &mut S) -> (OperationId, usize) {
        let op = self.operations.open(ActionKind::Load);
        let mut added = 0;
        for name in self.profile_names() {
            if self.load_member(op, &name, surface) {
                added += 1;
            }
        }
        (op, added)
    }

    fn load_member<S: SlotSurface<Io = Io>>(
        &mut self,
        op: OperationId,
        name: &str,
        surface: &mut S,
    ) -> bool {
        let cancelled_removal = self.cancel_removal(name);
        let added = self.fleet.add(name);
        let auto_login = self
            .vault
            .as_ref()
            .and_then(|v| v.get(name))
            .map(|p| p.settings.auto_login)
            .unwrap_or(false);
        let want_login = self.fleet.should_auto_login(name, auto_login);
        let running = self.play.as_ref().and_then(|play| play.arm(name));
        let result = match running {
            // Already running (re-load): refresh the saved auto intent. Only
            // a cancelled removal may reverse the clean logout it requested;
            // client idle-timeout and operator latches remain parked.
            Some(arm) => {
                arm.set_auto_login(auto_login);
                if cancelled_removal && want_login && arm.login_latched() {
                    arm.arm_explicit_login();
                }
                Ok(())
            }
            None => {
                let arm = self.arm_for_profile(name);
                self.ensure_slot(name, arm, false, surface)
            }
        };
        self.operations.set(op, name, completed_or_failed(result));
        added
    }

    /// Log in one member: cancel its removal, clear its latch, and arm an
    /// explicit one-shot handshake. A finished worker is reaped and a
    /// terminal lifetime recreated with its retained IO.
    pub fn login<S: SlotSurface<Io = Io>>(&mut self, name: &str, surface: &mut S) -> OperationId {
        let op = self.operations.open(ActionKind::Login);
        self.cancel_removal(name);
        self.fleet.clear_latch(name);
        if let Some(play) = self.play.as_mut() {
            play.reap_finished_workers();
        }
        let result = self.arm_login(name, surface);
        self.record_login(op, name, result);
        op
    }

    /// Log in every member. Queue membership follows worker arrival; the
    /// selected slot (or the first spawned one) is the sole priority
    /// exception.
    pub fn login_all<S: SlotSurface<Io = Io>>(&mut self, surface: &mut S) -> OperationId {
        let op = self.operations.open(ActionKind::Login);
        let names = self.fleet.members().to_vec();
        for name in &names {
            self.cancel_removal(name);
            self.fleet.clear_latch(name);
        }
        if let Some(play) = self.play.as_mut() {
            play.reap_finished_workers();
        }
        for name in &names {
            let result = self.arm_login(name, surface);
            self.record_login(op, name, result);
        }
        let head = self
            .selected
            .clone()
            .filter(|n| self.slots.contains_key(n))
            .or_else(|| self.slots.keys().next().cloned());
        if let Some(play) = self.play.as_ref() {
            if let Some(head) = head {
                play.prefer_login(&head);
            }
            play.wake_all();
        }
        op
    }

    fn arm_login<S: SlotSurface<Io = Io>>(
        &mut self,
        name: &str,
        surface: &mut S,
    ) -> Result<(), String> {
        if let Some(arm) = self.play.as_ref().and_then(|play| play.arm(name)) {
            arm.arm_explicit_login();
            return Ok(());
        }
        let arm = self.arm_for_profile(name);
        if let Some(arm) = arm.as_ref() {
            arm.arm_explicit_login();
        }
        self.ensure_slot(name, arm, true, surface)
    }

    fn record_login(&mut self, op: OperationId, name: &str, result: Result<(), String>) {
        self.operations.cancel_pending(ActionKind::Logout, name);
        self.operations.cancel_pending(ActionKind::Login, name);
        let outcome = match result {
            Err(error) => Outcome::Failed(error),
            Ok(()) if self.play.is_none() => Outcome::Failed("vault locked".into()),
            Ok(()) if self.play.as_ref().and_then(|p| p.arm(name)).is_none() => {
                Outcome::Failed(format!("no profile {name}"))
            }
            Ok(()) => Outcome::Pending,
        };
        self.operations.set(op, name, outcome);
    }

    /// Log out one member: latch it so auto-login is blocked until Log in,
    /// then arm a clean logout. The slot stays up; only intent changes. A
    /// queued or connecting worker cancels its handshake at the next check.
    pub fn logout(&mut self, name: &str) -> OperationId {
        let op = self.operations.open(ActionKind::Logout);
        self.logout_member(op, name);
        if let Some(play) = self.play.as_ref() {
            play.wake(name);
        }
        op
    }

    /// Log out every member and every other running slot.
    pub fn logout_all(&mut self) -> OperationId {
        let op = self.operations.open(ActionKind::Logout);
        let mut names = self.fleet.members().to_vec();
        if let Some(play) = &self.play {
            for s in play.statuses() {
                if !names.iter().any(|n| n == &s.username) {
                    names.push(s.username);
                }
            }
        }
        for name in names {
            self.logout_member(op, &name);
        }
        if let Some(play) = self.play.as_ref() {
            play.wake_all();
        }
        op
    }

    fn logout_member(&mut self, op: OperationId, name: &str) {
        self.fleet.latch_logout(name);
        self.operations.cancel_pending(ActionKind::Login, name);
        self.operations.cancel_pending(ActionKind::Logout, name);
        if let Some(arm) = self.play.as_ref().and_then(|p| p.arm(name)) {
            arm.request_logout();
        }
        self.operations.set(op, name, Outcome::Pending);
    }

    /// Adopt every running slot into the fleet (MultiBox on), cancelling
    /// their pending removals so the retained IO returns.
    pub fn seed_running(&mut self) {
        let running: Vec<String> = self
            .play
            .as_ref()
            .map(|p| p.statuses().into_iter().map(|s| s.username).collect())
            .unwrap_or_default();
        for name in &running {
            self.cancel_removal(name);
        }
        self.fleet.seed_running(&running);
    }

    /// Remove a member and return immediately. A connected slot gets a
    /// bounded clean-logout window advanced by [`Self::poll`]; a
    /// disconnected worker is stopped asynchronously. Neither path sleeps
    /// or joins on the caller. When the removed member was selected, its
    /// neighbour is selected (or the selection cleared).
    pub fn remove<S: SlotSurface<Io = Io>>(
        &mut self,
        name: &str,
        now: Instant,
        surface: &mut S,
    ) -> Removal {
        let op = self.operations.open(ActionKind::Remove);
        let selected = self.selected.clone();
        let neighbour = self.fleet.focus_neighbour(name, selected.as_deref());
        self.fleet.remove(name);
        self.fleet.clear_latch(name);
        self.operations.cancel_pending(ActionKind::Login, name);
        let connected = self.play.as_ref().is_some_and(|play| {
            play.statuses()
                .iter()
                .any(|status| status.username == name && status.connected)
        });
        let retained = self
            .slots
            .remove(name)
            .or_else(|| self.removals.remove(name).and_then(|pending| pending.io));
        let arm = self.play.as_ref().and_then(|play| play.arm(name));
        match (connected, self.play.as_mut(), arm) {
            (true, Some(play), Some(arm)) => {
                // Clean logout only; Stop follows disconnect or timeout.
                arm.request_logout();
                play.wake(name);
                self.removals.insert(
                    name.to_string(),
                    PendingRemoval {
                        started: now,
                        arm,
                        io: retained,
                        op,
                    },
                );
            }
            (_, Some(play), _) => {
                self.removals.remove(name);
                play.begin_stop_slot(name);
            }
            (_, None, _) => {
                self.removals.remove(name);
            }
        }
        self.operations.set(op, name, Outcome::Pending);
        surface.lifetime_reset(name);
        surface.released(name);
        let mut removal = Removal {
            op,
            reselected: None,
            selection_cleared: false,
        };
        if selected.as_deref() == Some(name) {
            match neighbour {
                Some(next) => {
                    self.select(&next);
                    removal.reselected = Some(next);
                }
                None => {
                    self.selected = None;
                    removal.selection_cleared = true;
                }
            }
        }
        removal
    }

    /// Advance removals, reap finished workers, resolve script Start/Stop
    /// for every slot (an offline or queued slot has no observe of its own),
    /// refresh status rows, record transitions and settle operations. Call
    /// once per UI frame or headless tick.
    pub fn poll(&mut self) {
        self.poll_at(Instant::now());
    }

    pub fn poll_at(&mut self, now: Instant) {
        self.advance_removals(now);
        self.poll_host();
    }

    /// Second half of [`Self::poll`]: resolve script Start/Stop, refresh
    /// rows and transitions, settle operations. Front ends that must run
    /// their own work between the phases call the halves directly.
    pub fn poll_host(&mut self) {
        self.transitions.clear();
        let Some(play) = self.play.as_ref() else {
            return;
        };
        play.pump_script_lifecycles();
        let current = play.statuses();
        self.poll_starts();
        record_transitions(&self.statuses, &current, &mut self.transitions);
        self.statuses = current;
        self.settle_operations();
    }

    /// First half of [`Self::poll`]: reap finished workers and advance
    /// pending removals (stop on disconnect or timeout). Never joins a live
    /// worker.
    pub fn advance_removals(&mut self, now: Instant) {
        if let Some(play) = self.play.as_mut() {
            play.pump_worker_reaps();
        }
        if self.removals.is_empty() {
            return;
        }
        let statuses = self.play.as_ref().map(Play::statuses).unwrap_or_default();
        let ready: Vec<(String, bool)> = self
            .removals
            .iter()
            .map(|(name, pending)| {
                let current_arm = self.play.as_ref().and_then(|play| play.arm(name));
                let owns_current_lifetime = current_arm
                    .as_ref()
                    .is_some_and(|arm| Arc::ptr_eq(arm, &pending.arm));
                let disconnected = !statuses
                    .iter()
                    .any(|status| status.username == name.as_str() && status.connected);
                let timed_out =
                    now.saturating_duration_since(pending.started) >= SLOT_REMOVE_TIMEOUT;
                // A missing arm means the pending lifetime ended by itself;
                // retire its preserved terminal row. A different arm is a
                // replacement: drop the stale entry, never stop the
                // replacement.
                (
                    name.clone(),
                    current_arm.is_none() || (owns_current_lifetime && (disconnected || timed_out)),
                )
            })
            .collect();
        for (name, stop) in ready {
            if stop {
                if let Some(play) = self.play.as_mut() {
                    play.begin_stop_slot(&name);
                }
            }
            let still_owner = self
                .play
                .as_ref()
                .and_then(|play| play.arm(&name))
                .is_some_and(|arm| {
                    self.removals
                        .get(&name)
                        .is_some_and(|pending| Arc::ptr_eq(&arm, &pending.arm))
                });
            if stop || !still_owner {
                self.removals.remove(&name);
            }
        }
    }

    fn poll_starts(&mut self) {
        let Some(play) = self.play.as_ref() else {
            return;
        };
        if self.starts.is_empty() {
            return;
        }
        let mut settled = Vec::new();
        for (name, op) in &self.starts {
            match play.script_poll_start(name) {
                script::StartPoll::Pending => {}
                script::StartPoll::Settled(outcome) => {
                    settled.push((name.clone(), *op, Some(outcome)))
                }
                // The slot was removed (its Stop cancelled the Start).
                script::StartPoll::NotOwed => settled.push((name.clone(), *op, None)),
            }
        }
        for (slot, op, outcome) in settled {
            self.starts.remove(&slot);
            let result = match &outcome {
                Some(script::StartOutcome::Ready) => Outcome::Completed,
                Some(script::StartOutcome::Failed(error)) => Outcome::Failed(error.clone()),
                Some(script::StartOutcome::Cancelled) | None => Outcome::Cancelled,
            };
            self.operations.set(op, &slot, result);
            self.settled_starts.push(StartSettled { slot, op, outcome });
        }
    }

    fn settle_operations(&mut self) {
        let play = self.play.as_ref();
        let statuses = &self.statuses;
        let removals = &self.removals;
        self.operations.settle(|action, slot| {
            let row = statuses.iter().find(|s| s.username == slot);
            let arm = play.and_then(|p| p.arm(slot));
            match action {
                ActionKind::Login => {
                    let row = match row {
                        Some(row) => row,
                        None if arm.is_none() => return Some(Outcome::Cancelled),
                        None => return None,
                    };
                    if row.ingame {
                        Some(Outcome::Completed)
                    } else if let Some(error) = row.terminal_startup_error() {
                        Some(Outcome::Failed(error.to_string()))
                    } else if row.worker_terminal.is_some() {
                        Some(Outcome::Failed(
                            row.error.clone().unwrap_or_else(|| "worker ended".into()),
                        ))
                    } else if row.login_latched {
                        Some(Outcome::Cancelled)
                    } else {
                        None
                    }
                }
                ActionKind::Logout => match row {
                    Some(row) if row.connected => None,
                    _ => Some(Outcome::Completed),
                },
                ActionKind::Remove => {
                    let stopping = play.is_some_and(|p| p.slot_stopping(slot));
                    (!removals.contains_key(slot) && arm.is_none() && !stopping)
                        .then_some(Outcome::Completed)
                }
                ActionKind::ScriptStop => {
                    let state = play.map_or(script::RunState::Idle, |p| p.script_state(slot));
                    matches!(state, script::RunState::Idle | script::RunState::Error)
                        .then_some(Outcome::Completed)
                }
                ActionKind::ScriptPause => {
                    match play.map_or(script::RunState::Idle, |p| p.script_state(slot)) {
                        script::RunState::Paused => Some(Outcome::Completed),
                        script::RunState::Idle | script::RunState::Error => {
                            Some(Outcome::Skipped("no running script".into()))
                        }
                        _ => None,
                    }
                }
                ActionKind::ScriptResume => {
                    match play.map_or(script::RunState::Idle, |p| p.script_state(slot)) {
                        script::RunState::Paused => None,
                        script::RunState::Idle | script::RunState::Error => {
                            Some(Outcome::Skipped("no script".into()))
                        }
                        _ => Some(Outcome::Completed),
                    }
                }
                // Settled at dispatch (Load, Select) or by poll_starts.
                ActionKind::Select | ActionKind::Load | ActionKind::ScriptStart => None,
            }
        });
    }

    /// Start a prepared script on `name`. A compiled card starts
    /// synchronously; a Load Start returns before isolate setup and settles
    /// through [`Self::poll`] even while the slot is offline or queued.
    /// `identity` is the canonical card identity reloads fence on.
    pub fn start_script(
        &mut self,
        name: &str,
        start: ScriptStart,
        identity: Option<String>,
    ) -> Result<OperationId, script::StartLoadError> {
        let play = self
            .play
            .as_ref()
            .ok_or_else(|| script::StartLoadError::Refused("no play".into()))?;
        let compiled = matches!(start, ScriptStart::Compiled(_));
        match start {
            ScriptStart::Compiled(id) => play
                .script_start(name, id)
                .map_err(script::StartLoadError::Refused)?,
            ScriptStart::Load {
                js,
                shape,
                bag,
                siblings,
            } => play.script_start_load_typed(name, js, shape, bag, siblings)?,
        }
        if let Some(identity) = identity {
            play.script_attach_identity(name, identity);
        }
        let op = self.operations.open(ActionKind::ScriptStart);
        if compiled {
            self.operations.set(op, name, Outcome::Completed);
        } else {
            self.operations.set(op, name, Outcome::Pending);
            self.starts.insert(name.to_string(), op);
        }
        Ok(op)
    }

    /// Pause `name`'s script, or resume it when paused.
    pub fn toggle_pause(&mut self, name: &str) -> OperationId {
        let paused = self
            .play
            .as_ref()
            .is_some_and(|play| play.script_state(name) == script::RunState::Paused);
        let action = if paused {
            ActionKind::ScriptResume
        } else {
            ActionKind::ScriptPause
        };
        let op = self.operations.open(action);
        match self.play.as_ref() {
            Some(play) if paused => play.script_resume(name),
            Some(play) => play.script_pause(name),
            None => {
                self.operations
                    .set(op, name, Outcome::Failed("no play".into()));
                return op;
            }
        }
        self.operations.set(op, name, Outcome::Pending);
        op
    }

    /// Stop `name`'s script (teardown hook, instance dropped). Settles when
    /// the host reports the slot idle, including while offline.
    pub fn stop_script(&mut self, name: &str) -> OperationId {
        let op = self.operations.open(ActionKind::ScriptStop);
        match self.play.as_ref() {
            Some(play) => {
                play.script_stop(name);
                self.operations.set(op, name, Outcome::Pending);
            }
            None => self
                .operations
                .set(op, name, Outcome::Failed("no play".into())),
        }
        op
    }

    /// Persist a profile's auto-login and mirror it onto a running slot's
    /// arm. Never spawns or stops a slot.
    pub fn set_auto_login(&mut self, name: &str, on: bool) -> Result<(), String> {
        let vault = self
            .vault
            .as_mut()
            .ok_or_else(|| "auto-login: vault locked".to_string())?;
        let mut profile = vault
            .get(name)
            .cloned()
            .ok_or_else(|| format!("auto-login: no profile {name}"))?;
        profile.settings.auto_login = on;
        vault
            .upsert(profile)
            .map_err(|e| format!("auto-login: {e}"))?;
        if let Some(play) = self.play.as_ref() {
            if let Some(arm) = play.arm(name) {
                arm.set_auto_login(on);
            }
            play.wake(name);
        }
        Ok(())
    }

    /// Persist a profile's random-event guardian settings and mirror them
    /// onto a running slot's arm so toggling off never acts or holds without
    /// a respawn. The vault is written first; the arm only on success.
    pub fn set_random_settings(
        &mut self,
        name: &str,
        random_events: bool,
        lamp_skill: &str,
        lamp_auto: bool,
    ) -> Result<(), String> {
        let vault = self
            .vault
            .as_mut()
            .ok_or_else(|| "random: vault locked".to_string())?;
        let mut profile = vault
            .get(name)
            .cloned()
            .ok_or_else(|| format!("random: no profile {name}"))?;
        profile.settings.random_events = random_events;
        profile.settings.lamp_skill = lamp_skill.to_string();
        profile.settings.lamp_auto = lamp_auto;
        vault.upsert(profile).map_err(|e| format!("random: {e}"))?;
        if let Some(arm) = self.play.as_ref().and_then(|p| p.arm(name)) {
            arm.random_events.store(random_events, Ordering::Relaxed);
            arm.lamp_auto.store(lamp_auto, Ordering::Relaxed);
            *arm.lamp_skill.lock().unwrap() = lamp_skill.to_string();
        }
        Ok(())
    }

    /// Delete a vault profile only. A live member is not logged out or
    /// dropped. Returns whether a row was removed.
    pub fn vault_remove(&mut self, name: &str) -> Result<bool, String> {
        let vault = self
            .vault
            .as_mut()
            .ok_or_else(|| "chooser: vault locked".to_string())?;
        vault.remove(name).map_err(|e| format!("chooser: {e}"))
    }
}

#[cfg(any(test, feature = "test-support"))]
impl<Io> OperatorSession<Io> {
    /// Fixture seam: spawn attaches the arm to the play without a worker
    /// thread, so lifecycle tests never contact a server.
    pub fn set_spawn_workers(&mut self, on: bool) {
        self.spawn_workers = on;
    }

    pub fn set_vault(&mut self, vault: Option<Vault>) {
        self.vault = vault;
    }

    pub fn set_play(&mut self, play: Option<Play>) {
        self.play = play;
    }

    pub fn insert_slot_io(&mut self, name: &str, io: Io) {
        self.slots.insert(name.to_string(), io);
    }

    pub fn fleet_mut(&mut self) -> &mut Fleet {
        &mut self.fleet
    }

    pub fn set_statuses(&mut self, statuses: Vec<SlotStatus>) {
        self.statuses = statuses;
    }
}

fn completed_or_failed(result: Result<(), String>) -> Outcome {
    match result {
        Ok(()) => Outcome::Completed,
        Err(error) => Outcome::Failed(error),
    }
}

fn record_transitions(
    previous: &[SlotStatus],
    current: &[SlotStatus],
    out: &mut Vec<SlotTransition>,
) {
    let mut push = |slot: &str, transition| {
        out.push(SlotTransition {
            slot: slot.to_string(),
            transition,
        })
    };
    for s in current {
        let name = s.username.as_str();
        match previous.iter().find(|p| p.username == s.username) {
            None => {
                push(name, Transition::SlotUp);
                if let Some(e) = &s.error {
                    push(name, Transition::LoginError(e.clone()));
                }
            }
            Some(p) => {
                if p.error.is_none() {
                    if let Some(e) = &s.error {
                        push(name, Transition::LoginError(e.clone()));
                    }
                }
                if !p.ingame && s.ingame {
                    push(name, Transition::Ingame);
                }
                if p.scene_state != s.scene_state {
                    push(name, Transition::Scene(s.scene_state));
                }
                if p.welcome_notice != s.welcome_notice {
                    if let Some(line) = &s.welcome_notice {
                        push(name, Transition::Welcome(line.clone()));
                    }
                }
                if p.welcome_failure.is_none() {
                    if let Some(line) = &s.welcome_failure {
                        push(name, Transition::WelcomeFailure(line.clone()));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
