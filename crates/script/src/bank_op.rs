//! Rust-owned bank item ops: the `bank_op` [`crate::machine`] family, the
//! `bank_note_mode` family, and the pieces the bank sequences
//! (`bank_deposit`, `bank_withdraw_to`, `bank_nearest`) share.
//!
//! Deposit and labelled withdraw are one verb and one awaited host result
//! on `bank_op_result_seq`. `withdrawX*` is the frozen whole request: the
//! [`crate::bank::ops`] kernel picks each click, every click awaits its
//! `withdraw_x_result_seq`, and the request completes on the held count
//! (or a full pack after progress). A closed bank or a new bank session
//! settles false. JavaScript passes the caller's arguments and awaits the
//! boolean.

use crate::bank::ops::{self, NoteIntent, Progress, WithdrawGoal};
use crate::machine::{self, Begin, Cx, Family, Step};
use crate::observed::{self, ItemRow, Scene};
use crate::shim::InteractReq;
use serde::Deserialize;
use serde_json::Value;
use std::cell::Cell;
use std::sync::Arc;

/// `Bank.close()`'s own bound.
pub const CLOSE_MS: u64 = 3_000;

/// Families that drive bank item ops. One runs at a time, as the shim's
/// pending guards allowed: a second start settles false.
const BANK_OP_FAMILIES: &[&str] = &[
    BankOp::NAME,
    crate::bank_deposit::BankDeposit::NAME,
    crate::bank_withdraw::WithdrawTo::NAME,
];

/// Whether a bank-op family row is live.
pub(crate) fn busy() -> bool {
    BANK_OP_FAMILIES.iter().any(|family| machine::live(family))
}

/// The posted bank facts an op decides and settles from.
pub(crate) struct BankView {
    pub(crate) open: bool,
    /// `Bank.snapshotReady()`.
    pub(crate) loaded: bool,
    /// `Bank.loaded()`: the item list is non-empty.
    pub(crate) has_rows: bool,
    pub(crate) generation: u64,
    op_seq: u64,
    op_result: bool,
    x_seq: u64,
    x_result: bool,
    count_dialog_open: bool,
}

impl BankView {
    pub(crate) fn from_scene(scene: &Scene) -> Self {
        let session = scene.since_login();
        Self {
            open: session.bank_open().unwrap_or(false),
            loaded: session.bank_loaded().unwrap_or(false),
            has_rows: session.bank().is_some_and(|rows| !rows.is_empty()),
            generation: session.bank_generation().unwrap_or(0),
            op_seq: session.bank_op_result_seq().unwrap_or(0),
            op_result: session.bank_op_result().unwrap_or(false),
            x_seq: session.withdraw_x_result_seq().unwrap_or(0),
            x_result: session.withdraw_x_result().unwrap_or(false),
            count_dialog_open: session.count_dialog_open().unwrap_or(false),
        }
    }

    pub(crate) fn now() -> Self {
        observed::with(Self::from_scene)
    }

