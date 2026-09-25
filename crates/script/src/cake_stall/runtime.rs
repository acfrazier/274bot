//! Rust-owned Baker stall sequencing: the frozen `stealCakes` loop.
//! `api::cake_stall` owns the posted pins and row types; this module owns
//! selection, restock and stall-food predicates and the whole steal loop:
//! its exits, its waits, and the stand, refusal and lockout state local to
//! one `cake_stall` step-machine row; JavaScript only marshals the options and
//! awaits its completion.

use super::{counts_as_stall_food, needs_cake_restock, select_baker_stall};
use crate::machine::{AbortReason, Begin, Call, Cx, Family, Reply, Step};
use crate::observed::{self, Scene, SceneRow};
use crate::shim::InteractReq;
use api::cake_stall::{StallLoc, BAKER_STALL};
use serde::Deserialize;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::time::{Duration, Instant};

/// Frozen `DEADLINE_MS`: one call steals for at most this long.
pub const DEADLINE_MS: u64 = 90_000;
/// Frozen `CLAIM_TIMEOUT_MS`: the walk to the stand.
pub const CLAIM_TIMEOUT_MS: u64 = 15_000;
/// Frozen `RESOLVE_MS`: how long a sent steal may take to resolve.
pub const RESOLVE_MS: u64 = 2_400;
/// Frozen `RESTOCK_WAIT_MS`: the wait for an emptied stall.
pub const RESTOCK_WAIT_MS: u64 = 8_000;
/// Frozen bound on the post-combat lockout wait.
pub const LOCKOUT_WAIT_MS: u64 = 12_000;
/// Frozen `NEAR_STALL`: a claim this close to the stall tile may steal.
const NEAR_STALL: i32 = 2;
/// Frozen `RESET_AFTER_REFUSALS`: consecutive steals that gained nothing and
/// were not caught (the stall owner was watching) before the other stand is
/// tried.
pub const RESET_AFTER_REFUSALS: u32 = 3;
/// Frozen `LOCKOUT_TICKS`: how long a steal refused for recent combat waits
/// before the next attempt.
pub const LOCKOUT_TICKS: i64 = 10;
/// Frozen `LOCKOUT_RE`: the server refuses stall steals within ten ticks of
/// combat with this line. It is neither a catch nor a watched-stand refusal.
const LOCKOUT_LINE: &str = "can't steal from the market stall during combat";

const ABORT: usize = 0;
const SHOULD_EAT: usize = 1;
const LOCKED_OUT_UNTIL: usize = 2;
const SET_STATUS: usize = 3;
const LOG: usize = 4;
const ON_STEAL: usize = 5;
const ON_RESET: usize = 6;

#[derive(Deserialize)]
pub(crate) struct CakeStallArgs {
    #[serde(default)]
    fill_to: Value,
}

#[derive(Clone, Copy)]
enum DriverPhase {
    ReportStatus,
    ReportLog,
    Handle,
    Observe {
        callbacks: bool,
        lockout: bool,
        stage: ObserveStage,
    },
    AfterEvent,
    Poll {
        callbacks: bool,
    },
}

#[derive(Clone, Copy)]
enum ObserveStage {
    Abort,
    ShouldEat,
    Lockout,
    Dispatch,
}

#[derive(Clone, Copy)]
enum PendingHook {
    Abort,
    ShouldEat,
    Lockout,
    Notify,
}

/// One frozen `stealCakes` call. The existing policy runtime owns its token
/// and waits; this family owns the former JavaScript begin/next pump.
pub(crate) struct CakeStall {
    token: u64,
    step: Value,
    phase: DriverPhase,
    pending: Option<PendingHook>,
    poll_callbacks: bool,
    abort: bool,
    should_eat: bool,
    locked_out_until: i64,
}

