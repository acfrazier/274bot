//! Rust-owned `stealRules` bank sequences: frozen `withdrawTo` (the
//! `bank_withdraw_to` [`crate::machine`] family) and
//! `closeBankAndConfirmCount` (`bank_close_confirm`), plus the frozen
//! `nextWithdrawChunk` ladder they and the hunt bank share.
//!
//! The caller's `count` callback is called through the callback path in the
//! frozen order; without one the count is `Inventory.count(name)` read from
//! the posted backpack. Pack-full is read from the scene.

use crate::bank_op::{js_number, same_name, BankView, Closing, Op, Sent};
use crate::machine::{Begin, Call, Cx, Family, Reply, Step, Thrown};
use crate::observed::{self, Scene};
use serde::Deserialize;
use serde_json::{json, Value};

/// Frozen withdraw rounds (`guard < 40`).
const ROUNDS: u32 = 40;
/// Frozen wait for a Withdraw-10/5/1 to land (`delayUntil(…, 2500)`).
pub const LAND_MS: u64 = 2_500;
/// Frozen `closeBankAndConfirmCount` count wait.
pub const CONFIRM_MS: u64 = 3_000;

/// One frozen `nextWithdrawChunk` step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Chunk {
    /// Withdraw-X for this many.
    X(f64),
    /// `Withdraw-10` / `Withdraw-5` / `Withdraw-1`.
    Op(&'static str),
}

/// Frozen `nextWithdrawChunk(need)`: Withdraw-X above 10, else the 10/5/1
/// ladder; nothing for no need.
pub(crate) fn next_chunk(need: f64) -> Option<Chunk> {
    if need <= 0.0 {
        None
    } else if need > 10.0 {
        Some(Chunk::X(need))
    } else if need >= 10.0 {
        Some(Chunk::Op("Withdraw-10"))
    } else if need >= 5.0 {
        Some(Chunk::Op("Withdraw-5"))
    } else {
        Some(Chunk::Op("Withdraw-1"))
    }
}

/// `Inventory.count(name)` from the posted backpack.
fn inventory_count(scene: &Scene, name: &str) -> f64 {
    scene.since_login().inv().map_or(0.0, |rows| {
        rows.iter()
            .filter(|row| row.name.as_deref().is_some_and(|got| same_name(got, name)))
            .map(|row| f64::from(row.count))
            .sum()
    })
}

/// `Inventory.isFull()` from the posted backpack.
fn inventory_full(scene: &Scene) -> bool {
    let session = scene.since_login();
    let size = session.inv_size().unwrap_or(0);
    let used = session.inv().map_or(0, |rows| {
        rows.iter()
            .filter(|row| row.name.as_deref().is_some_and(|name| !name.is_empty()))
            .count()
    });
    size > 0 && used >= usize::try_from(size).unwrap_or(usize::MAX)
}

/// The caller's `count` hook in both families.
const COUNT: usize = 0;

/// Reads the count: the caller's hook (one call, answered on the next
/// step), or `Inventory.count(name)` from the posted backpack when the hook
/// is optional (`name`) and absent. A required absent hook throws, as the
/// frozen call does.
struct Counter {
    name: Option<String>,
    asked: bool,
}

enum Counted {
    Ask(Call),
    Value(f64),
    Threw(Thrown),
}

impl Counter {
    fn read(&mut self, cx: &mut Cx<'_>) -> Counted {
        if let Some(name) = self.name.as_deref().filter(|_| !cx.has(COUNT)) {
            return Counted::Value(observed::with(|scene| inventory_count(scene, name)));
        }
        if !self.asked {
            self.asked = true;
            return Counted::Ask(Call {
                hook: COUNT,
                args: Vec::new(),
            });
        }
        self.asked = false;
        match cx.reply() {
            Some(Reply::Value(value)) => Counted::Value(js_number(&value)),
            Some(Reply::Threw(thrown)) => Counted::Threw(thrown),
            None => Counted::Value(f64::NAN),
        }
    }
}

#[derive(Clone, Copy)]
enum Phase {
    Start,
    /// The loop condition's `countInInv() < target`.
    Check,
    /// `const before = countInInv()`.
    Before,
    /// A Withdraw-X in flight.
    AwaitX {
        before: f64,
        need: f64,
    },
    /// `if (countInInv() > before) continue` after a Withdraw-X.
    AfterX {
        before: f64,
        need: f64,
    },
    /// A Withdraw-10/5/1 in flight.
    AwaitOp {
        before: f64,
    },
    /// `delayUntil(() => countInInv() > before, 2500)`.
    Land {
        before: f64,
    },
    /// `return countInInv() - start`.
    Finish,
}

