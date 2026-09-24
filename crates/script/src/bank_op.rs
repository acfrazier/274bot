//! Rust-owned bank item ops: the `bank_op` [`crate::machine`] family and
//! the pieces the bank sequences (`bank_deposit`, `bank_withdraw_to`,
//! `bank_nearest`) share.
//!
//! One op is one verb and one awaited host result: deposit and withdraw
//! settle on `bank_op_result_seq`, Withdraw-X on `withdraw_x_result_seq`; a
//! closed bank or a new bank session settles false. JavaScript passes the
//! caller's arguments and awaits the boolean. The withdraw amount → action
//! decision and the Withdraw-X op choice are made here from the posted bank
//! rows, the way the shim did.

use crate::machine::{self, Begin, Cx, Family, Step};
use crate::observed::{self, ItemRow, Scene};
use crate::shim::InteractReq;
use serde::Deserialize;
use serde_json::Value;

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
#[derive(Clone, Copy, Debug)]
pub(crate) struct Awaiting {
    channel: Channel,
    seq: u64,
    generation: u64,
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
        }
    }

    /// The op's result once the host answered, the bank closed or a new
    /// bank session began.
    pub(crate) fn poll(&self, view: &BankView) -> Option<bool> {
        let (seq, result) = match self.channel {
            Channel::Op => (view.op_seq, view.op_result),
            Channel::WithdrawX => (view.x_seq, view.x_result),
        };
        let same_session = view.open && view.generation == self.generation;
        if same_session && seq == self.seq {
            return None;
        }
        Some(same_session && seq != self.seq && result)
    }
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
    /// Settled without a verb (not ready, no row, a zero count).
    Settled(bool),
    /// The shim's explicit `not impl` (`Bank.withdrawX` with no row or op).
    NotImpl(&'static str),
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
                    let Some(row) = side
                        .iter()
                        .find(|row| row.name.as_deref().is_some_and(|got| same_name(got, name)))
                    else {
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
                    let Some(row) = bank
                        .iter()
                        .find(|row| row.name.as_deref().is_some_and(|got| same_name(got, name)))
                    else {
                        return Sent::NotImpl("Bank.withdrawX");
                    };
                    withdraw_x(&view, busy, row, amount, f64::from(row.id), cx)
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
                    withdraw_x(&view, busy, row, amount, js_number(lands_as), cx)
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
        .find(|row| row.name.as_deref().is_some_and(|got| same_name(got, name)))
        .map_or(0.0, |row| f64::from(row.count));
    if n >= 10.0 && n >= count {
        "Withdraw All".into()
    } else if n >= 10.0 {
        "Withdraw 10".into()
    } else {
        "Withdraw 1".into()
    }
}

/// The shim's `withdrawXRow`: Withdraw-1/5/10 when the take is one of those
/// and the row offers it, else the row's Withdraw-X op.
fn withdraw_x(
    view: &BankView,
    busy: bool,
    row: &ItemRow,
    amount: f64,
    lands_as: f64,
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
    let available = row.count.max(0);
    if available == 0 {
        return Sent::Settled(false);
    }
    // In range: checked above.
    let take = (amount as i32).min(available);
    let fixed = matches!(take, 1 | 5 | 10)
        .then(|| {
            row.ops
                .iter()
                .find(|action| !action.is_empty() && fixed_withdraw(action, take))
        })
        .flatten();
    let Some(action) = fixed.or_else(|| {
        row.ops.iter().find(|action| {
            !action.is_empty() && action.replace('-', " ").trim().to_lowercase() == "withdraw x"
        })
    }) else {
        return Sent::NotImpl("Bank.withdrawX");
    };
    let req = InteractReq::WithdrawX {
        name: row.name_or_empty().to_string(),
        count: take,
        bank_item_id: row.id,
        lands_as_id: lands_as as i32,
        action: action.to_string(),
        bank_generation: view.generation,
    };
    Sent::Awaiting(Awaiting::send(Channel::WithdrawX, view, req, cx))
}

/// `/^withdraw[\s-]*<take>$/i` on the trimmed label.
fn fixed_withdraw(action: &str, take: i32) -> bool {
    let action = action.trim().to_lowercase();
    action.strip_prefix("withdraw").is_some_and(|rest| {
        rest.trim_start_matches(|c: char| c.is_whitespace() || c == '-') == take.to_string()
    })
}

fn row_by_id<'a>(rows: &'a [ItemRow], id: &Value) -> Option<&'a ItemRow> {
    let id = id.as_f64()?;
    rows.iter().find(|row| f64::from(row.id) == id)
}

fn has_name(row: &ItemRow) -> bool {
    row.name.as_deref().is_some_and(|name| !name.is_empty())
}

/// JS `a.toLowerCase() === b.toLowerCase()` without allocating for ASCII.
pub(crate) fn same_name(a: &str, b: &str) -> bool {
    if a.is_ascii() && b.is_ascii() {
        a.eq_ignore_ascii_case(b)
    } else {
        a.to_lowercase() == b.to_lowercase()
    }
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
            Sent::NotImpl(feature) => Begin::Refuse(feature.into()),
        }
    }

    fn step(&mut self, _cx: &mut Cx<'_>) -> Step<bool> {
        match self.waiting.poll(&BankView::now()) {
            Some(ok) => Step::Done(ok),
            None => Step::Wait,
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

    /// A take of 1/5/10 prefers the fixed op; other takes, or a row
    /// without it, use Withdraw-X; a row with neither is `not impl`.
    #[test]
    fn withdraw_x_picks_the_fixed_op_then_x() {
        let x_action = |ops: &[&str], count: i32| {
            reset();
            post(vec![row("Feather", 314, 50, ops)], 0, false);
            let started = start(json!({ "kind": "withdraw-x", "name": "Feather", "count": count }));
            match started {
                Started::Refused(reason) => reason,
                _ => match drain().as_slice() {
                    [InteractReq::WithdrawX { action, count, .. }] => format!("{action}/{count}"),
                    other => panic!("expected one withdraw-x, got {other:?}"),
                },
            }
        };
        let ops = ["Withdraw-1", "Withdraw-5", "Withdraw-10", "Withdraw-X"];
        assert_eq!(x_action(&ops, 5), "Withdraw-5/5");
        assert_eq!(x_action(&ops, 7), "Withdraw-X/7");
        assert_eq!(x_action(&["Withdraw-X"], 10), "Withdraw-X/10");
        assert_eq!(
            x_action(&ops, 80),
            "Withdraw-X/50",
            "take caps at the row count"
        );
        assert_eq!(x_action(&["Withdraw-1"], 7), "Bank.withdrawX");
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