    /// `Bank.ready()`.
    pub(crate) fn ready(&self) -> bool {
        self.open && (self.loaded || self.has_rows)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Channel {
    Op,
    WithdrawX,
}

/// One sent op, waiting for its host result.
#[derive(Debug)]
pub(crate) struct Awaiting {
    channel: Channel,
    seq: u64,
    generation: u64,
    /// `withdrawX*`: the whole request this click serves.
    withdraw: Option<WithdrawGoal>,
}

impl Awaiting {
    fn send(channel: Channel, view: &BankView, req: InteractReq, cx: &mut Cx<'_>) -> Self {
        cx.emit(req);
        Self {
            channel,
            seq: match channel {
                Channel::Op => view.op_seq,
                Channel::WithdrawX => view.x_seq,
            },
            generation: view.generation,
            withdraw: None,
        }
    }

    /// The op's result once the host answered, the bank closed or a new
    /// bank session began. A `withdrawX*` click that landed short of its
    /// request sends the kernel's next click and keeps waiting.
    pub(crate) fn poll(&mut self, view: &BankView, cx: &mut Cx<'_>) -> Option<bool> {
        let (seq, result) = match self.channel {
            Channel::Op => (view.op_seq, view.op_result),
            Channel::WithdrawX => (view.x_seq, view.x_result),
        };
        let same_session = view.open && view.generation == self.generation;
        if same_session && seq == self.seq {
            return None;
        }
        let landed = same_session && seq != self.seq && result;
        let Some(goal) = self.withdraw.as_ref().filter(|_| landed) else {
            return Some(landed);
        };
        match observed::with(|scene| next_withdraw(scene, view, goal)) {
            Ok(done) => Some(done),
            Err(next) => {
                cx.emit(next);
                self.seq = view.x_seq;
                None
            }
        }
    }
}

/// After a landed `withdrawX*` click: `Ok` with the request's result, or
/// `Err` with the next click toward it.
fn next_withdraw(scene: &Scene, view: &BankView, goal: &WithdrawGoal) -> Result<bool, InteractReq> {
    let session = scene.since_login();
    let inv = session.inv().map(Vec::as_slice).unwrap_or_default();
    let held = ops::count_id(inv, goal.lands_as_id);
    let full = ops::pack_full(inv, session.inv_size().unwrap_or(0));
    let same_session = view.open && view.generation == goal.generation;
    match ops::withdraw_progress(goal, held, full, same_session) {
        Progress::Complete | Progress::PackFull => Ok(true),
        Progress::SessionGone | Progress::OverTarget => Ok(false),
        Progress::Incomplete => {
            if view.count_dialog_open {
                return Ok(false);
            }
            session
                .bank()
                .and_then(|bank| {
                    bank.iter()
                        .find(|row| row.id == goal.bank_item_id && row.count > 0)
                })
                .and_then(|row| ops::withdraw_click(row, ops::withdraw_remaining(goal, held), goal))
                .map_or(Ok(false), Err)
        }
    }
}

thread_local! {
    /// `Bank.setNoteMode`'s `(bank generation, intent)`: Rust-owned, read
    /// only for the open bank session it was set in.
    static NOTE_INTENT: Cell<(u64, NoteIntent)> = const { Cell::new((0, NoteIntent::Item)) };
}

/// A new connection: no note intent survives it.
pub(crate) fn on_reset() {
    NOTE_INTENT.set((0, NoteIntent::Item));
}

/// The script's note intent for the open bank session `generation`.
fn note_intent(generation: u64) -> NoteIntent {
    let (stored_generation, stored) = NOTE_INTENT.get();
    ops::note_intent(stored_generation, stored, generation)
}

/// One `Bank` item op, as the caller passed it.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum Op {
    /// `Bank.deposit(name)`: Deposit-All of the named backpack row.
    Deposit { name: String },
    /// `Bank.withdraw(name, amount)`: an action label, or a number.
    Withdraw {
        name: String,
        #[serde(default)]
        amount: Value,
    },
    /// `Bank.withdrawById(id, op)`.
    WithdrawId { id: Value, op: String },
    /// `Bank.withdrawX(name, count)`.
    WithdrawX {
        name: String,
        #[serde(default)]
        count: Value,
    },
    /// `Bank.withdrawXById(id, count, landsAsId)`.
    WithdrawXId {
        id: Value,
        #[serde(default)]
        count: Value,
        #[serde(default)]
        lands_as: Value,
    },
}

/// What sending an op did.
pub(crate) enum Sent {
    Awaiting(Awaiting),
    /// Settled without a verb (not ready, no row or op, a zero count):
    /// frozen answers the same boolean.
    Settled(bool),
}

impl Op {
    /// Decide and emit this op. `busy`: another op is in flight, which
    /// settles false where the shim's pending guard stood.
    pub(crate) fn send(&self, busy: bool, cx: &mut Cx<'_>) -> Sent {
        let view = BankView::now();
        observed::with(|scene| {
            let session = scene.since_login();
            let bank = session.bank().map(Vec::as_slice).unwrap_or_default();
            match self {
                Op::Deposit { name } => {
                    let side = session.bank_side().map(Vec::as_slice).unwrap_or_default();
                    let Some(row) = side.iter().find(|row| {
                        row.name
                            .as_deref()
                            .is_some_and(|got| ops::same_name(got, name))
                    }) else {
                        return Sent::Settled(false);
                    };
                    op_request(
                        &view,
                        busy,
                        InteractReq::Deposit {
                            name: row.name_or_empty().to_string(),
                        },
                        cx,
                    )
                }
                Op::Withdraw { name, amount } => {
                    let action = withdraw_action(bank, name, amount);
                    op_request(
                        &view,
                        busy,
                        InteractReq::Withdraw {
                            name: name.clone(),
                            action,
                        },
                        cx,
                    )
                }
                Op::WithdrawId { id, op } => {
                    let Some(row) = row_by_id(bank, id).filter(|row| has_name(row)) else {
                        return Sent::Settled(false);
                    };
                    let action = match op.to_lowercase().as_str() {
                        "withdraw-all" | "all" => "Withdraw All".to_string(),
                        _ => op.replace('-', " "),
                    };
                    op_request(
                        &view,
                        busy,
                        InteractReq::Withdraw {
                            name: row.name_or_empty().to_string(),
                            action,
                        },
                        cx,
                    )
                }
                Op::WithdrawX { name, count } => {
                    let amount = js_number(count);
                    if is_safe_integer(amount) && amount <= 0.0 {
                        return Sent::Settled(true);
                    }
                    let Some(row) = bank.iter().find(|row| {
                        row.name
                            .as_deref()
                            .is_some_and(|got| ops::same_name(got, name))
                    }) else {
                        return Sent::Settled(false);
                    };
                    // Frozen counts the backpack by name, so a noted landing
                    // counts: in Note mode the row lands as its note id.
                    let lands_as = match note_intent(view.generation) {
                        NoteIntent::Noted if !row.noted && row.cert >= 0 => row.cert,
                        _ => row.id,
                    };
                    withdraw_x(&view, busy, row, amount, f64::from(lands_as), scene, cx)
                }
                Op::WithdrawXId {
                    id,
                    count,
                    lands_as,
                } => {
                    let amount = js_number(count);
                    if is_safe_integer(amount) && amount <= 0.0 {
                        return Sent::Settled(true);
                    }
                    let Some(row) = row_by_id(bank, id).filter(|row| has_name(row)) else {
                        return Sent::Settled(false);
                    };
                    withdraw_x(&view, busy, row, amount, js_number(lands_as), scene, cx)
                }
            }
        })
    }
}

fn op_request(view: &BankView, busy: bool, req: InteractReq, cx: &mut Cx<'_>) -> Sent {
    if !view.ready() || busy {
        return Sent::Settled(false);
    }
    Sent::Awaiting(Awaiting::send(Channel::Op, view, req, cx))
}

/// The shim's amount → action: a label is used as is (`'all'` is Withdraw
/// All); a number withdraws all when it covers the row's whole count (and
/// is 10+), else 10, else 1.
fn withdraw_action(bank: &[ItemRow], name: &str, amount: &Value) -> String {
    if let Value::String(label) = amount {
        return if label.to_lowercase() == "all" {
            "Withdraw All".into()
        } else {
            label.clone()
        };
    }
    let n = js_number(amount);
    let count = bank
        .iter()
        .find(|row| {
            row.name
                .as_deref()
                .is_some_and(|got| ops::same_name(got, name))
        })
        .map_or(0.0, |row| f64::from(row.count));
    if n >= 10.0 && n >= count {
        "Withdraw All".into()
    } else if n >= 10.0 {
        "Withdraw 10".into()
    } else {
        "Withdraw 1".into()
    }
}

/// Frozen `withdrawX` / `withdrawXById`: admit the request, clip it once to
/// the row's stock, and send the kernel's first click. Later clicks follow
/// in [`Awaiting::poll`] until the held count reaches the clipped target or
/// the pack fills after progress.
fn withdraw_x(
    view: &BankView,
    busy: bool,
    row: &ItemRow,
    amount: f64,
    lands_as: f64,
    scene: &Scene,
    cx: &mut Cx<'_>,
) -> Sent {
    if !view.ready()
        || !is_safe_integer(amount)
        || amount <= 0.0
        || amount > f64::from(i32::MAX)
        || !is_safe_integer(lands_as)
        || lands_as < f64::from(i32::MIN)
        || lands_as > f64::from(i32::MAX)
        || view.count_dialog_open
        || busy
    {
        return Sent::Settled(false);
    }
    // A positive request for no stock is false, never an empty success.
    let available = row.count.max(0);
    if available == 0 {
        return Sent::Settled(false);
    }
    // In range: checked above.
    let lands_as = lands_as as i32;
    let inv = scene
        .since_login()
        .inv()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let goal = WithdrawGoal::available_limited(
        row.id,
        Arc::from(row.name_or_empty()),
        lands_as,
        amount as i32,
        available,
        ops::count_id(inv, lands_as),
        view.generation,
    );
    let Some(req) = ops::withdraw_click(row, ops::withdraw_remaining(&goal, goal.baseline), &goal)
    else {
        return Sent::Settled(false);
    };
    let mut waiting = Awaiting::send(Channel::WithdrawX, view, req, cx);
    waiting.withdraw = Some(goal);
    Sent::Awaiting(waiting)
}

/// Frozen `withdrawOp` amounts (`api/bank/bankOps.ts:5`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WithdrawAmount {
    All,
    Ten,
    Five,
    X,
    One,
    Any,
}

