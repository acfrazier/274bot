//! Rust-owned PeriodicBank and `Banking.bankNearest`.
//!
//! - [`Run`] is one frozen `bankNearest`: share [`crate::banking_open`]'s
//!   local/preset/reachable selection and object/NPC access continuation,
//!   deposit through the caller's matcher ([`crate::bank_deposit`]), run
//!   `afterDeposit`, wait a tick,
//!   and walk back to `returnTo` (the bank left open, as frozen). It is the
//!   `bank_nearest` [`crate::machine`] family, and the body of
//!   `periodic_bank`.
//! - `periodic_bank` is frozen `PeriodicBank.execute`: status, the caller's
//!   destination/commonJunk/returnTo, one run, the bank clocks, the frozen
//!   log line and the 3-tick pause after a failure.
//! - The due check (`validate`) is one typed helper
//!   (`load/bank_tasks_v8.rs`) over [`should_bank_now`], [`suppressed`] and
//!   [`minutes_since_last_bank`]; the clocks freeze across Pause and
//!   guardian hold.
//!
//! Scene facts come from the isolate scene; script callbacks go through the
//! machine's callback path.

use crate::bank_deposit::{truthy, Deposit, Matcher};
use crate::banking_open::{read_dest, read_tile, BankingOpen, Dest};
use crate::load::reach_query::arrived;
use crate::machine::{Begin, Call, Cx, Family, Reply, Step};
use crate::observed;
use crate::shim::InteractReq;
use api::snapshot::WorldTile as Tile;
use serde::Deserialize;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::time::{Duration, Instant};

/// Frozen `walkResilient(returnTo, { timeoutMs: 120_000 })`.
pub const RETURN_WALK_MS: u64 = 120_000;
pub const FAILURE_BACKOFF_MS: u64 = 180_000;
pub const RETURN_RADIUS: i32 = 6;
/// Frozen `delayTicks(3)` after a failed run.
const FAIL_TICKS: u64 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BankStrategy {
    Off,
    Items,
    Time,
    Either,
}

/// Parse frozen labels, the shim token `loot`, and the source token `items`.
/// Those two tokens are not treated as identical strings; both map to Items.
pub fn parse_bank_strategy(raw: &str) -> BankStrategy {
    match raw.trim().to_ascii_lowercase().as_str() {
        "loot count" | "loot" | "items" => BankStrategy::Items,
        "time" => BankStrategy::Time,
        "either" => BankStrategy::Either,
        _ => BankStrategy::Off,
    }
}

/// Frozen `shouldBankNow`.
pub fn should_bank_now(
    strategy: BankStrategy,
    loot_count: f64,
    items_threshold: f64,
    minutes_since_last_bank: f64,
    minutes_threshold: f64,
) -> bool {
    if loot_count <= 0.0 {
        return false;
    }
    let by_items = loot_count >= items_threshold;
    let by_time = minutes_since_last_bank >= minutes_threshold;
    match strategy {
        BankStrategy::Off => false,
        BankStrategy::Items => by_items,
        BankStrategy::Time => by_time,
        BankStrategy::Either => by_items || by_time,
    }
}

/// `lastBankAt` / `suppressUntil`, frozen while paused or held.
struct Clocks {
    last_success: Instant,
    suppress_until: Option<Instant>,
    paused: bool,
    held: bool,
    frozen_at: Option<Instant>,
}

thread_local! {
    static CLOCKS: RefCell<Clocks> = RefCell::new(Clocks::new());
}

impl Clocks {
    fn new() -> Self {
        Self {
            last_success: Instant::now(),
            suppress_until: None,
            paused: false,
            held: false,
            frozen_at: None,
        }
    }

    fn now(&self) -> Instant {
        self.frozen_at.unwrap_or_else(Instant::now)
    }

    fn set_freeze(&mut self, paused: bool, held: bool) {
        let was = self.paused || self.held;
        self.paused = paused;
        self.held = held;
        let frozen = self.paused || self.held;
        if !was && frozen {
            self.frozen_at = Some(Instant::now());
        } else if was && !frozen {
            if let Some(at) = self.frozen_at.take() {
                let dt = Instant::now().saturating_duration_since(at);
                self.last_success += dt;
                if let Some(until) = self.suppress_until.as_mut() {
                    *until += dt;
                }
            }
        }
    }