#[derive(Deserialize)]
pub(crate) struct WithdrawToArgs {
    name: String,
    #[serde(default)]
    target: Value,
}

/// One awaited frozen `withdrawTo(name, target, countInInv)`.
pub(crate) struct WithdrawTo {
    name: String,
    target: f64,
    counter: Counter,
    start: f64,
    round: u32,
    phase: Phase,
    waiting: Option<crate::bank_op::Awaiting>,
}

impl WithdrawTo {
    /// Send the labelled withdraw and wait for it; its result is not read.
    fn withdraw_op(&mut self, label: &'static str, before: f64, cx: &mut Cx<'_>) {
        let op = Op::Withdraw {
            name: self.name.clone(),
            amount: json!(label),
        };
        match op.send(false, cx) {
            Sent::Awaiting(waiting) => {
                self.waiting = Some(waiting);
                self.phase = Phase::AwaitOp { before };
            }
            Sent::Settled(_) => self.land(before, cx),
        }
    }

    fn land(&mut self, before: f64, cx: &mut Cx<'_>) {
        cx.clock().arm(LAND_MS);
        self.phase = Phase::Land { before };
    }

    /// The Withdraw-X failed or gained nothing: one labelled op instead.
    fn fallback(&mut self, before: f64, need: f64, cx: &mut Cx<'_>) -> bool {
        match next_chunk(need.min(10.0)) {
            Some(Chunk::Op(label)) => {
                self.withdraw_op(label, before, cx);
                true
            }
            _ => false,
        }
    }
}