impl WithdrawAmount {
    /// The frozen `switch (amount)` cases; anything else matches none.
    pub(crate) fn parse(amount: &str) -> Option<Self> {
        Some(match amount {
            "all" => Self::All,
            "10" => Self::Ten,
            "5" => Self::Five,
            "x" => Self::X,
            "1" => Self::One,
            "any" => Self::Any,
            _ => return None,
        })
    }
}

const WITHDRAW: &str = "withdraw";

/// The text after `withdraw[\s-]*` at byte `at` of `op` (ASCII-folded
/// `withdraw`), or `None` when `withdraw` does not start there.
fn after_withdraw(op: &str, at: usize) -> Option<&str> {
    let word = op.get(at..at + WITHDRAW.len())?;
    word.eq_ignore_ascii_case(WITHDRAW).then(|| {
        op[at + WITHDRAW.len()..].trim_start_matches(|c: char| c.is_whitespace() || c == '-')
    })
}

/// Frozen `withdrawOp(ops, amount)` (`api/bank/bankOps.ts:5-21`): the index
/// of the first op whose label matches the amount's pattern, read off the
/// row's own ops. Unanchored patterns match `withdraw` anywhere in the label.
pub(crate) fn withdraw_op<'a>(
    ops: impl IntoIterator<Item = &'a str>,
    amount: WithdrawAmount,
) -> Option<usize> {
    let unanchored = |op: &str, rest_ok: fn(&str) -> bool| {
        (0..op.len()).any(|at| after_withdraw(op, at).is_some_and(rest_ok))
    };
    ops.into_iter().position(|op| match amount {
        // `/withdraw[\s-]*all/i`
        WithdrawAmount::All => unanchored(op, |rest| {
            rest.get(..3)
                .is_some_and(|all| all.eq_ignore_ascii_case("all"))
        }),
        // `/withdraw[\s-]*10/i`
        WithdrawAmount::Ten => unanchored(op, |rest| rest.starts_with("10")),
        // `/withdraw[\s-]*5\b/i`
        WithdrawAmount::Five => unanchored(op, |rest| {
            rest.strip_prefix('5').is_some_and(|tail| {
                !tail
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
            })
        }),
        // `/withdraw[\s-]*x/i`
        WithdrawAmount::X => unanchored(op, |rest| rest.starts_with('x') || rest.starts_with('X')),
        // `/^withdraw[\s-]*1$/i`
        WithdrawAmount::One => after_withdraw(op, 0) == Some("1"),
        // `/^withdraw/i`
        WithdrawAmount::Any => after_withdraw(op, 0).is_some(),
    })
}