    /// Frozen `execute`'s bookkeeping after `bankNearest`.
    fn record(&mut self, ok: bool) {
        let now = self.now();
        self.last_success = now;
        self.suppress_until = (!ok).then(|| now + Duration::from_millis(FAILURE_BACKOFF_MS));
    }
}

/// `performance.now() < this.suppressUntil`.
pub(crate) fn suppressed() -> bool {
    CLOCKS.with(|clocks| {
        let clocks = clocks.borrow();
        let now = clocks.now();
        clocks.suppress_until.is_some_and(|until| now < until)
    })
}

/// `(performance.now() - this.lastBankAt) / 60_000`.
pub(crate) fn minutes_since_last_bank() -> f64 {
    CLOCKS.with(|clocks| {
        let clocks = clocks.borrow();
        clocks
            .now()
            .saturating_duration_since(clocks.last_success)
            .as_secs_f64()
            / 60.0
    })
}

pub fn on_pause() {
    CLOCKS.with(|clocks| {
        let held = clocks.borrow().held;
        clocks.borrow_mut().set_freeze(true, held);
    });
}

pub fn on_resume() {
    CLOCKS.with(|clocks| {
        let held = clocks.borrow().held;
        clocks.borrow_mut().set_freeze(false, held);
    });
}

pub fn on_hold(held: bool) {
    CLOCKS.with(|clocks| {
        let paused = clocks.borrow().paused;
        clocks.borrow_mut().set_freeze(paused, held);
    });
}

/// What the run deposits.
pub(crate) enum DepositPlan {
    /// No deposit (the shim's falsy `deposit`).
    None,
    /// PeriodicBank without its required `deposit`: the run fails there.
    Missing,
    /// Everything (the shim's non-function `deposit`).
    All,
    /// `depositMatcher(hook, common)`.
    Hook { hook: usize, common: bool },
}

enum RunPhase {
    Open,
    Deposit(Deposit),
    AfterDeposit { asked: bool },
    Settle(u64),
    Return,
    ReturnWalk(Tile),
}

/// One frozen `Banking.bankNearest`.
pub(crate) struct Run {
    open: BankingOpen,
    return_to: Option<Tile>,
    deposit: Option<DepositPlan>,
    after_hook: Option<usize>,
    phase: RunPhase,
}

/// One decision: keep deciding, wait for the next tick, or end the step.
enum Next {
    Decide,
    Wait,
    Out(Step<bool>),
}

impl Run {
    pub(crate) fn new(
        dest: Option<Dest>,
        return_to: Option<Tile>,
        deposit: DepositPlan,
        after_hook: Option<usize>,
        log_hook: Option<usize>,
    ) -> Self {
        Self {
            open: BankingOpen::for_destination(dest, log_hook),
            return_to,
            deposit: Some(deposit),
            after_hook,
            phase: RunPhase::Open,
        }
    }

    pub(crate) fn step(&mut self, cx: &mut Cx<'_>) -> Step<bool> {
        loop {
            match self.decide(cx) {
                Next::Decide => {}
                Next::Wait => return Step::Wait,
                Next::Out(step) => return step,
            }
        }
    }

    fn to(&mut self, phase: RunPhase) -> Next {
        self.phase = phase;
        Next::Decide
    }