impl Family for WithdrawTo {
    const NAME: &'static str = "bank_withdraw_to";
    const CALLBACKS: &'static [&'static str] = &["count"];
    /// Frozen `countInInv()` is a synchronous call: a returned promise is a
    /// value (`NaN` once it meets a number).
    const AWAIT_CALLBACKS: bool = false;
    /// `const start = countInInv()` and the first withdraw join the
    /// caller's tick.
    const KICK_ON_START: bool = true;
    type Args = WithdrawToArgs;
    type Output = Value;

    fn begin(args: WithdrawToArgs, _cx: &mut Cx<'_>) -> Begin<Self> {
        // Another bank op is in flight: as the shim's pending guard, no op
        // of this run could be sent, so nothing is gained.
        if crate::bank_op::busy() {
            return Begin::Done(json!(0));
        }
        Begin::Run(Self {
            counter: Counter {
                name: Some(args.name.clone()),
                asked: false,
            },
            name: args.name,
            target: js_number(&args.target),
            start: 0.0,
            round: 0,
            phase: Phase::Start,
            waiting: None,
        })
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
        loop {
            let phase = self.phase;
            if matches!(phase, Phase::Check) && self.round >= ROUNDS {
                self.phase = Phase::Finish;
                continue;
            }
            let count = match phase {
                Phase::AwaitX { .. } | Phase::AwaitOp { .. } => None,
                _ => match self.counter.read(cx) {
                    Counted::Ask(call) => return Step::Call(call),
                    Counted::Threw(thrown) => return Step::Fail(thrown),
                    Counted::Value(count) => Some(count),
                },
            };
            let count = count.unwrap_or(f64::NAN);
            match phase {
                Phase::Start => {
                    self.start = count;
                    self.phase = Phase::Check;
                }
                Phase::Check => {
                    let full = observed::with(inventory_full);
                    self.phase = if count < self.target && !full {
                        Phase::Before
                    } else {
                        Phase::Finish
                    };
                }
                Phase::Before => {
                    let before = count;
                    let need = self.target - before;
                    match next_chunk(need) {
                        None => self.phase = Phase::Finish,
                        Some(Chunk::Op(label)) => self.withdraw_op(label, before, cx),
                        Some(Chunk::X(n)) => {
                            let op = Op::WithdrawX {
                                name: self.name.clone(),
                                count: json!(n),
                            };
                            match op.send(false, cx) {
                                Sent::Awaiting(waiting) => {
                                    self.waiting = Some(waiting);
                                    self.phase = Phase::AwaitX { before, need };
                                }
                                Sent::Settled(true) => self.phase = Phase::AfterX { before, need },
                                // Frozen `Bank.withdrawX` answers false with no row or op.
                                Sent::Settled(false) => {
                                    if !self.fallback(before, need, cx) {
                                        self.phase = Phase::Finish;
                                    }
                                }
                            }
                        }
                    }
                }
                Phase::AwaitX { before, need } => {
                    let Some(ok) = self.waiting.and_then(|w| w.poll(&BankView::now())) else {
                        return Step::Wait;
                    };
                    self.waiting = None;
                    if ok {
                        self.phase = Phase::AfterX { before, need };
                    } else if !self.fallback(before, need, cx) {
                        self.phase = Phase::Finish;
                    }
                }
                Phase::AfterX { before, need } => {
                    if count > before {
                        self.round += 1;
                        self.phase = Phase::Check;
                    } else if !self.fallback(before, need, cx) {
                        self.phase = Phase::Finish;
                    }
                }
                Phase::AwaitOp { before } => {
                    if self
                        .waiting
                        .and_then(|w| w.poll(&BankView::now()))
                        .is_none()
                    {
                        return Step::Wait;
                    }
                    self.waiting = None;
                    self.land(before, cx);
                }
                Phase::Land { before } => {
                    if count > before {
                        self.round += 1;
                        self.phase = Phase::Check;
                    } else if cx.clock().bound_reached() {
                        self.phase = Phase::Finish;
                    } else {
                        return Step::Wait;
                    }
                }
                Phase::Finish => return Step::Done(json!(count - self.start)),
            }
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct CloseConfirmArgs {
    #[serde(default)]
    expected: Value,
}

enum ClosePhase {
    Closing(Option<Closing>),
    /// `delayTicks(1)` after the close, from this posted tick.
    Tick(u64),
    Confirm,
}

/// One awaited frozen `closeBankAndConfirmCount(expected, count)`.
pub(crate) struct CloseConfirm {
    expected: f64,
    counter: Counter,
    phase: ClosePhase,
}

fn posted_tick() -> u64 {
    observed::with(|scene| scene.tick().unwrap_or(0))
}

impl Family for CloseConfirm {
    const NAME: &'static str = "bank_close_confirm";
    const CALLBACKS: &'static [&'static str] = &["count"];
    /// Frozen `count()` is a synchronous call.
    const AWAIT_CALLBACKS: bool = false;
    type Args = CloseConfirmArgs;
    type Output = bool;

    fn begin(args: CloseConfirmArgs, cx: &mut Cx<'_>) -> Begin<Self> {
        let closing = Closing::begin(&BankView::now(), cx);
        let phase = match closing {
            Some(closing) => ClosePhase::Closing(Some(closing)),
            None => ClosePhase::Tick(posted_tick()),
        };
        Begin::Run(Self {
            expected: js_number(&args.expected),
            counter: Counter {
                name: None,
                asked: false,
            },
            phase,
        })
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<bool> {
        loop {
            match &mut self.phase {
                ClosePhase::Closing(closing) => {
                    match closing.as_ref().and_then(|c| c.poll(&BankView::now(), cx)) {
                        None => return Step::Wait,
                        Some(false) => return Step::Done(false),
                        Some(true) => self.phase = ClosePhase::Tick(posted_tick()),
                    }
                }
                ClosePhase::Tick(from) => {
                    if posted_tick() <= *from {
                        return Step::Wait;
                    }
                    cx.clock().arm(CONFIRM_MS);
                    self.phase = ClosePhase::Confirm;
                }
                ClosePhase::Confirm => {
                    // The caller's `count` is required, as frozen.
                    let count = match self.counter.read(cx) {
                        Counted::Ask(call) => return Step::Call(call),
                        Counted::Threw(thrown) => return Step::Fail(thrown),
                        Counted::Value(count) => count,
                    };
                    if count >= self.expected {
                        return Step::Done(true);
                    }
                    if cx.clock().bound_reached() {
                        return Step::Done(false);
                    }
                    return Step::Wait;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::callback_v8::HeldCallback;
    use crate::machine::{self, Called, Js, Outcome, Pending, Reply, Started, Take};
    use crate::observed::{ItemRow, Ops};
    use crate::shim::InteractReq;
    use std::rc::Rc;

    /// Without a `count` hook the run reads `Inventory.count` itself.
    struct NoJs;

    impl Js for NoJs {
        fn queue_len(&mut self) -> usize {
            0
        }

        fn call(&mut self, _hook: Option<&HeldCallback>, _args: &[Value]) -> Called {
            panic!("no count hook was passed");
        }

        fn poll(&mut self, _pending: &Pending) -> Option<Reply> {
            panic!("no count hook was passed");
        }

        fn claimed(&mut self) -> bool {
            false
        }
    }

    fn row(name: &str, count: i32, ops: &[&str]) -> ItemRow {
        ItemRow {
            id: 379,
            count,
            name: Some(Rc::from(name)),
            ops: ops.iter().map(|op| Rc::from(*op)).collect::<Ops>(),
            ..ItemRow::default()
        }
    }

    /// An open bank holding 100 lobsters with `ops`, a pack holding `held`
    /// lobsters plus `filler` other rows, and the op result seq.
    fn post(tick: u64, ops: &[&str], held: i32, filler: usize, seq: u64) {
        let mut inv: Vec<ItemRow> = (0..filler).map(|_| row("Bones", 1, &[])).collect();
        if held > 0 {
            inv.push(row("Lobster", held, &[]));
        }
        let bank = vec![row("Lobster", 100, ops)];
        observed::post(tick, |post| {
            post.session(true)
                .bank_open(true)
                .bank_loaded(true)
                .bank_generation(3)
                .bank_op_result_seq(seq)
                .bank_op_result(true)
                .inv_size(28)
                .inv(inv)
                .bank(bank);
        });
    }

    fn start(target: i32) -> Started {
        machine::on_reset();
        machine::start(
            WithdrawTo::NAME,
            json!({ "name": "Lobster", "target": target }),
            Vec::new(),
            0,
        )
    }

    fn tick() -> Vec<InteractReq> {
        machine::step(&mut NoJs);
        machine::merge_ops(Vec::new())
    }

    fn running(started: Started) -> machine::Handle {
        match started {
            Started::Running(handle) => handle,
            other => panic!("expected a running row, got {other:?}"),
        }
    }

    const LADDER: [&str; 4] = ["Withdraw-1", "Withdraw-5", "Withdraw-10", "Withdraw-X"];

    /// Frozen `!Inventory.isFull()` ends the loop before any withdraw.
    #[test]
    fn a_full_pack_withdraws_nothing() {
        observed::on_reset();
        post(1, &LADDER, 2, 27, 0);
        let handle = running(start(22));
        assert!(tick().is_empty(), "a full pack sends no withdraw");
        assert_eq!(
            machine::take(handle),
            Take::Settled(Outcome::Done(json!(0.0)))
        );
    }

    /// A labelled withdraw that never lands ends the loop at the frozen
    /// 2.5 s bound, with no second withdraw.
    #[test]
    fn a_withdraw_that_never_lands_ends_at_the_land_bound() {
        observed::on_reset();
        post(1, &LADDER, 0, 0, 0);
        let handle = running(start(4));
        assert_eq!(
            tick(),
            vec![InteractReq::Withdraw {
                name: "Lobster".into(),
                action: "Withdraw-1".into(),
            }]
        );
        post(2, &LADDER, 0, 0, 1);
        assert!(
            tick().is_empty(),
            "the result is in; the pack has not grown"
        );
        assert_eq!(machine::take(handle), Take::Pending, "inside the bound");
        machine::tests::expire_deadlines();
        assert!(tick().is_empty(), "the lapsed bound sends nothing more");
        assert_eq!(
            machine::take(handle),
            Take::Settled(Outcome::Done(json!(0.0)))
        );
    }

    /// A Withdraw-X the bank row cannot take (no X op) falls back to the
    /// labelled Withdraw-10, as frozen does when `withdrawX` answers false.
    #[test]
    fn a_refused_withdraw_x_falls_back_to_the_labelled_ten() {
        observed::on_reset();
        post(1, &["Withdraw-1", "Withdraw-5", "Withdraw-10"], 2, 0, 0);
        let handle = running(start(22));
        assert_eq!(
            tick(),
            vec![InteractReq::Withdraw {
                name: "Lobster".into(),
                action: "Withdraw-10".into(),
            }]
        );
        post(2, &["Withdraw-1", "Withdraw-5", "Withdraw-10"], 12, 0, 1);
        assert_eq!(
            tick(),
            vec![InteractReq::Withdraw {
                name: "Lobster".into(),
                action: "Withdraw-10".into(),
            }],
            "landed: the next round needs 10"
        );
        post(3, &["Withdraw-1", "Withdraw-5", "Withdraw-10"], 22, 0, 2);
        assert!(tick().is_empty());
        assert_eq!(
            machine::take(handle),
            Take::Settled(Outcome::Done(json!(20.0)))
        );
    }

    #[test]
    fn next_chunk_is_the_frozen_ladder() {
        assert_eq!(next_chunk(0.0), None);
        assert_eq!(next_chunk(-3.0), None);
        assert_eq!(next_chunk(11.0), Some(Chunk::X(11.0)));
        assert_eq!(next_chunk(10.0), Some(Chunk::Op("Withdraw-10")));
        assert_eq!(next_chunk(7.0), Some(Chunk::Op("Withdraw-5")));
        assert_eq!(next_chunk(4.0), Some(Chunk::Op("Withdraw-1")));
    }
}