fn row_by_id<'a>(rows: &'a [ItemRow], id: &Value) -> Option<&'a ItemRow> {
    let id = id.as_f64()?;
    rows.iter().find(|row| f64::from(row.id) == id)
}

fn has_name(row: &ItemRow) -> bool {
    row.name.as_deref().is_some_and(|name| !name.is_empty())
}

/// JS `Number(value)` for the values a caller passes (an absent argument
/// arrives as `null` and reads as `undefined`: NaN).
pub(crate) fn js_number(value: &Value) -> f64 {
    match value {
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::String(s) => {
            let s = s.trim();
            if s.is_empty() {
                0.0
            } else {
                s.parse().unwrap_or(f64::NAN)
            }
        }
        _ => f64::NAN,
    }
}

/// JS `Number.isSafeInteger`.
pub(crate) fn is_safe_integer(n: f64) -> bool {
    n.is_finite() && n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0
}

/// `Bank.close()`: nothing to do when the bank is already closed.
pub(crate) struct Closing;

impl Closing {
    /// `None` when the bank is not open (the close is already true).
    pub(crate) fn begin(view: &BankView, cx: &mut Cx<'_>) -> Option<Self> {
        if !view.open {
            return None;
        }
        cx.clock().arm(CLOSE_MS);
        cx.emit(InteractReq::Close);
        Some(Self)
    }