thread_local! {
    static RUNTIME: RefCell<CakeStallRuntime> = const { RefCell::new(CakeStallRuntime::new()) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Tile {
    x: i32,
    z: i32,
    level: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SelectedLoc {
    id: i32,
    x: i32,
    z: i32,
    level: i32,
}

struct Observation {
    ingame: bool,
    tick: i64,
    here: Option<Tile>,
    in_combat: bool,
    abort: bool,
    should_eat: bool,
    inv_size: i32,
    inv_len: usize,
    carried: i32,
    stall: Option<SelectedLoc>,
    locked_out_until: Option<i64>,
    chat_max_seq: i32,
    lockout_seq: Option<i32>,
}

/// The posted facts this module decides from, read from the isolate scene.
struct NativeObservation {
    ingame: bool,
    tick: i64,
    here: Option<Tile>,
    in_combat: bool,
    inv_size: i32,
    inv_len: usize,
    carried: i32,
    stall: Option<SelectedLoc>,
    chat_max_seq: i32,
    lockout_seq: Option<i32>,
}

impl NativeObservation {
    /// A logout forgets the session: only pages posted since login count.
    /// The stall is picked only for the ops that walk to or steal from it.
    fn from_scene(scene: &Scene, pick_stall: bool) -> Self {
        let session = scene.since_login();
        let inv = session.inv();
        let lines = session.chat_lines();
        Self {
            ingame: session.ingame().unwrap_or(false),
            tick: scene
                .session_tick()
                .map_or(0, |tick| i64::try_from(tick).unwrap_or(i64::MAX)),
            here: session.here().map(|tile| Tile {
                x: tile.x,
                z: tile.z,
                level: tile.level,
            }),
            in_combat: session.in_combat().unwrap_or(false),
            inv_size: session.inv_size().unwrap_or(0),
            inv_len: inv.map_or(0, Vec::len),
            carried: inv.map_or(0, |rows| {
                sum_carried(
                    rows.iter()
                        .filter_map(|row| row.name.as_deref().map(|name| (name, row.count))),
                )
            }),
            stall: if pick_stall {
                session.locs().and_then(|locs| selected_stall(locs))
            } else {
                None
            },
            chat_max_seq: lines.map_or(-1, |lines| {
                lines.iter().map(|line| line.seq).max().unwrap_or(-1)
            }),
            lockout_seq: lines.and_then(|lines| {
                lines
                    .iter()
                    .filter(|line| contains_ascii_ci(&line.text, LOCKOUT_LINE))
                    .map(|line| line.seq)
                    .max()
            }),
        }
    }

    fn with_callback_values(
        &self,
        abort: bool,
        should_eat: bool,
        locked_out_until: Option<i64>,
    ) -> Observation {
        Observation {
            ingame: self.ingame,
            tick: self.tick,
            here: self.here,
            in_combat: self.in_combat,
            abort,
            should_eat,
            inv_size: self.inv_size,
            inv_len: self.inv_len,
            carried: self.carried,
            stall: self.stall,
            locked_out_until,
            chat_max_seq: self.chat_max_seq,
            lockout_seq: self.lockout_seq,
        }
    }

    fn with_callbacks(&self, input: &Value) -> Observation {
        self.with_callback_values(
            input.get("abort").and_then(Value::as_bool).unwrap_or(false),
            input
                .get("should_eat")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            input.get("locked_out_until").and_then(Value::as_i64),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    /// The loop head: waiting for the callback and lockout observe.
    Head,
    /// `delayUntil(tick >= until || abort(), 12_000)`.
    WaitLockout {
        until: i64,
    },
    /// `walkTo(stand, { radius: 0, timeoutMs: CLAIM_TIMEOUT_MS })`.
    WaitStand,
    /// `delayTicks(1)` after a claim fell short.
    WaitTick {
        until: i64,
    },
    /// `delayUntil(stockedStall() !== null || abort(), RESTOCK_WAIT_MS)`.
    WaitRestock,
    /// `delayUntil(gained || inCombat || lockoutSeen, RESOLVE_MS)`.
    WaitSteal,
    /// `onSteal` was handed out; back to the head next.
    AfterSteal,
    /// `onReset` was handed out; back to the head next.
    AfterReset,
}

/// One frozen `stealCakes` call at a time. Everything but the freeze state
/// belongs to the current call and is cleared when a new one begins.
struct CakeStallRuntime {
    paused: bool,
    held: bool,
    frozen_at: Option<Instant>,
    token: u64,
    phase: Phase,
    fill_to: Option<i32>,
    /// Stall food carried when the current steal was sent.
    before: i32,
    /// Bound of the current wait.
    deadline: Option<Instant>,
    /// Frozen `DEADLINE_MS` bound of the whole call.
    call_deadline: Option<Instant>,
    /// Frozen `stand`: `false` is the main stand, `true` the alternate.
    alt_stand: bool,
    /// Frozen `refusals`: consecutive refused steals from this stand.
    refusals: u32,
    /// Frozen `selfLockout`: the tick a steal refused for recent combat may
    /// be retried from.
    self_lockout_until: i64,
    /// Newest chat seq when the current steal was sent; only later lines
    /// resolve it.
    mark_seq: i32,
}

impl CakeStallRuntime {
    const fn new() -> Self {
        Self {
            paused: false,
            held: false,
            frozen_at: None,
            token: 0,
            phase: Phase::Idle,
            fill_to: None,
            before: 0,
            deadline: None,
            call_deadline: None,
            alt_stand: false,
            refusals: 0,
            self_lockout_until: 0,
            mark_seq: -1,
        }
    }

    fn frozen(&self) -> bool {
        self.paused || self.held
    }

    fn now(&self) -> Instant {
        self.frozen_at.unwrap_or_else(Instant::now)
    }

    fn set_freeze(&mut self, paused: bool, held: bool) {
        let was_frozen = self.frozen();
        self.paused = paused;
        self.held = held;
        let frozen = self.frozen();
        if !was_frozen && frozen {
            self.frozen_at = Some(Instant::now());
        } else if was_frozen && !frozen {
            if let Some(at) = self.frozen_at.take() {
                let gap = Instant::now().saturating_duration_since(at);
                for deadline in [self.deadline.as_mut(), self.call_deadline.as_mut()]
                    .into_iter()
                    .flatten()
                {
                    *deadline += gap;
                }
            }
        }
    }

    /// Drop the current call: a new token, and none of its state survives.
    fn abort_runtime(&mut self) {
        self.token = self.token.wrapping_add(1);
        self.phase = Phase::Idle;
        self.fill_to = None;
        self.before = 0;
        self.deadline = None;
        self.call_deadline = None;
        self.alt_stand = false;
        self.refusals = 0;
        self.self_lockout_until = 0;
        self.mark_seq = -1;
    }

    fn stand(&self) -> Tile {
        let stand = if self.alt_stand {
            BAKER_STALL.stand_alt
        } else {
            BAKER_STALL.stand
        };
        Tile {
            x: stand.x,
            z: stand.z,
            level: stand.level,
        }
    }

    fn done(&mut self, result: &str, log: Option<String>) -> Value {
        self.phase = Phase::Idle;
        self.deadline = None;
        self.call_deadline = None;
        let mut step = json!({
            "kind": "done",
            "token": self.token,
            "result": result,
        });
        if let Some(log) = log {
            step["log"] = json!(log);
        }
        step
    }

    fn begin(&mut self, fill_to: Option<i32>) -> Value {
        self.abort_runtime();
        self.fill_to = fill_to;
        self.call_deadline = Some(self.now() + Duration::from_millis(DEADLINE_MS));
        self.head_observe()
    }

    /// Back to the loop head: it needs the caller's `abort`, `shouldEat` and
    /// `lockedOutUntil` answers.
    fn head_observe(&mut self) -> Value {
        self.phase = Phase::Head;
        self.deadline = None;
        json!({
            "kind": "observe",
            "token": self.token,
            "callbacks": true,
            "lockout": true,
        })
    }

    /// Enter a bounded wait. `callbacks` asks the pump to answer `abort` and
    /// `shouldEat` on every poll.
    fn pause(
        &mut self,
        phase: Phase,
        bound_ms: Option<u64>,
        callbacks: bool,
        status: Option<&str>,
        log: Option<String>,
    ) -> Value {
        self.phase = phase;
        self.deadline = bound_ms.map(|ms| self.now() + Duration::from_millis(ms));
        let mut step = json!({
            "kind": "pause",
            "token": self.token,
            "callbacks": callbacks,
        });
        if let Some(status) = status {
            step["status"] = json!(status);
        }
        if let Some(log) = log {
            step["log"] = json!(log);
        }
        step
    }

    /// The frozen loop head, in its order: deadline, abort / eat, combat,
    /// filled, lockout, walk to the stand, then the stall.
    fn head(&mut self, obs: &Observation) -> Value {
        if self
            .call_deadline
            .is_some_and(|deadline| self.now() >= deadline)
        {
            return self.done("no-progress", None);
        }
        if !obs.ingame || obs.abort || obs.should_eat {
            return self.done("aborted", None);
        }
        if obs.in_combat {
            return self.done("combat", None);
        }
        if at_goal(obs, self.fill_to) {
            let log = format!("stocked {} stall food ({} slots)", obs.carried, obs.inv_len);
            return self.done("stocked", Some(log));
        }
        let until = obs
            .locked_out_until
            .unwrap_or(0)
            .max(self.self_lockout_until);
        if obs.tick < until {
            return self.pause(
                Phase::WaitLockout { until },
                Some(LOCKOUT_WAIT_MS),
                true,
                Some("waiting out the post-combat steal lockout"),
                None,
            );
        }
        let stand = self.stand();
        if obs.here.is_some_and(|here| here != stand) {
            self.phase = Phase::WaitStand;
            self.deadline = Some(self.now() + Duration::from_millis(CLAIM_TIMEOUT_MS));
            return json!({
                "kind": "walk-to",
                "token": self.token,
                "x": stand.x,
                "z": stand.z,
                "level": stand.level,
            });
        }
        self.try_steal(obs)
    }

    /// After the walk: steal only from the stall side of the market.
    fn claim(&mut self, obs: &Observation) -> Value {
        let stall = BAKER_STALL.stall;
        let near = obs.here.is_some_and(|here| {
            here.level == stall.level
                && (here.x - stall.x).abs().max((here.z - stall.z).abs()) <= NEAR_STALL
        });
        if !near {
            let at = obs
                .here
                .map(|here| format!(" at ({},{})", here.x, here.z))
                .unwrap_or_default();
            return self.pause(
                Phase::WaitTick {
                    until: obs.tick.saturating_add(1),
                },
                None,
                false,
                None,
                Some(format!(
                    "claim fell short{at} — not stealing from the market side"
                )),
            );
        }
        self.try_steal(obs)
    }

    fn try_steal(&mut self, obs: &Observation) -> Value {
        let Some(loc) = obs.stall else {
            return self.pause(
                Phase::WaitRestock,
                Some(RESTOCK_WAIT_MS),
                true,
                None,
                Some("stall emptied — waiting for the restock".to_string()),
            );
        };
        self.before = obs.carried;
        self.mark_seq = obs.chat_max_seq;
        self.phase = Phase::WaitSteal;
        self.deadline = Some(self.now() + Duration::from_millis(RESOLVE_MS));
        let status = match self.fill_to {
            Some(fill_to) => format!("stealing cake ({}/{fill_to})", obs.carried),
            None => format!("stealing cake ({}/{} slots)", obs.inv_len, obs.inv_size),
        };
        json!({
            "kind": "loc",
            "token": self.token,
            "x": loc.x,
            "z": loc.z,
            "level": loc.level,
            "id": loc.id,
            "action": BAKER_STALL.op,
            "status": status,
        })
    }

    /// Frozen `classifySteal`: success, caught, lockout, else refused.
    fn classify(&mut self, obs: &Observation) -> Value {
        if obs.carried > self.before {
            self.refusals = 0;
            self.phase = Phase::AfterSteal;
            self.deadline = None;
            return json!({"kind": "on-steal", "token": self.token});
        }
        if obs.in_combat {
            return self.done("combat", None);
        }
        if self.lockout_seen(obs) {
            self.self_lockout_until = obs.tick.saturating_add(LOCKOUT_TICKS);
            return self.head_observe();
        }
        self.refusals += 1;
        if self.refusals >= RESET_AFTER_REFUSALS {
            let refusals = self.refusals;
            self.alt_stand = !self.alt_stand;
            self.refusals = 0;
            self.phase = Phase::AfterReset;
            self.deadline = None;
            let stand = self.stand();
            return json!({
                "kind": "on-reset",
                "token": self.token,
                "status": "watched — swapping stands",
                "log": format!(
                    "{refusals} refused steals — swapping to the stand at ({},{})",
                    stand.x, stand.z
                ),
            });
        }
        self.head_observe()
    }

    fn next(&mut self, token: u64, obs: &Observation) -> Value {
        if token != self.token || self.phase == Phase::Idle {
            return json!({"kind": "aborted", "token": self.token});
        }
        if self.frozen() {
            return self.wait();
        }
        if !obs.ingame && self.phase != Phase::Head {
            return self.done("aborted", None);
        }
        match self.phase {
            Phase::Idle => json!({"kind": "aborted", "token": self.token}),
            Phase::Head => self.head(obs),
            Phase::WaitLockout { until } => {
                if obs.abort || obs.tick >= until || self.expired() {
                    self.head_observe()
                } else {
                    self.wait()
                }
            }
            Phase::WaitStand => {
                if obs.here == Some(self.stand()) || self.expired() {
                    self.claim(obs)
                } else {
                    self.wait()
                }
            }
            Phase::WaitTick { until } => {
                if obs.tick >= until {
                    self.head_observe()
                } else {
                    self.wait()
                }
            }
            Phase::WaitRestock => {
                if obs.abort || obs.stall.is_some() || self.expired() {
                    self.head_observe()
                } else {
                    self.wait()
                }
            }
            Phase::WaitSteal => {
                if obs.carried > self.before
                    || obs.in_combat
                    || self.lockout_seen(obs)
                    || self.expired()
                {
                    self.classify(obs)
                } else {
                    self.wait()
                }
            }
            Phase::AfterSteal | Phase::AfterReset => self.head_observe(),
        }
    }

    fn wait(&self) -> Value {
        json!({"kind": "wait", "token": self.token})
    }

    fn expired(&self) -> bool {
        self.deadline.is_some_and(|deadline| self.now() >= deadline)
    }

    fn lockout_seen(&self, obs: &Observation) -> bool {
        obs.lockout_seq.is_some_and(|seq| seq > self.mark_seq)
    }
}

#[derive(Clone, Copy)]
enum CakeStepKind {
    Observe,
    OnReset,
    OnSteal,
    WalkTo,
    Loc,
    Wait,
    Pause,
    Done,
    Aborted,
    Unknown,
}

impl CakeStall {
    fn kind(&self) -> CakeStepKind {
        match self.step.get("kind").and_then(Value::as_str).unwrap_or("") {
            "observe" => CakeStepKind::Observe,
            "on-reset" => CakeStepKind::OnReset,
            "on-steal" => CakeStepKind::OnSteal,
            "walk-to" => CakeStepKind::WalkTo,
            "loc" => CakeStepKind::Loc,
            "wait" => CakeStepKind::Wait,
            "pause" => CakeStepKind::Pause,
            "done" => CakeStepKind::Done,
            "aborted" => CakeStepKind::Aborted,
            _ => CakeStepKind::Unknown,
        }
    }

    fn begin_observe(&mut self, callbacks: bool, lockout: bool) {
        self.abort = false;
        self.should_eat = false;
        self.locked_out_until = 0;
        self.phase = DriverPhase::Observe {
            callbacks,
            lockout,
            stage: ObserveStage::Abort,
        };
    }

    fn call(&mut self, hook: usize, pending: PendingHook, args: Vec<Value>) -> Step<String> {
        self.pending = Some(pending);
        Step::Call(Call { hook, args })
    }

    fn receive(&mut self, cx: &mut Cx<'_>) -> Result<(), Step<String>> {
        let Some(reply) = cx.reply() else {
            return Ok(());
        };
        match reply {
            Reply::Threw(thrown) => Err(Step::Fail(thrown)),
            Reply::Value(value) => {
                match self.pending.take() {
                    Some(PendingHook::Abort) => {
                        self.abort = crate::bank_deposit::truthy(&value);
                    }
                    Some(PendingHook::ShouldEat) => {
                        self.should_eat = crate::bank_deposit::truthy(&value);
                    }
                    Some(PendingHook::Lockout) => {
                        self.locked_out_until = value.as_i64().unwrap_or(0);
                    }
                    Some(PendingHook::Notify) | None => {}
                }
                Ok(())
            }
        }
    }

    fn advance_runtime(&mut self, lockout: bool) {
        let obs = observed::with(|scene| NativeObservation::from_scene(scene, true))
            .with_callback_values(
                self.abort,
                self.should_eat,
                lockout.then_some(self.locked_out_until),
            );
        self.step = RUNTIME.with(|runtime| runtime.borrow_mut().next(self.token, &obs));
        self.phase = DriverPhase::ReportStatus;
    }

    fn emit_current(&self, cx: &mut Cx<'_>) -> bool {
        let Some(x) = step_i32(&self.step, "x") else {
            return false;
        };
        let Some(z) = step_i32(&self.step, "z") else {
            return false;
        };
        let Some(level) = step_i32(&self.step, "level") else {
            return false;
        };
        match self.kind() {
            CakeStepKind::WalkTo => cx.emit(InteractReq::WalkTo { x, z, level }),
            CakeStepKind::Loc => {
                let Some(action) = self.step.get("action").and_then(Value::as_str) else {
                    return false;
                };
                let Some(id) = step_i32(&self.step, "id") else {
                    return false;
                };
                cx.emit(InteractReq::Loc {
                    x,
                    z,
                    level,
                    action: action.to_string(),
                    id: Some(id),
                });
            }
            _ => return false,
        }
        true
    }
}

impl Family for CakeStall {
    const NAME: &'static str = "cake_stall";
    const EXCLUSIVE: bool = true;
    const CALLBACKS: &'static [&'static str] = &[
        "abort",
        "shouldEat",
        "lockedOutUntil",
        "setStatus",
        "log",
        "onSteal",
        "onReset",
    ];
    /// Every frozen callback is a plain call; a returned promise is a truthy
    /// value for predicates and is ignored for notifications.
    const SYNC_HOOKS: &'static [usize] = &[
        ABORT,
        SHOULD_EAT,
        LOCKED_OUT_UNTIL,
        SET_STATUS,
        LOG,
        ON_STEAL,
        ON_RESET,
    ];
    /// Begin and the first steal join the caller's turn.
    const KICK_ON_START: bool = true;
    type Args = CakeStallArgs;
    type Output = String;

    fn begin(args: CakeStallArgs, _cx: &mut Cx<'_>) -> Begin<Self> {
        let fill_to = args.fill_to.as_i64().map(|value| value as i32);
        let step = RUNTIME.with(|runtime| runtime.borrow_mut().begin(fill_to));
        let token = step.get("token").and_then(Value::as_u64).unwrap_or(0);
        Begin::Run(Self {
            token,
            step,
            phase: DriverPhase::ReportStatus,
            pending: None,
            poll_callbacks: false,
            abort: false,
            should_eat: false,
            locked_out_until: 0,
        })
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<String> {
        if let Err(step) = self.receive(cx) {
            return step;
        }
        loop {
            match self.phase {
                DriverPhase::ReportStatus => {
                    self.phase = DriverPhase::ReportLog;
                    if cx.has(SET_STATUS) {
                        if let Some(status) = self.step.get("status").cloned() {
                            return self.call(SET_STATUS, PendingHook::Notify, vec![status]);
                        }
                    }
                }
                DriverPhase::ReportLog => {
                    self.phase = DriverPhase::Handle;
                    if cx.has(LOG) {
                        if let Some(log) = self.step.get("log").cloned() {
                            return self.call(LOG, PendingHook::Notify, vec![log]);
                        }
                    }
                }
                DriverPhase::Handle => match self.kind() {
                    CakeStepKind::Observe => {
                        self.begin_observe(
                            self.step.get("callbacks").and_then(Value::as_bool) == Some(true),
                            self.step.get("lockout").and_then(Value::as_bool) == Some(true),
                        );
                    }
                    CakeStepKind::OnReset | CakeStepKind::OnSteal => {
                        let hook = if matches!(self.kind(), CakeStepKind::OnReset) {
                            ON_RESET
                        } else {
                            ON_STEAL
                        };
                        self.phase = DriverPhase::AfterEvent;
                        if cx.has(hook) {
                            return self.call(hook, PendingHook::Notify, Vec::new());
                        }
                    }
                    CakeStepKind::WalkTo | CakeStepKind::Loc => {
                        if !self.emit_current(cx) {
                            return Step::Done("no-progress".to_string());
                        }
                        self.poll_callbacks =
                            self.step.get("callbacks").and_then(Value::as_bool) == Some(true);
                        self.phase = DriverPhase::Poll {
                            callbacks: self.poll_callbacks,
                        };
                        return Step::Wait;
                    }
                    CakeStepKind::Pause => {
                        self.poll_callbacks =
                            self.step.get("callbacks").and_then(Value::as_bool) == Some(true);
                        self.phase = DriverPhase::Poll {
                            callbacks: self.poll_callbacks,
                        };
                        return Step::Wait;
                    }
                    CakeStepKind::Wait => {
                        self.phase = DriverPhase::Poll {
                            callbacks: self.poll_callbacks,
                        };
                        return Step::Wait;
                    }
                    CakeStepKind::Done => {
                        return Step::Done(
                            self.step
                                .get("result")
                                .and_then(Value::as_str)
                                .unwrap_or("no-progress")
                                .to_string(),
                        );
                    }
                    CakeStepKind::Aborted => return Step::Done("aborted".to_string()),
                    CakeStepKind::Unknown => return Step::Done("no-progress".to_string()),
                },
                DriverPhase::AfterEvent => self.begin_observe(false, false),
                DriverPhase::Poll { callbacks } => self.begin_observe(callbacks, false),
                DriverPhase::Observe {
                    callbacks,
                    lockout,
                    stage,
                } => match stage {
                    ObserveStage::Abort => {
                        self.phase = DriverPhase::Observe {
                            callbacks,
                            lockout,
                            stage: ObserveStage::ShouldEat,
                        };
                        if callbacks && cx.has(ABORT) {
                            return self.call(ABORT, PendingHook::Abort, Vec::new());
                        }
                    }
                    ObserveStage::ShouldEat => {
                        self.phase = DriverPhase::Observe {
                            callbacks,
                            lockout,
                            stage: ObserveStage::Lockout,
                        };
                        if callbacks && !self.abort && cx.has(SHOULD_EAT) {
                            return self.call(SHOULD_EAT, PendingHook::ShouldEat, Vec::new());
                        }
                    }
                    ObserveStage::Lockout => {
                        self.phase = DriverPhase::Observe {
                            callbacks,
                            lockout,
                            stage: ObserveStage::Dispatch,
                        };
                        if lockout && cx.has(LOCKED_OUT_UNTIL) {
                            return self.call(LOCKED_OUT_UNTIL, PendingHook::Lockout, Vec::new());
                        }
                    }
                    ObserveStage::Dispatch => self.advance_runtime(lockout),
                },
            }
        }
    }

    fn abort(&mut self, _why: AbortReason) {
        RUNTIME.with(|runtime| {
            let mut runtime = runtime.borrow_mut();
            if runtime.token == self.token && runtime.phase != Phase::Idle {
                runtime.abort_runtime();
            }
        });
    }
}

fn step_i32(step: &Value, key: &str) -> Option<i32> {
    step.get(key)
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok())
}

/// ASCII case-insensitive substring test, without the copy.
fn contains_ascii_ci(hay: &str, needle: &str) -> bool {
    hay.as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

fn sum_carried<'a>(inv: impl IntoIterator<Item = (&'a str, i32)>) -> i32 {
    inv.into_iter()
        .filter(|(name, _)| counts_as_stall_food(name))
        .fold(0_i32, |sum, (_, count)| sum.saturating_add(count))
}

fn pack_full(obs: &Observation) -> bool {
    obs.inv_size > 0 && obs.inv_len >= obs.inv_size as usize
}

fn at_goal(obs: &Observation, fill_to: Option<i32>) -> bool {
    pack_full(obs) || fill_to.is_some_and(|target| obs.carried >= target)
}

fn selected_stall(locs: &[SceneRow]) -> Option<SelectedLoc> {
    // `select_baker_stall` only takes the pinned loc id, so only those rows
    // are viewed.
    let pinned: Vec<&SceneRow> = locs
        .iter()
        .filter(|loc| loc.id == BAKER_STALL.loc_id)
        .collect();
    let action_refs: Vec<Vec<&str>> = pinned
        .iter()
        .map(|loc| loc.actions.iter().map(|action| &**action).collect())
        .collect();
    let views: Vec<StallLoc<'_>> = pinned
        .iter()
        .zip(action_refs.iter())
        .map(|(loc, actions)| StallLoc {
            id: loc.id,
            name: loc.name.as_deref(),
            x: loc.x,
            z: loc.z,
            level: loc.level,
            distance: loc.distance,
            actions,
        })
        .collect();
    select_baker_stall(Some(&BAKER_STALL), &views).map(|loc| SelectedLoc {
        id: loc.id,
        x: loc.x,
        z: loc.z,
        level: loc.level,
    })
}

pub fn on_pause() {
    RUNTIME.with(|runtime| {
        let held = runtime.borrow().held;
        runtime.borrow_mut().set_freeze(true, held);
    });
}

pub fn on_resume() {
    RUNTIME.with(|runtime| {
        let held = runtime.borrow().held;
        runtime.borrow_mut().set_freeze(false, held);
    });
}

pub fn on_hold(held: bool) {
    RUNTIME.with(|runtime| {
        let paused = runtime.borrow().paused;
        runtime.borrow_mut().set_freeze(paused, held);
    });
}

pub fn on_reset() {
    RUNTIME.with(|runtime| runtime.borrow_mut().abort_runtime());
}

pub fn dispatch(input: &Value) -> Value {
    let observe = |pick_stall: bool| {
        observed::with(|scene| NativeObservation::from_scene(scene, pick_stall))
            .with_callbacks(input)
    };
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "count" => json!(observe(false).carried),
        "needs_restock" => {
            let obs = observe(false);
            json!(needs_cake_restock(
                obs.carried,
                input
                    .get("target")
                    .and_then(Value::as_i64)
                    .map(|n| n as i32),
                pack_full(&obs),
            ))
        }
        _ => json!({"kind": "done", "result": "no-progress"}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation() -> Observation {
        Observation {
            ingame: true,
            tick: 1,
            here: Some(Tile {
                x: BAKER_STALL.stand.x,
                z: BAKER_STALL.stand.z,
                level: BAKER_STALL.stand.level,
            }),
            in_combat: false,
            abort: false,
            should_eat: false,
            inv_size: 28,
            inv_len: 0,
            carried: 0,
            stall: Some(SelectedLoc {
                id: BAKER_STALL.loc_id,
                x: BAKER_STALL.stall.x,
                z: BAKER_STALL.stall.z,
                level: BAKER_STALL.stall.level,
            }),
            locked_out_until: None,
            chat_max_seq: -1,
            lockout_seq: None,
        }
    }

    fn at(stand: api::snapshot::WorldTile) -> Observation {
        Observation {
            here: Some(Tile {
                x: stand.x,
                z: stand.z,
                level: stand.level,
            }),
            ..observation()
        }
    }

    fn carrying(carried: i32) -> Observation {
        Observation {
            carried,
            inv_len: carried as usize,
            ..observation()
        }
    }

    fn token(step: &Value) -> u64 {
        step["token"].as_u64().unwrap()
    }

    /// From the head, send a steal and let its resolve window run out with
    /// nothing gained. Returns what the refusal leads to.
    fn refuse(runtime: &mut CakeStallRuntime, token: u64, obs: &Observation) -> Value {
        let steal = runtime.next(token, obs);
        assert_eq!(steal["kind"], "loc", "{steal}");
        runtime.deadline = Some(Instant::now() - Duration::from_millis(1));
        runtime.next(token, obs)
    }

    #[test]
    fn native_predicates_own_count_and_restock() {
        let mut obs = observation();
        assert_eq!(
            sum_carried([("Chocolate cake", 1), ("Chocolate slice", 2)]),
            2
        );
        assert!(needs_cake_restock(obs.carried, Some(1), pack_full(&obs)));
        obs.carried = 2;
        assert!(!needs_cake_restock(obs.carried, Some(1), pack_full(&obs)));
    }

    /// Frozen `stealCakes` keeps stealing inside one call until the pack
    /// holds `fillTo`: a gain goes back to the loop head, not to the caller.
    #[test]
    fn one_call_keeps_stealing_until_fill_to() {
        let mut runtime = CakeStallRuntime::new();
        let begin = runtime.begin(Some(2));
        assert_eq!(begin["kind"], "observe");
        assert_eq!(begin["callbacks"], true);
        assert_eq!(begin["lockout"], true);
        let token = token(&begin);

        let steal = runtime.next(token, &carrying(0));
        assert_eq!(steal["kind"], "loc", "{steal}");
        assert_eq!(steal["status"], "stealing cake (0/2)");
        assert_eq!(runtime.next(token, &carrying(1))["kind"], "on-steal");
        assert_eq!(runtime.next(token, &carrying(1))["kind"], "observe");
        let again = runtime.next(token, &carrying(1));
        assert_eq!(again["kind"], "loc", "same call steals again: {again}");
        assert_eq!(again["status"], "stealing cake (1/2)");
        assert_eq!(runtime.next(token, &carrying(2))["kind"], "on-steal");
        assert_eq!(runtime.next(token, &carrying(2))["kind"], "observe");
        let done = runtime.next(token, &carrying(2));
        assert_eq!(done["result"], "stocked", "{done}");
        assert_eq!(done["log"], "stocked 2 stall food (2 slots)");
    }

    /// The whole call is bounded by the frozen 90 s deadline, and a stale
    /// token is refused.
    #[test]
    fn call_deadline_ends_in_no_progress() {
        let mut runtime = CakeStallRuntime::new();
        let token = token(&runtime.begin(Some(28)));
        let refused = refuse(&mut runtime, token, &observation());
        assert_eq!(refused["kind"], "observe", "a refusal loops: {refused}");
        runtime.call_deadline = Some(Instant::now() - Duration::from_millis(1));
        assert_eq!(runtime.next(token, &observation())["result"], "no-progress");
        assert_eq!(runtime.next(token, &observation())["kind"], "aborted");

        let stale = token_of_new_call(&mut runtime);
        runtime.abort_runtime();
        assert_eq!(runtime.next(stale, &observation())["kind"], "aborted");
    }

    fn token_of_new_call(runtime: &mut CakeStallRuntime) -> u64 {
        token(&runtime.begin(Some(28)))
    }

    #[test]
    fn loop_head_exits_in_frozen_order() {
        let mut runtime = CakeStallRuntime::new();
        for (obs, result) in [
            (
                Observation {
                    abort: true,
                    in_combat: true,
                    ..observation()
                },
                "aborted",
            ),
            (
                Observation {
                    should_eat: true,
                    ..observation()
                },
                "aborted",
            ),
            (
                Observation {
                    in_combat: true,
                    inv_len: 28,
                    ..observation()
                },
                "combat",
            ),
            (
                Observation {
                    inv_len: 28,
                    ..observation()
                },
                "stocked",
            ),
        ] {
            let token = token(&runtime.begin(Some(28)));
            assert_eq!(runtime.next(token, &obs)["result"], result);
        }
        // Caught by a guard mid-steal.
        let token = token(&runtime.begin(Some(28)));
        assert_eq!(runtime.next(token, &observation())["kind"], "loc");
        let caught = Observation {
            in_combat: true,
            ..observation()
        };
        assert_eq!(runtime.next(token, &caught)["result"], "combat");
    }

    /// Three refusals swap the stand inside the call; a new call starts from
    /// the main stand again, as a fresh frozen `stealCakes` call does.
    #[test]
    fn three_refused_steals_move_to_the_other_stand() {
        let mut runtime = CakeStallRuntime::new();
        let main = at(BAKER_STALL.stand);
        let token = token(&runtime.begin(Some(28)));
        for _ in 1..RESET_AFTER_REFUSALS {
            assert_eq!(refuse(&mut runtime, token, &main)["kind"], "observe");
        }
        let reset = refuse(&mut runtime, token, &main);
        assert_eq!(reset["kind"], "on-reset", "{reset}");
        assert_eq!(
            reset["log"],
            "3 refused steals — swapping to the stand at (2669,3310)"
        );
        assert_eq!(runtime.next(token, &main)["kind"], "observe");
        let walk = runtime.next(token, &main);
        assert_eq!(walk["kind"], "walk-to", "{walk}");
        assert_eq!(
            (walk["x"].as_i64(), walk["z"].as_i64()),
            (Some(2669), Some(3310))
        );
        let alt = at(BAKER_STALL.stand_alt);
        assert_eq!(runtime.next(token, &alt)["kind"], "loc");

        let token = token_of_new_call(&mut runtime);
        let walk = runtime.next(token, &alt);
        assert_eq!(walk["kind"], "walk-to", "{walk}");
        assert_eq!(
            (walk["x"].as_i64(), walk["z"].as_i64()),
            (Some(2668), Some(3312))
        );
    }

    #[test]
    fn a_steal_clears_the_refusal_count() {
        let mut runtime = CakeStallRuntime::new();
        let token = token(&runtime.begin(Some(28)));
        for _ in 1..RESET_AFTER_REFUSALS {
            refuse(&mut runtime, token, &carrying(0));
        }
        assert_eq!(runtime.next(token, &carrying(0))["kind"], "loc");
        assert_eq!(runtime.next(token, &carrying(1))["kind"], "on-steal");
        assert_eq!(runtime.next(token, &carrying(1))["kind"], "observe");
        for _ in 1..RESET_AFTER_REFUSALS {
            assert_eq!(refuse(&mut runtime, token, &carrying(1))["kind"], "observe");
        }
        assert_eq!(
            refuse(&mut runtime, token, &carrying(1))["kind"],
            "on-reset"
        );
    }

    /// The server's "can't steal ... during combat" line resolves the steal
    /// at once, holds the next steal for `LOCKOUT_TICKS` from the same stand,
    /// and is not a watched-stand refusal (frozen `classifySteal` 'lockout').
    #[test]
    fn combat_lockout_line_waits_ten_ticks_and_is_not_a_refusal() {
        let mut runtime = CakeStallRuntime::new();
        let main = at(BAKER_STALL.stand);
        let token = token(&runtime.begin(Some(28)));
        for _ in 1..RESET_AFTER_REFUSALS {
            assert_eq!(refuse(&mut runtime, token, &main)["kind"], "observe");
        }
        let locked_at = |tick: i64| Observation {
            tick,
            chat_max_seq: 0,
            lockout_seq: Some(0),
            ..at(BAKER_STALL.stand)
        };

        assert_eq!(runtime.next(token, &main)["kind"], "loc");
        // The line resolves the steal before its resolve window runs out.
        assert_eq!(runtime.next(token, &locked_at(5))["kind"], "observe");
        let hold = runtime.next(token, &locked_at(5));
        assert_eq!(hold["kind"], "pause", "{hold}");
        assert_eq!(hold["status"], "waiting out the post-combat steal lockout");
        assert_eq!(hold["callbacks"], true);
        for tick in 5..5 + LOCKOUT_TICKS {
            assert_eq!(runtime.next(token, &locked_at(tick))["kind"], "wait");
        }

        // Same stand after the lockout; the old line does not resolve it.
        let after = locked_at(5 + LOCKOUT_TICKS);
        assert_eq!(runtime.next(token, &after)["kind"], "observe");
        assert_eq!(runtime.next(token, &after)["kind"], "loc");
        assert_eq!(runtime.next(token, &after)["kind"], "wait");
        // Only now is the third refusal counted.
        runtime.deadline = Some(Instant::now() - Duration::from_millis(1));
        assert_eq!(runtime.next(token, &after)["kind"], "on-reset");
    }

    #[test]
    fn caller_lockout_waits_and_abort_ends_it() {
        let mut runtime = CakeStallRuntime::new();
        let token = token(&runtime.begin(Some(28)));
        let locked = Observation {
            locked_out_until: Some(10),
            ..observation()
        };
        assert_eq!(runtime.next(token, &locked)["kind"], "pause");
        assert_eq!(runtime.next(token, &locked)["kind"], "wait");
        let aborted = Observation {
            abort: true,
            ..observation()
        };
        assert_eq!(runtime.next(token, &aborted)["kind"], "observe");
        assert_eq!(runtime.next(token, &aborted)["result"], "aborted");
    }

    /// An emptied stall is waited on inside the call, with the frozen log.
    #[test]
    fn emptied_stall_waits_for_the_restock_then_steals() {
        let mut runtime = CakeStallRuntime::new();
        let token = token(&runtime.begin(Some(28)));
        let empty = Observation {
            stall: None,
            ..observation()
        };
        let pause = runtime.next(token, &empty);
        assert_eq!(pause["kind"], "pause", "{pause}");
        assert_eq!(pause["log"], "stall emptied — waiting for the restock");
        assert_eq!(pause["callbacks"], true);
        assert_eq!(runtime.next(token, &empty)["kind"], "wait");
        assert_eq!(runtime.next(token, &observation())["kind"], "observe");
        assert_eq!(runtime.next(token, &observation())["kind"], "loc");
    }

    /// The walk to the stand is bounded; landing next to the stall still
    /// steals, landing short waits a tick and loops.
    #[test]
    fn claim_near_the_stall_steals_and_short_claim_retries() {
        let tile = |x: i32, z: i32| Tile { x, z, level: 0 };
        let mut runtime = CakeStallRuntime::new();
        let token = token(&runtime.begin(Some(28)));
        let far = Observation {
            here: Some(tile(2661, 3306)),
            ..observation()
        };
        assert_eq!(runtime.next(token, &far)["kind"], "walk-to");
        assert_eq!(runtime.next(token, &far)["kind"], "wait");
        runtime.deadline = Some(Instant::now() - Duration::from_millis(1));
        let short = runtime.next(token, &far);
        assert_eq!(short["kind"], "pause", "{short}");
        assert_eq!(
            short["log"],
            "claim fell short at (2661,3306) — not stealing from the market side"
        );
        assert_eq!(runtime.next(token, &far)["kind"], "wait");
        let later = Observation { tick: 2, ..far };
        assert_eq!(runtime.next(token, &later)["kind"], "observe");
        assert_eq!(runtime.next(token, &later)["kind"], "walk-to");
        runtime.deadline = Some(Instant::now() - Duration::from_millis(1));
        let beside = Observation {
            here: Some(tile(2666, 3312)),
            ..observation()
        };
        assert_eq!(runtime.next(token, &beside)["kind"], "loc");
    }
}
