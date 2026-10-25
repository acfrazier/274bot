//! Local compatibility calculations and argument materialization.
//!
//! This module owns no action, continuation, timer, callback handle, or wire.
use super::request::{BankSeatDestination, BankSeatNpcAccess, BankSeatObjectAccess};
use api::snapshot::WorldTile;
use serde_json::Value;

/// Existing compatibility approach bound, retained during seat activation.
pub(crate) const WALK_BOUND_MS: u64 = 60_000;
/// Existing compatibility item-snapshot bound, retained during seat activation.
pub(crate) const BANK_READY_MS: u64 = 5_000;

/// JS truthiness for values already converted at a callback boundary.
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value
            .as_f64()
            .is_some_and(|value| value != 0.0 && !value.is_nan()),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// Read a posted tile, including the supported nested `tile` form.
pub fn read_tile(value: &Value) -> Option<WorldTile> {
    let value = value.get("tile").unwrap_or(value);
    Some(WorldTile {
        x: i32::try_from(value.get("x")?.as_i64()?).ok()?,
        z: i32::try_from(value.get("z")?.as_i64()?).ok()?,
        level: value
            .get("level")
            .and_then(Value::as_i64)
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or(0),
    })
}

/// Read a bank destination and its optional object or NPC access declaration.
pub fn read_destination(value: &Value) -> Option<BankSeatDestination> {
    let tile = read_tile(value)?;
    let mut destination = BankSeatDestination {
        tile,
        object_access: None,
        npc_access: None,
    };
    if let Some(access) = value.get("npcAccess").filter(|value| !value.is_null()) {
        destination.npc_access = Some(BankSeatNpcAccess {
            name: access.get("name")?.as_str()?.into(),
            op: access.get("op")?.as_str()?.into(),
            choose: match access.get("choose") {
                Some(choose) => Some(choose.as_str()?.into()),
                None => None,
            },
        });
    } else if let Some(access) = value.get("access").filter(|value| !value.is_null()) {
        let opener = access
            .get("openFirst")
            .or_else(|| access.get("open_first"))
            .filter(|value| !value.is_null());
        destination.object_access = Some(BankSeatObjectAccess {
            name: access.get("name")?.as_str()?.into(),
            op: access.get("op")?.as_str()?.into(),
            open_first_name: match opener {
                Some(opener) => Some(opener.get("name")?.as_str()?.into()),
                None => None,
            },
            open_first_op: match opener {
                Some(opener) => Some(opener.get("op")?.as_str()?.into()),
                None => None,
            },
        });
    }
    Some(destination)
}
/// Frozen `withdrawOp` amounts (`api/bank/bankOps.ts:5`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WithdrawAmount {
    All,
    Ten,
    Five,
    X,
    One,
    Any,
}

impl WithdrawAmount {
    /// The frozen `switch (amount)` cases; anything else matches none.
    pub fn parse(amount: &str) -> Option<Self> {
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
pub fn withdraw_op<'a>(
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
/// JS `Number(value)` for the values a caller passes (an absent argument
/// arrives as `null` and reads as `undefined`: NaN).
pub fn js_number(value: &Value) -> f64 {
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