    fn decide(&mut self, cx: &mut Cx<'_>) -> Next {
        let phase = std::mem::replace(&mut self.phase, RunPhase::Return);
        match phase {
            RunPhase::Open => {
                self.phase = RunPhase::Open;
                match self.open.run(cx) {
                    Step::Done(true) => self.deposit_phase(),
                    Step::Done(false) => Next::Out(Step::Done(false)),
                    other => Next::Out(other),
                }
            }
            RunPhase::Deposit(mut deposit) => match deposit.step(cx) {
                Step::Done(()) => self.to(RunPhase::AfterDeposit { asked: false }),
                Step::Wait => {
                    self.phase = RunPhase::Deposit(deposit);
                    Next::Wait
                }
                Step::Call(call) => {
                    self.phase = RunPhase::Deposit(deposit);
                    Next::Out(Step::Call(call))
                }
                Step::Fail(thrown) => Next::Out(Step::Fail(thrown)),
            },
            RunPhase::AfterDeposit { asked } => {
                if let Some(hook) = self.after_hook.filter(|hook| cx.has(*hook)) {
                    if !asked {
                        self.phase = RunPhase::AfterDeposit { asked: true };
                        return Next::Out(Step::Call(Call {
                            hook,
                            args: Vec::new(),
                        }));
                    }
                    if let Some(Reply::Threw(thrown)) = cx.reply() {
                        return Next::Out(Step::Fail(thrown));
                    }
                }
                // Frozen `await Execution.delayTicks(1)`.
                self.to(RunPhase::Settle(posted_tick().saturating_add(1)))
            }
            RunPhase::Settle(due) => {
                if posted_tick() < due {
                    self.phase = RunPhase::Settle(due);
                    return Next::Wait;
                }
                // Frozen never closes the bank: the walk back (if any) starts
                // with it open.
                self.to(RunPhase::Return)
            }
            RunPhase::Return => {
                let Some(ret) = self.return_to else {
                    return Next::Out(Step::Done(true));
                };
                if arrived(ret, RETURN_RADIUS) {
                    return Next::Out(Step::Done(true));
                }
                cx.clock().arm(RETURN_WALK_MS);
                cx.emit(walk_near(ret, RETURN_RADIUS));
                self.phase = RunPhase::ReturnWalk(ret);
                Next::Wait
            }
            RunPhase::ReturnWalk(ret) => {
                // Frozen: the walk's own result does not fail the run.
                if arrived(ret, RETURN_RADIUS) || cx.clock().bound_reached() {
                    return Next::Out(Step::Done(true));
                }
                self.phase = RunPhase::ReturnWalk(ret);
                Next::Wait
            }
        }
    }

    fn deposit_phase(&mut self) -> Next {
        let matcher = match self.deposit.take() {
            None | Some(DepositPlan::None) => {
                return self.to(RunPhase::AfterDeposit { asked: false })
            }
            Some(DepositPlan::Missing) => return Next::Out(Step::Done(false)),
            Some(DepositPlan::All) => Matcher::All,
            Some(DepositPlan::Hook { hook, common }) => Matcher::Hook {
                hook,
                with_id: false,
                common,
            },
        };
        self.to(RunPhase::Deposit(Deposit::new(matcher, None)))
    }
}

fn walk_near(tile: Tile, radius: i32) -> InteractReq {
    InteractReq::WalkNear {
        x: tile.x,
        z: tile.z,
        level: tile.level,
        radius,
        allow_teleports: false,
        allow_wilderness: true,
        allow_bank_fetch: true,
        request_id: 0,
    }
}

fn posted_tick() -> u64 {
    observed::with(|scene| scene.tick().unwrap_or(0))
}

/// `Banking.bankNearest(opts)`: the shim passes the destination, return
/// tile, whether a non-function `deposit` means everything, and the common
/// junk flag; `deposit`/`afterDeposit`/`log` are hooks.
#[derive(Deserialize)]
pub(crate) struct BankNearestArgs {
    #[serde(default)]
    destination: Value,
    #[serde(default)]
    booth_name: Option<String>,
    #[serde(default)]
    booth_op: Option<String>,
    #[serde(default)]
    return_to: Value,
    #[serde(default)]
    deposit_all: bool,
    #[serde(default = "yes")]
    common_junk: bool,
}

fn yes() -> bool {
    true
}

const NEAREST_DEPOSIT: usize = 0;
const NEAREST_AFTER_DEPOSIT: usize = 1;
const NEAREST_LOG: usize = 2;

/// One awaited `Banking.bankNearest`.
pub(crate) struct BankNearest(Run);