    pub(crate) fn poll(&self, view: &BankView, cx: &mut Cx<'_>) -> Option<bool> {
        if !view.open {
            Some(true)
        } else if cx.clock().bound_reached() {
            Some(false)
        } else {
            None
        }
    }
}

/// One `Bank` item op awaited by the script.
pub(crate) struct BankOp {
    waiting: Awaiting,
}

impl Family for BankOp {
    const NAME: &'static str = "bank_op";
    type Args = Op;
    type Output = bool;

    fn begin(op: Op, cx: &mut Cx<'_>) -> Begin<Self> {
        match op.send(busy(), cx) {
            Sent::Awaiting(waiting) => Begin::Run(Self { waiting }),
            Sent::Settled(ok) => Begin::Done(ok),
        }
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<bool> {
        match self.waiting.poll(&BankView::now(), cx) {
            Some(ok) => Step::Done(ok),
            None => Step::Wait,
        }
    }
}

/// `Bank.setNoteMode(on)`'s argument.
#[derive(Deserialize)]
pub(crate) struct NoteModeArgs {
    #[serde(default)]
    on: Value,
}

/// Frozen `Bank.setNoteMode(on): Promise<void>`: a closed bank does
/// nothing; an open one presses the live Note/Item control, records the
/// intent for this bank session, and waits one tick. It never throws.
pub(crate) struct BankNoteMode {
    from_tick: u64,
}

impl Family for BankNoteMode {
    const NAME: &'static str = "bank_note_mode";
    type Args = NoteModeArgs;
    type Output = Value;

    fn begin(args: NoteModeArgs, cx: &mut Cx<'_>) -> Begin<Self> {
        let view = BankView::now();
        if !view.open {
            return Begin::Done(Value::Null);
        }
        let want = if crate::bank_deposit::truthy(&args.on) {
            NoteIntent::Noted
        } else {
            NoteIntent::Item
        };
        cx.emit(ops::note_req(want));
        NOTE_INTENT.set((view.generation, want));
        Begin::Run(Self {
            from_tick: observed::with(|scene| scene.tick().unwrap_or(0)),
        })
    }

