//! PeriodicBank task bookkeeping and its native bank child.
//!
//! The task owns its status callbacks and pause-aware schedule. Bank selection,
//! access, deposit, afterDeposit, and return walking belong to the native seat.

use crate::machine::{Begin, Call, Cx, Family, NativeBankChild, Reply, Step, Thrown};
use crate::observed;
use api::snapshot::WorldTile as Tile;
use script::bank::compat::{read_destination, read_tile, truthy};
use script::bank::request::{BankSeatArgs, BankSeatDestination, BankSeatOperation};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::time::{Duration, Instant};

pub const FAILURE_BACKOFF_MS: u64 = 180_000;
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

/// Parse the exact `shouldBankNow` strategy tokens without label aliases.
pub(crate) fn parse_bank_strategy_token(raw: &str) -> Option<BankStrategy> {
    match raw {
        "off" => Some(BankStrategy::Off),
        "items" => Some(BankStrategy::Items),
        "time" => Some(BankStrategy::Time),
        "either" => Some(BankStrategy::Either),
        _ => None,
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

struct Run {
    child: NativeBankChild,
    missing_deposit: bool,
}

impl Run {
    fn step(&mut self, cx: &mut Cx<'_>) -> Step<bool> {
        match self.child.step(cx) {
            Step::Done(value) => Step::Done(!self.missing_deposit && value == Value::Bool(true)),
            Step::Wait => Step::Wait,
            Step::Call(call) => Step::Call(call),
            Step::Fail(thrown) => Step::Fail(thrown),
        }
    }
}

fn posted_tick() -> u64 {
    observed::with(|scene| scene.tick().unwrap_or(0))
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
    dest: Option<BankSeatDestination>,
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
    const SNAPSHOT_SENSITIVE: bool = true;
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
                        self.dest = value.as_ref().and_then(read_destination);
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
                        let missing_deposit = !cx.has(DEPOSIT);
                        let operation = if missing_deposit {
                            BankSeatOperation::Open
                        } else {
                            BankSeatOperation::BankNearest
                        };
                        let mut args = BankSeatArgs::new(operation);
                        args.set_destination(self.dest.take());
                        if missing_deposit {
                            args.set_hooks_present(u32::from(cx.has(LOG)));
                        } else {
                            args.set_return_to(self.return_to);
                            args.set_common_junk(self.common_junk);
                            args.set_hooks_present(
                                1 | (u32::from(cx.has(AFTER_DEPOSIT)) << 1)
                                    | (u32::from(cx.has(LOG)) << 2),
                            );
                        }
                        let hooks: &[(&str, usize)] = if missing_deposit {
                            &[("log", LOG)]
                        } else {
                            &[
                                ("deposit", DEPOSIT),
                                ("afterDeposit", AFTER_DEPOSIT),
                                ("log", LOG),
                            ]
                        };
                        match NativeBankChild::start(args, hooks, cx) {
                            Ok(child) => {
                                self.phase = TaskPhase::Run(Box::new(Run {
                                    child,
                                    missing_deposit,
                                }));
                            }
                            Err(error) => return Step::Fail(Thrown::new(error)),
                        }
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