impl Family for BankNearest {
    const NAME: &'static str = "bank_nearest";
    const CALLBACKS: &'static [&'static str] = &["deposit", "afterDeposit", "log"];
    /// Frozen awaits only `afterDeposit`; the matcher and `log` are
    /// synchronous calls.
    const SYNC_HOOKS: &'static [usize] = &[NEAREST_DEPOSIT, NEAREST_LOG];
    /// The first verb joins the caller's tick.
    const KICK_ON_START: bool = true;
    type Args = BankNearestArgs;
    type Output = bool;

    fn begin(args: BankNearestArgs, cx: &mut Cx<'_>) -> Begin<Self> {
        let deposit = if cx.has(NEAREST_DEPOSIT) {
            DepositPlan::Hook {
                hook: NEAREST_DEPOSIT,
                common: args.common_junk,
            }
        } else if args.deposit_all {
            DepositPlan::All
        } else {
            DepositPlan::None
        };
        let mut run = Run::new(
            read_dest(&args.destination),
            read_tile(&args.return_to),
            deposit,
            Some(NEAREST_AFTER_DEPOSIT),
            Some(NEAREST_LOG),
        );
        run.open.booth(args.booth_name, args.booth_op);
        Begin::Run(Self(run))
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<bool> {
        self.0.step(cx)
    }
}

const SET_STATUS: usize = 0;
const DEPOSIT: usize = 1;
const AFTER_DEPOSIT: usize = 2;
const DESTINATION: usize = 3;
const COMMON_JUNK: usize = 4;
const RETURN_TO: usize = 5;
const LOG: usize = 6;

enum TaskPhase {
    Status,
    Destination,
    CommonJunk,
    ReturnTo,
    Run(Box<Run>),
    Log(bool),
    Backoff(u64),
}

/// One awaited frozen `PeriodicBank.execute`.
pub(crate) struct PeriodicBank {
    phase: TaskPhase,
    asked: bool,
    dest: Option<Dest>,
    common_junk: bool,
    return_to: Option<Tile>,
}

impl PeriodicBank {
    /// Call `hook` once (when present) and hand its settled value to the
    /// next step; `None` while the call is out.
    fn ask(
        &mut self,
        hook: usize,
        args: Vec<Value>,
        cx: &mut Cx<'_>,
    ) -> Result<Option<Value>, Step<Value>> {
        if !cx.has(hook) {
            return Ok(Some(Value::Null));
        }
        if !self.asked {
            self.asked = true;
            return Err(Step::Call(Call { hook, args }));
        }
        self.asked = false;
        match cx.reply() {
            Some(Reply::Threw(thrown)) => Err(Step::Fail(thrown)),
            Some(Reply::Value(value)) => Ok(Some(value)),
            None => Ok(Some(Value::Null)),
        }
    }
}