    /// Frozen `delayTicks(1)`.
    fn step(&mut self, _cx: &mut Cx<'_>) -> Step<Value> {
        if observed::with(|scene| scene.tick().unwrap_or(0)) > self.from_tick {
            Step::Done(Value::Null)
        } else {
            Step::Wait
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::callback_v8::HeldCallback;
    use crate::machine::{Called, Js, Outcome, Pending, Reply, Started, Take};
    use crate::observed::Ops;
    use serde_json::json;
    use std::rc::Rc;

    /// A bank op calls no script callback.
    struct NoJs;

    impl Js for NoJs {
        fn queue_len(&mut self) -> usize {
            0
        }

        fn call(&mut self, _hook: Option<&HeldCallback>, _args: &[Value]) -> Called {
            panic!("a bank op calls no script callback");
        }

        fn poll(&mut self, _pending: &Pending) -> Option<Reply> {
            panic!("a bank op calls no script callback");
        }

        fn claimed(&mut self) -> bool {
            false
        }
    }

    fn row(name: &str, id: i32, count: i32, ops: &[&str]) -> ItemRow {
        ItemRow {
            id,
            count,
            name: Some(Rc::from(name)),
            ops: ops.iter().map(|op| Rc::from(*op)).collect::<Ops>(),
            ..ItemRow::default()
        }
    }

    fn post(bank: Vec<ItemRow>, seq: u64, result: bool) {
        observed::post(0, |post| {
            post.session(true)
                .bank_open(true)
                .bank_loaded(true)
                .bank_generation(3)
                .bank_op_result_seq(seq)
                .bank_op_result(result)
                .bank(bank);
        });
    }

    fn reset() {
        machine::on_reset();
        observed::on_reset();
    }

    fn start(args: Value) -> Started {
        machine::start(BankOp::NAME, args, Vec::new(), 0)
    }

    fn drain() -> Vec<InteractReq> {
        machine::merge_ops(Vec::new())
    }

    fn withdrawn(amount: Value) -> String {
        reset();
        post(vec![row("Lobster", 379, 12, &[])], 0, false);
        let started = start(json!({ "kind": "withdraw", "name": "lobster", "amount": amount }));
        assert!(matches!(started, Started::Running(_)), "{started:?}");
        match drain().as_slice() {
            [InteractReq::Withdraw { action, .. }] => action.clone(),
            other => panic!("expected one withdraw, got {other:?}"),
        }
    }

    /// The amount → action decision the shim made: labels verbatim, a
    /// number takes all only when it covers the whole row.
    #[test]
    fn withdraw_amount_picks_the_shim_action() {
        assert_eq!(withdrawn(json!("all")), "Withdraw All");
        assert_eq!(withdrawn(json!("Withdraw 5")), "Withdraw 5");
        assert_eq!(withdrawn(json!(12)), "Withdraw All");
        assert_eq!(withdrawn(json!(11)), "Withdraw 10");
        assert_eq!(withdrawn(json!(4)), "Withdraw 1");
        assert_eq!(withdrawn(Value::Null), "Withdraw 1");
    }

    /// The first click of a frozen `withdrawX`: an exact Withdraw-1/5/10,
    /// else Withdraw-X, else the largest fixed op not above the need (native
    /// wins: a fixed-only row still serves 7 as 5 + 1 + 1). A row with no
    /// withdraw op, no row, or no stock answers frozen `false` with no verb.
    #[test]
    fn withdraw_x_picks_the_fixed_op_then_x() {
        let x_action = |ops: &[&str], name: &str, count: i32, stock: i32| {
            reset();
            post(vec![row("Feather", 314, stock, ops)], 0, false);
            let started = start(json!({ "kind": "withdraw-x", "name": name, "count": count }));
            match started {
                Started::Settled(Outcome::Done(value)) => {
                    assert!(drain().is_empty(), "a settled start sends nothing");
                    value.to_string()
                }
                _ => match drain().as_slice() {
                    [InteractReq::WithdrawX { action, count, .. }] => format!("{action}/{count}"),
                    other => panic!("expected one withdraw-x, got {other:?}"),
                },
            }
        };
        let ops = ["Withdraw-1", "Withdraw-5", "Withdraw-10", "Withdraw-X"];
        assert_eq!(x_action(&ops, "Feather", 5, 50), "Withdraw-5/5");
        assert_eq!(x_action(&ops, "Feather", 7, 50), "Withdraw-X/7");
        assert_eq!(
            x_action(&["Withdraw-X"], "Feather", 10, 50),
            "Withdraw-X/10"
        );
        assert_eq!(
            x_action(&ops, "Feather", 80, 50),
            "Withdraw-X/50",
            "take caps at the row count"
        );
        assert_eq!(x_action(&["Withdraw-1"], "Feather", 7, 50), "Withdraw-1/1");
        assert_eq!(x_action(&["Withdraw_1"], "Feather", 1, 50), "Withdraw_1/1");
        assert_eq!(
            x_action(&["Deposit-1"], "Feather", 7, 50),
            "false",
            "no usable op"
        );
        assert_eq!(x_action(&ops, "Arrow", 7, 50), "false", "no bank row");
        assert_eq!(x_action(&ops, "Feather", 7, 0), "false", "no stock");
        assert_eq!(x_action(&ops, "Feather", 0, 0), "true", "nothing asked");
        assert_eq!(x_action(&ops, "Feather", -3, 50), "true", "nothing asked");
        // The published frozen `withdrawOp` stays the regex helper.
        assert_eq!(withdraw_op(["Withdraw_1"], WithdrawAmount::One), None);
        assert_eq!(withdraw_op(["Withdraw-1"], WithdrawAmount::One), Some(0));
    }

    fn held(id: i32, count: i32) -> ItemRow {
        ItemRow {
            id,
            count,
            ..ItemRow::default()
        }
    }

    /// A bank-open post carrying the withdraw-x result channel and the pack.
    fn post_x(bank: Vec<ItemRow>, inv: Vec<ItemRow>, generation: u64, seq: u64, ok: bool) {
        observed::post(seq, |post| {
            post.session(true)
                .bank_open(true)
                .bank_loaded(true)
                .bank_generation(generation)
                .withdraw_x_result_seq(seq)
                .withdraw_x_result(ok)
                .inv_size(28)
                .inv(inv)
                .bank(bank);
        });
    }

    fn x_click() -> (String, i32, i32) {
        match drain().as_slice() {
            [InteractReq::WithdrawX {
                action,
                count,
                lands_as_id,
                ..
            }] => (action.clone(), *count, *lands_as_id),
            other => panic!("expected one withdraw-x, got {other:?}"),
        }
    }

    /// P-ladder (compat): one landed click is not the request. A fixed-only
    /// 7 is clicked 5 + 1 + 1 and completes on the held count.
    #[test]
    fn withdraw_x_loops_until_the_held_count_completes() {
        reset();
        let fixed = ["Withdraw-1", "Withdraw-5", "Withdraw-10"];
        let bank = |stock| vec![row("Feather", 314, stock, &fixed)];
        post_x(bank(50), vec![held(314, 2)], 3, 1, false);
        let Started::Running(handle) =
            start(json!({ "kind": "withdraw-x", "name": "Feather", "count": 7 }))
        else {
            panic!("expected a running withdraw");
        };
        assert_eq!(x_click(), ("Withdraw-5".into(), 5, 314));
        post_x(bank(45), vec![held(314, 7)], 3, 2, true);
        machine::step(&mut NoJs);
        assert_eq!(machine::take(handle), Take::Pending);
        assert_eq!(x_click(), ("Withdraw-1".into(), 1, 314));
        post_x(bank(44), vec![held(314, 8)], 3, 3, true);
        machine::step(&mut NoJs);
        assert_eq!(x_click(), ("Withdraw-1".into(), 1, 314));
        post_x(bank(43), vec![held(314, 9)], 3, 4, true);
        machine::step(&mut NoJs);
        assert!(drain().is_empty());
        assert_eq!(
            machine::take(handle),
            Take::Settled(Outcome::Done(json!(true)))
        );

        // Stock clipped once at begin: 7 asked, 5 banked, done at +5.
        reset();
        post_x(bank(5), Vec::new(), 3, 1, false);
        let Started::Running(handle) =
            start(json!({ "kind": "withdraw-x", "name": "Feather", "count": 7 }))
        else {
            panic!("expected a running withdraw");
        };
        assert_eq!(x_click(), ("Withdraw-5".into(), 5, 314));
        post_x(bank(0), vec![held(314, 5)], 3, 2, true);
        machine::step(&mut NoJs);
        assert!(drain().is_empty());
        assert_eq!(
            machine::take(handle),
            Take::Settled(Outcome::Done(json!(true)))
        );

        // A full pack after progress ends the frozen request true.
        reset();
        post_x(bank(50), Vec::new(), 3, 1, false);
        let Started::Running(handle) =
            start(json!({ "kind": "withdraw-x", "name": "Feather", "count": 7 }))
        else {
            panic!("expected a running withdraw");
        };
        let _ = x_click();
        let full: Vec<ItemRow> = (0..28).map(|id| held(1000 + id, 1)).collect();
        let mut full = full;
        full[0] = held(314, 5);
        post_x(bank(45), full, 3, 2, true);
        machine::step(&mut NoJs);
        assert!(drain().is_empty());
        assert_eq!(
            machine::take(handle),
            Take::Settled(Outcome::Done(json!(true)))
        );

        // A failed click or a new bank session ends it false.
        reset();
        post_x(bank(50), Vec::new(), 3, 1, false);
        let Started::Running(handle) =
            start(json!({ "kind": "withdraw-x", "name": "Feather", "count": 7 }))
        else {
            panic!("expected a running withdraw");
        };
        let _ = x_click();
        post_x(bank(45), vec![held(314, 5)], 4, 2, true);
        machine::step(&mut NoJs);
        assert!(drain().is_empty());
        assert_eq!(
            machine::take(handle),
            Take::Settled(Outcome::Done(json!(false)))
        );
    }

    fn start_note(on: bool) -> Started {
        machine::start(BankNoteMode::NAME, json!({ "on": on }), Vec::new(), 0)
    }

    /// P-note (compat): `setNoteMode` is void, a closed-bank no-op, and an
    /// intent for the open bank session only. An explicit `landsAsId` is
    /// counted as given whatever the intent, and no compat withdraw forces
    /// Item mode.
    #[test]
    fn note_mode_is_session_scoped_and_lands_as_is_explicit() {
        reset();
        on_reset();
        observed::post(1, |post| {
            post.session(true).bank_open(false);
        });
        assert!(matches!(
            start_note(true),
            Started::Settled(Outcome::Done(Value::Null))
        ));
        assert!(drain().is_empty(), "a closed bank sends no verb");

        // withdrawXById counts the explicit landing id even with Item intent.
        let mut lobster = row("Lobster", 379, 9, &["Withdraw-1", "Withdraw-X"]);
        lobster.cert = 380;
        post_x(vec![lobster.clone()], Vec::new(), 3, 1, false);
        let Started::Running(handle) =
            start(json!({ "kind": "withdraw-x-id", "id": 379, "count": 3, "lands_as": 380 }))
        else {
            panic!("expected a running withdraw");
        };
        assert_eq!(x_click(), ("Withdraw-X".into(), 3, 380));
        post_x(vec![lobster.clone()], vec![held(380, 3)], 3, 2, true);
        machine::step(&mut NoJs);
        assert_eq!(
            machine::take(handle),
            Take::Settled(Outcome::Done(json!(true)))
        );

        // setNoteMode(true): one verb, void after a tick; the next withdraw
        // sends no Item press and lands by name as the note.
        let Started::Running(note) = start_note(true) else {
            panic!("an open bank presses the toggle");
        };
        assert_eq!(drain(), vec![InteractReq::SetNoteMode { on: true }]);
        machine::step(&mut NoJs);
        assert_eq!(machine::take(note), Take::Pending);
        post_x(vec![lobster.clone()], vec![held(380, 3)], 3, 3, false);
        machine::step(&mut NoJs);
        assert_eq!(
            machine::take(note),
            Take::Settled(Outcome::Done(Value::Null))
        );
        assert!(matches!(
            start(json!({ "kind": "withdraw-x", "name": "Lobster", "count": 2 })),
            Started::Running(_)
        ));
        assert_eq!(
            x_click(),
            ("Withdraw-X".into(), 2, 380),
            "no Item press first"
        );

        // A reopened bank (new generation) is back in Item mode.
        reset();
        post_x(vec![lobster], Vec::new(), 4, 1, false);
        assert!(matches!(
            start(json!({ "kind": "withdraw-x", "name": "Lobster", "count": 2 })),
            Started::Running(_)
        ));
        assert_eq!(x_click(), ("Withdraw-X".into(), 2, 379));
    }

    /// The op settles on a new result for the same bank session; a closed
    /// bank settles false.
    #[test]
    fn op_settles_on_the_result_seq_and_a_closed_bank() {
        reset();
        post(vec![row("Lobster", 379, 12, &[])], 4, false);
        let Started::Running(handle) =
            start(json!({ "kind": "withdraw", "name": "Lobster", "amount": "all" }))
        else {
            panic!("expected a running op");
        };
        machine::step(&mut NoJs);
        assert_eq!(machine::take(handle), Take::Pending);
        post(vec![row("Lobster", 379, 12, &[])], 5, true);
        machine::step(&mut NoJs);
        assert_eq!(
            machine::take(handle),
            Take::Settled(Outcome::Done(json!(true)))
        );

        let Started::Running(handle) =
            start(json!({ "kind": "withdraw", "name": "Lobster", "amount": 1 }))
        else {
            panic!("expected a running op");
        };
        observed::post(1, |post| {
            post.bank_open(false);
        });
        machine::step(&mut NoJs);
        assert_eq!(
            machine::take(handle),
            Take::Settled(Outcome::Done(json!(false)))
        );
    }
}