impl Family for PeriodicBank {
    const NAME: &'static str = "periodic_bank";
    const CALLBACKS: &'static [&'static str] = &[
        "setStatus",
        "deposit",
        "afterDeposit",
        "destination",
        "commonJunk",
        "returnTo",
        "log",
    ];
    /// Frozen awaits only `afterDeposit` (inside `bankNearest`); every other
    /// option is a synchronous call.
    const SYNC_HOOKS: &'static [usize] = &[
        SET_STATUS,
        DEPOSIT,
        DESTINATION,
        COMMON_JUNK,
        RETURN_TO,
        LOG,
    ];
    /// Status, the caller's reads and the first verb join the caller's tick.
    const KICK_ON_START: bool = true;
    type Args = Value;
    type Output = Value;

    fn begin(_args: Value, _cx: &mut Cx<'_>) -> Begin<Self> {
        Begin::Run(Self {
            phase: TaskPhase::Status,
            asked: false,
            dest: None,
            common_junk: true,
            return_to: None,
        })
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
        loop {
            match &mut self.phase {
                TaskPhase::Status => {
                    if let Err(step) = self.ask(SET_STATUS, vec![json!("periodic bank run")], cx) {
                        return step;
                    }
                    self.phase = TaskPhase::Destination;
                }
                TaskPhase::Destination => match self.ask(DESTINATION, Vec::new(), cx) {
                    Err(step) => return step,
                    Ok(value) => {
                        self.dest = value.as_ref().and_then(read_dest);
                        self.phase = TaskPhase::CommonJunk;
                    }
                },
                TaskPhase::CommonJunk => match self.ask(COMMON_JUNK, Vec::new(), cx) {
                    Err(step) => return step,
                    // Frozen `commonJunk?.() ?? true`.
                    Ok(value) => {
                        self.common_junk = value.as_ref().is_none_or(|v| v.is_null() || truthy(v));
                        self.phase = TaskPhase::ReturnTo;
                    }
                },
                TaskPhase::ReturnTo => match self.ask(RETURN_TO, Vec::new(), cx) {
                    Err(step) => return step,
                    Ok(value) => {
                        self.return_to = value.as_ref().and_then(read_tile);
                        // `deposit` is required: a run without it fails at
                        // the deposit, as the shim's did.
                        let deposit = if cx.has(DEPOSIT) {
                            DepositPlan::Hook {
                                hook: DEPOSIT,
                                common: self.common_junk,
                            }
                        } else {
                            DepositPlan::Missing
                        };
                        self.phase = TaskPhase::Run(Box::new(Run::new(
                            self.dest.take(),
                            self.return_to,
                            deposit,
                            Some(AFTER_DEPOSIT),
                            Some(LOG),
                        )));
                    }
                },
                TaskPhase::Run(run) => match run.step(cx) {
                    Step::Wait => return Step::Wait,
                    Step::Call(call) => return Step::Call(call),
                    Step::Fail(thrown) => return Step::Fail(thrown),
                    Step::Done(ok) => {
                        CLOCKS.with(|clocks| clocks.borrow_mut().record(ok));
                        self.phase = TaskPhase::Log(ok);
                    }
                },
                TaskPhase::Log(ok) => {
                    let ok = *ok;
                    let line = if ok {
                        "periodic bank: completed"
                    } else {
                        "periodic bank: no bank reachable — will retry later"
                    };
                    if let Err(step) = self.ask(LOG, vec![json!(line)], cx) {
                        return step;
                    }
                    if ok {
                        return Step::Done(Value::Null);
                    }
                    self.phase = TaskPhase::Backoff(posted_tick().saturating_add(FAIL_TICKS));
                }
                TaskPhase::Backoff(due) => {
                    if posted_tick() < *due {
                        return Step::Wait;
                    }
                    return Step::Done(Value::Null);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_tokens_map_to_items_without_equating_loot_and_items_strings() {
        assert_eq!(parse_bank_strategy("Loot count"), BankStrategy::Items);
        assert_eq!(parse_bank_strategy("loot"), BankStrategy::Items);
        assert_eq!(parse_bank_strategy("items"), BankStrategy::Items);
        assert_eq!(parse_bank_strategy("Off"), BankStrategy::Off);
        assert_eq!(parse_bank_strategy("Time"), BankStrategy::Time);
        assert_eq!(parse_bank_strategy("Either"), BankStrategy::Either);
        assert_eq!(parse_bank_strategy("unknown"), BankStrategy::Off);
    }

    #[test]
    fn frozen_should_bank_now_contract() {
        assert!(!should_bank_now(BankStrategy::Items, 0.0, 1.0, 100.0, 1.0));
        assert!(should_bank_now(BankStrategy::Items, 15.0, 15.0, 0.0, 10.0));
        assert!(!should_bank_now(
            BankStrategy::Items,
            14.0,
            15.0,
            100.0,
            1.0
        ));
        assert!(should_bank_now(BankStrategy::Time, 1.0, 99.0, 10.0, 10.0));
        assert!(!should_bank_now(BankStrategy::Time, 1.0, 1.0, 9.9, 10.0));
        assert!(should_bank_now(BankStrategy::Either, 15.0, 15.0, 0.0, 10.0));
        assert!(should_bank_now(BankStrategy::Either, 1.0, 15.0, 10.0, 10.0));
        assert!(!should_bank_now(BankStrategy::Off, 27.0, 1.0, 100.0, 1.0));
    }

    #[test]
    fn failure_backoff_suppresses_and_pause_freezes_elapsed() {
        CLOCKS.with(|clocks| *clocks.borrow_mut() = Clocks::new());
        CLOCKS.with(|clocks| clocks.borrow_mut().record(false));
        assert!(suppressed(), "180 s backoff after a failed run");
        CLOCKS.with(|clocks| clocks.borrow_mut().record(true));
        assert!(!suppressed(), "a completed run clears the backoff");
        on_pause();
        let paused = minutes_since_last_bank();
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(
            minutes_since_last_bank(),
            paused,
            "Pause stops the bank clock"
        );
        on_resume();
        assert!(
            minutes_since_last_bank() - paused < 0.0002,
            "the paused span does not count as banking minutes"
        );
    }
}
