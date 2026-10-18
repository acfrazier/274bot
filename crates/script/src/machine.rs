//! Per-isolate step-machine host (fence F03).
//!
//! A **step machine** owns one multi-tick behavior in Rust. JS starts it
//! with one native call and awaits one completion; Rust owns the loop,
//! the sequencing, the game ops and the retries.
//!
//! # Family registration
//!
//! A family is a Rust type implementing [`Family`] and listed once in
//! [`FAMILIES`] (its [`Family::NAME`] is the string JS passes to
//! `runMachine`). The family supplies:
//!
//! - [`Family::begin`]: validate the typed [`Family::Args`] against the
//!   isolate scene ([`crate::observed`]) and selected game data, emit the
//!   first ops through [`Cx::emit`], and return [`Begin::Run`] (a live
//!   row), [`Begin::Done`] (settled at once, e.g. a refused precondition)
//!   or [`Begin::Refuse`] (nothing started; ops emitted by a refusing
//!   begin are discarded). Begin never calls script callbacks, and a
//!   machine start from inside a begin is refused.
//! - [`Family::step`]: read the scene, emit ops, arm/read deadlines on
//!   [`Cx::clock`], read the last callback's [`Cx::reply`], and return
//!   [`Step::Wait`] (done for this tick), [`Step::Call`] (call one script
//!   callback, then step again), [`Step::Done`] or [`Step::Fail`].
//! - [`Family::CALLBACKS`]: the script callbacks the family may call, by
//!   key on the `hooks` object passed to `runMachine`; [`Call::hook`]
//!   indexes this list. Each key is read once, at start (getters run
//!   then), with receiver `hooks`. [`Cx::has`] says whether it was
//!   present (not `undefined`/`null`), so a family keeps the frozen
//!   `hook?.()` semantics by skipping an absent hook; calling an absent
//!   hook throws the frozen `<name> is not a function`. A script that
//!   needs call-time reads passes a wrapper
//!   (`{ walkBack: () => opts.walkBack?.() }`).
//! - [`Family::abort`]: optional cleanup when the host drops a live row
//!   (ResetSession, a superseding start). It emits nothing.
//! - [`Family::release`]: optional op that stops host work the row armed
//!   (its walk follow). A superseding start sends it at the new start's
//!   queue position; a reset does not (the host drops that work itself).
//!
//! # Lifecycle
//!
//! - **Start** (`runMachine(family, args, hooks)` in `_kernel.js`, native
//!   `__rs2b0t_machine_start`): holds `hooks[name]` for every
//!   [`Family::CALLBACKS`] name (receiver `hooks`), then runs `begin`
//!   synchronously inside the caller's JS. Ops it emits join this tick's
//!   InteractReq batch at the caller's position in the JS queue. A
//!   [`Family::EXCLUSIVE`] family aborts its older live row as
//!   `superseded` when a new one runs; families that share an
//!   [`Family::EXCLUSIVE_GROUP`] supersede each other. A
//!   [`Family::KICK_ON_START`] row is then driven once ([`kick`]) inside
//!   the same call, so its first
//!   callbacks and ops run in the caller's own synchronous turn, as the
//!   frozen driver's first stretch did. The kick does not drain the
//!   isolate microtask queue: unrelated tasks queued before the call run
//!   after the caller continues, and an already-settled callback promise
//!   is observed by polling it. The kick ignores pause and
//!   guardian hold on purpose: the frozen driver ran whenever the caller
//!   ran (a held tick still runs paint, native events and the event-loop
//!   drain), and ops emitted on a held tick are dropped with the JS queue
//!   there too ([`drop_ops`]). Only later steps are held back.
//! - **Step**: the isolate thread records the tick number, then calls
//!   [`step`] once per eligible tick, before any other of that tick's JS
//!   (the scene was applied by the Snapshot command before the Tick). A
//!   row started during tick N is first stepped on tick N+1. Ops a step
//!   emits join the batch in emit order, after any JS rows already queued
//!   or queued by its callbacks.
//! - **Changed evidence**: [`snapshot_step`] drives opted-in families once
//!   per distinct `(tick, evidence sequence)` without recording another tick.
//!   It carries the tick's callback count through callback continuations;
//!   families retain their existing interaction/batch dispatch guards.
//!   This pass can settle an already-observed result and its await without
//!   running `loop`, tick listeners, or explicit tick delays.
//! - **Callbacks**: a [`Step::Call`] invokes the held script function
//!   through the one callback path (`load/callback_v8.rs`), in a fresh
//!   handle scope per call. On a tick step a microtask checkpoint
//!   follows, so a plain return value, or a promise that settles through
//!   microtasks alone, is the [`Reply`] of the very next step in the same
//!   tick. A kick runs no checkpoint (see Kick above): its promise settles
//!   when the caller's own microtasks run, and the row takes it on its
//!   next step. A promise still pending is held; the row does not step
//!   until it settles. After the tick's pump (and its microtasks) the isolate runs
//!   [`resume`], which drives only the rows whose promise was pending, so
//!   a callback that waited on an Execution wait resumes its row in the
//!   tick the wait settled, as a frozen `await` would; a row that ends
//!   there settles its await in the same tick ([`any_settled`]). A throw
//!   or rejection is [`Reply::Threw`] carrying the thrown value itself; a
//!   family that fails with it ([`Step::Fail`]) rejects the script's
//!   `runMachine` await with that same value. At most [`CALLS_PER_TICK`]
//!   callbacks run per row per tick, across all passes; the rest
//!   continue next tick. A row whose callback superseded it stops at once.
//!   A callback ended by termination (join's or the watchdog's) is never
//!   shown to the family: the row is aborted `terminated` and no further
//!   callback runs in that pass or kick. Termination inside a pending
//!   callback promise's continuation (it runs in the caller's or the
//!   pump's microtasks) does not abort the row; the promise stays pending
//!   until Stop or Reset. A hook in [`Family::SYNC_HOOKS`]
//!   keeps frozen synchronous-call semantics instead: a returned promise is
//!   not awaited, and its reply is the promise as a value (`{}`: an object
//!   with no own properties).
//! - **Synchronous asks** ([`Cx::ask`]): a step may call a held hook
//!   inline, through the same path, for a script getter or predicate its
//!   decision reads where it stands (frozen `host.hpFraction()`,
//!   `site.inArea(t)`) or a synchronous notification (`host.log(m)`).
//!   Join's claim is checked before and after each ask. An ask never
//!   waits: a returned promise is `not impl`. It cannot be absorbed: a
//!   throw fails the row with the thrown value, a terminate or claim
//!   aborts it `terminated` (the flag above is set), and more than
//!   [`ASKS_PER_STEP`] asks in one step fail it; the host enforces that
//!   after the step returns and drops that step's ops. Asks do not count
//!   toward [`CALLS_PER_TICK`]. Begin has no script side.
//! - **Completion**: a `Done`/`Fail` row ends at once (no JS `end` op). Its
//!   outcome waits in the host until the JS await helper — a wait parked
//!   on the `Execution` park list — takes it, so it settles exactly once:
//!   in the tick's pump, or, for an outcome reached after the pump (the
//!   resume pass, a supersede by the tick's own JS), in the machine-only
//!   settle that follows the resume pass in the same tick.
//! - **ResetSession, session ended** ([`on_reset`]): every live row is
//!   aborted and its await settles `{ kind: 'aborted', reason: 'reset' }`;
//!   held callbacks and pending promises are released; pending ops are
//!   dropped with the JS queue, including ops of machines started during
//!   the reset drain ([`drop_ops`]).
//! - **ResetSession, reconnect**: nothing here is reset. Every row, held
//!   callback and pending promise stays, frozen like a Pause, so the whole
//!   chain the script had in flight (a walk started inside another row's
//!   callback included) resumes together on the relogged session's first
//!   tick, as frozen AutoRelogin resumes the paused script
//!   (`AutoRelogin.ts:180-190`, `159-163`). Pending ops are dropped with
//!   the JS queue; the host re-arms the script walk it was following.
//! - **Pause / guardian hold** ([`on_pause`], [`on_resume`], [`on_hold`]):
//!   rows are not stepped nor promises polled, and every row's
//!   [`InstantTaskClock`] freezes, so deadlines resume where they stopped.
//!   Ops emitted on a held tick are dropped with the JS queue
//!   ([`drop_ops`]).
//! - **Stop** ([`on_stop`]): every row, outcome and op is dropped before
//!   the isolate is; nothing settles.
//!
//! JS sees one envelope per start: `{ kind: 'done', value }`,
//! `{ kind: 'refused', reason }` or `{ kind: 'aborted', reason }`; a
//! `Fail` outcome rejects the `runMachine` promise with its [`Thrown`].

use crate::load::callback_v8::HeldCallback;
use crate::shim::{InteractReq, MaybeInteractReq};
use crate::task_clock::InstantTaskClock;
use crate::{isolate_fb, native};
use api::identity::{ExecutionId, RequestId};
use api::script_contract::{CallbackCancelReason, DecisionSequence, RequestStamp};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

/// Closed bank family descriptor. This table does not depend on the old
/// behaviorful machine implementations.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BankFamilyDescriptor {
    pub(crate) name: &'static str,
    pub(crate) callbacks: &'static [&'static str],
    pub(crate) kind: native::request::NativeKind,
    pub(crate) exclusive_group: Option<&'static str>,
}

const BANK_NO_CALLBACKS: &[&str] = &[];
const BANK_LOG_CALLBACK: &[&str] = &["log"];
const BANK_DEPOSIT_CALLBACKS: &[&str] = &["match", "log"];
const BANK_NEAREST_CALLBACKS: &[&str] = &["deposit", "afterDeposit", "log"];

pub(crate) const BANK_FAMILY_DESCRIPTORS: &[BankFamilyDescriptor] = &[
    BankFamilyDescriptor {
        name: "bank_op",
        callbacks: BANK_NO_CALLBACKS,
        kind: native::request::NativeKind::BankPlan,
        exclusive_group: None,
    },
    BankFamilyDescriptor {
        name: "bank_withdraw_load",
        callbacks: BANK_NO_CALLBACKS,
        kind: native::request::NativeKind::BankPlan,
        exclusive_group: None,
    },
    BankFamilyDescriptor {
        name: "bank_note_mode",
        callbacks: BANK_NO_CALLBACKS,
        kind: native::request::NativeKind::BankPlan,
        exclusive_group: None,
    },
    BankFamilyDescriptor {
        name: "bank_close",
        callbacks: BANK_NO_CALLBACKS,
        kind: native::request::NativeKind::BankPlan,
        exclusive_group: None,
    },
    BankFamilyDescriptor {
        name: "bank_deposit",
        callbacks: BANK_DEPOSIT_CALLBACKS,
        kind: native::request::NativeKind::BankPlan,
        exclusive_group: None,
    },
    BankFamilyDescriptor {
        name: "bank_open",
        callbacks: BANK_NO_CALLBACKS,
        kind: native::request::NativeKind::BankPlan,
        exclusive_group: Some("bank_open"),
    },
    BankFamilyDescriptor {
        name: "bank_access",
        callbacks: BANK_LOG_CALLBACK,
        kind: native::request::NativeKind::BankPlan,
        exclusive_group: Some("bank_open"),
    },
    BankFamilyDescriptor {
        name: "bank_npc_access",
        callbacks: BANK_LOG_CALLBACK,
        kind: native::request::NativeKind::BankPlan,
        exclusive_group: Some("bank_open"),
    },
    BankFamilyDescriptor {
        name: "bank_open_access",
        callbacks: BANK_LOG_CALLBACK,
        kind: native::request::NativeKind::BankPlan,
        exclusive_group: Some("bank_open"),
    },
    BankFamilyDescriptor {
        name: "banking_open",
        callbacks: BANK_LOG_CALLBACK,
        kind: native::request::NativeKind::BankPlan,
        exclusive_group: Some("bank_open"),
    },
    BankFamilyDescriptor {
        name: "bank_select",
        callbacks: BANK_NO_CALLBACKS,
        kind: native::request::NativeKind::BankPlan,
        exclusive_group: Some("bank_select"),
    },
    BankFamilyDescriptor {
        name: "bank_nearest",
        callbacks: BANK_NEAREST_CALLBACKS,
        kind: native::request::NativeKind::BankNearest,
        exclusive_group: None,
    },
    BankFamilyDescriptor {
        name: "bank_wait",
        callbacks: BANK_LOG_CALLBACK,
        kind: native::request::NativeKind::BankPlan,
        exclusive_group: None,
    },
];

fn bank_family_descriptor(name: &str) -> Option<&'static BankFamilyDescriptor> {
    BANK_FAMILY_DESCRIPTORS
        .iter()
        .find(|descriptor| descriptor.name == name)
}

fn bank_family_for_operation(
    operation: crate::bank::request::BankSeatOperation,
) -> &'static str {
    use crate::bank::request::BankSeatOperation as Op;
    match operation {
        Op::DepositAll | Op::DepositKeep | Op::DepositMatch => "bank_deposit",
        Op::OpenBooth | Op::OpenNearest | Op::OpenNearestWorld => "bank_open",
        Op::OpenObject => "bank_access",
        Op::OpenNpc => "bank_npc_access",
        Op::OpenAccess => "bank_open_access",
        Op::Open => "banking_open",
        Op::Select => "bank_select",
        Op::BankNearest => "bank_nearest",
        Op::WaitReady | Op::WaitSnapshotAfter => "bank_wait",
        Op::DepositLabel | Op::WithdrawLabel | Op::WithdrawDelta => "bank_op",
        Op::WithdrawLoad => "bank_withdraw_load",
        Op::SetNoteMode => "bank_note_mode",
        Op::Close => "bank_close",
        Op::None => unreachable!("bank start operation is validated"),
    }
}

fn bank_hook_bit(
    operation: crate::bank::request::BankSeatOperation,
    member: &str,
) -> u32 {
    use crate::bank::request::{
        BankSeatOperation as Op, BANK_NEAREST_HOOK_AFTER_DEPOSIT,
        BANK_NEAREST_HOOK_DEPOSIT, BANK_NEAREST_HOOK_LOG, DEPOSIT_MATCH_HOOK_LOG,
        DEPOSIT_MATCH_HOOK_MATCH, HOOK_LOG,
    };
    match (operation, member) {
        (Op::DepositMatch, "match") => DEPOSIT_MATCH_HOOK_MATCH,
        (Op::DepositMatch, "log") => DEPOSIT_MATCH_HOOK_LOG,
        (Op::BankNearest, "deposit") => BANK_NEAREST_HOOK_DEPOSIT,
        (Op::BankNearest, "afterDeposit") => BANK_NEAREST_HOOK_AFTER_DEPOSIT,
        (Op::BankNearest, "log") => BANK_NEAREST_HOOK_LOG,
        (
            Op::OpenObject
            | Op::OpenNpc
            | Op::OpenAccess
            | Op::Open
            | Op::WaitReady,
            "log",
        ) => HOOK_LOG,
        _ => 0,
    }
}

fn bank_transfer_operation(operation: crate::bank::request::BankSeatOperation) -> bool {
    use crate::bank::request::BankSeatOperation as Op;
    matches!(
        operation,
        Op::DepositLabel
            | Op::WithdrawLabel
            | Op::WithdrawDelta
            | Op::WithdrawLoad
            | Op::DepositAll
            | Op::DepositKeep
            | Op::DepositMatch
    )
}

fn bank_object<'a>(
    value: &'a Value,
    description: &str,
) -> Result<&'a serde_json::Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{description} must be an object"))
}

fn bank_field<'a>(
    object: &'a serde_json::Map<String, Value>,
    name: &str,
    alias: Option<&str>,
) -> Option<&'a Value> {
    object
        .get(name)
        .or_else(|| alias.and_then(|alias| object.get(alias)))
}

fn bank_string(
    object: &serde_json::Map<String, Value>,
    name: &str,
    alias: Option<&str>,
) -> Result<Option<String>, String> {
    match bank_field(object, name, alias) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("{name} must be a string")),
    }
}

fn bank_required_string(
    object: &serde_json::Map<String, Value>,
    name: &str,
    alias: Option<&str>,
) -> Result<String, String> {
    bank_string(object, name, alias)?.ok_or_else(|| format!("{name} is required"))
}

fn bank_boolean(
    object: &serde_json::Map<String, Value>,
    name: &str,
    default: bool,
) -> Result<bool, String> {
    match object.get(name) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(format!("{name} must be a boolean")),
    }
}

fn bank_number(
    object: &serde_json::Map<String, Value>,
    name: &str,
) -> Result<Option<f64>, String> {
    match object.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => value
            .as_f64()
            .map(Some)
            .ok_or_else(|| format!("{name} is not representable")),
        Some(_) => Err(format!("{name} must be a number")),
    }
}

fn bank_integer(value: &Value, name: &str) -> Result<i64, String> {
    let Some(number) = value.as_f64() else {
        return Err(format!("{name} must be an integer"));
    };
    if !number.is_finite()
        || number.fract() != 0.0
        || number < i64::MIN as f64
        || number >= (i64::MAX as f64)
    {
        return Err(format!("{name} must be an integer"));
    }
    Ok(number as i64)
}

fn bank_i32(value: &Value, name: &str) -> Result<i32, String> {
    i32::try_from(bank_integer(value, name)?).map_err(|_| format!("{name} is outside i32 range"))
}

fn bank_tile(value: &Value, name: &str) -> Result<api::snapshot::WorldTile, String> {
    serde_json::from_value(value.clone()).map_err(|error| format!("{name}: {error}"))
}

fn bank_optional_tile(
    object: &serde_json::Map<String, Value>,
    name: &str,
    alias: Option<&str>,
) -> Result<Option<api::snapshot::WorldTile>, String> {
    match bank_field(object, name, alias) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => bank_tile(value, name).map(Some),
    }
}

fn bank_access(value: &Value, description: &str) -> Result<crate::bank::request::BankSeatObjectAccess, String> {
    use crate::bank::request::BankSeatObjectAccess;
    let object = bank_object(value, description)?;
    let open_first = bank_field(object, "open_first", Some("openFirst"));
    let (open_first_name, open_first_op) = match open_first {
        None | Some(Value::Null) => (None, None),
        Some(value) => {
            let opener = bank_object(value, "open_first")?;
            (
                Some(bank_required_string(opener, "name", None)?),
                Some(bank_required_string(opener, "op", None)?),
            )
        }
    };
    Ok(BankSeatObjectAccess {
        name: bank_required_string(object, "name", None)?,
        op: bank_required_string(object, "op", None)?,
        open_first_name,
        open_first_op,
    })
}

fn bank_optional_access(
    value: Option<&Value>,
    description: &str,
) -> Result<Option<crate::bank::request::BankSeatObjectAccess>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => bank_access(value, description).map(Some),
    }
}

fn bank_npc_access(
    value: &Value,
    description: &str,
) -> Result<crate::bank::request::BankSeatNpcAccess, String> {
    use crate::bank::request::BankSeatNpcAccess;
    let object = bank_object(value, description)?;
    Ok(BankSeatNpcAccess {
        name: bank_required_string(object, "name", None)?,
        op: bank_required_string(object, "op", None)?,
        choose: bank_string(object, "choose", None)?.unwrap_or_default(),
    })
}

fn bank_optional_npc_access(
    value: Option<&Value>,
    description: &str,
) -> Result<Option<crate::bank::request::BankSeatNpcAccess>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => bank_npc_access(value, description).map(Some),
    }
}

fn bank_destination(
    value: Option<&Value>,
) -> Result<Option<crate::bank::request::BankSeatDestination>, String> {
    use crate::bank::request::BankSeatDestination;
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let object = bank_object(value, "destination")?;
    let tile_value = object.get("tile").unwrap_or(value);
    Ok(Some(BankSeatDestination {
        tile: bank_tile(tile_value, "destination.tile")?,
        object_access: bank_optional_access(
            bank_field(object, "object_access", Some("access")),
            "destination.access",
        )?,
        npc_access: bank_optional_npc_access(
            bank_field(object, "npc_access", Some("npcAccess")),
            "destination.npcAccess",
        )?,
    }))
}

fn bank_string_list(value: Option<&Value>, name: &str) -> Result<Vec<String>, String> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| format!("{name} must be an array"))?;
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{name} entries must be strings"))
        })
        .collect()
}

/// The frozen `bank_op.withdraw` numeric tier before the bank's posted
/// stock is known. `quantity` carries the numeric threshold; the native
/// body selects All only when that threshold also covers the observed row.
fn bank_withdraw_threshold(value: &Value) -> i64 {
    let amount = match value {
        Value::Number(number) => number.as_f64().unwrap_or(f64::NAN),
        Value::Bool(value) => f64::from(u8::from(*value)),
        _ => f64::NAN,
    };
    if amount >= 10.0 {
        amount.floor().min(9_007_199_254_740_991.0) as i64
    } else {
        1
    }
}
fn bank_js_number(value: &Value) -> f64 {
    match value {
        Value::Number(number) => number.as_f64().unwrap_or(f64::NAN),
        Value::Bool(value) => f64::from(u8::from(*value)),
        Value::String(value) => {
            let value = value.trim();
            if value.is_empty() {
                0.0
            } else {
                value.parse().unwrap_or(f64::NAN)
            }
        }
        _ => f64::NAN,
    }
}

fn bank_safe_integer(number: f64) -> bool {
    number.is_finite()
        && number.fract() == 0.0
        && number.abs() <= 9_007_199_254_740_991.0
}

fn bank_numeric_i32(number: f64) -> Option<i32> {
    (bank_safe_integer(number)
        && number >= f64::from(i32::MIN)
        && number <= f64::from(i32::MAX))
    .then_some(number as i32)
}

fn bank_id_number(value: &Value) -> Option<i32> {
    let Value::Number(number) = value else {
        return None;
    };
    bank_numeric_i32(number.as_f64()?)
}

fn bank_withdraw_count(value: &Value) -> Option<i64> {
    bank_withdraw_count_number(bank_js_number(value))
}

fn bank_withdraw_count_number(count: f64) -> Option<i64> {
    (bank_safe_integer(count) && (1.0..=f64::from(i32::MAX)).contains(&count))
        .then_some(count as i64)
}

fn bank_withdraw_lands_as(value: &Value) -> Option<i32> {
    bank_numeric_i32(bank_js_number(value))
}

fn bank_op_immediate_outcome(
    family: &str,
    value: &Value,
) -> Result<Option<Outcome>, String> {
    if family != "bank_op" {
        return Ok(None);
    }
    let object = bank_object(value, "bank arguments")?;
    let kind = bank_required_string(object, "kind", None)?;
    let outcome = match kind.as_str() {
        "withdraw-id" => {
            let id = object.get("id").ok_or("id is required")?;
            bank_required_string(object, "op", None)?;
            bank_id_number(id).is_none().then_some(false)
        }
        "withdraw-x" => {
            bank_required_string(object, "name", None)?;
            let count = bank_js_number(object.get("count").unwrap_or(&Value::Null));
            if bank_safe_integer(count) && count <= 0.0 {
                Some(true)
            } else if bank_withdraw_count_number(count).is_none() {
                Some(false)
            } else {
                None
            }
        }
        "withdraw-x-id" => {
            let id = object.get("id").ok_or("id is required")?;
            let count = bank_js_number(object.get("count").unwrap_or(&Value::Null));
            if bank_safe_integer(count) && count <= 0.0 {
                Some(true)
            } else {
                let lands_as = object.get("lands_as").unwrap_or(&Value::Null);
                (bank_withdraw_count_number(count).is_none()
                    || bank_id_number(id).is_none()
                    || bank_withdraw_lands_as(lands_as).is_none())
                .then_some(false)
            }
        }
        _ => None,
    };
    Ok(outcome.map(|value| Outcome::Done(Value::Bool(value))))
}

fn bank_positive_withdraw_count(value: &Value, name: &str) -> Result<i64, String> {
    bank_withdraw_count(value).ok_or_else(|| format!("{name} must be a positive i32"))
}

fn bank_required_id(value: &Value, name: &str) -> Result<i32, String> {
    bank_id_number(value).ok_or_else(|| format!("{name} must be an i32 item id"))
}

fn bank_required_lands_as(value: &Value) -> Result<i32, String> {
    bank_withdraw_lands_as(value)
        .ok_or_else(|| "lands_as must be a safe i32".into())
}


fn parse_bank_start(family: &str, value: Value) -> Result<crate::bank::request::BankSeatArgs, String> {
    use crate::bank::request::{BankSeatArgs as Args, BankSeatOperation as Op};
    let object = bank_object(&value, "bank arguments")?;
    let operation = match family {
        "bank_op" => {
            let kind = bank_required_string(object, "kind", None)?;
            match kind.as_str() {
                "deposit" => {
                    let mut args = Args::new(Op::DepositLabel);
                    args.item_name = Some(bank_required_string(object, "name", None)?);
                    args.menu_op = Some(bank_required_string(object, "op", None)?);
                    args
                }
                "withdraw" => {
                    let amount = object.get("amount").unwrap_or(&Value::Null);
                    let mut args = Args::new(Op::WithdrawLabel);
                    args.item_name = Some(bank_required_string(object, "name", None)?);
                    if let Some(label) = amount.as_str() {
                        args.menu_op = Some(if label.eq_ignore_ascii_case("all") {
                            "Withdraw All".into()
                        } else {
                            label.to_owned()
                        });
                    } else {
                        args.quantity = Some(bank_withdraw_threshold(amount));
                    }
                    args
                }
                "withdraw-id" => {
                    let mut args = Args::new(Op::WithdrawLabel);
                    args.item_id = Some(bank_i32(
                        object.get("id").ok_or("id is required")?,
                        "id",
                    )?);
                    let op = bank_required_string(object, "op", None)?;
                    args.menu_op = Some(
                        if matches!(op.to_lowercase().as_str(), "withdraw-all" | "all") {
                            "Withdraw All".into()
                        } else {
                            op.replace('-', " ")
                        },
                    );
                    args
                }
                "withdraw-x" => {
                    let mut args = Args::new(Op::WithdrawDelta);
                    args.item_name = Some(bank_required_string(object, "name", None)?);
                    args.quantity = Some(bank_positive_withdraw_count(
                        object.get("count").unwrap_or(&Value::Null),
                        "count",
                    )?);
                    args
                }
                "withdraw-x-id" => {
                    let id = bank_required_id(
                        object.get("id").ok_or("id is required")?,
                        "id",
                    )?;
                    let mut args = Args::new(Op::WithdrawDelta);
                    args.item_id = Some(id);
                    args.quantity = Some(bank_positive_withdraw_count(
                        object.get("count").unwrap_or(&Value::Null),
                        "count",
                    )?);
                    args.lands_as_id = Some(bank_required_lands_as(
                        object.get("lands_as").unwrap_or(&Value::Null),
                    )?);
                    args
                }
                _ => return Err(format!("unknown bank operation {kind:?}")),
            }
        }
        "bank_withdraw_load" => {
            let mut args = Args::new(Op::WithdrawLoad);
            args.item_name = Some(bank_required_string(object, "name", None)?);
            args
        }
        "bank_note_mode" => {
            let mut args = Args::new(Op::SetNoteMode);
            args.note_mode = bank_boolean(object, "on", false)?;
            args
        }
        "bank_close" => {
            let mut args = Args::new(Op::Close);
            args.timeout_ms = bank_number(object, "timeout_ms")?;
            args
        }
        "bank_deposit" => {
            if bank_boolean(object, "all", false)? {
                Args::new(Op::DepositAll)
            } else if object.get("keep").is_some_and(|keep| !keep.is_null()) {
                let mut args = Args::new(Op::DepositKeep);
                args.keep_names = bank_string_list(object.get("keep"), "keep")?;
                args
            } else {
                Args::new(Op::DepositMatch)
            }
        }
        "bank_open" => {
            let mode = bank_string(object, "mode", None)?.unwrap_or_default();
            let stand = bank_optional_tile(object, "stand", None)?;
            let booth_name = bank_string(object, "booth_name", None)?;
            let booth_op = bank_string(object, "booth_action", None)?;
            let object_access = match (booth_name, booth_op) {
                (None, None) => None,
                (name, op) => Some(crate::bank::request::BankSeatObjectAccess {
                    name: name.unwrap_or_else(|| "Bank booth".into()),
                    op: op.unwrap_or_else(|| "Use-quickly".into()),
                    open_first_name: None,
                    open_first_op: None,
                }),
            };
            let operation = match mode.as_str() {
                "open-booth" => Op::OpenBooth,
                "open-nearest" => Op::OpenNearest,
                "open-nearest-world" => Op::OpenNearestWorld,
                _ => return Err(format!("unknown bank open mode {mode:?}")),
            };
            let mut args = Args::new(operation);
            args.stand = stand;
            args.object_access = object_access;
            args
        }
        "bank_access" => {
            let mut args = Args::new(Op::OpenObject);
            args.object_access = Some(bank_access(&value, family)?);
            args
        }
        "bank_npc_access" => {
            let mut args = Args::new(Op::OpenNpc);
            args.npc_access = Some(bank_npc_access(&value, family)?);
            args
        }
        "bank_open_access" => {
            let mut args = Args::new(Op::OpenAccess);
            args.object_access = bank_optional_access(
                bank_field(object, "access", None),
                "access",
            )?;
            args.npc_access = bank_optional_npc_access(
                bank_field(object, "npc_access", Some("npcAccess")),
                "npc_access",
            )?;
            args.fallback_access =
                bank_optional_access(bank_field(object, "fallback", None), "fallback")?;
            args
        }
        "banking_open" => {
            let mut args = Args::new(Op::Open);
            args.stand = bank_optional_tile(object, "stand", None)?;
            args.destination = bank_destination(object.get("destination"))?;
            let booth_name = bank_string(object, "boothName", None)?;
            let booth_op = bank_string(object, "boothOp", None)?;
            args.object_access = match (booth_name, booth_op) {
                (None, None) => None,
                (name, op) => Some(crate::bank::request::BankSeatObjectAccess {
                    name: name.unwrap_or_else(|| "Bank booth".into()),
                    op: op.unwrap_or_else(|| "Use-quickly".into()),
                    open_first_name: None,
                    open_first_op: None,
                }),
            };
            args.prefer_nearby = match object.get("preferNearby") {
                None | Some(Value::Null) => None,
                Some(Value::Bool(value)) => Some(*value),
                Some(_) => return Err("preferNearby must be a boolean".into()),
            };
            args.nearby_radius = bank_number(object, "nearbyRadius")?;
            args.obstacles = bank_string_list(object.get("obstacles"), "obstacles")?;
            args
        }
        "bank_select" => {
            let mut args = Args::new(Op::Select);
            args.stand = bank_optional_tile(object, "from", None)?;
            args.allow_wilderness = bank_boolean(object, "allow_wilderness", false)?;
            args.use_mage_bank = match object.get("use_mage_bank") {
                None | Some(Value::Null) => None,
                Some(Value::Bool(value)) => Some(*value),
                Some(_) => return Err("use_mage_bank must be a boolean".into()),
            };
            args.use_zanaris_bank = match object.get("use_zanaris_bank") {
                None | Some(Value::Null) => None,
                Some(Value::Bool(value)) => Some(*value),
                Some(_) => return Err("use_zanaris_bank must be a boolean".into()),
            };
            args
        }
        "bank_nearest" => {
            let mut args = Args::new(Op::BankNearest);
            args.destination = bank_destination(object.get("destination"))?;
            let booth_name = bank_string(object, "booth_name", None)?;
            let booth_op = bank_string(object, "booth_op", None)?;
            args.object_access = match (booth_name, booth_op) {
                (None, None) => None,
                (name, op) => Some(crate::bank::request::BankSeatObjectAccess {
                    name: name.unwrap_or_else(|| "Bank booth".into()),
                    op: op.unwrap_or_else(|| "Use-quickly".into()),
                    open_first_name: None,
                    open_first_op: None,
                }),
            };
            args.return_to = bank_optional_tile(object, "return_to", None)?;
            args.deposit_all = bank_boolean(object, "deposit_all", false)?;
            args.common_junk = bank_boolean(object, "common_junk", true)?;
            args
        }
        "bank_wait" => match bank_required_string(object, "kind", None)?.as_str() {
            "ready" => {
                let mut args = Args::new(Op::WaitReady);
                args.timeout_ms = bank_number(object, "timeout_ms")?;
                args
            }
            "snapshot-after" => {
                let mut args = Args::new(Op::WaitSnapshotAfter);
                args.timeout_ms = bank_number(object, "timeout_ms")?;
                args.baseline_generation = bank_number(object, "baseline_generation")?;
                args
            }
            other => return Err(format!("unknown bank wait kind {other:?}")),
        },
        _ => return Err(format!("unknown bank family {family:?}")),
    };
    Ok(operation)
}

/// A started machine's id, unique for the isolate thread's life.
pub(crate) type Handle = u64;

/// Handles are returned to JavaScript as Number and encoded in isolate request ids.
const MAX_MACHINE_HANDLE: Handle = 9_007_199_254_740_991;

const MAX_PENDING_SEAT_STARTS: usize = api::script_contract::MAX_PENDING_STARTS;

/// Callbacks one row may run in one tick before it yields to the next.
///
/// The bound exists so a family that keeps calling back cannot hold the
/// tick forever; it must not split frozen-sized work across ticks. The
/// largest frozen driver iteration is bounded by the 28-slot pack: the
/// partner-trade receiver counts their offer twice (2 × 28
/// `theirProductMatch`) plus under ten fixed hooks, and `Trade` `pick`
/// sees at most 28 rows. 256 leaves about 4× headroom over that while
/// still yielding a runaway family within one tick.
pub(crate) const CALLS_PER_TICK: usize = 256;

/// One machine family: a Rust type the host begins, steps and aborts.
pub(crate) trait Family: Sized + 'static {
    /// The name JS passes to `runMachine`.
    const NAME: &'static str;
    /// At most one live row: a newer running start supersedes the older.
    const EXCLUSIVE: bool = false;
    /// The exclusive set this family belongs to: by default itself. Two
    /// families naming the same group supersede each other's rows (for
    /// example every bank open, whichever surface started it).
    const EXCLUSIVE_GROUP: &'static str = Self::NAME;
    /// Script callbacks by `hooks` key; [`Call::hook`] indexes this.
    const CALLBACKS: &'static [&'static str] = &[];
    /// Drive this row once inside `start`, so the first callbacks and
    /// verbs join the starting tick. Default is the host's next-tick
    /// first step (teleport clicks in begin). Clue's first next is a
    /// callback, so it opts in.
    const KICK_ON_START: bool = false;
    /// Whether changed evidence may drive this family without a game tick.
    /// Opt in only when `snapshot_step` preserves the family's per-tick
    /// interaction/batch quota and any server-required tick gates.
    const SNAPSHOT_SENSITIVE: bool = false;
    /// This family owns a walking operation while its row is live.
    const WALKING_OPERATION: bool = false;
    /// [`Family::CALLBACKS`] indexes called synchronously (frozen calls
    /// these hooks without `await`). A family whose callbacks are all
    /// synchronous lists every callback index here.
    const SYNC_HOOKS: &'static [usize] = &[];
    /// Typed start arguments, decoded from the JS value.
    type Args: DeserializeOwned;
    /// The completion value JS receives as `value`.
    type Output: Into<Value>;

    fn begin(args: Self::Args, cx: &mut Cx<'_>) -> Begin<Self>;

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Self::Output>;
    /// Drive a changed-evidence pass when opted in above.
    fn snapshot_step(&mut self, cx: &mut Cx<'_>) -> Step<Self::Output> {
        if Self::SNAPSHOT_SENSITIVE {
            self.step(cx)
        } else {
            Step::Wait
        }
    }

    fn abort(&mut self, _why: AbortReason) {}

    /// The op that stops the host work this row armed (a host walk follow),
    /// sent when a newer exclusive start supersedes the row and when the
    /// row fails or is terminated. A reset needs none: the host drops its
    /// session work itself.
    fn release(&self) -> Option<InteractReq> {
        None
    }
}

/// What `begin` decided.
pub(crate) enum Begin<F: Family> {
    /// A live row; it is stepped from the next eligible tick.
    Run(F),
    /// Settled without a row.
    Done(F::Output),
    /// Nothing started (missing facts or control, bad arguments).
    Refuse(String),
}

/// What one step decided.
pub(crate) enum Step<T> {
    /// Nothing more this tick.
    Wait,
    /// Call one script callback; its [`Reply`] reaches the next step.
    Call(Call),
    Done(T),
    /// End the row; the `runMachine` promise rejects with this value.
    Fail(Thrown),
}

/// One script-callback request: `hooks[CALLBACKS[hook]](...args)`.
pub(crate) struct Call {
    pub(crate) hook: usize,
    pub(crate) args: Vec<Value>,
}

/// A settled script callback.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Reply {
    /// The returned (or promise-fulfilled) value.
    Value(Value),
    /// The throw or rejection.
    Threw(Thrown),
}

/// A thrown JS value (kept as is) or a family's own failure message.
#[derive(Clone)]
pub(crate) struct Thrown {
    message: String,
    value: Option<v8::Global<v8::Value>>,
}

impl Thrown {
    /// A failure the family raises itself; JS sees `new Error(message)`.
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            value: None,
        }
    }

    /// What script code threw or rejected with, and its message.
    pub(crate) fn js(message: String, value: v8::Global<v8::Value>) -> Self {
        Self {
            message,
            value: Some(value),
        }
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    /// The thrown value itself, when script code threw it.
    pub(crate) fn value(&self) -> Option<&v8::Global<v8::Value>> {
        self.value.as_ref()
    }
}

impl std::fmt::Debug for Thrown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Thrown").field(&self.message).finish()
    }
}

/// By message: V8 handles have no value equality.
impl PartialEq for Thrown {
    fn eq(&self, other: &Self) -> bool {
        self.message == other.message
    }
}

/// Why the host ended a live row without its own completion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AbortReason {
    /// ResetSession (reconnect / new session).
    Reset,
    /// A newer start of the same exclusive family.
    Superseded,
    /// A callback of this row was terminated (join or the watchdog).
    Terminated,
    /// A child of a revoked in-isolate parent.
    ParentAborted,
    /// Manual Combat takeover.
    UserInput,
    /// The handle was never started here or was already taken.
    Unknown,
}

impl AbortReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Reset => "reset",
            Self::Superseded => "superseded",
            Self::Terminated => "terminated",
            Self::ParentAborted => "parent-aborted",
            Self::UserInput => "user-input",
            Self::Unknown => "unknown",
        }
    }
}

/// A machine's single settlement.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Outcome {
    Done(Value),
    Failed(Thrown),
    Aborted(AbortReason),
    Refused(String),
}

/// The synchronous result of a start.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Started {
    Running(Handle),
    Settled(Outcome),
    Refused(String),
}

/// One `take` by the JS await helper.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Take {
    Pending,
    Settled(Outcome),
}

/// What a family sees while it begins or steps.
pub(crate) struct Cx<'a> {
    ops: &'a mut Vec<InteractReq>,
    clock: &'a mut InstantTaskClock,
    reply: Option<Reply>,
    hooks: &'a [Rc<Hook>],
    handle: Handle,
    /// The script side while a row steps; `None` in begin.
    js: Option<&'a mut (dyn Js + 'a)>,
    /// The outer operation's user-movement intent baseline.
    intent_baseline: u64,
    /// Asks this step made, and how the first failing one ended the row.
    asks: usize,
    ending: Option<Ending>,
    callback_path: Option<native::abi::HookPath>,
}
impl<'a> Cx<'a> {
    /// A bare context for a family's own unit tests: no hooks held.
    #[cfg(test)]
    pub(crate) fn test(
        ops: &'a mut Vec<InteractReq>,
        clock: &'a mut InstantTaskClock,
        reply: Option<Reply>,
    ) -> Self {
        Self {
            ops,
            clock,
            reply,
            hooks: &[],
            handle: 0,
            context: StartContext::default(),
            js: None,
            asks: 0,
            ending: None,
            callback_path: None,
        }
    }
}

impl Cx<'_> {
    /// Append one game op to this tick's InteractReq batch.
    pub(crate) fn emit(&mut self, op: InteractReq) {
        self.ops.push(op);
    }

    /// This row's isolate request identity, when it has a wire-safe handle.
    pub(crate) fn request_id(&self) -> Option<RequestId> {
        RequestId::isolate(self.handle).ok()
    }

    /// The raw seat token shared with this row's machine handle.
    pub(crate) fn seat_token(&self) -> Option<u64> {
        self.request_id().map(RequestId::get)
    }

    /// This row's pause/hold-aware deadline clock.
    pub(crate) fn clock(&mut self) -> &mut InstantTaskClock {
        self.clock
    }

    /// The settlement of the callback the previous step asked for; `None`
    /// on any other step.
    pub(crate) fn reply(&mut self) -> Option<Reply> {
        self.reply.take()
    }
    /// The outer operation's user-movement baseline, inherited by nested rows.
    pub(crate) fn user_move_intent_baseline(&self) -> u64 {
        self.intent_baseline
    }

    /// Whether a user movement intent has taken over this operation.
    pub(crate) fn user_move_intent_interrupted(&self) -> bool {
        user_move_intent_seq() > self.intent_baseline
    }

    /// Whether `hooks[CALLBACKS[hook]]` was present (not `undefined` or
    /// `null`) when the machine started.
    pub(crate) fn has(&self, hook: usize) -> bool {
        self.hooks.get(hook).is_some_and(|hook| hook.present)
    }

    /// Invoke one bank prompt callback using its source-backed path.
    pub(crate) fn call_prompt(
        &mut self,
        hook: Option<usize>,
        args: &[Value],
        path: native::abi::HookPath,
    ) -> Result<Called, Ended> {
        if self.ending.is_some() {
            return Err(Ended);
        }
        let Some(js) = self.js.as_deref_mut() else {
            return Err(Ended);
        };
        if js.claimed() {
            self.ending = Some(Ending::Stopped);
            return Err(Ended);
        }
        self.asks += 1;
        if self.asks > ASKS_PER_STEP {
            self.ending = Some(Ending::Failed(Thrown::new(format!(
                "not impl: more than {ASKS_PER_STEP} synchronous callbacks in one step"
            ))));
            return Err(Ended);
        }
        let callback = hook
            .and_then(|hook| self.hooks.get(hook))
            .map(|hook| &hook.callback);
        let called = js.call_for(self.handle, callback, args);
        if matches!(called, Called::Terminated) {
            TERMINATED.with(|flag| flag.set(true));
            self.ending = Some(Ending::Stopped);
            return Err(Ended);
        }
        if js.claimed() {
            self.ending = Some(Ending::Stopped);
            return Err(Ended);
        }
        match called {
            Called::Settled(Reply::Threw(thrown)) => {
                self.ending = Some(Ending::Failed(thrown));
                Err(Ended)
            }
            Called::Pending(_) if path == native::abi::HookPath::CallSync => Ok(
                Called::Settled(Reply::Value(serde_json::json!({}))),
            ),
            Called::Pending(_) if path == native::abi::HookPath::Ask => {
                self.ending = Some(Ending::Failed(Thrown::new(
                    "not impl: a synchronous machine callback returned a promise",
                )));
                Err(Ended)
            }
            Called::Terminated => unreachable!("termination returned above"),
            called => Ok(called),
        }
    }

    /// Poll the one Promise returned by an awaited bank prompt. A settled
    /// rejection ends this row with its original V8 value.
    pub(crate) fn poll_prompt(&mut self, pending: &Pending) -> Option<Reply> {
        if self.ending.is_some() {
            return None;
        }
        let Some(js) = self.js.as_deref_mut() else {
            return None;
        };
        if js.claimed() {
            self.ending = Some(Ending::Stopped);
            return None;
        }
        let reply = js.poll(pending);
        if js.claimed() {
            self.ending = Some(Ending::Stopped);
            return None;
        }
        if let Some(Reply::Threw(thrown)) = &reply {
            self.ending = Some(Ending::Failed(thrown.clone()));
        }
        reply
    }

    /// Call `hooks[CALLBACKS[hook]](...args)` now, inside this step.
    /// Promises are never awaited here.
    pub(crate) fn ask(&mut self, hook: usize, args: &[Value]) -> Result<Value, Ended> {
        match self.call_prompt(Some(hook), args, native::abi::HookPath::Ask)? {
            Called::Settled(Reply::Value(value)) => Ok(value),
            Called::Settled(Reply::Threw(_)) => Err(Ended),
            Called::Pending(_) | Called::Terminated => unreachable!("ask path is synchronous"),
        }
    }
}


/// An [`Cx::ask`] that ended its row; the family returns at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ended;

/// Synchronous callbacks one step may make ([`Cx::ask`]).
pub(crate) const ASKS_PER_STEP: usize = 512;

/// How an ask ended its row.
enum Ending {
    /// Join claimed the tick or terminated the call: the row is aborted
    /// `terminated`; the family has already seen a missing answer.
    Stopped,
    /// The row fails with this value.
    Failed(Thrown),
}

/// One declared script callback, held from start.
pub(crate) struct Hook {
    pub(crate) callback: HeldCallback,
    /// Not `undefined`/`null` when read at start.
    pub(crate) present: bool,
}

/// The isolate side of a step: script callbacks and the JS queue.
pub(crate) trait Js {
    /// `__rs2b0t_host.interact.length` now.
    fn queue_len(&mut self) -> usize;
    /// Call `hook` (`None`: the family asked for an undeclared hook). A
    /// tick step then drains microtasks so a promise that settles through
    /// them alone comes back settled. A kick is nested in the caller's JS
    /// and must not drain that queue; it polls the callback promise
    /// instead.
    fn call(&mut self, hook: Option<&HeldCallback>, args: &[Value]) -> Called;
    /// Call a row-owned hook. Implementations that need row attribution
    /// override this method; test hosts can keep the context-free default.
    fn call_for(&mut self, _handle: Handle, hook: Option<&HeldCallback>, args: &[Value]) -> Called {
        self.call(hook, args)
    }
    /// Poll a promise a callback returned; `None` while pending.
    fn poll(&mut self, pending: &Pending) -> Option<Reply>;
    /// Join has claimed this tick for Stop, or the slow-tick watchdog armed
    /// a terminate: no more script may run. A callback may have absorbed
    /// the terminate, so a later one would otherwise spin past it.
    fn claimed(&mut self) -> bool;
}

/// A promise a script callback returned, held until it settles.
pub(crate) type Pending = v8::Global<v8::Promise>;

pub(crate) enum Called {
    Settled(Reply),
    Pending(Pending),
    /// Execution was terminated inside the callback; nothing else may run.
    Terminated,
}

/// One registered family.
struct Entry {
    name: &'static str,
    callbacks: &'static [&'static str],
    kick_on_start: bool,
    begin: fn(Handle, Value, Vec<Hook>, usize) -> Started,
    /// [`Family::SYNC_HOOKS`]. The ABI coverage test reads this field.
    #[cfg(test)]
    sync_hooks: &'static [usize],
}

const fn entry<F: Family>() -> Entry {
    Entry {
        name: F::NAME,
        callbacks: F::CALLBACKS,
        kick_on_start: F::KICK_ON_START,
        begin: begin_row::<F>,
        #[cfg(test)]
        sync_hooks: F::SYNC_HOOKS,
    }
}

/// Every registered family, by name. The only machine dispatch table.
const FAMILIES: &[Entry] = &[
    entry::<crate::teleport::Teleport>(),
    entry::<crate::prayer::Prayer>(),
    entry::<crate::autocast::Autocast>(),
    entry::<crate::special::Special>(),
    entry::<crate::modals::Modals>(),
    entry::<crate::fire::LightFire>(),
    entry::<crate::fight_upkeep::BuryInFight>(),
    entry::<crate::shop::Shop>(),
    entry::<crate::cake_stall::CakeStall>(),
    entry::<crate::production::ChatDialog>(),
    entry::<crate::reach::NpcDialog>(),
    entry::<crate::dialog::Dialog>(),
    entry::<crate::trade::Trade>(),
    entry::<crate::drive_partner_trade::PartnerTrade>(),
    entry::<crate::hunt::Hunt<crate::hunt_fight::FightKind>>(),
    entry::<crate::hunt::Hunt<crate::hunt_fight::HoldKind>>(),
    entry::<crate::hunt::Hunt<crate::hunt_fight::RetreatKind>>(),
    entry::<crate::hunt::Hunt<crate::hunt_fight::WalkSpotKind>>(),
    entry::<crate::hunt::Hunt<crate::hunt_lair::Enter>>(),
    entry::<crate::hunt::Hunt<crate::hunt_leave::Leave>>(),
    entry::<crate::hunt::Hunt<crate::hunt_key::Key>>(),
    entry::<crate::hunt::Hunt<crate::hunt_cell::Cell>>(),
    entry::<crate::hunt::Hunt<crate::hunt_bank::Bank>>(),
    entry::<crate::hunt::WaitFed>(),
    entry::<crate::hunt::TeleportOut>(),
    entry::<crate::hunt::Acquire>(),
    entry::<crate::clue::Clue>(),
    entry::<crate::duel::Handshake>(),
    entry::<crate::duel::Helper>(),
    entry::<crate::duel::Travel>(),
    entry::<crate::duel::Close>(),
    entry::<crate::api_session::GatherSession>(),
    entry::<crate::api_session::GatherStop>(),
    entry::<crate::api_session::CombatSession>(),
    entry::<crate::api_session::CombatStop>(),
    entry::<crate::api_session::ProgressQuery>(),
    entry::<crate::quest_journal::QuestJournal>(),
    entry::<crate::reach_entity::EntityOp>(),
    entry::<crate::reach_entity::WalkHops>(),
    entry::<crate::walk::WalkResilient>(),
    entry::<crate::walk::WalkOpening>(),
    entry::<crate::walk::DirectWalk>(),
    entry::<crate::walk::DirectClick>(),
    entry::<crate::boat_fare::WalkTo>(),
    entry::<crate::anchor_return::ReturnToAnchor>(),
    entry::<crate::bank_withdraw::WithdrawTo>(),
    entry::<crate::bank_withdraw::CloseConfirm>(),
    entry::<crate::death_recovery::DeathRecovery>(),
    #[cfg(test)]
    entry::<tests::Probe>(),
    #[cfg(test)]
    entry::<tests::Solo>(),
    #[cfg(test)]
    entry::<tests::Hooked>(),
    #[cfg(test)]
    entry::<tests::Burst>(),
    #[cfg(test)]
    entry::<tests::Present>(),
    #[cfg(test)]
    entry::<tests::Nested>(),
    #[cfg(test)]
    entry::<tests::SoloHooked>(),
    #[cfg(test)]
    entry::<tests::Kicked>(),
    #[cfg(test)]
    entry::<tests::Ask>(),
    #[cfg(test)]
    entry::<tests::NotificationOrder>(),
    #[cfg(test)]
    entry::<tests::Inline>(),
    #[cfg(test)]
    entry::<tests::SoloAsker>(),
    #[cfg(test)]
    entry::<tests::Walker>(),
];

/// The callback names `family` holds at start, or `None` if unregistered.
pub(crate) fn callbacks_of(family: &str) -> Option<&'static [&'static str]> {
    bank_family_descriptor(family)
        .map(|descriptor| descriptor.callbacks)
        .or_else(|| {
            FAMILIES
                .iter()
                .find(|entry| entry.name == family)
                .map(|entry| entry.callbacks)
        })
}

/// Whether a live row of `family` is running (its begin returned
/// [`Begin::Run`] and it has not settled).
///
/// A family whose frozen surface admits one operation at a time asks
/// inside its own begin, before it emits anything; a start that refuses
/// this way supersedes nothing, so the admitted row keeps its await.
/// A row a [`pass`] is stepping counts: its callback runs inside that
/// pass, and script code started from it must see the row it belongs to.
pub(crate) fn live(family: &str) -> bool {
    HOST.with(|host| {
        let host = host.borrow();
        host.rows.iter().any(|row| row.family == family)
            || host.stepping.iter().any(|row| row.family == family)
    })
}

/// Whether a native bank transfer is pending or admitted on this isolate.
pub(crate) fn bank_transfer_busy() -> bool {
    flush_deferred_bank_cancels();
    HOST.with(|host| {
        host.borrow().bank_mirrors.iter().any(|mirror| {
            mirror.transfer_busy && mirror.terminal.is_none()
        })
    })
}

/// Live walking ownership copied from the existing machine rows. The takeover
/// watermark is shared with the host so an in-flight callback cannot republish
/// an already interrupted operation before its queued notice is processed.
#[derive(Default)]
pub(crate) struct WalkingOwnership {
    live: std::sync::atomic::AtomicBool,
    baseline: std::sync::atomic::AtomicU64,
    takeover: std::sync::atomic::AtomicU64,
}

impl WalkingOwnership {
    pub(crate) fn live(&self) -> bool {
        self.live.load(std::sync::atomic::Ordering::Acquire)
            && self.baseline.load(std::sync::atomic::Ordering::Acquire)
                >= self.takeover.load(std::sync::atomic::Ordering::Acquire)
    }

    pub(crate) fn note_takeover(&self, seq: u64) {
        self.takeover
            .fetch_max(seq, std::sync::atomic::Ordering::AcqRel);
    }
}

pub(crate) fn set_walking_publication(publication: std::sync::Arc<WalkingOwnership>) {
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        host.walking_publication = Some(publication);
        host.publish_walking();
    });
}

/// Guardian hold freezes stepping, not walking ownership. Reconnect-held rows
/// are carried work instead; operator Pause is gated by the host's run state.
#[cfg(test)]
pub(crate) fn walking_live() -> bool {
    HOST.with(|host| host.borrow().walking_baseline().is_some())
}
/// The type-erased row the host steps.
trait Machine {
    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value>;
    fn snapshot_sensitive(&self) -> bool;
    fn snapshot_step(&mut self, cx: &mut Cx<'_>) -> Step<Value>;
    fn abort(&mut self, why: AbortReason);
    fn release(&self) -> Option<InteractReq>;
    fn walking_operation(&self) -> bool;
}

impl<F: Family> Machine for F {
    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
        match Family::step(self, cx) {
            Step::Wait => Step::Wait,
            Step::Call(call) => Step::Call(call),
            Step::Done(out) => Step::Done(out.into()),
            Step::Fail(reason) => Step::Fail(reason),
        }
    }

    fn snapshot_sensitive(&self) -> bool {
        F::SNAPSHOT_SENSITIVE
    }

    fn snapshot_step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
        match Family::snapshot_step(self, cx) {
            Step::Wait => Step::Wait,
            Step::Call(call) => Step::Call(call),
            Step::Done(out) => Step::Done(out.into()),
            Step::Fail(reason) => Step::Fail(reason),
        }
    }

    fn abort(&mut self, why: AbortReason) {
        Family::abort(self, why);
    }

    fn release(&self) -> Option<InteractReq> {
        Family::release(self)
    }

    fn walking_operation(&self) -> bool {
        F::WALKING_OPERATION
    }
}

/// Test seam: pull a live row's deadline `millis` closer to now, so a
/// family's own window is exercised without sleeping the bound out. A
/// deadline the row never armed stays unarmed.
#[cfg(test)]
pub(crate) fn age(handle: Handle, millis: u64) {
    use std::time::Duration;
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        if let Some(row) = host.rows.iter_mut().find(|row| row.handle == handle) {
            row.clock.deadline = row
                .clock
                .deadline
                .and_then(|deadline| deadline.checked_sub(Duration::from_millis(millis)));
        }
    });
}

/// Whether `start` should drive `family` once before returning Running.
pub(crate) fn kick_on_start(family: &str) -> bool {
    FAMILIES
        .iter()
        .find(|entry| entry.name == family)
        .is_some_and(|entry| entry.kick_on_start)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CapturedLineage {
    pub(crate) lineage: Option<RequestId>,
    pub(crate) mirror_owner: Option<RequestId>,
    pub(crate) dropped: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MachineEventKind {
    Phase(crate::native::NativePhase),
    Done,
    Failed,
    Aborted(api::script_contract::AbortReason),
    ManualWalk,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MachineEvent {
    pub(crate) id: RequestId,
    pub(crate) kind: MachineEventKind,
    pub(crate) admitted: bool,
}

#[derive(Clone, Copy, Default)]
struct StartContext {
    parent_row: Option<Handle>,
    seat_parent: Option<RequestId>,
    local_parent: Option<RequestId>,
    parent_revoked: bool,
    history_expired: bool,
}
struct StartContextGuard(Option<StartContext>);
impl Drop for StartContextGuard {
    fn drop(&mut self) {
        START_CONTEXT.with(|slot| slot.set(self.0));
    }
}

fn with_start_context<T>(context: StartContext, f: impl FnOnce() -> T) -> T {
    let previous = START_CONTEXT.with(|slot| slot.replace(Some(context)));
    let _restore = StartContextGuard(previous);
    f()
}

#[derive(Clone)]
pub(crate) enum SeatStartPayload {
    Empty,
    Gather(std::sync::Arc<crate::native::SettingsBag>),
    Combat(std::sync::Arc<crate::api_combat::CombatSessionRequest>),
    QuestProgress(std::sync::Arc<str>),
    Bank(crate::bank::request::BankSeatArgs),
}

impl SeatStartPayload {
    fn as_ref(&self) -> isolate_fb::SeatStartArgsRef<'_> {
        match self {
            Self::Empty => isolate_fb::SeatStartArgsRef::Empty,
            Self::Gather(settings) => isolate_fb::SeatStartArgsRef::Gather(settings),
            Self::Combat(request) => isolate_fb::SeatStartArgsRef::Combat(request),
            Self::QuestProgress(name) => isolate_fb::SeatStartArgsRef::QuestProgress(name),
            Self::Bank(args) => isolate_fb::SeatStartArgsRef::Bank(args),
        }
    }
}

#[derive(Clone)]
pub(crate) struct PendingSeatStart {
    id: RequestId,
    parent: Option<RequestId>,
    local_parent: Option<RequestId>,
    kind: native::request::NativeKind,
    args: SeatStartPayload,
}

impl PendingSeatStart {
    pub(crate) fn as_ref(
        &self,
        execution: ExecutionId,
    ) -> Result<isolate_fb::SeatStartRef<'_>, String> {
        let stamp = RequestStamp::new(self.id, self.parent, self.local_parent, execution)
            .map_err(|error| error.to_string())?;
        Ok(isolate_fb::SeatStartRef {
            stamp,
            kind: self.kind,
            args: self.args.as_ref(),
        })
    }
}

#[derive(Clone, Copy)]
struct PendingSeatCancel {
    of: RequestId,
    reason: CallbackCancelReason,
}

#[derive(Clone)]
struct BankPrompt {
    sequence: DecisionSequence,
    point: String,
    args: Vec<u8>,
}

#[derive(Clone)]
struct PendingSeatDecision {
    of: RequestId,
    sequence: DecisionSequence,
    answer: Vec<u8>,
}

struct BankMirror {
    id: RequestId,
    family: &'static str,
    transfer_busy: bool,
    prompt: Option<BankPrompt>,
    terminal: Option<Outcome>,
    admitted: bool,
    orphaned: bool,
}

/// One isolate-side continuation over an admitted native bank seat row.
pub(crate) struct NativeBankChild {
    id: RequestId,
    family: &'static str,
    hook_indices: Vec<Option<usize>>,
    seen_prompt: u32,
    awaiting_reply: Option<u32>,
    cancel_on_drop: bool,
    finished: bool,
}

fn bank_hook_indices(
    descriptor: &BankFamilyDescriptor,
    operation: crate::bank::request::BankSeatOperation,
    hook_map: &[(&str, usize)],
    hook_count: usize,
    mut present: impl FnMut(usize) -> bool,
) -> Result<(Vec<Option<usize>>, u32), String> {
    let mut indices = vec![None; descriptor.callbacks.len()];
    let mut hooks_present = 0;
    for (member, index) in hook_map {
        if *index >= hook_count {
            return Err(format!("bank callback {member:?} index {index} is out of range"));
        }
        let Some(slot) = descriptor
            .callbacks
            .iter()
            .position(|callback| callback == member)
        else {
            return Err(format!(
                "bank callback {member:?} is not declared by {}",
                descriptor.name
            ));
        };
        if indices[slot].replace(*index).is_some() {
            return Err(format!("bank callback {member:?} is mapped more than once"));
        }
        if present(*index) {
            hooks_present |= bank_hook_bit(operation, member);
        }
    }
    Ok((indices, hooks_present))
}

fn require_bank_hooks(
    operation: crate::bank::request::BankSeatOperation,
    hooks_present: u32,
) -> Result<(), String> {
    use crate::bank::request::{BankSeatOperation as Op, DEPOSIT_MATCH_HOOK_MATCH};
    if operation == Op::DepositMatch && hooks_present & DEPOSIT_MATCH_HOOK_MATCH == 0 {
        return Err("bank_deposit requires a match function".into());
    }
    Ok(())
}

fn enqueue_bank_child(
    id: RequestId,
    family: &'static str,
    args: crate::bank::request::BankSeatArgs,
    context: StartContext,
) -> Result<(), String> {
    let descriptor =
        bank_family_descriptor(family).ok_or_else(|| format!("unknown bank family {family:?}"))?;
    if bank_family_for_operation(args.operation) != family {
        return Err(format!(
            "bank operation {:?} does not belong to {family}",
            args.operation
        ));
    }
    args.validate(descriptor.kind)?;
    let mut host = HOST.with(|host| host.borrow_mut());
    if host.execution.is_none() {
        return Err("native bank seat requires an active execution".into());
    }
    if host.pending_starts.len() >= MAX_PENDING_SEAT_STARTS {
        return Err("native seat start carry is full".into());
    }
    if host.bank_mirrors.iter().any(|mirror| mirror.id == id) {
        return Err("native bank seat identity is already registered".into());
    }
    host.pending_starts.push(PendingSeatStart {
        id,
        parent: context.seat_parent,
        local_parent: context.local_parent,
        kind: descriptor.kind,
        args: SeatStartPayload::Bank(args.clone()),
    });
    let transfer_busy = bank_transfer_operation(args.operation)
        || (args.operation == crate::bank::request::BankSeatOperation::BankNearest
            && (args.deposit_all
                || args.hooks_present & crate::bank::request::BANK_NEAREST_HOOK_DEPOSIT != 0));
    host.bank_mirrors.push(BankMirror {
        id,
        family,
        transfer_busy,
        prompt: None,
        terminal: None,
        admitted: false,
        orphaned: false,
    });
    Ok(())
}

impl NativeBankChild {
    /// Start one typed native bank request under this row's captured lineage.
    pub(crate) fn start(
        mut args: crate::bank::request::BankSeatArgs,
        hook_map: &[(&str, usize)],
        cx: &Cx<'_>,
    ) -> Result<Self, String> {
        flush_deferred_bank_cancels();
        if IN_BEGIN.with(Cell::get) {
            return Err("a machine begin may not start a machine".into());
        }
        if cx.context.history_expired {
            return Err("history expired".into());
        }
        if cx.context.parent_revoked {
            return Err("parent not live".into());
        }
        let family = bank_family_for_operation(args.operation);
        let descriptor = bank_family_descriptor(family)
            .ok_or_else(|| format!("unknown bank family {family:?}"))?;
        let (hook_indices, hooks_present) = bank_hook_indices(
            descriptor,
            args.operation,
            hook_map,
            cx.hooks.len(),
            |index| cx.has(index),
        )?;
        require_bank_hooks(args.operation, hooks_present)?;
        args.hooks_present = hooks_present;
        args.validate(descriptor.kind)?;
        let handle = HOST
            .with(|host| host.borrow_mut().allocate_handle())
            .ok_or_else(|| "native bank seat identity exhausted".to_owned())?;
        let id = RequestId::isolate(handle)
            .map_err(|error| format!("native bank seat identity: {error}"))?;
        enqueue_bank_child(id, family, args, cx.context)?;
        Ok(Self {
            id,
            family,
            hook_indices,
            seen_prompt: 0,
            awaiting_reply: None,
            cancel_on_drop: true,
            finished: false,
        })
    }

    /// Drive an admitted native bank request once.
    pub(crate) fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
        if self.finished {
            return Step::Fail(Thrown::new("native bank child is already settled"));
        }
        let mirror = HOST.with(|host| {
            host.borrow()
                .bank_mirrors
                .iter()
                .find(|mirror| mirror.id == self.id)
                .map(|mirror| {
                    (
                        mirror.admitted,
                        mirror.prompt.clone(),
                        mirror.terminal.clone(),
                    )
                })
        });
        let Some((admitted, prompt, terminal)) = mirror else {
            self.finished = true;
            return Step::Fail(Thrown::new("native bank seat mirror is unavailable"));
        };
        if let Some(outcome) = terminal {
            self.finished = true;
            clear_bank_mirror(self.id);
            return match outcome {
                Outcome::Done(value) => Step::Done(value),
                Outcome::Failed(thrown) => Step::Fail(thrown),
                Outcome::Aborted(reason) => Step::Fail(Thrown::new(format!(
                    "native bank seat aborted: {}",
                    reason.as_str()
                ))),
                Outcome::Refused(reason) => Step::Fail(Thrown::new(format!(
                    "native bank seat refused: {reason}"
                ))),
            };
        }
        if !admitted {
            return Step::Wait;
        }
        if let Some(sequence) = self.awaiting_reply {
            match cx.reply() {
                None => return Step::Wait,
                Some(Reply::Value(value)) => {
                    if let Err(error) = queue_bank_decision(self.id, sequence, value) {
                        self.fail_callback();
                        return Step::Fail(Thrown::new(error));
                    }
                    self.awaiting_reply = None;
                    return Step::Wait;
                }
                Some(Reply::Threw(thrown)) => {
                    self.fail_callback();
                    return Step::Fail(thrown);
                }
            }
        }
        let Some(prompt) = prompt else {
            return Step::Wait;
        };
        let sequence = prompt.sequence.get();
        if sequence <= self.seen_prompt {
            return Step::Wait;
        }
        self.seen_prompt = sequence;
        let Some((point, hook_index)) =
            bank_prompt_hook(self.family, &prompt.point, &self.hook_indices)
        else {
            self.fail_callback();
            return Step::Fail(Thrown::new(format!(
                "not impl: no source-backed bank prompt {} / {:?}",
                self.family, prompt.point
            )));
        };
        if !cx.has(hook_index) {
            self.fail_callback();
            return Step::Fail(Thrown::new(format!(
                "not impl: bank prompt {} / {:?} has no held callback",
                self.family, prompt.point
            )));
        }
        let args = match crate::channel::decode(&prompt.args) {
            Ok(Value::Array(args)) => args,
            Ok(_) => {
                self.fail_callback();
                return Step::Fail(Thrown::new("native bank prompt arguments are not an array"));
            }
            Err(error) => {
                self.fail_callback();
                return Step::Fail(Thrown::new(format!(
                    "native bank prompt arguments: {error}"
                )));
            }
        };
        match point.path() {
            native::abi::HookPath::Awaited => {
                self.awaiting_reply = Some(sequence);
                cx.callback_path = Some(native::abi::HookPath::Awaited);
                Step::Call(Call {
                    hook: hook_index,
                    args,
                })
            }
            path @ (native::abi::HookPath::CallSync | native::abi::HookPath::Ask) => {
                match cx.call_prompt(Some(hook_index), &args, path) {
                    Ok(Called::Settled(Reply::Value(value))) => {
                        if let Err(error) = queue_bank_decision(self.id, prompt.sequence, value) {
                            self.fail_callback();
                            Step::Fail(Thrown::new(error))
                        } else {
                            Step::Wait
                        }
                    }
                    Ok(Called::Settled(Reply::Threw(_)))
                    | Ok(Called::Pending(_))
                    | Ok(Called::Terminated) => {
                        unreachable!("call_prompt resolves synchronous bank callbacks")
                    }
                    Err(Ended) => {
                        if matches!(cx.ending.as_ref(), Some(Ending::Failed(_))) {
                            self.fail_callback();
                        } else {
                            self.mark_orphaned();
                        }
                        Step::Wait
                    }
                }
            }
        }
    }

    fn cancel_and_orphan(&mut self, reason: CallbackCancelReason) {
        HOST.with(|host| {
            let mut host = host.borrow_mut();
            host.pending_decisions.retain(|decision| decision.of != self.id);
            host.queue_cancel(self.id, reason);
            if let Some(mirror) = host.bank_mirrors.iter_mut().find(|row| row.id == self.id) {
                mirror.orphaned = true;
            }
        });
        self.awaiting_reply = None;
        self.finished = true;
    }

    fn fail_callback(&mut self) {
        if self.cancel_on_drop {
            self.cancel_and_orphan(CallbackCancelReason::HookThrew);
        } else {
            self.mark_orphaned();
        }
    }

    fn mark_orphaned(&mut self) {
        HOST.with(|host| {
            if let Some(mirror) = host
                .borrow_mut()
                .bank_mirrors
                .iter_mut()
                .find(|row| row.id == self.id)
            {
                mirror.orphaned = true;
            }
        });
        self.finished = true;
    }

    pub(crate) fn cancel(&mut self) {
        if !self.finished {
            self.awaiting_reply = None;
            self.finished = true;
            defer_bank_cancel(self.id);
        }
    }
}

impl Drop for NativeBankChild {
    fn drop(&mut self) {
        if self.cancel_on_drop && !self.finished {
            self.cancel();
        }
    }
}

fn bank_prompt_hook(
    family: &str,
    point: &str,
    hook_indices: &[Option<usize>],
) -> Option<(native::PromptPoint, usize)> {
    let descriptor = bank_family_descriptor(family)?;
    let mut rows = native::abi::HOOK_CHARACTERIZATIONS
        .iter()
        .filter(|row| row.families().contains(&family) && row.member() == point);
    let characterization = rows.next()?;
    if rows.next().is_some() {
        return None;
    }
    let point = native::PromptPoint::from_contract_row(characterization)?;
    let index = descriptor
        .callbacks
        .iter()
        .position(|member| *member == point.member())?;
    Some((point, hook_indices.get(index).copied().flatten()?))
}

fn queue_bank_decision(
    id: RequestId,
    sequence: DecisionSequence,
    answer: Value,
) -> Result<(), String> {
    let answer = crate::channel::encode(&answer)?;
    if answer.len() > isolate_fb::MAX_SEAT_BINARY_BYTES {
        return Err(format!(
            "native bank decision exceeds {} bytes",
            isolate_fb::MAX_SEAT_BINARY_BYTES
        ));
    }
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        if host.execution.is_none() {
            return Err("native bank decision has no active execution".into());
        }
        if host
            .pending_decisions
            .iter()
            .any(|decision| decision.of == id && decision.sequence == sequence)
        {
            return Ok(());
        }
        if host.pending_decisions.len() >= isolate_fb::MAX_SEAT_DECISIONS {
            return Err("native bank decision carry is full".into());
        }
        host.pending_decisions.push(PendingSeatDecision {
            of: id,
            sequence,
            answer,
        });
        Ok(())
    })
}

fn clear_bank_mirror(id: RequestId) {
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        host.bank_mirrors.retain(|mirror| mirror.id != id);
        host.pending_decisions.retain(|decision| decision.of != id);
        host.pending_cancels.retain(|cancel| cancel.of != id);
    });
}

pub(crate) struct SeatBatchCarry<'a> {
    pub(crate) consumed_terminal_seq: u64,
    pub(crate) terminal_ack_pending: bool,
    pub(crate) starts: Result<Vec<isolate_fb::SeatStartRef<'a>>, String>,
    pub(crate) decisions: Vec<isolate_fb::SeatDecisionRef<'a>>,
    pub(crate) cancels: Vec<isolate_fb::SeatCancelInput>,
    pub(crate) mirror_revokes: Vec<isolate_fb::SeatMirrorRevoke>,
    pub(crate) machine_events: Vec<MachineEvent>,
}

impl SeatBatchCarry<'_> {
    pub(crate) fn has_work(&self) -> bool {
        self.terminal_ack_pending
            || self.starts.as_ref().is_ok_and(|starts| !starts.is_empty())
            || self.starts.is_err()
            || !self.decisions.is_empty()
            || !self.cancels.is_empty()
            || !self.mirror_revokes.is_empty()
            || !self.machine_events.is_empty()
    }
}

#[derive(Clone, Copy)]
struct SteppingRow {
    family: &'static str,
    handle: Handle,
    walking: bool,
    baseline: u64,
    lineage: CapturedLineage,
    seat_parent: Option<RequestId>,
    native_kind: Option<native::request::NativeKind>,
    authority_revoked: bool,
}

impl SteppingRow {
    fn capture(self) -> CapturedLineage {
        if self.native_kind.is_some() {
            CapturedLineage {
                lineage: RequestId::isolate(self.handle).ok(),
                mirror_owner: None,
                dropped: self.authority_revoked,
            }
        } else {
            CapturedLineage {
                dropped: self.authority_revoked,
                ..self.lineage
            }
        }
    }

    fn start_context(self) -> StartContext {
        StartContext {
            parent_row: Some(self.handle),
            seat_parent: if self.native_kind.is_some() {
                RequestId::isolate(self.handle).ok()
            } else {
                self.seat_parent
            },
            local_parent: if self.native_kind.is_none() {
                RequestId::isolate(self.handle).ok()
            } else {
                None
            },
            parent_revoked: self.authority_revoked,
            history_expired: false,
        }
    }
}

#[derive(Clone)]
struct PlacedOp {
    at: usize,
    request: InteractReq,
    lineage: CapturedLineage,
}

#[derive(Clone, Copy)]
struct WalkReleaseReservation {
    id: Handle,
    queued: bool,
}

const MAX_WALK_RELEASES: usize = api::script_contract::MAX_WALK_RELEASES;

#[derive(Clone, Copy)]
struct MirrorRevokeCarry {
    wire: isolate_fb::SeatMirrorRevoke,
}
#[derive(Clone, Copy)]
struct LocalRevocation {
    handle: Handle,
    lineage: CapturedLineage,
    context: StartContext,
}

struct Row {
    handle: Handle,
    family: &'static str,
    /// [`Family::EXCLUSIVE_GROUP`] of an exclusive family.
    exclusive: Option<&'static str>,
    /// [`Family::SYNC_HOOKS`].
    sync_hooks: &'static [usize],
    clock: InstantTaskClock,
    hooks: Vec<Rc<Hook>>,
    /// Callbacks run this tick, across both passes.
    calls: usize,
    /// Last `(tick, evidence sequence)` pass this row consumed.
    last_snapshot: Option<(u64, u64)>,
    /// The settled callback the next step reads.
    reply: Option<Reply>,
    /// The callback promise the row waits on.
    pending: Option<Pending>,
    machine: Box<dyn Machine>,
    /// The nearest seat entry this local row mirrors.
    seat_parent: Option<RequestId>,
    /// The local row that started this row, including across seat cutover.
    parent_row: Option<Handle>,
    /// A native seat owner remains distinct from an in-isolate mirror.
    native_kind: Option<native::request::NativeKind>,
    /// The seat acknowledged this start as live.
    admitted: bool,
    /// Local-parent cancellation has been carried; only the seat terminal
    /// may settle this native child's JS promise.
    awaiting_seat_terminal: bool,
    /// A revoked owner retains its token so late pushes fail closed.
    authority_revoked: bool,
    /// Current truthful phase reported for this request.
    phase: native::NativePhase,
    /// ManualWalk publishes a Done terminal before this local row settles.
    manual_walk_event_sent: bool,
    /// The outer walking intent baseline carried through nested rows.
    intent_baseline: u64,
    /// This row belongs to an outer walking operation or owns one.
    carries_walk_baseline: bool,
    /// This family's live row represents a walking operation.
    walking_operation: bool,
    /// Rebaseline only an uncancelled operator-paused walk on resume.
    rebaseline_on_resume: bool,
}
impl Row {
    fn capture(&self) -> CapturedLineage {
        if self.native_kind.is_some() {
            CapturedLineage {
                lineage: RequestId::isolate(self.handle).ok(),
                mirror_owner: None,
                dropped: self.authority_revoked,
            }
        } else {
            CapturedLineage {
                lineage: self.seat_parent,
                mirror_owner: RequestId::isolate(self.handle).ok(),
                dropped: self.authority_revoked,
            }
        }
    }

    fn start_context(&self) -> StartContext {
        StartContext {
            parent_row: Some(self.handle),
            seat_parent: if self.native_kind.is_some() {
                RequestId::isolate(self.handle).ok()
            } else {
                self.seat_parent
            },
            local_parent: if self.native_kind.is_none() {
                RequestId::isolate(self.handle).ok()
            } else {
                None
            },
            parent_revoked: self.authority_revoked,
            history_expired: false,
        }
    }

    fn revoked_record(&self) -> LocalRevocation {
        let mut context = self.start_context();
        context.parent_revoked = true;
        LocalRevocation {
            handle: self.handle,
            lineage: self.capture(),
            context,
        }
    }
    fn stepping_record(&self) -> SteppingRow {
        SteppingRow {
            family: self.family,
            handle: self.handle,
            walking: self.walking_operation,
            baseline: self.intent_baseline,
            lineage: self.capture(),
            seat_parent: self.seat_parent,
            native_kind: self.native_kind,
            authority_revoked: self.authority_revoked,
        }
    }
}

struct Host {
    next: Handle,
    rows: Vec<Row>,
    /// Rows a pass is stepping while user callbacks may start children.
    stepping: Vec<SteppingRow>,
    /// The newest running row of each exclusive group.
    newest: Vec<(&'static str, Handle)>,
    settled: Vec<(Handle, Outcome)>,
    ops: Vec<PlacedOp>,
    pending_starts: Vec<PendingSeatStart>,
    pending_decisions: Vec<PendingSeatDecision>,
    pending_cancels: Vec<PendingSeatCancel>,
    bank_mirrors: Vec<BankMirror>,
    mirror_revokes: Vec<MirrorRevokeCarry>,
    machine_events: Vec<MachineEvent>,
    consumed_terminal_seq: u64,
    terminal_ack_pending: bool,
    mirror_revokes_consumed: u64,
    revoked_local: VecDeque<LocalRevocation>,
    revoked_floor: Handle,
    walk_releases: Vec<WalkReleaseReservation>,
    next_mirror_revoke_seq: u64,
    execution: Option<ExecutionId>,
    paused: bool,
    held: bool,
    walking_publication: Option<std::sync::Arc<WalkingOwnership>>,
}

thread_local! {
    static HOST: RefCell<Host> = const { RefCell::new(Host::new()) };
    /// A family `begin` is running: a nested start is refused.
    static IN_BEGIN: Cell<bool> = const { Cell::new(false) };
    /// The push-time local/seat lineage for the next machine start.
    static START_CONTEXT: Cell<Option<StartContext>> = const { Cell::new(None) };
    /// A reconnect holds the session's work (`ResetSession { keep_work }`):
    /// the host carries the held walk, so no row sends a release op.
    static SESSION_HELD: Cell<bool> = const { Cell::new(false) };
    /// A callback was terminated in this pass or kick: drive no further.
    static TERMINATED: Cell<bool> = const { Cell::new(false) };
    /// The active walking-operation lineage for synchronous nested starts.
    static WALK_BASELINE: Cell<Option<u64>> = const { Cell::new(None) };
    /// Latest takeover sequence delivered through the ordered host queue.
    static TAKEOVER_SEQ: Cell<u64> = const { Cell::new(0) };
    /// Child drops that occur while `HOST` is borrowed are reconciled before
    /// the next seat batch is inspected or encoded.
    static DEFERRED_BANK_CANCELS: RefCell<Vec<RequestId>> = const { RefCell::new(Vec::new()) };
}

fn defer_bank_cancel(id: RequestId) {
    DEFERRED_BANK_CANCELS.with(|pending| {
        let mut pending = pending.borrow_mut();
        if !pending.contains(&id) {
            pending.push(id);
        }
    });
}

fn flush_deferred_bank_cancels() {
    let pending = DEFERRED_BANK_CANCELS.with(|pending| std::mem::take(&mut *pending.borrow_mut()));
    if pending.is_empty() {
        return;
    }
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        for id in pending {
            host.pending_decisions
                .retain(|decision| decision.of != id);
            host.queue_cancel(id, CallbackCancelReason::ParentAborted);
            if let Some(mirror) = host.bank_mirrors.iter_mut().find(|row| row.id == id) {
                mirror.orphaned = true;
            }
        }
    });
}

struct WalkBaselineGuard(Option<u64>);

impl Drop for WalkBaselineGuard {
    fn drop(&mut self) {
        WALK_BASELINE.with(|baseline| baseline.set(self.0));
    }
}

fn with_walk_baseline<T>(baseline: Option<u64>, f: impl FnOnce() -> T) -> T {
    let previous = WALK_BASELINE.with(|active| active.replace(baseline));
    let _restore = WalkBaselineGuard(previous);
    f()
}

fn observed_user_move_intent_seq() -> u64 {
    crate::observed::with(|scene| scene.since_login().user_move_intent_seq().unwrap_or(0))
}

pub(crate) fn user_move_intent_seq() -> u64 {
    observed_user_move_intent_seq().max(HOST.with(|host| host.borrow().takeover_seq()))
}

impl Host {
    const fn new() -> Self {
        Self {
            next: 1,
            rows: Vec::new(),
            stepping: Vec::new(),
            newest: Vec::new(),
            settled: Vec::new(),
            ops: Vec::new(),
            pending_starts: Vec::new(),
            pending_decisions: Vec::new(),
            pending_cancels: Vec::new(),
            bank_mirrors: Vec::new(),
            mirror_revokes: Vec::new(),
            machine_events: Vec::new(),
            consumed_terminal_seq: 0,
            terminal_ack_pending: false,
            mirror_revokes_consumed: 0,
            revoked_local: VecDeque::new(),
            revoked_floor: 1,
            walk_releases: Vec::new(),
            next_mirror_revoke_seq: 1,
            execution: None,
            paused: false,
            held: false,
            walking_publication: None,
        }
    }

    fn takeover_seq(&self) -> u64 {
        self.walking_publication
            .as_ref()
            .map_or(0, |publication| {
                publication
                    .takeover
                    .load(std::sync::atomic::Ordering::Acquire)
            })
            .max(TAKEOVER_SEQ.with(Cell::get))
    }

    fn walking_baseline(&self) -> Option<u64> {
        if SESSION_HELD.with(Cell::get) {
            return None;
        }
        let takeover = self.takeover_seq();
        self.rows
            .iter()
            .filter(|row| row.walking_operation && !row.awaiting_seat_terminal)
            .map(|row| row.intent_baseline)
            .chain(
                self.stepping
                    .iter()
                    .filter_map(|row| row.walking.then_some(row.baseline)),
            )
            .filter(|baseline| *baseline >= takeover)
            .max()
    }

    fn publish_walking(&self) {
        let Some(publication) = &self.walking_publication else {
            return;
        };
        if let Some(baseline) = self.walking_baseline() {
            publication
                .baseline
                .store(baseline, std::sync::atomic::Ordering::Release);
            publication
                .live
                .store(true, std::sync::atomic::Ordering::Release);
        } else {
            publication
                .live
                .store(false, std::sync::atomic::Ordering::Release);
        }
    }

    fn fresh_clock(&self) -> InstantTaskClock {
        let mut clock = InstantTaskClock::new();
        clock.set_freeze(self.paused, self.held);
        clock
    }

    fn allocate_handle(&mut self) -> Option<Handle> {
        if self.next == 0 || self.next > MAX_MACHINE_HANDLE {
            return None;
        }
        let handle = self.next;
        self.next = handle.checked_add(1)?;
        Some(handle)
    }

    fn start_context_for(&self, parent: Option<Handle>) -> StartContext {
        let Some(handle) = parent else {
            return StartContext::default();
        };
        if let Some(row) = self.rows.iter().find(|row| row.handle == handle) {
            return row.start_context();
        }
        if let Some(row) = self.stepping.iter().find(|row| row.handle == handle) {
            return row.start_context();
        }
        if let Some(revoked) = self
            .revoked_local
            .iter()
            .find(|revoked| revoked.handle == handle)
        {
            return revoked.context;
        }
        if handle < self.revoked_floor {
            return StartContext {
                history_expired: true,
                ..StartContext::default()
            };
        }
        if handle < self.next {
            // Ordinary completion is not a continuation authority.
            StartContext::default()
        } else {
            StartContext {
                parent_revoked: true,
                ..StartContext::default()
            }
        }
    }

    fn capture_push(&self, parent: Option<Handle>) -> CapturedLineage {
        if self.execution.is_none() {
            return CapturedLineage::default();
        }
        let Some(handle) = parent else {
            return CapturedLineage::default();
        };
        if let Some(row) = self.rows.iter().find(|row| row.handle == handle) {
            return row.capture();
        }
        if let Some(row) = self.stepping.iter().find(|row| row.handle == handle) {
            return row.capture();
        }
        if let Some(revoked) = self
            .revoked_local
            .iter()
            .find(|revoked| revoked.handle == handle)
        {
            return CapturedLineage {
                dropped: true,
                ..revoked.lineage
            };
        }
        if handle < self.revoked_floor || handle >= self.next {
            return CapturedLineage {
                dropped: true,
                ..CapturedLineage::default()
            };
        }
        CapturedLineage::default()
    }

    fn remember_revoked(&mut self, row: &Row) {
        if row.handle < self.revoked_floor
            || self
                .revoked_local
                .iter()
                .any(|revoked| revoked.handle == row.handle)
        {
            return;
        }
        if self.revoked_local.len() == api::script_contract::REVOKED_RING {
            if let Some(oldest) = self.revoked_local.pop_front() {
                self.revoked_floor = self.revoked_floor.max(oldest.handle.saturating_add(1));
            }
        }
        self.revoked_local.push_back(row.revoked_record());
    }

    fn queue_machine_event(&mut self, id: RequestId, kind: MachineEventKind, admitted: bool) {
        if self.execution.is_none() {
            return;
        }
        if matches!(kind, MachineEventKind::Phase(_)) {
            if let Some(event) = self
                .machine_events
                .iter_mut()
                .find(|event| event.id == id && matches!(event.kind, MachineEventKind::Phase(_)))
            {
                event.kind = kind;
                event.admitted |= admitted;
                return;
            }
        } else {
            self.machine_events.retain(|event| {
                event.id != id || !matches!(event.kind, MachineEventKind::Phase(_))
            });
            if let Some(event) = self
                .machine_events
                .iter_mut()
                .find(|event| event.id == id && event.kind == kind)
            {
                event.admitted |= admitted;
                return;
            }
        }
        self.machine_events
            .push(MachineEvent { id, kind, admitted });
    }

    fn set_phase(&mut self, row: &mut Row, phase: native::NativePhase) {
        if row.phase == phase || row.phase == native::NativePhase::Blocked {
            return;
        }
        row.phase = phase;
        if let Some(id) = RequestId::isolate(row.handle)
            .ok()
            .filter(|_| row.native_kind.is_some())
        {
            self.queue_machine_event(id, MachineEventKind::Phase(phase), row.admitted);
        }
    }

    fn carry_typed_start(&mut self, request: &InteractReq) -> bool {
        if self.execution.is_none() {
            return false;
        }
        let (raw_id, args) = match request {
            InteractReq::GatherRun {
                request_id,
                settings,
            } => (
                *request_id,
                SeatStartPayload::Gather(std::sync::Arc::clone(settings)),
            ),
            InteractReq::CombatFight {
                request_id,
                request,
            } => (
                *request_id,
                SeatStartPayload::Combat(std::sync::Arc::clone(request)),
            ),
            InteractReq::ProgressRead { request_id, name } => (
                *request_id,
                SeatStartPayload::QuestProgress(std::sync::Arc::from(name.as_str())),
            ),
            _ => return false,
        };
        if let Ok(id) = RequestId::isolate(raw_id) {
            if let Some(start) = self.pending_starts.iter_mut().find(|start| start.id == id) {
                start.args = args;
            }
        }
        true
    }

    fn place(&mut self, at: usize, ops: Vec<InteractReq>, lineage: CapturedLineage) {
        for request in ops {
            if self.carry_typed_start(&request) {
                continue;
            }
            if let InteractReq::AbortWalk { request_id } = &request {
                if *request_id > 0 {
                    if let Some(slot) = self
                        .walk_releases
                        .iter_mut()
                        .find(|slot| slot.id == *request_id)
                    {
                        if slot.queued {
                            continue;
                        }
                        slot.queued = true;
                    } else if self.walk_releases.len() < MAX_WALK_RELEASES {
                        self.walk_releases.push(WalkReleaseReservation {
                            id: *request_id,
                            queued: true,
                        });
                    }
                }
            }
            self.ops.push(PlacedOp {
                at,
                request,
                lineage,
            });
        }
    }

    fn free_completed_walk(&mut self, id: Handle) {
        if self
            .walk_releases
            .iter()
            .any(|slot| slot.id == id && slot.queued)
        {
            return;
        }
        self.walk_releases.retain(|slot| slot.id != id);
    }

    fn mark_walk_release_encoded(&mut self, id: Handle) {
        self.walk_releases
            .retain(|slot| slot.id != id || !slot.queued);
    }

    fn queue_cancel(&mut self, of: RequestId, reason: CallbackCancelReason) {
        if self.execution.is_none() {
            return;
        }
        if let Some(existing) = self
            .pending_cancels
            .iter_mut()
            .find(|cancel| cancel.of == of)
        {
            if reason == CallbackCancelReason::ParentAborted {
                existing.reason = reason;
            }
            return;
        }
        self.pending_cancels.push(PendingSeatCancel { of, reason });
    }

    fn queue_mirror_revoke(&mut self, owner: RequestId) {
        if self.execution.is_none()
            || self
                .mirror_revokes
                .iter()
                .any(|revoke| revoke.wire.owner == owner)
        {
            return;
        }
        let seq = self.next_mirror_revoke_seq;
        let Some(next) = seq.checked_add(1) else {
            return;
        };
        self.next_mirror_revoke_seq = next;
        self.mirror_revokes.push(MirrorRevokeCarry {
            wire: isolate_fb::SeatMirrorRevoke { owner, seq },
        });
    }

    fn unstep(&mut self, family: &'static str, handle: Handle) {
        self.stepping
            .retain(|row| !(row.family == family && row.handle == handle));
        self.publish_walking();
    }
    fn is_revoked(&self, handle: Handle) -> bool {
        self.revoked_local
            .iter()
            .any(|revoked| revoked.handle == handle)
    }

    fn revoke_descendants(&mut self, root: Handle, at: usize) {
        let mut frontier = vec![root];
        let mut descendants = Vec::new();
        let mut index = 0;
        while index < frontier.len() {
            let parent = frontier[index];
            for row in &self.rows {
                if row.parent_row == Some(parent) && !descendants.contains(&row.handle) {
                    descendants.push(row.handle);
                    frontier.push(row.handle);
                }
            }
            index += 1;
        }

        while let Some(handle) = descendants
            .iter()
            .copied()
            .filter(|handle| {
                !self.rows.iter().any(|child| {
                    child.parent_row == Some(*handle) && descendants.contains(&child.handle)
                })
            })
            .min()
        {
            descendants.retain(|candidate| *candidate != handle);
            let Some(position) = self.rows.iter().position(|row| row.handle == handle) else {
                continue;
            };
            let mut row = self.rows.remove(position);
            if row.awaiting_seat_terminal {
                self.rows.push(row);
                continue;
            }
            self.remember_revoked(&row);
            if let Ok(id) = RequestId::isolate(row.handle) {
                if row.native_kind.is_some() {
                    self.queue_cancel(id, CallbackCancelReason::ParentAborted);
                } else {
                    self.queue_mirror_revoke(id);
                }
            }
            if let Some(op) = release_op(&*row.machine) {
                self.place(at, vec![op], row.capture());
            }
            row.machine.abort(AbortReason::ParentAborted);
            self.free_completed_walk(row.handle);
            if row.native_kind.is_some() {
                row.authority_revoked = true;
                row.awaiting_seat_terminal = true;
                self.rows.push(row);
            } else {
                self.settled
                    .push((row.handle, Outcome::Aborted(AbortReason::ParentAborted)));
            }
        }
    }

    fn revoke_row(&mut self, row: &Row, reason: AbortReason, at: usize) {
        self.remember_revoked(row);
        if let Ok(id) = RequestId::isolate(row.handle) {
            if row.native_kind.is_some() {
                if !row.admitted || reason == AbortReason::ParentAborted {
                    self.queue_cancel(id, CallbackCancelReason::ParentAborted);
                } else if let Some(reason) = match reason {
                    AbortReason::Superseded => Some(api::script_contract::AbortReason::Superseded),
                    AbortReason::UserInput => Some(api::script_contract::AbortReason::UserInput),
                    AbortReason::Reset | AbortReason::Terminated | AbortReason::Unknown => None,
                    AbortReason::ParentAborted => None,
                } {
                    self.queue_machine_event(id, MachineEventKind::Aborted(reason), row.admitted);
                }
            } else {
                self.queue_mirror_revoke(id);
            }
        }
        self.revoke_descendants(row.handle, at);
    }

    fn finish_row(&mut self, row: &Row, outcome: Outcome, at: usize) {
        match &outcome {
            Outcome::Done(_) if row.manual_walk_event_sent => {
                self.remember_revoked(row);
                if let Ok(id) = RequestId::isolate(row.handle) {
                    if row.native_kind.is_some() {
                        self.queue_machine_event(id, MachineEventKind::ManualWalk, row.admitted);
                    } else {
                        self.queue_mirror_revoke(id);
                    }
                }
                self.revoke_descendants(row.handle, at);
            }
            Outcome::Done(_) => {
                if let Some(id) = RequestId::isolate(row.handle)
                    .ok()
                    .filter(|_| row.native_kind.is_some())
                {
                    self.queue_machine_event(id, MachineEventKind::Done, row.admitted);
                }
            }
            Outcome::Failed(_) => {
                if let Some(id) = RequestId::isolate(row.handle)
                    .ok()
                    .filter(|_| row.native_kind.is_some())
                {
                    self.queue_machine_event(id, MachineEventKind::Failed, row.admitted);
                }
            }
            Outcome::Aborted(reason) => self.revoke_row(row, *reason, at),
            Outcome::Refused(_) => {}
        }
        self.free_completed_walk(row.handle);
        self.settled.push((row.handle, outcome));
    }

    fn superseded(&self, row: &Row) -> bool {
        row.exclusive.is_some_and(|group| {
            self.newest
                .iter()
                .any(|(newest, handle)| *newest == group && *handle != row.handle)
        })
    }

    /// Abort every superseded row; its release operation stays at `at`.
    fn abort_superseded(&mut self, at: usize) {
        let mut i = 0;
        while i < self.rows.len() {
            if self.rows[i].native_kind.is_some() {
                i += 1;
                continue;
            }
            if self.superseded(&self.rows[i]) {
                let mut row = self.rows.remove(i);
                if let Some(op) = release_op(&*row.machine) {
                    self.place(at, vec![op], row.capture());
                }
                self.revoke_row(&row, AbortReason::Superseded, at);
                row.machine.abort(AbortReason::Superseded);
                self.free_completed_walk(row.handle);
                self.settled
                    .push((row.handle, Outcome::Aborted(AbortReason::Superseded)));
            } else {
                i += 1;
            }
        }
    }

    fn take(&mut self, handle: Handle) -> Take {
        if let Some(i) = self.settled.iter().position(|(h, _)| *h == handle) {
            return Take::Settled(self.settled.remove(i).1);
        }
        if self.rows.iter().any(|row| row.handle == handle) {
            return Take::Pending;
        }
        Take::Settled(Outcome::Aborted(AbortReason::Unknown))
    }

    fn set_freeze(&mut self, paused: bool, held: bool) {
        self.paused = paused;
        self.held = held;
        for row in &mut self.rows {
            row.clock.set_freeze(paused, held);
        }
        self.publish_walking();
    }

    fn reset(&mut self) {
        for mut row in self.rows.drain(..) {
            row.machine.abort(AbortReason::Reset);
            self.settled
                .push((row.handle, Outcome::Aborted(AbortReason::Reset)));
        }
        self.newest.clear();
        self.stepping.clear();
        self.ops.clear();
        self.pending_starts.clear();
        self.pending_decisions.clear();
        self.pending_cancels.clear();
        self.bank_mirrors.clear();
        self.mirror_revokes.clear();
        self.machine_events.clear();
        self.consumed_terminal_seq = 0;
        self.terminal_ack_pending = false;
        self.mirror_revokes_consumed = 0;
        self.publish_walking();
        DEFERRED_BANK_CANCELS.with(|pending| pending.borrow_mut().clear());
    }

    fn stop(&mut self) {
        self.rows.clear();
        self.newest.clear();
        self.stepping.clear();
        self.settled.clear();
        self.ops.clear();
        self.pending_starts.clear();
        self.pending_decisions.clear();
        self.pending_cancels.clear();
        self.bank_mirrors.clear();
        self.mirror_revokes.clear();
        self.machine_events.clear();
        self.terminal_ack_pending = false;
        self.publish_walking();
        self.walking_publication = None;
        DEFERRED_BANK_CANCELS.with(|pending| pending.borrow_mut().clear());
    }
}

fn seat_phase_from_wire(phase: isolate_fb::SeatPhase) -> Result<native::NativePhase, String> {
    match phase {
        isolate_fb::SeatPhase::Preparing => Ok(native::NativePhase::Preparing),
        isolate_fb::SeatPhase::Working => Ok(native::NativePhase::Working),
        isolate_fb::SeatPhase::Waiting => Ok(native::NativePhase::Waiting),
        isolate_fb::SeatPhase::Blocked => Ok(native::NativePhase::Blocked),
        isolate_fb::SeatPhase::Complete => Ok(native::NativePhase::Complete),
        isolate_fb::SeatPhase::None => Err("seat live row has no phase".into()),
        _ => Err("seat live row has unknown phase enum".into()),
    }
}

fn seat_result_outcome(
    row: isolate_fb::SeatResult<'_>,
) -> Result<(RequestId, u64, Outcome), String> {
    let id = RequestId::from_raw(row.id())
        .ok_or_else(|| format!("seat result has invalid identity {}", row.id()))?;
    let outcome = row
        .outcome()
        .ok_or_else(|| format!("seat result {} has no outcome", id.get()))?;
    let value = outcome.value();
    let result = match outcome.kind() {
        isolate_fb::SeatOutcomeKind::Done => {
            let value = value
                .map(|bytes| serde_json::from_slice(bytes.bytes()))
                .transpose()
                .map_err(|error| format!("seat result {} value: {error}", id.get()))?
                .unwrap_or(Value::Null);
            Outcome::Done(value)
        }
        isolate_fb::SeatOutcomeKind::Failed => {
            let message = value
                .map(|bytes| serde_json::from_slice::<Value>(bytes.bytes()))
                .transpose()
                .map_err(|error| format!("seat result {} failure value: {error}", id.get()))?
                .map(|value| match value {
                    Value::String(message) => message,
                    value => value.to_string(),
                })
                .unwrap_or_else(|| "native seat failed".into());
            Outcome::Failed(Thrown::new(message))
        }
        isolate_fb::SeatOutcomeKind::Refused => {
            let reason = match outcome.refusal() {
                isolate_fb::SeatRefusal::Busy => "busy",
                isolate_fb::SeatRefusal::ParentBusy => "parent-busy",
                isolate_fb::SeatRefusal::ParentNotLive => "parent-not-live",
                isolate_fb::SeatRefusal::LineageDepth => "lineage-depth",
                isolate_fb::SeatRefusal::HistoryExpired => "history-expired",
                isolate_fb::SeatRefusal::Unknown => "unknown",
                isolate_fb::SeatRefusal::AwaitingDecision => "awaiting-decision",
                isolate_fb::SeatRefusal::None | isolate_fb::SeatRefusal::ArgsTooLarge => {
                    return Err(format!("seat result {} has invalid refusal", id.get()));
                }
                _ => return Err(format!("seat result {} has unknown refusal", id.get())),
            };
            Outcome::Refused(reason.into())
        }
        isolate_fb::SeatOutcomeKind::Aborted => {
            let reason = match outcome.abort_reason() {
                isolate_fb::SeatAbortReason::Superseded => AbortReason::Superseded,
                isolate_fb::SeatAbortReason::UserInput => AbortReason::UserInput,
                isolate_fb::SeatAbortReason::ParentAborted => AbortReason::ParentAborted,
                isolate_fb::SeatAbortReason::None
                | isolate_fb::SeatAbortReason::Reset
                | isolate_fb::SeatAbortReason::Terminated => {
                    return Err(format!("seat result {} has invalid abort reason", id.get()));
                }
                _ => return Err(format!("seat result {} has unknown abort reason", id.get())),
            };
            Outcome::Aborted(reason)
        }
        isolate_fb::SeatOutcomeKind::None => {
            return Err(format!("seat result {} has no outcome kind", id.get()));
        }
        _ => return Err(format!("seat result {} has unknown outcome kind", id.get())),
    };
    Ok((id, row.terminal_seq(), result))
}

fn settle_seat_result(host: &mut Host, id: RequestId, outcome: Outcome) {
    let orphaned = if let Some(mirror) = host.bank_mirrors.iter_mut().find(|row| row.id == id) {
        mirror.prompt = None;
        mirror.terminal = Some(outcome.clone());
        mirror.orphaned
    } else {
        false
    };
    if orphaned {
        host.bank_mirrors.retain(|mirror| mirror.id != id);
        host.pending_decisions.retain(|decision| decision.of != id);
        return;
    }
    let Ok(handle) = RequestId::isolate(id.get()) else {
        return;
    };
    let handle = handle.get();
    let Some(position) = host
        .rows
        .iter()
        .position(|row| row.handle == handle && row.native_kind.is_some())
    else {
        return;
    };
    let native_kind = host.rows[position].native_kind;
    // The seat's Done/Failed is an authority receipt, not the product
    // family's result. A Gather or Combat terminal can reach the seat page
    // before its detailed family page. Leave the machine row pending until
    // that page arrives; an absent page must not settle its JS Promise with
    // the seat's deliberately empty value.
    let product_result = if matches!(&outcome, Outcome::Done(_) | Outcome::Failed(_)) {
        crate::observed::with(|scene| {
            let page = scene.since_login();
            match native_kind {
                Some(native::request::NativeKind::Gather) => page
                    .api_gather_outcome()
                    .filter(|terminal| terminal.request_id == id.get())
                    .map(|terminal| terminal.end.clone().into()),
                Some(native::request::NativeKind::Combat) => page
                    .api_combat_outcome()
                    .filter(|terminal| terminal.request_id == id.get())
                    .map(|terminal| terminal.end.clone().into()),
                Some(native::request::NativeKind::QuestProgress) => page
                    .api_progress()
                    .filter(|terminal| {
                        terminal.token() == id.get()
                            && !matches!(
                                terminal,
                                crate::api_progress::ProgressPage::Reading { .. }
                            )
                    })
                    .cloned()
                    .map(Into::into),
                _ => None,
            }
        })
    } else {
        None
    };
    if matches!(
        native_kind,
        Some(native::request::NativeKind::Gather | native::request::NativeKind::Combat)
    ) && matches!(&outcome, Outcome::Done(_) | Outcome::Failed(_))
        && product_result.is_none()
    {
        return;
    }
    let mut row = host.rows.remove(position);
    host.bank_mirrors.retain(|mirror| mirror.id != id);
    host.pending_decisions
        .retain(|decision| decision.of != id);
    let outcome = product_result.map_or(outcome, Outcome::Done);
    let abort_reason = match &outcome {
        Outcome::Aborted(reason) => Some(*reason),
        _ => None,
    };
    if abort_reason.is_some() || matches!(&outcome, Outcome::Refused(_)) {
        row.authority_revoked = true;
        host.remember_revoked(&row);
        host.revoke_descendants(row.handle, 0);
    }
    let cleanup_reason = abort_reason.unwrap_or(AbortReason::Reset);
    row.machine.abort(cleanup_reason);
    host.ops
        .retain(|op| op.lineage.lineage != Some(id) && op.lineage.mirror_owner != Some(id));
    host.free_completed_walk(row.handle);
    host.settled.push((row.handle, outcome));
}

pub(crate) fn set_execution(execution: Option<ExecutionId>) {
    HOST.with(|host| host.borrow_mut().execution = execution);
}

pub(crate) fn has_pending_seat_batch() -> bool {
    flush_deferred_bank_cancels();
    HOST.with(|host| {
        let host = host.borrow();
        host.execution.is_some()
            && (!host.pending_starts.is_empty()
                || !host.pending_decisions.is_empty()
                || !host.pending_cancels.is_empty()
                || !host.mirror_revokes.is_empty()
                || host.machine_events.iter().any(|event| event.admitted)
                || host.terminal_ack_pending)
    })
}

pub(crate) fn with_seat_batch<R>(f: impl FnOnce(Option<SeatBatchCarry<'_>>) -> R) -> R {
    flush_deferred_bank_cancels();
    HOST.with(|host| {
        let host = host.borrow();
        let Some(execution) = host.execution else {
            return f(None);
        };
        let starts = host
            .pending_starts
            .iter()
            .map(|start| start.as_ref(execution))
            .collect();
        let decisions = host
            .pending_decisions
            .iter()
            .map(|decision| isolate_fb::SeatDecisionRef {
                of: decision.of,
                sequence: decision.sequence,
                answer: &decision.answer,
            })
            .collect();
        let cancels = host
            .pending_cancels
            .iter()
            .map(|cancel| isolate_fb::SeatCancelInput {
                of: cancel.of,
                reason: cancel.reason,
            })
            .collect();
        let mirror_revokes = host
            .mirror_revokes
            .iter()
            .map(|revoke| revoke.wire)
            .collect();
        f(Some(SeatBatchCarry {
            consumed_terminal_seq: host.consumed_terminal_seq,
            terminal_ack_pending: host.terminal_ack_pending,
            starts,
            decisions,
            cancels,
            mirror_revokes,
            machine_events: machine_events_for_batch(&host),
        }))
    })
}

fn machine_events_for_batch(host: &Host) -> Vec<MachineEvent> {
    host.machine_events
        .iter()
        .filter(|event| event.admitted)
        .copied()
        .collect()
}

fn reconcile_machine_events(
    host: &mut Host,
    live_rows: &[(RequestId, native::NativePhase)],
    result_ids: &[RequestId],
) {
    for event in &mut host.machine_events {
        if live_rows.iter().any(|(id, _)| *id == event.id) {
            event.admitted = true;
        }
    }
    host.machine_events.retain(|event| {
        !result_ids.contains(&event.id)
            && !matches!(
                event.kind,
                MachineEventKind::Phase(phase)
                    if live_rows.iter().any(|(id, live_phase)| {
                        *id == event.id && *live_phase == phase
                    })
            )
    });
}
pub(crate) fn on_snapshot(snapshot: &isolate_fb::Snapshot<'_>) -> Result<(), String> {
    flush_deferred_bank_cancels();
    let live = snapshot.seat_live();
    let results = snapshot.seat_results();
    let mut live_rows = Vec::with_capacity(live.as_ref().map_or(0, |rows| rows.len()));
    let mut bank_live_rows = Vec::new();
    if let Some(rows) = live.as_ref() {
        for row in rows.iter() {
            let id = RequestId::from_raw(row.id())
                .ok_or_else(|| format!("seat live row has invalid identity {}", row.id()))?;
            let phase = seat_phase_from_wire(row.phase())?;
            live_rows.push((id, phase));
            if HOST.with(|host| {
                host.borrow()
                    .bank_mirrors
                    .iter()
                    .any(|mirror| mirror.id == id)
            }) {
                let prompt = row
                    .prompt()
                    .map(|prompt| {
                        let sequence = DecisionSequence::from_raw(prompt.seq())
                            .ok_or_else(|| format!("bank prompt for {} has zero sequence", id.get()))?;
                        let point = prompt
                            .point()
                            .ok_or_else(|| format!("bank prompt for {} has no point", id.get()))?;
                        let args = prompt
                            .args()
                            .ok_or_else(|| format!("bank prompt for {} has no arguments", id.get()))?;
                        Ok(BankPrompt {
                            sequence,
                            point: point.to_owned(),
                            args: args.bytes().to_vec(),
                        })
                    })
                    .transpose()?;
                bank_live_rows.push((id, row.decisions_consumed(), prompt));
            }
        }
    }
    let has_results_page = results.is_some();
    let mut result_rows = Vec::with_capacity(results.as_ref().map_or(0, |rows| rows.len()));
    if let Some(rows) = results.as_ref() {
        for row in rows.iter() {
            result_rows.push(seat_result_outcome(row)?);
        }
    }
    let mirror_revokes_consumed = snapshot.mirror_revokes_consumed();
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        for (id, phase) in &live_rows {
            if let Some(row) = host.rows.iter_mut().find(|row| {
                row.native_kind.is_some() && RequestId::isolate(row.handle).ok() == Some(*id)
            }) {
                row.admitted = true;
                row.phase = *phase;
            }
        }
        for (id, _, prompt) in &bank_live_rows {
            if let Some(mirror) = host.bank_mirrors.iter_mut().find(|mirror| mirror.id == *id) {
                mirror.admitted = true;
                mirror.prompt = prompt.clone();
            }
        }
        let result_ids: Vec<_> = result_rows.iter().map(|(id, _, _)| *id).collect();
        host.pending_decisions.retain(|decision| {
            !result_ids.contains(&decision.of)
                && !bank_live_rows.iter().any(|(id, consumed, _)| {
                    decision.of == *id && decision.sequence.get() <= *consumed
                })
        });
        host.pending_starts.retain(|start| {
            !live_rows.iter().any(|(id, _)| *id == start.id) && !result_ids.contains(&start.id)
        });
        host.pending_cancels
            .retain(|cancel| !result_ids.contains(&cancel.of));
        reconcile_machine_events(&mut host, &live_rows, &result_ids);
        if let Some(consumed) = mirror_revokes_consumed {
            host.mirror_revokes_consumed = host.mirror_revokes_consumed.max(consumed);
            host.mirror_revokes
                .retain(|revoke| revoke.wire.seq > consumed);
        }
        if has_results_page {
            for (_, seq, _) in &result_rows {
                host.consumed_terminal_seq = host.consumed_terminal_seq.max(*seq);
            }
            host.terminal_ack_pending = result_rows
                .iter()
                .any(|(_, seq, _)| *seq <= host.consumed_terminal_seq);
        }
        for (id, _, outcome) in result_rows {
            settle_seat_result(&mut host, id, outcome);
        }
    });
    Ok(())
}

pub(crate) fn capture_push(parent: Option<Handle>) -> CapturedLineage {
    HOST.with(|host| host.borrow().capture_push(parent))
}

fn native_kind_for_family(family: &str) -> Option<native::request::NativeKind> {
    if let Some(descriptor) = bank_family_descriptor(family) {
        return Some(descriptor.kind);
    }
    use native::request::NativeKind;
    Some(match family {
        "gather" => NativeKind::Gather,
        "combat" => NativeKind::Combat,
        "quest-progress" => NativeKind::QuestProgress,
        "hunt-fight" | "hunt-hold" | "hunt-retreat" | "hunt-walkspot" | "hunt-enter" => {
            NativeKind::Hunt
        }
        "periodic_bank" => NativeKind::PeriodicBank,
        _ => return None,
    })
}

fn seat_start_payload(
    kind: native::request::NativeKind,
    id: RequestId,
    ops: &mut Vec<InteractReq>,
) -> Result<SeatStartPayload, String> {
    let position = ops.iter().position(|request| match (kind, request) {
        (native::request::NativeKind::Gather, InteractReq::GatherRun { request_id, .. })
        | (native::request::NativeKind::Combat, InteractReq::CombatFight { request_id, .. })
        | (
            native::request::NativeKind::QuestProgress,
            InteractReq::ProgressRead { request_id, .. },
        ) => *request_id == id.get(),
        _ => false,
    });
    match kind {
        native::request::NativeKind::Gather => {
            let Some(position) = position else {
                return Err("native Gather start is missing its typed settings".into());
            };
            let InteractReq::GatherRun { settings, .. } = ops.remove(position) else {
                unreachable!("matched GatherRun")
            };
            Ok(SeatStartPayload::Gather(settings))
        }
        native::request::NativeKind::Combat => {
            let Some(position) = position else {
                return Err("native Combat start is missing its typed request".into());
            };
            let InteractReq::CombatFight { request, .. } = ops.remove(position) else {
                unreachable!("matched CombatFight")
            };
            Ok(SeatStartPayload::Combat(request))
        }
        native::request::NativeKind::QuestProgress => {
            let Some(position) = position else {
                return Err("native QuestProgress start is missing its quest name".into());
            };
            let InteractReq::ProgressRead { name, .. } = ops.remove(position) else {
                unreachable!("matched ProgressRead")
            };
            Ok(SeatStartPayload::QuestProgress(std::sync::Arc::from(name)))
        }
        _ => Ok(SeatStartPayload::Empty),
    }
}

fn begin_row<F: Family>(handle: Handle, args: Value, hooks: Vec<Hook>, at: usize) -> Started {
    let args: F::Args = match serde_json::from_value(args) {
        Ok(args) => args,
        Err(e) => return Started::Refused(format!("{} arguments: {e}", F::NAME)),
    };
    let hooks = hooks.into_iter().map(Rc::new).collect::<Vec<_>>();
    let context = START_CONTEXT.with(Cell::get).unwrap_or_default();
    let native_owner = HOST.with(|host| {
        host.borrow().execution.is_some() && native_kind_for_family(F::NAME).is_some()
    });
    let row_context = StartContext {
        parent_row: Some(handle),
        seat_parent: if native_owner {
            RequestId::isolate(handle).ok()
        } else {
            context.seat_parent
        },
        local_parent: if native_owner {
            None
        } else {
            RequestId::isolate(handle).ok()
        },
        ..StartContext::default()
    };
    let inherited_baseline = WALK_BASELINE.with(Cell::get);
    let intent_baseline = inherited_baseline.unwrap_or_else(observed_user_move_intent_seq);
    let carries_walk_baseline = inherited_baseline.is_some() || F::WALKING_OPERATION;
    let mut clock = HOST.with(|host| host.borrow().fresh_clock());
    let mut ops = Vec::new();
    IN_BEGIN.with(|flag| flag.set(true));
    let _begun = BeginGuard;
    let begun = with_walk_baseline(carries_walk_baseline.then_some(intent_baseline), || {
        F::begin(
            args,
            &mut Cx {
                ops: &mut ops,
                clock: &mut clock,
                reply: None,
                hooks: &hooks,
                handle,
                context: row_context,
                js: None,
                intent_baseline,
                asks: 0,
                ending: None,
                callback_path: None,
            },
        )
    });
    drop(_begun);
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        match begun {
            Begin::Refuse(reason) => Started::Refused(reason),
            Begin::Done(out) => {
                host.place(
                    at,
                    ops,
                    CapturedLineage {
                        lineage: context.seat_parent,
                        mirror_owner: context.local_parent,
                        dropped: false,
                    },
                );
                Started::Settled(Outcome::Done(out.into()))
            }
            Begin::Run(mut machine) => {
                let native_kind = host
                    .execution
                    .is_some()
                    .then(|| native_kind_for_family(F::NAME))
                    .flatten();
                let owner_id = native_kind.and_then(|_| RequestId::isolate(handle).ok());
                if native_kind.is_some() && host.pending_starts.len() >= MAX_PENDING_SEAT_STARTS {
                    machine.abort(AbortReason::Reset);
                    return Started::Refused("native seat start carry is full".into());
                }
                let lineage = if let Some(kind) = native_kind {
                    let Some(id) = owner_id else {
                        machine.abort(AbortReason::Reset);
                        return Started::Refused("native seat request identity exhausted".into());
                    };
                    let payload = match seat_start_payload(kind, id, &mut ops) {
                        Ok(payload) => payload,
                        Err(reason) => {
                            machine.abort(AbortReason::Reset);
                            return Started::Refused(reason);
                        }
                    };
                    host.pending_starts.push(PendingSeatStart {
                        id,
                        parent: context.seat_parent,
                        local_parent: context.local_parent,
                        kind,
                        args: payload,
                    });
                    CapturedLineage {
                        lineage: Some(id),
                        mirror_owner: None,
                        dropped: false,
                    }
                } else {
                    CapturedLineage {
                        lineage: context.seat_parent,
                        mirror_owner: context.local_parent,
                        dropped: false,
                    }
                };
                if F::EXCLUSIVE {
                    host.newest
                        .retain(|(group, _)| *group != F::EXCLUSIVE_GROUP);
                    host.newest.push((F::EXCLUSIVE_GROUP, handle));
                    host.abort_superseded(at);
                }
                host.place(at, ops, lineage);
                let walking_operation = machine.walking_operation();
                host.rows.push(Row {
                    handle,
                    family: F::NAME,
                    exclusive: F::EXCLUSIVE.then_some(F::EXCLUSIVE_GROUP),
                    sync_hooks: F::SYNC_HOOKS,
                    clock,
                    hooks,
                    calls: 0,
                    last_snapshot: None,
                    reply: None,
                    pending: None,
                    machine: Box::new(machine),
                    seat_parent: context.seat_parent,
                    parent_row: context.parent_row,
                    native_kind,
                    admitted: native_kind.is_none(),
                    awaiting_seat_terminal: false,
                    authority_revoked: false,
                    phase: native::NativePhase::Preparing,
                    manual_walk_event_sent: false,
                    intent_baseline,
                    carries_walk_baseline,
                    walking_operation,
                    rebaseline_on_resume: false,
                });
                host.publish_walking();
                Started::Running(handle)
            }
        }
    })
}

/// Clears [`IN_BEGIN`] however the begin returns.
struct BeginGuard;

impl Drop for BeginGuard {
    fn drop(&mut self) {
        IN_BEGIN.with(|flag| flag.set(false));
    }
}

#[cfg(test)]
pub(crate) fn start(family: &str, args: Value, hooks: Vec<Hook>, at: usize) -> Started {
    start_with_parent(family, args, hooks, at, None)
}

fn start_bank_family(
    family: &'static str,
    value: Value,
    hooks: Vec<Hook>,
    at: usize,
    parent: Option<Handle>,
) -> Started {
    flush_deferred_bank_cancels();
    let Some(descriptor) = bank_family_descriptor(family) else {
        return Started::Refused(format!("unknown bank family {family:?}"));
    };
    let context = HOST.with(|host| host.borrow().start_context_for(parent));
    if context.history_expired {
        return Started::Refused("history expired".into());
    }
    if context.parent_revoked {
        return Started::Refused("parent not live".into());
    }
    if !HOST.with(|host| host.borrow().execution.is_some()) {
        return Started::Refused("native bank seat requires an active execution".into());
    }
    match bank_op_immediate_outcome(family, &value) {
        Ok(Some(outcome)) => return Started::Settled(outcome),
        Ok(None) => {}
        Err(error) => return Started::Refused(error),
    }
    let mut args = match parse_bank_start(family, value) {
        Ok(args) => args,
        Err(error) => return Started::Refused(error),
    };
    if bank_family_for_operation(args.operation) != family {
        return Started::Refused(format!(
            "bank operation {:?} does not belong to {family}",
            args.operation
        ));
    }
    if let Err(error) = args.validate(descriptor.kind) {
        return Started::Refused(error);
    }
    if family == "bank_deposit"
        && HOST.with(|host| {
            host.borrow().bank_mirrors.iter().any(|mirror| {
                mirror.family == family && mirror.terminal.is_none()
            })
        })
    {
        return Started::Settled(Outcome::Done(Value::Null));
    }
    if hooks.len() != descriptor.callbacks.len() {
        return Started::Refused(format!(
            "{family} callback count does not match its descriptor"
        ));
    }
    let mut hook_map = [("", 0usize); 3];
    if descriptor.callbacks.len() > hook_map.len() {
        return Started::Refused(format!("{family} has too many callback members"));
    }
    for (index, callback) in descriptor.callbacks.iter().enumerate() {
        hook_map[index] = (*callback, index);
    }
    let hook_map = &hook_map[..descriptor.callbacks.len()];
    let (hook_indices, hooks_present) = match bank_hook_indices(
        descriptor,
        args.operation,
        hook_map,
        hooks.len(),
        |index| hooks[index].present,
    ) {
        Ok(result) => result,
        Err(error) => return Started::Refused(error),
    };
    if let Err(error) = require_bank_hooks(args.operation, hooks_present) {
        return Started::Refused(error);
    }
    args.hooks_present = hooks_present;
    if let Err(error) = args.validate(descriptor.kind) {
        return Started::Refused(error);
    }
    if HOST.with(|host| host.borrow().pending_starts.len() >= MAX_PENDING_SEAT_STARTS) {
        return Started::Refused("native seat start carry is full".into());
    }
    let Some(handle) = HOST.with(|host| host.borrow_mut().allocate_handle()) else {
        return Started::Refused("machine handle exhausted".into());
    };
    let id = match RequestId::isolate(handle) {
        Ok(id) => id,
        Err(_) => return Started::Refused("native bank seat identity exhausted".into()),
    };
    if let Err(error) = enqueue_bank_child(id, family, args, context) {
        return Started::Refused(error);
    }
    let hooks = hooks.into_iter().map(Rc::new).collect();
    let mut host = HOST.with(|host| host.borrow_mut());
    if let Some(group) = descriptor.exclusive_group {
        host.newest.retain(|(newest, _)| *newest != group);
        host.newest.push((group, handle));
        host.abort_superseded(at);
    }
    let clock = host.fresh_clock();
    let inherited_baseline = WALK_BASELINE.with(Cell::get);
    let intent_baseline = inherited_baseline.unwrap_or_else(observed_user_move_intent_seq);
    let carries_walk_baseline = inherited_baseline.is_some();
    host.rows.push(Row {
        handle,
        family,
        exclusive: descriptor.exclusive_group,
        sync_hooks: &[],
        clock,
        hooks,
        calls: 0,
        last_snapshot: None,
        reply: None,
        pending: None,
        machine: Box::new(BankSeatMachine(NativeBankChild {
            id,
            family,
            hook_indices,
            seen_prompt: 0,
            awaiting_reply: None,
            cancel_on_drop: false,
            finished: false,
        })),
        seat_parent: context.seat_parent,
        parent_row: context.parent_row,
        native_kind: Some(descriptor.kind),
        admitted: false,
        awaiting_seat_terminal: false,
        authority_revoked: false,
        phase: native::NativePhase::Preparing,
        manual_walk_event_sent: false,
        intent_baseline,
        carries_walk_baseline,
        walking_operation: false,
        rebaseline_on_resume: false,
    });
    host.publish_walking();
    Started::Running(handle)
}

struct BankSeatMachine(NativeBankChild);

impl Machine for BankSeatMachine {
    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
        self.0.step(cx)
    }

    fn snapshot_sensitive(&self) -> bool {
        true
    }

    fn snapshot_step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
        self.0.step(cx)
    }

    fn abort(&mut self, _why: AbortReason) {}

    fn release(&self) -> Option<InteractReq> {
        None
    }

    fn walking_operation(&self) -> bool {
        false
    }
}

pub(crate) fn start_with_parent(
    family: &str,
    args: Value,
    hooks: Vec<Hook>,
    at: usize,
    parent: Option<Handle>,
) -> Started {
    if IN_BEGIN.with(Cell::get) {
        return Started::Refused("a machine begin may not start a machine".into());
    }
    if let Some(descriptor) = bank_family_descriptor(family) {
        return start_bank_family(descriptor.name, args, hooks, at, parent);
    }
    let Some(entry) = FAMILIES.iter().find(|entry| entry.name == family) else {
        return Started::Refused(format!("unknown machine family {family:?}"));
    };
    let context = HOST.with(|host| host.borrow().start_context_for(parent));
    if context.history_expired {
        return Started::Refused("history expired".into());
    }
    if context.parent_revoked {
        return Started::Refused("parent not live".into());
    }
    let needs_seat = HOST.with(|host| {
        let host = host.borrow();
        host.execution.is_some() && native_kind_for_family(entry.name).is_some()
    });
    if needs_seat && HOST.with(|host| host.borrow().pending_starts.len() >= MAX_PENDING_SEAT_STARTS)
    {
        return Started::Refused("native seat start carry is full".into());
    }
    let Some(handle) = HOST.with(|host| host.borrow_mut().allocate_handle()) else {
        return Started::Refused("machine handle exhausted".into());
    };
    if needs_seat && RequestId::isolate(handle).is_err() {
        return Started::Refused("native seat request identity exhausted".into());
    }
    with_start_context(context, || (entry.begin)(handle, args, hooks, at))
}

/// Drive one live row once (the starting tick for [`Family::KICK_ON_START`]).
pub(crate) fn kick(handle: Handle, js: &mut impl Js) {
    TERMINATED.with(|flag| flag.set(false));
    let mut at = js.queue_len();
    let Some(mut row) = HOST.with(|host| {
        let mut host = host.borrow_mut();
        host.rows
            .iter()
            .position(|row| row.handle == handle)
            .map(|index| host.rows.remove(index))
    }) else {
        return;
    };
    if row.native_kind.is_some() && !row.admitted {
        HOST.with(|host| host.borrow_mut().rows.push(row));
        return;
    }
    let stepping = row.stepping_record();
    HOST.with(|host| host.borrow_mut().stepping.push(stepping));
    let outcome = drive(&mut row, js, &mut at);
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        match outcome {
            // Termination unwinds runMachine before it can park this result.
            Some(Outcome::Aborted(AbortReason::Terminated)) => {}
            Some(outcome) => host.finish_row(&row, outcome, at),
            None => host.rows.push(row),
        }
        host.unstep(stepping.family, stepping.handle);
    });
}

/// Step every live row once. Called at the start of each eligible tick,
/// after the tick number is recorded.
pub(crate) fn step(js: &mut impl Js) {
    pass(js, Pass::Step);
}

/// Drive the rows waiting on a callback promise, if it settled. Called
/// once per eligible tick, after the pump and its microtasks.
pub(crate) fn resume(js: &mut impl Js) {
    pass(js, Pass::Resume);
}

/// Recheck opted-in rows against one changed snapshot without starting a
/// new tick's callback or per-family event budget.
pub(crate) fn snapshot_step(js: &mut impl Js, tick: u64, evidence_sequence: u64) {
    pass(
        js,
        Pass::Snapshot {
            tick,
            evidence_sequence,
        },
    );
}

/// Whether an outcome waits for its JS await.
pub(crate) fn any_settled() -> bool {
    HOST.with(|host| !host.borrow().settled.is_empty())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pass {
    /// Every row; starts the tick's callback budget.
    Step,
    /// Rows waiting on a promise; the budget carries over.
    Resume,
    /// Opted-in rows, once per distinct changed snapshot; carries the budget.
    Snapshot { tick: u64, evidence_sequence: u64 },
}

/// Rows are taken out of the host while they step, so a callback may
/// start another machine; the host is borrowed only between JS calls.
/// [`Host::stepping`] keeps those rows visible to [`live`].
fn pass(js: &mut impl Js, pass: Pass) {
    let entered_scene = crate::observed::with(crate::observed::Scene::entered_scene);
    let Some(mut rows) = HOST.with(|host| {
        let mut host = host.borrow_mut();
        if host.paused || host.held {
            return None;
        }
        let rows = std::mem::take(&mut host.rows);
        host.stepping = rows.iter().map(Row::stepping_record).collect();
        Some(rows)
    }) else {
        return;
    };
    TERMINATED.with(|flag| flag.set(false));
    // Script code may have queued rows before this pass (a probe, the
    // recovery anchor, this tick's own JS); step ops land after them.
    let mut at = js.queue_len();
    rows.retain_mut(|row| {
        if row.awaiting_seat_terminal {
            return true;
        }
        if row.native_kind.is_some() && !row.admitted {
            return true;
        }
        match pass {
            Pass::Step => row.calls = 0,
            Pass::Resume if row.pending.is_none() => return true,
            Pass::Resume => {}
            Pass::Snapshot {
                tick,
                evidence_sequence,
            } => {
                if !row.machine.snapshot_sensitive() || row.pending.is_some() {
                    return true;
                }
                let snapshot = (tick, evidence_sequence);
                if row.last_snapshot == Some(snapshot) {
                    return true;
                }
                row.last_snapshot = Some(snapshot);
            }
        }
        if row.native_kind.is_none() && HOST.with(|host| host.borrow().superseded(row)) {
            if let Some(op) = release_op(&*row.machine) {
                HOST.with(|host| host.borrow_mut().place(at, vec![op], row.capture()));
            }
            row.machine.abort(AbortReason::Superseded);
            HOST.with(|host| host.borrow_mut().unstep(row.family, row.handle));
            settle(row, Outcome::Aborted(AbortReason::Superseded), at);
            return false;
        }
        if HOST.with(|host| host.borrow().is_revoked(row.handle)) {
            row.machine.abort(AbortReason::ParentAborted);
            HOST.with(|host| host.borrow_mut().unstep(row.family, row.handle));
            settle(row, Outcome::Aborted(AbortReason::ParentAborted), at);
            return false;
        }
        // PLAYER_INFO can expose the new level before that scene's loc
        // changes. Opted-in rows must keep that fence on every pass, including
        // promise resumes and receipt re-observes, without delaying revocation.
        if entered_scene && row.machine.snapshot_sensitive() {
            return true;
        }
        let outcome = match pass {
            Pass::Snapshot { .. } => drive_snapshot(row, js, &mut at),
            Pass::Step | Pass::Resume => drive(row, js, &mut at),
        };
        match outcome {
            Some(outcome) => {
                HOST.with(|host| host.borrow_mut().unstep(row.family, row.handle));
                settle(row, outcome, at);
                false
            }
            None => true,
        }
    });
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        host.stepping.clear();
        let started = std::mem::replace(&mut host.rows, rows);
        host.rows.extend(started);
        host.abort_superseded(at);
        host.publish_walking();
    });
}

fn settle(row: &Row, outcome: Outcome, at: usize) {
    HOST.with(|host| host.borrow_mut().finish_row(row, outcome, at));
}

/// One row's steps for this pass; `Some` when it ended. Stops, leaving
/// the row as it is, once join has claimed the tick.
fn drive(row: &mut Row, js: &mut impl Js, at: &mut usize) -> Option<Outcome> {
    drive_in_pass(row, js, at, false)
}

fn drive_snapshot(row: &mut Row, js: &mut impl Js, at: &mut usize) -> Option<Outcome> {
    drive_in_pass(row, js, at, true)
}

fn drive_in_pass(
    row: &mut Row,
    js: &mut impl Js,
    at: &mut usize,
    snapshot: bool,
) -> Option<Outcome> {
    if row.rebaseline_on_resume {
        // Pause and reconnect holds produce no runnable observations. Refresh
        // on the first eligible resumed step, after its Running snapshot.
        // An actual post-hold takeover (including its early atomic notice)
        // must never be absorbed into the carried operation's baseline.
        if !SESSION_HELD.with(Cell::get)
            && HOST.with(|host| host.borrow().takeover_seq()) <= row.intent_baseline
        {
            row.intent_baseline = observed_user_move_intent_seq();
        }
        row.rebaseline_on_resume = false;
    }
    let baseline = row.carries_walk_baseline.then_some(row.intent_baseline);
    with_walk_baseline(baseline, || drive_inner(row, js, at, snapshot))
}

fn drive_inner(row: &mut Row, js: &mut impl Js, at: &mut usize, snapshot: bool) -> Option<Outcome> {
    loop {
        if js.claimed() || TERMINATED.with(Cell::get) {
            return None;
        }
        if let Some(pending) = &row.pending {
            row.reply = Some(js.poll(pending)?);
            row.pending = None;
        }
        if row.calls == CALLS_PER_TICK {
            return None;
        }
        let mut ops = Vec::new();
        let mut cx = Cx {
            ops: &mut ops,
            clock: &mut row.clock,
            reply: row.reply.take(),
            hooks: &row.hooks,
            handle: row.handle,
            context: row.start_context(),
            js: Some(&mut *js),
            asks: 0,
            intent_baseline: row.intent_baseline,
            ending: None,
            callback_path: None,
        };
        // Callback continuations belong to the same pass: a snapshot-only
        // pass must not fall back to the normal tick step after a JS call.
        let step = if snapshot {
            row.machine.snapshot_step(&mut cx)
        } else {
            row.machine.step(&mut cx)
        };
        let callback_path = cx.callback_path.take();
        let asked = cx.asks > 0;
        // An ask that failed ends the row whatever the step returned; the
        // ops of that step are dropped with it.
        // Every ending stops the host work the row armed.
        match cx.ending.take() {
            Some(Ending::Stopped) => {
                if let Some(op) = release_op(&*row.machine) {
                    HOST.with(|host| host.borrow_mut().place(*at, vec![op], row.capture()));
                }
                row.machine.abort(AbortReason::Terminated);
                return Some(Outcome::Aborted(AbortReason::Terminated));
            }
            Some(Ending::Failed(thrown)) => {
                if let Some(op) = release_op(&*row.machine) {
                    HOST.with(|host| host.borrow_mut().place(*at, vec![op], row.capture()));
                }
                return Some(Outcome::Failed(thrown));
            }
            None => {}
        }
        // An ask started a newer row of this exclusive family: that row
        // owns the tick, so this step's decision and ops are dropped.
        if asked && HOST.with(|host| host.borrow().superseded(row)) {
            if let Some(op) = release_op(&*row.machine) {
                HOST.with(|host| host.borrow_mut().place(*at, vec![op], row.capture()));
            }
            row.machine.abort(AbortReason::Superseded);
            return Some(Outcome::Aborted(AbortReason::Superseded));
        }
        HOST.with(|host| host.borrow_mut().place(*at, ops, row.capture()));
        match step {
            Step::Wait => {
                if row.native_kind.is_some() {
                    HOST.with(|host| {
                        host.borrow_mut()
                            .set_phase(row, native::NativePhase::Waiting);
                    });
                }
                return None;
            }
            Step::Done(value) => return Some(Outcome::Done(value)),
            Step::Fail(thrown) => {
                if let Some(op) = release_op(&*row.machine) {
                    HOST.with(|host| host.borrow_mut().place(*at, vec![op], row.capture()));
                }
                return Some(Outcome::Failed(thrown));
            }
            Step::Call(call) => {
                let hook = row.hooks.get(call.hook).map(|hook| &hook.callback);
                let called = js.call_for(row.handle, hook, &call.args);
                let synchronous = callback_path.map_or_else(
                    || row.sync_hooks.contains(&call.hook),
                    |path| path == native::abi::HookPath::CallSync,
                );
                row.calls += 1;
                *at = js.queue_len();
                match called {
                    Called::Settled(reply) => {
                        row.reply = Some(reply);
                        if row.native_kind.is_some() {
                            HOST.with(|host| {
                                host.borrow_mut()
                                    .set_phase(row, native::NativePhase::Working);
                            });
                        }
                    }
                    Called::Pending(pending) if !synchronous => {
                        row.pending = Some(pending);
                        if row.native_kind.is_some() {
                            HOST.with(|host| {
                                host.borrow_mut()
                                    .set_phase(row, native::NativePhase::Waiting);
                            });
                        }
                    }
                    // A frozen synchronous call sees the promise object.
                    Called::Pending(_) => {
                        row.reply = Some(Reply::Value(Value::Object(Default::default())));
                        if row.native_kind.is_some() {
                            HOST.with(|host| {
                                host.borrow_mut()
                                    .set_phase(row, native::NativePhase::Working);
                            });
                        }
                    }
                    Called::Terminated => {
                        TERMINATED.with(|flag| flag.set(true));
                        row.machine.abort(AbortReason::Terminated);
                        return Some(Outcome::Aborted(AbortReason::Terminated));
                    }
                }
                // Local exclusivity still supersedes in the callback turn;
                // native replacement waits for the seat's projected admission.
                if row.native_kind.is_none() && HOST.with(|host| host.borrow().superseded(row)) {
                    row.machine.abort(AbortReason::Superseded);
                    return Some(Outcome::Aborted(AbortReason::Superseded));
                }
            }
        }
    }
}

/// The JS await helper's poll: an outcome is handed out exactly once.
pub(crate) fn take(handle: Handle) -> Take {
    HOST.with(|host| host.borrow_mut().take(handle))
}

/// Merge this tick's machine ops into the drained JS rows in emit order.
pub(crate) fn merge_ops(rows: Vec<MaybeInteractReq>) -> Vec<InteractReq> {
    let mut ops = HOST.with(|host| std::mem::take(&mut host.borrow_mut().ops));
    ops.sort_by_key(|op| op.at);
    let mut out = Vec::with_capacity(rows.len() + ops.len());
    let mut ops = ops.into_iter().peekable();
    for (index, row) in rows.into_iter().enumerate() {
        while ops.peek().is_some_and(|op| op.at <= index) {
            out.push(ops.next().expect("peeked placed op").request);
        }
        if let MaybeInteractReq::Req(request) = row {
            out.push(request);
        }
    }
    out.extend(ops.map(|op| op.request));
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        for request in &out {
            if let InteractReq::AbortWalk { request_id } = request {
                host.mark_walk_release_encoded(*request_id);
            }
        }
    });
    out
}

fn survives_revocation(request: &InteractReq, lineage: CapturedLineage) -> bool {
    !lineage.dropped
        || !request.is_game()
        || matches!(
            request,
            InteractReq::AbortWalk { .. }
                | InteractReq::GatherStop { .. }
                | InteractReq::CombatStop { .. }
        )
}

pub(crate) fn merge_ops_with_lineage(
    rows: Vec<MaybeInteractReq>,
    lineages: Vec<CapturedLineage>,
) -> Result<Vec<(InteractReq, CapturedLineage)>, String> {
    if rows.len() != lineages.len() {
        return Err(format!(
            "interact lineage count {} does not match row count {}",
            lineages.len(),
            rows.len()
        ));
    }
    let mut ops = HOST.with(|host| std::mem::take(&mut host.borrow_mut().ops));
    ops.sort_by_key(|op| op.at);
    let mut out = Vec::with_capacity(rows.len() + ops.len());
    let mut ops = ops.into_iter().peekable();
    for (index, (row, lineage)) in rows.into_iter().zip(lineages).enumerate() {
        while ops.peek().is_some_and(|op| op.at <= index) {
            let op = ops.next().expect("peeked placed op");
            if survives_revocation(&op.request, op.lineage) {
                out.push((op.request, op.lineage));
            }
        }
        if let MaybeInteractReq::Req(request) = row {
            if survives_revocation(&request, lineage) {
                out.push((request, lineage));
            }
        }
    }
    out.extend(
        ops.filter(|op| survives_revocation(&op.request, op.lineage))
            .map(|op| (op.request, op.lineage)),
    );
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        for (request, _) in &out {
            if let InteractReq::AbortWalk { request_id } = request {
                host.mark_walk_release_encoded(*request_id);
            }
        }
    });
    Ok(out)
}

/// A held tick drops its public actions; machine ops go with them.
pub(crate) fn drop_ops() {
    HOST.with(|host| host.borrow_mut().ops.clear());
}

pub(crate) fn on_pause() {
    let intent = user_move_intent_seq();
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        for row in &mut host.rows {
            row.rebaseline_on_resume = row.carries_walk_baseline && intent <= row.intent_baseline;
        }
        let held = host.held;
        host.set_freeze(true, held);
    });
}

pub(crate) fn on_resume() {
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        let held = host.held;
        host.set_freeze(false, held);
    });
}

/// Record the queued intent before the host applies the corresponding pause.
pub(crate) fn on_manual_walk_takeover(seq: u64) {
    TAKEOVER_SEQ.with(|watermark| watermark.set(watermark.get().max(seq)));
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        let mut index = 0;
        while index < host.rows.len() {
            let event = {
                let row = &mut host.rows[index];
                if row.carries_walk_baseline && row.intent_baseline < seq {
                    row.rebaseline_on_resume = false;
                    if row.walking_operation && !row.manual_walk_event_sent {
                        row.manual_walk_event_sent = true;
                        row.authority_revoked = true;
                        Some((
                            row.handle,
                            RequestId::isolate(row.handle).ok(),
                            row.native_kind.is_some(),
                            row.admitted,
                        ))
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            if let Some((handle, id, native, admitted)) = event {
                if let Some(id) = id {
                    for op in &mut host.ops {
                        if op.lineage.lineage == Some(id) || op.lineage.mirror_owner == Some(id) {
                            op.lineage.dropped = true;
                        }
                    }
                    if native {
                        host.queue_machine_event(id, MachineEventKind::ManualWalk, admitted);
                    } else {
                        host.queue_mirror_revoke(id);
                    }
                }
                host.revoke_descendants(handle, 0);
                index = host
                    .rows
                    .iter()
                    .position(|row| row.handle == handle)
                    .expect("manual walking owner remains live")
                    + 1;
            } else {
                index += 1;
            }
        }
        host.publish_walking();
    });
}

pub(crate) fn on_hold(held: bool) {
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        let paused = host.paused;
        host.set_freeze(paused, held);
    });
}

pub(crate) fn on_reset() {
    SESSION_HELD.with(|flag| flag.set(false));
    TAKEOVER_SEQ.with(|seq| seq.set(0));
    HOST.with(|host| host.borrow_mut().reset());
}

/// A reconnect began (`true`) or the relogged session's first tick ended
/// it (`false`). While held, the connection's queue was reset and the host
/// carry owns the held walk: [`Family::release`] ops are not sent.
pub(crate) fn on_session_hold(held: bool) {
    SESSION_HELD.with(|flag| flag.set(held));
    HOST.with(|host| host.borrow().publish_walking());
}

/// The row's [`Family::release`] op, unless a reconnect holds the session.
fn release_op(machine: &dyn Machine) -> Option<InteractReq> {
    if SESSION_HELD.with(Cell::get) {
        return None;
    }
    machine.release()
}

pub(crate) fn on_stop() {
    SESSION_HELD.with(|flag| flag.set(false));
    TAKEOVER_SEQ.with(|seq| seq.set(0));
    HOST.with(|host| host.borrow_mut().stop());
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn bank_op_withdraw_preserves_frozen_numeric_and_string_tiers() {
        use crate::bank::request::BankSeatOperation;

        let parse = |amount: Value| {
            parse_bank_start(
                "bank_op",
                json!({ "kind": "withdraw", "name": "Feather", "amount": amount }),
            )
            .unwrap()
        };
        for (amount, threshold) in [
            (json!(12), 12),
            (json!(11), 11),
            (json!(10.9), 10),
            (json!(4), 1),
            (json!(null), 1),
            (json!(true), 1),
        ] {
            let args = parse(amount);
            assert_eq!(args.operation, BankSeatOperation::WithdrawLabel);
            assert_eq!(args.item_name.as_deref(), Some("Feather"));
            assert_eq!(args.menu_op, None);
            assert_eq!(args.quantity, Some(threshold));
            args.validate(native::request::NativeKind::BankPlan)
                .unwrap();
        }
        let args = parse(json!(9_007_199_254_741_000u64));
        assert_eq!(args.quantity, Some(9_007_199_254_740_991));

        for (amount, label) in [
            (json!("all"), "Withdraw All"),
            (json!("ALL"), "Withdraw All"),
            (json!("Withdraw 5"), "Withdraw 5"),
        ] {
            let args = parse(amount);
            assert_eq!(args.operation, BankSeatOperation::WithdrawLabel);
            assert_eq!(args.menu_op.as_deref(), Some(label));
            assert_eq!(args.quantity, None);
            args.validate(native::request::NativeKind::BankPlan)
                .unwrap();
        }
        for (op, action) in [
            ("all", "Withdraw All"),
            ("withdraw-all", "Withdraw All"),
            ("withdraw-5", "withdraw 5"),
            ("Withdraw 5", "Withdraw 5"),
        ] {
            let args = parse_bank_start(
                "bank_op",
                json!({ "kind": "withdraw-id", "id": 314, "op": op }),
            )
            .unwrap();
            assert_eq!(args.operation, BankSeatOperation::WithdrawLabel);
            assert_eq!(args.item_id, Some(314));
            assert_eq!(args.menu_op.as_deref(), Some(action));
            assert_eq!(args.quantity, None);
            args.validate(native::request::NativeKind::BankPlan)
                .unwrap();
        }
        let absent = parse_bank_start(
            "bank_op",
            json!({ "kind": "withdraw", "name": "Feather" }),
        )
        .unwrap();
        assert_eq!(absent.quantity, Some(1));
        assert_eq!(absent.menu_op, None);

        let args = parse_bank_start(
            "bank_op",
            json!({ "kind": "withdraw-x", "name": "Feather", "count": "5" }),
        )
        .unwrap();
        assert_eq!(args.operation, BankSeatOperation::WithdrawDelta);
        assert_eq!(args.item_name.as_deref(), Some("Feather"));
        assert_eq!(args.quantity, Some(5));
        args.validate(native::request::NativeKind::BankPlan)
            .unwrap();

        let args = parse_bank_start(
            "bank_op",
            json!({ "kind": "withdraw-x-id", "id": 314.0, "count": true, "lands_as": "9" }),
        )
        .unwrap();
        assert_eq!(args.operation, BankSeatOperation::WithdrawDelta);
        assert_eq!(args.item_id, Some(314));
        assert_eq!(args.quantity, Some(1));
        assert_eq!(args.lands_as_id, Some(9));
        args.validate(native::request::NativeKind::BankPlan)
            .unwrap();
    }

    #[test]
    fn bank_op_withdraw_x_invalid_and_zero_counts_settle_before_seat_start() {
        let immediate = |value| bank_op_immediate_outcome("bank_op", &value).unwrap();
        assert_eq!(
            immediate(json!({ "kind": "withdraw-x", "name": "Feather", "count": 0 })),
            Some(Outcome::Done(json!(true)))
        );
        assert_eq!(
            immediate(json!({ "kind": "withdraw-x", "name": "Feather", "count": -3 })),
            Some(Outcome::Done(json!(true)))
        );
        assert_eq!(
            immediate(json!({ "kind": "withdraw-x", "name": "Feather", "count": 1.5 })),
            Some(Outcome::Done(json!(false)))
        );
        assert_eq!(
            immediate(json!({ "kind": "withdraw-x", "name": "Feather", "count": "bad" })),
            Some(Outcome::Done(json!(false)))
        );
        assert_eq!(
            immediate(json!({
                "kind": "withdraw-x-id",
                "id": null,
                "count": 0,
            })),
            Some(Outcome::Done(json!(true)))
        );
        assert_eq!(
            immediate(json!({
                "kind": "withdraw-x-id",
                "id": "314",
                "count": 2,
                "lands_as": 9,
            })),
            Some(Outcome::Done(json!(false)))
        );
        assert_eq!(
            immediate(json!({
                "kind": "withdraw-x-id",
                "id": 314,
                "count": 2,
            })),
            Some(Outcome::Done(json!(false)))
        );
        assert_eq!(
            immediate(json!({
                "kind": "withdraw-id",
                "id": "314",
                "op": "Withdraw 1",
            })),
            Some(Outcome::Done(json!(false)))
        );
        assert_eq!(
            immediate(json!({
                "kind": "withdraw-x",
                "name": "Feather",
                "count": 5,
            })),
            None
        );
    }

    /// Test family: emits `if-button {button}` at begin, one `if-button`
    /// per step for `steps` steps, then completes with the step count.
    /// `deadline_ms` arms the clock at begin; a reached bound completes
    /// with `"timeout"`.
    pub(crate) struct Probe {
        left: u32,
        done: u32,
        button: i32,
        deadline: bool,
    }

    #[derive(Deserialize)]
    pub(crate) struct ProbeArgs {
        button: i32,
        steps: u32,
        #[serde(default)]
        refuse: bool,
        #[serde(default)]
        deadline_ms: Option<u64>,
    }

    impl Family for Probe {
        const NAME: &'static str = "probe";
        const SNAPSHOT_SENSITIVE: bool = true;
        type Args = ProbeArgs;
        type Output = Value;

        fn begin(args: ProbeArgs, cx: &mut Cx<'_>) -> Begin<Self> {
            cx.emit(InteractReq::IfButton {
                component_id: args.button,
            });
            if args.refuse {
                return Begin::Refuse("probe refused".into());
            }
            if args.steps == 0 {
                return Begin::Done(json!(0));
            }
            if let Some(ms) = args.deadline_ms {
                cx.clock().arm(ms);
            }
            Begin::Run(Self {
                left: args.steps,
                done: 0,
                button: args.button,
                deadline: args.deadline_ms.is_some(),
            })
        }

        fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
            if self.deadline {
                return if cx.clock().bound_reached() {
                    Step::Done(json!("timeout"))
                } else {
                    Step::Wait
                };
            }
            if self.left == 0 {
                return Step::Done(json!(self.done));
            }
            self.left -= 1;
            self.done += 1;
            cx.emit(InteractReq::IfButton {
                component_id: self.button + self.done as i32,
            });
            Step::Wait
        }
    }

    /// [`Probe`] as an exclusive family; records its abort reason.
    pub(crate) struct Solo(Probe);

    thread_local! {
        static SOLO_ABORTS: std::cell::Cell<Option<AbortReason>> =
            const { std::cell::Cell::new(None) };
    }

    impl Family for Solo {
        const NAME: &'static str = "solo";
        const EXCLUSIVE: bool = true;
        type Args = ProbeArgs;
        type Output = Value;

        fn begin(args: ProbeArgs, cx: &mut Cx<'_>) -> Begin<Self> {
            match Probe::begin(args, cx) {
                Begin::Run(probe) => Begin::Run(Self(probe)),
                Begin::Done(out) => Begin::Done(out),
                Begin::Refuse(reason) => Begin::Refuse(reason),
            }
        }

        fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
            Family::step(&mut self.0, cx)
        }

        fn abort(&mut self, why: AbortReason) {
            SOLO_ABORTS.with(|cell| cell.set(Some(why)));
        }
    }

    /// An exclusive probe whose host work is a walk: its release is the
    /// walk's AbortWalk, request id = its button.
    pub(crate) struct Walker(Probe, u64);

    impl Family for Walker {
        const NAME: &'static str = "walker";
        const EXCLUSIVE: bool = true;
        const WALKING_OPERATION: bool = true;
        type Args = ProbeArgs;
        type Output = Value;

        fn begin(args: ProbeArgs, cx: &mut Cx<'_>) -> Begin<Self> {
            let id = u64::try_from(args.button).unwrap_or(0);
            match Probe::begin(args, cx) {
                Begin::Run(probe) => Begin::Run(Self(probe, id)),
                Begin::Done(out) => Begin::Done(out),
                Begin::Refuse(reason) => Begin::Refuse(reason),
            }
        }

        fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
            Family::step(&mut self.0, cx)
        }

        fn release(&self) -> Option<InteractReq> {
            Some(InteractReq::AbortWalk { request_id: self.1 })
        }
    }

    /// Test family over two script callbacks: `sync(1, 'a')`, then
    /// `later(<sync reply>)` (typically async), then `mark()` once
    /// `after_ticks` more steps passed, completing with every reply. A
    /// throwing callback fails the machine with its message.
    pub(crate) struct Hooked {
        replies: Vec<Value>,
        after_ticks: u32,
        emit: i32,
    }

    #[derive(Deserialize)]
    pub(crate) struct HookedArgs {
        #[serde(default)]
        after_ticks: u32,
        #[serde(default)]
        emit: i32,
    }

    impl Family for Hooked {
        const NAME: &'static str = "hooked";
        const CALLBACKS: &'static [&'static str] = &["sync", "later", "mark"];
        const SYNC_HOOKS: &'static [usize] = &[0];
        type Args = HookedArgs;
        type Output = Value;

        fn begin(args: HookedArgs, _cx: &mut Cx<'_>) -> Begin<Self> {
            Begin::Run(Self {
                replies: Vec::new(),
                after_ticks: args.after_ticks,
                emit: args.emit,
            })
        }

        fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
            match cx.reply() {
                Some(Reply::Threw(message)) => return Step::Fail(message),
                Some(Reply::Value(value)) => {
                    self.replies.push(value);
                    if self.emit != 0 {
                        cx.emit(InteractReq::IfButton {
                            component_id: self.emit + self.replies.len() as i32,
                        });
                    }
                }
                None => {}
            }
            match self.replies.len() {
                0 => Step::Call(Call {
                    hook: 0,
                    args: vec![json!(1), json!("a")],
                }),
                1 => Step::Call(Call {
                    hook: 1,
                    args: vec![self.replies[0].clone()],
                }),
                2 if self.after_ticks > 0 => {
                    self.after_ticks -= 1;
                    Step::Wait
                }
                2 => Step::Call(Call {
                    hook: 2,
                    args: Vec::new(),
                }),
                _ => Step::Done(Value::Array(std::mem::take(&mut self.replies))),
            }
        }
    }

    /// Calls `each(i)` for i in 0..calls, then completes with the replies.
    /// A throw fails the machine, unless `keep` records its message.
    pub(crate) struct Burst {
        calls: usize,
        keep: bool,
        replies: Vec<Value>,
    }

    #[derive(Deserialize)]
    pub(crate) struct BurstArgs {
        calls: usize,
        #[serde(default)]
        keep: bool,
    }

    impl Family for Burst {
        const NAME: &'static str = "burst";
        const SNAPSHOT_SENSITIVE: bool = true;
        const CALLBACKS: &'static [&'static str] = &["each"];
        type Args = BurstArgs;
        type Output = Value;

        fn begin(args: BurstArgs, _cx: &mut Cx<'_>) -> Begin<Self> {
            Begin::Run(Self {
                calls: args.calls,
                keep: args.keep,
                replies: Vec::new(),
            })
        }

        fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
            match cx.reply() {
                Some(Reply::Threw(thrown)) if self.keep => {
                    self.replies.push(json!(thrown.message()));
                }
                Some(Reply::Threw(thrown)) => return Step::Fail(thrown),
                Some(Reply::Value(value)) => self.replies.push(value),
                None => {}
            }
            if self.replies.len() == self.calls {
                return Step::Done(Value::Array(std::mem::take(&mut self.replies)));
            }
            Step::Call(Call {
                hook: 0,
                args: vec![json!(self.replies.len())],
            })
        }
    }

    /// Settles at begin with which of `a`, `b`, `c` were present.
    pub(crate) struct Present;

    impl Family for Present {
        const NAME: &'static str = "present";
        const CALLBACKS: &'static [&'static str] = &["a", "b", "c"];
        type Args = Value;
        type Output = Value;

        fn begin(_args: Value, cx: &mut Cx<'_>) -> Begin<Self> {
            Begin::Done(json!([cx.has(0), cx.has(1), cx.has(2), cx.has(3)]))
        }

        fn step(&mut self, _cx: &mut Cx<'_>) -> Step<Value> {
            Step::Wait
        }
    }

    /// A begin that (wrongly) starts another machine; settles with what
    /// that start returned.
    pub(crate) struct Nested;

    impl Family for Nested {
        const NAME: &'static str = "nested";
        type Args = Value;
        type Output = Value;

        fn begin(_args: Value, _cx: &mut Cx<'_>) -> Begin<Self> {
            let inner = start_with_parent(
                "probe",
                json!({ "button": 1, "steps": 1 }),
                Vec::new(),
                0,
                None,
            );
            Begin::Done(json!(format!("{inner:?}")))
        }

        fn step(&mut self, _cx: &mut Cx<'_>) -> Step<Value> {
            Step::Wait
        }
    }

    /// Exclusive: calls `again()` once, then emits `if-button 999` each
    /// step; `quiet` rows only wait.
    pub(crate) struct SoloHooked {
        quiet: bool,
        called: bool,
    }

    #[derive(Deserialize)]
    pub(crate) struct SoloHookedArgs {
        #[serde(default)]
        quiet: bool,
    }

    impl Family for SoloHooked {
        const NAME: &'static str = "solo-hooked";
        const EXCLUSIVE: bool = true;
        const CALLBACKS: &'static [&'static str] = &["again"];
        type Args = SoloHookedArgs;
        type Output = Value;

        fn begin(args: SoloHookedArgs, _cx: &mut Cx<'_>) -> Begin<Self> {
            Begin::Run(Self {
                quiet: args.quiet,
                called: false,
            })
        }

        fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
            if self.quiet {
                return Step::Wait;
            }
            if !self.called {
                self.called = true;
                return Step::Call(Call {
                    hook: 0,
                    args: Vec::new(),
                });
            }
            cx.emit(InteractReq::IfButton { component_id: 999 });
            Step::Wait
        }
    }

    /// Asks `hooks.get(i)` inline `asks` times in one step, emitting
    /// `if-button i` after each answer, then completes with the answers.
    /// Keeps stepping after a failed ask, to prove the host ends the row.
    pub(crate) struct Inline {
        asks: usize,
    }

    #[derive(Deserialize)]
    pub(crate) struct InlineArgs {
        asks: usize,
    }

    impl Family for Inline {
        const NAME: &'static str = "inline";
        const CALLBACKS: &'static [&'static str] = &["get"];
        type Args = InlineArgs;
        type Output = Value;

        fn begin(args: InlineArgs, cx: &mut Cx<'_>) -> Begin<Self> {
            assert_eq!(
                cx.ask(0, &[json!(0)]),
                Err(Ended),
                "begin has no script side"
            );
            Begin::Run(Self { asks: args.asks })
        }

        fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
            let mut answers = Vec::new();
            for i in 0..self.asks {
                if let Ok(value) = cx.ask(0, &[json!(i)]) {
                    answers.push(value);
                }
                cx.emit(InteractReq::IfButton {
                    component_id: i as i32,
                });
            }
            Step::Done(Value::Array(answers))
        }
    }

    /// Exclusive: asks `get()` once per step, then emits `if-button 5`.
    pub(crate) struct SoloAsker;

    impl Family for SoloAsker {
        const NAME: &'static str = "solo-asker";
        const EXCLUSIVE: bool = true;
        const CALLBACKS: &'static [&'static str] = &["get"];
        type Args = Value;
        type Output = Value;

        fn begin(_args: Value, _cx: &mut Cx<'_>) -> Begin<Self> {
            Begin::Run(Self)
        }

        fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
            if cx.ask(0, &[json!(0)]).is_err() {
                return Step::Wait;
            }
            cx.emit(InteractReq::IfButton { component_id: 5 });
            Step::Wait
        }
    }

    /// Calls `ask()` once and completes with its reply. Used to start
    /// another machine from inside a callback.
    pub(crate) struct Ask;

    impl Family for Ask {
        const NAME: &'static str = "ask";
        const CALLBACKS: &'static [&'static str] = &["ask"];
        type Args = Value;
        type Output = Value;

        fn begin(_args: Value, _cx: &mut Cx<'_>) -> Begin<Self> {
            Begin::Run(Self)
        }

        fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
            match cx.reply() {
                Some(Reply::Threw(thrown)) => Step::Fail(thrown),
                Some(Reply::Value(value)) => Step::Done(value),
                None => Step::Call(Call {
                    hook: 0,
                    args: Vec::new(),
                }),
            }
        }
    }

    /// Feeds a fixture effect through Hunt's real notification/decision
    /// consumer, without duplicating its notes loop or callback mapping.
    pub(crate) struct NotificationOrder {
        started: bool,
    }

    impl Family for NotificationOrder {
        const NAME: &'static str = "notification-order";
        const CALLBACKS: &'static [&'static str] = crate::hunt::HOOKS;
        const KICK_ON_START: bool = true;
        type Args = Value;
        type Output = Value;

        fn begin(_args: Value, _cx: &mut Cx<'_>) -> Begin<Self> {
            Begin::Run(Self { started: false })
        }

        fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
            if self.started {
                return match cx.reply() {
                    Some(Reply::Value(value)) => Step::Done(value),
                    Some(Reply::Threw(thrown)) => Step::Fail(thrown),
                    None => Step::Wait,
                };
            }
            self.started = true;
            let effect = json!({
                "kind": "eat",
                "notes": [{"kind": "count-kill"}, {"kind": "count-kill"}],
            });
            match crate::hunt::characterize_effect(&effect, cx) {
                Ok(Some(step)) => step,
                Ok(None) | Err(_) => Step::Wait,
            }
        }
    }

    /// KICK_ON_START: calls `each(0)`, `each(1)`, emits `if-button 60`,
    /// then completes with the replies (at once when `done`, else on the
    /// next step).
    pub(crate) struct Kicked {
        done: bool,
        replies: Vec<Value>,
    }

    #[derive(Deserialize)]
    pub(crate) struct KickedArgs {
        done: bool,
    }

    impl Family for Kicked {
        const NAME: &'static str = "kicked";
        const CALLBACKS: &'static [&'static str] = &["each"];
        const KICK_ON_START: bool = true;
        type Args = KickedArgs;
        type Output = Value;

        fn begin(args: KickedArgs, _cx: &mut Cx<'_>) -> Begin<Self> {
            Begin::Run(Self {
                done: args.done,
                replies: Vec::new(),
            })
        }

        fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
            if let Some(Reply::Value(value)) = cx.reply() {
                self.replies.push(value);
                if self.replies.len() == 2 {
                    cx.emit(InteractReq::IfButton { component_id: 60 });
                    if !self.done {
                        return Step::Wait;
                    }
                }
            }
            if self.replies.len() < 2 {
                return Step::Call(Call {
                    hook: 0,
                    args: vec![json!(self.replies.len())],
                });
            }
            Step::Done(Value::Array(self.replies.clone()))
        }
    }

    /// Answers every callback at once with its first argument.
    struct Echo {
        calls: usize,
    }

    impl Js for Echo {
        fn queue_len(&mut self) -> usize {
            0
        }

        fn call(&mut self, _hook: Option<&HeldCallback>, args: &[Value]) -> Called {
            self.calls += 1;
            Called::Settled(Reply::Value(args[0].clone()))
        }

        fn poll(&mut self, _pending: &Pending) -> Option<Reply> {
            unreachable!("Echo never returns a promise");
        }

        fn claimed(&mut self) -> bool {
            false
        }
    }

    /// No script callbacks: a machine that asks for one is a test bug.
    struct NoJs;

    impl Js for NoJs {
        fn queue_len(&mut self) -> usize {
            0
        }

        fn call(&mut self, _hook: Option<&HeldCallback>, _args: &[Value]) -> Called {
            panic!("no script callbacks in this test");
        }

        fn poll(&mut self, _pending: &Pending) -> Option<Reply> {
            panic!("no script callbacks in this test");
        }

        fn claimed(&mut self) -> bool {
            false
        }
    }

    fn tick() {
        step(&mut NoJs);
    }

    fn begin(family: &str, args: Value) -> Started {
        start(family, args, Vec::new(), 0)
    }

    fn running(started: Started) -> Handle {
        match started {
            Started::Running(handle) => handle,
            other => panic!("expected a running row, got {other:?}"),
        }
    }

    fn button(id: i32) -> InteractReq {
        InteractReq::IfButton { component_id: id }
    }

    fn drain() -> Vec<InteractReq> {
        merge_ops(Vec::new())
    }

    fn live_rows() -> usize {
        HOST.with(|host| host.borrow().rows.len())
    }

    fn freeze(paused: bool, held: bool) {
        HOST.with(|host| host.borrow_mut().set_freeze(paused, held));
    }

    /// Every live row's armed deadline has passed (family unit tests).
    pub(crate) fn expire_deadlines() {
        HOST.with(|host| {
            for row in &mut host.borrow_mut().rows {
                if row.clock.deadline.is_some() {
                    row.clock.deadline = Some(std::time::Instant::now() - Duration::from_millis(1));
                }
            }
        });
    }

    #[test]
    fn a_row_steps_emits_in_order_and_settles_exactly_once() {
        let h = running(begin("probe", json!({ "button": 10, "steps": 2 })));
        assert_eq!(drain(), vec![button(10)]);
        assert_eq!(take(h), Take::Pending);
        tick();
        tick();
        assert_eq!(drain(), vec![button(11), button(12)]);
        assert_eq!(take(h), Take::Pending);
        tick();
        assert_eq!(live_rows(), 0, "completion ends the row");
        assert_eq!(take(h), Take::Settled(Outcome::Done(json!(2))));
        assert_eq!(
            take(h),
            Take::Settled(Outcome::Aborted(AbortReason::Unknown)),
            "an outcome is handed out once"
        );
    }

    #[test]
    fn begin_settles_or_refuses_without_a_row() {
        assert_eq!(
            begin("probe", json!({ "button": 5, "steps": 0 })),
            Started::Settled(Outcome::Done(json!(0)))
        );
        assert_eq!(drain(), vec![button(5)]);
        assert_eq!(
            begin("probe", json!({ "button": 6, "steps": 1, "refuse": true })),
            Started::Refused("probe refused".into())
        );
        assert!(drain().is_empty(), "a refused begin sends nothing");
        assert!(matches!(
            begin("probe", json!({ "steps": 1 })),
            Started::Refused(reason) if reason.starts_with("probe arguments:")
        ));
        assert!(matches!(begin("nowhere", json!({})), Started::Refused(_)));
        assert_eq!(live_rows(), 0);
    }

    #[test]
    fn reset_aborts_every_row_and_drops_pending_ops() {
        let a = running(begin("probe", json!({ "button": 1, "steps": 5 })));
        let b = running(begin("probe", json!({ "button": 2, "steps": 5 })));
        on_reset();
        assert!(drain().is_empty());
        assert_eq!(live_rows(), 0);
        tick();
        assert!(drain().is_empty(), "an aborted row never steps");
        for h in [a, b] {
            assert_eq!(take(h), Take::Settled(Outcome::Aborted(AbortReason::Reset)));
        }
    }

    #[test]
    fn stop_drops_rows_and_outcomes() {
        let a = running(begin("probe", json!({ "button": 1, "steps": 1 })));
        tick();
        tick();
        let b = running(begin("probe", json!({ "button": 2, "steps": 5 })));
        on_stop();
        assert_eq!(live_rows(), 0);
        assert!(drain().is_empty());
        for h in [a, b] {
            assert_eq!(
                take(h),
                Take::Settled(Outcome::Aborted(AbortReason::Unknown)),
                "Stop settles nothing"
            );
        }
    }

    #[test]
    fn pause_and_hold_skip_steps_and_freeze_deadlines() {
        let h = running(begin(
            "probe",
            json!({ "button": 1, "steps": 1, "deadline_ms": 40 }),
        ));
        on_pause();
        std::thread::sleep(Duration::from_millis(60));
        tick();
        assert_eq!(take(h), Take::Pending, "a paused row is not stepped");
        on_hold(true);
        on_resume();
        tick();
        assert_eq!(take(h), Take::Pending, "a held row is not stepped");
        on_hold(false);
        tick();
        assert_eq!(
            take(h),
            Take::Pending,
            "the frozen span does not count toward the deadline"
        );
        std::thread::sleep(Duration::from_millis(60));
        tick();
        assert_eq!(take(h), Take::Settled(Outcome::Done(json!("timeout"))));
    }

    #[test]
    fn a_row_started_while_frozen_starts_frozen() {
        freeze(false, true);
        let h = running(begin(
            "probe",
            json!({ "button": 1, "steps": 1, "deadline_ms": 30 }),
        ));
        std::thread::sleep(Duration::from_millis(50));
        freeze(false, false);
        tick();
        assert_eq!(take(h), Take::Pending);
    }

    #[test]
    fn two_concurrent_rows_step_in_start_order_and_settle_independently() {
        let a = running(begin("probe", json!({ "button": 100, "steps": 1 })));
        let b = running(begin("probe", json!({ "button": 200, "steps": 2 })));
        assert_eq!(drain(), vec![button(100), button(200)]);
        tick();
        assert_eq!(drain(), vec![button(101), button(201)]);
        tick();
        assert_eq!(drain(), vec![button(202)]);
        assert_eq!(take(a), Take::Settled(Outcome::Done(json!(1))));
        assert_eq!(take(b), Take::Pending);
        tick();
        assert_eq!(take(b), Take::Settled(Outcome::Done(json!(2))));
    }

    #[test]
    fn an_exclusive_start_supersedes_only_its_own_family() {
        let other = running(begin("probe", json!({ "button": 1, "steps": 5 })));
        let old = running(begin("solo", json!({ "button": 2, "steps": 5 })));
        assert!(matches!(
            begin("solo", json!({ "button": 3, "steps": 1, "refuse": true })),
            Started::Refused(_)
        ));
        assert_eq!(
            take(old),
            Take::Pending,
            "a refused start supersedes nothing"
        );
        let new = running(begin("solo", json!({ "button": 4, "steps": 5 })));
        assert_eq!(
            take(old),
            Take::Settled(Outcome::Aborted(AbortReason::Superseded))
        );
        assert_eq!(SOLO_ABORTS.with(|c| c.get()), Some(AbortReason::Superseded));
        assert_eq!(take(new), Take::Pending);
        assert_eq!(take(other), Take::Pending);
    }

    #[test]
    fn native_exclusive_replacement_waits_for_seat_admission_and_result() {
        on_reset();
        set_execution(Some(ExecutionId::first(api::selected::RunKey {
            slot: 1,
            run: 1,
            session: 1,
        })));
        let old = running(start(
            "quest-progress",
            json!({"quest": "first"}),
            vec![],
            0,
        ));
        let replacement = running(start(
            "quest-progress",
            json!({"quest": "second"}),
            vec![],
            0,
        ));
        assert_eq!(take(old), Take::Pending);
        assert_eq!(take(replacement), Take::Pending);

        let replacement_id = RequestId::isolate(replacement).unwrap();
        HOST.with(|host| {
            settle_seat_result(
                &mut host.borrow_mut(),
                replacement_id,
                Outcome::Refused("Busy".into()),
            );
        });
        assert_eq!(
            take(replacement),
            Take::Settled(Outcome::Refused("Busy".into()))
        );
        assert_eq!(
            take(old),
            Take::Pending,
            "a refused replacement does not supersede its admitted predecessor"
        );

        let old_id = RequestId::isolate(old).unwrap();
        HOST.with(|host| {
            settle_seat_result(
                &mut host.borrow_mut(),
                old_id,
                Outcome::Aborted(AbortReason::Superseded),
            );
        });
        assert_eq!(
            take(old),
            Take::Settled(Outcome::Aborted(AbortReason::Superseded))
        );
        on_reset();
        set_execution(None);
    }

    #[test]
    fn a_reconnect_hold_sends_no_release_op() {
        on_reset();
        let aborts = |ops: Vec<InteractReq>| -> Vec<u64> {
            ops.into_iter()
                .filter_map(|op| match op {
                    InteractReq::AbortWalk { request_id } => Some(request_id),
                    _ => None,
                })
                .collect()
        };
        running(begin("walker", json!({ "button": 7, "steps": 5 })));
        drain();
        running(begin("walker", json!({ "button": 8, "steps": 5 })));
        assert_eq!(aborts(drain()), vec![7], "a superseded row stops its walk");
        on_session_hold(true);
        running(begin("walker", json!({ "button": 9, "steps": 5 })));
        assert!(
            aborts(drain()).is_empty(),
            "the reconnect's host carry owns the held walk"
        );
        on_session_hold(false);
        on_reset();
    }

    /// Answers each call with its argument; call `throw_at` throws, call
    /// `terminate_at` is terminated, and from call `claim_at` on join has
    /// claimed the tick.
    struct Inliner {
        calls: usize,
        throw_at: Option<usize>,
        terminate_at: Option<usize>,
        claim_at: Option<usize>,
    }

    impl Inliner {
        fn new() -> Self {
            Self {
                calls: 0,
                throw_at: None,
                terminate_at: None,
                claim_at: None,
            }
        }
    }

    impl Js for Inliner {
        fn queue_len(&mut self) -> usize {
            0
        }

        fn call(&mut self, _hook: Option<&HeldCallback>, args: &[Value]) -> Called {
            self.calls += 1;
            if self.terminate_at == Some(self.calls) {
                return Called::Terminated;
            }
            if self.throw_at == Some(self.calls) {
                return Called::Settled(Reply::Threw(Thrown::new("boom")));
            }
            Called::Settled(Reply::Value(args[0].clone()))
        }

        fn poll(&mut self, _pending: &Pending) -> Option<Reply> {
            unreachable!("Inliner never returns a promise");
        }

        fn claimed(&mut self) -> bool {
            self.claim_at.is_some_and(|at| self.calls >= at)
        }
    }

    #[test]
    fn manual_takeover_replays_a_real_walk_event_and_replaces_its_phase() {
        on_reset();
        let execution = ExecutionId::first(api::selected::RunKey {
            slot: 1,
            run: 1,
            session: 1,
        });
        set_execution(Some(execution));
        let handle = running(begin("walker", json!({ "button": 7, "steps": 5 })));
        let id = RequestId::isolate(handle).expect("machine request identity");
        HOST.with(|host| {
            let mut host = host.borrow_mut();
            let row = host
                .rows
                .iter_mut()
                .find(|row| row.handle == handle)
                .unwrap();
            row.native_kind = Some(native::request::NativeKind::Walk);
            row.admitted = true;
            row.intent_baseline = 0;
        });
        let child = running(start_with_parent(
            "probe",
            json!({ "button": 8, "steps": 5 }),
            Vec::new(),
            0,
            Some(handle),
        ));
        step(&mut Inliner::new());
        HOST.with(|host| {
            assert_eq!(
                machine_events_for_batch(&host.borrow()),
                vec![MachineEvent {
                    id,
                    kind: MachineEventKind::Phase(native::NativePhase::Waiting),
                    admitted: true,
                }]
            );
        });
        on_manual_walk_takeover(1);
        on_manual_walk_takeover(2);
        assert_eq!(
            take(child),
            Take::Settled(Outcome::Aborted(AbortReason::ParentAborted)),
            "the hybrid descendant settles before the walking owner"
        );
        HOST.with(|host| {
            let host = host.borrow();
            assert!(host.rows.iter().any(|row| {
                row.handle == handle && row.manual_walk_event_sent && row.authority_revoked
            }));
            assert!(
                host.ops.iter().any(|op| {
                    op.lineage.dropped && matches!(op.request, InteractReq::IfButton { .. })
                }),
                "the owner-scoped gameplay is marked revoked before host drain"
            );
            assert_eq!(
                machine_events_for_batch(&host),
                vec![MachineEvent {
                    id,
                    kind: MachineEventKind::ManualWalk,
                    admitted: true,
                }],
                "the manual takeover revokes once, before the walk settles"
            );
        });
        let lineage = HOST.with(|host| {
            host.borrow()
                .rows
                .iter()
                .find(|row| row.handle == handle)
                .unwrap()
                .capture()
        });
        HOST.with(|host| {
            host.borrow_mut()
                .place(0, vec![InteractReq::AbortWalk { request_id: 7 }], lineage);
        });
        let controls = merge_ops_with_lineage(Vec::new(), Vec::new()).unwrap();
        assert_eq!(
            controls.into_iter().map(|(req, _)| req).collect::<Vec<_>>(),
            vec![
                InteractReq::IfButton { component_id: 7 },
                InteractReq::AbortWalk { request_id: 7 },
            ],
            "the pre-start root action survives; revoked owner gameplay drops and the physical release remains"
        );
        on_reset();
        on_stop();
        set_execution(None);
    }

    #[test]
    fn asks_answer_inside_one_step_outside_the_callback_budget() {
        let asks = CALLS_PER_TICK + 3;
        let h = running(begin("inline", json!({ "asks": asks })));
        let mut js = Inliner::new();
        step(&mut js);
        assert_eq!(js.calls, asks);
        assert_eq!(
            take(h),
            Take::Settled(Outcome::Done(json!((0..asks).collect::<Vec<_>>())))
        );
        assert_eq!(drain().len(), asks, "the step's ops are kept");
    }

    #[test]
    fn a_throwing_ask_fails_the_row_with_the_throw_and_drops_its_ops() {
        let h = running(begin("inline", json!({ "asks": 4 })));
        let mut js = Inliner::new();
        js.throw_at = Some(2);
        step(&mut js);
        assert_eq!(js.calls, 2, "no ask runs after the throw");
        assert_eq!(take(h), Take::Settled(Outcome::Failed(Thrown::new("boom"))));
        assert!(drain().is_empty());
    }

    #[test]
    fn a_terminated_or_claimed_ask_aborts_the_row() {
        let h = running(begin("inline", json!({ "asks": 4 })));
        let mut js = Inliner::new();
        js.terminate_at = Some(1);
        step(&mut js);
        assert_eq!(js.calls, 1);
        assert_eq!(
            take(h),
            Take::Settled(Outcome::Aborted(AbortReason::Terminated))
        );
        assert!(drain().is_empty());

        let h = running(begin("inline", json!({ "asks": 4 })));
        let mut js = Inliner::new();
        js.claim_at = Some(1);
        // The pass sees no claim before the step; the first ask's call
        // claims the tick, and the pass settles the row `terminated`.
        step(&mut js);
        assert_eq!(js.calls, 1, "no ask runs once the tick is claimed");
        assert_eq!(
            take(h),
            Take::Settled(Outcome::Aborted(AbortReason::Terminated))
        );
        assert!(drain().is_empty());
    }

    #[test]
    fn an_ask_that_starts_a_newer_exclusive_row_supersedes_the_asker() {
        /// The first call starts a newer `solo-asker` row.
        struct Starter {
            calls: usize,
        }
        impl Js for Starter {
            fn queue_len(&mut self) -> usize {
                0
            }
            fn call(&mut self, _hook: Option<&HeldCallback>, _args: &[Value]) -> Called {
                self.calls += 1;
                if self.calls == 1 {
                    running(begin("solo-asker", json!({})));
                }
                Called::Settled(Reply::Value(json!(true)))
            }
            fn poll(&mut self, _pending: &Pending) -> Option<Reply> {
                unreachable!("Starter never returns a promise");
            }
            fn claimed(&mut self) -> bool {
                false
            }
        }
        let old = running(begin("solo-asker", json!({})));
        let mut js = Starter { calls: 0 };
        step(&mut js);
        assert_eq!(
            take(old),
            Take::Settled(Outcome::Aborted(AbortReason::Superseded))
        );
        assert!(
            drain().is_empty(),
            "the superseded step's ops are dropped; the new row has not stepped"
        );
    }

    #[test]
    fn a_runaway_step_is_bounded() {
        let h = running(begin("inline", json!({ "asks": ASKS_PER_STEP + 10 })));
        let mut js = Inliner::new();
        step(&mut js);
        assert_eq!(js.calls, ASKS_PER_STEP);
        let Take::Settled(Outcome::Failed(thrown)) = take(h) else {
            panic!("the bound fails the row");
        };
        assert!(
            thrown.message().contains("synchronous callbacks"),
            "{thrown:?}"
        );
    }

    #[test]
    fn a_row_runs_at_most_the_callback_budget_per_tick() {
        let total = CALLS_PER_TICK + 8;
        let h = running(begin("burst", json!({ "calls": total })));
        let mut js = Echo { calls: 0 };
        step(&mut js);
        assert_eq!(js.calls, CALLS_PER_TICK);
        resume(&mut js);
        assert_eq!(
            js.calls, CALLS_PER_TICK,
            "a row not waiting on a promise does not resume"
        );
        assert_eq!(take(h), Take::Pending);
        step(&mut js);
        assert_eq!(js.calls, total);
        let Take::Settled(Outcome::Done(Value::Array(replies))) = take(h) else {
            panic!("the burst completes on the second tick");
        };
        assert_eq!(replies, (0..total).map(|i| json!(i)).collect::<Vec<_>>());
    }

    #[test]
    fn snapshot_pass_deduplicates_the_same_tick_and_evidence_sequence() {
        let handle = running(begin("probe", json!({ "button": 10, "steps": 3 })));
        merge_ops(Vec::new());
        let mut js = Echo { calls: 0 };
        snapshot_step(&mut js, 1, 1);
        assert_eq!(
            merge_ops(Vec::new()),
            vec![InteractReq::IfButton { component_id: 11 }]
        );
        snapshot_step(&mut js, 1, 1);
        assert!(merge_ops(Vec::new()).is_empty());
        snapshot_step(&mut js, 1, 2);
        assert_eq!(
            merge_ops(Vec::new()),
            vec![InteractReq::IfButton { component_id: 12 }]
        );
        snapshot_step(&mut js, 2, 2);
        assert_eq!(
            merge_ops(Vec::new()),
            vec![InteractReq::IfButton { component_id: 13 }]
        );
        assert_eq!(take(handle), Take::Pending);
    }

    #[test]
    fn snapshot_passes_do_not_replenish_the_callback_budget() {
        let total = CALLS_PER_TICK + 8;
        let handle = running(begin("burst", json!({ "calls": total })));
        let mut js = Echo { calls: 0 };
        step(&mut js);
        for evidence_sequence in 1..=4 {
            snapshot_step(&mut js, 1, evidence_sequence);
        }
        assert_eq!(js.calls, CALLS_PER_TICK);
        assert_eq!(take(handle), Take::Pending);
        step(&mut js);
        assert_eq!(js.calls, total);
        assert!(matches!(take(handle), Take::Settled(Outcome::Done(_))));
    }

    #[test]
    fn the_budget_does_not_split_a_frozen_sized_iteration() {
        // The partner-trade receiver's worst frozen iteration: two passes
        // over 28 offer rows plus its fixed hooks.
        let h = running(begin("burst", json!({ "calls": 2 * 28 + 10 })));
        step(&mut Echo { calls: 0 });
        assert!(matches!(take(h), Take::Settled(Outcome::Done(_))));
    }

    /// Answers like [`Echo`], and moves the JS queue one row per call (a
    /// callback that queued a JS op), until call `terminate_at`, which is
    /// terminated.
    struct Scripted {
        calls: usize,
        queued: usize,
        terminate_at: Option<usize>,
    }

    impl Js for Scripted {
        fn queue_len(&mut self) -> usize {
            self.queued
        }

        fn call(&mut self, _hook: Option<&HeldCallback>, args: &[Value]) -> Called {
            self.calls += 1;
            if self.terminate_at == Some(self.calls) {
                return Called::Terminated;
            }
            self.queued += 1;
            Called::Settled(Reply::Value(args[0].clone()))
        }

        fn poll(&mut self, _pending: &Pending) -> Option<Reply> {
            unreachable!("Scripted never returns a promise");
        }

        fn claimed(&mut self) -> bool {
            false
        }
    }

    fn scripted(terminate_at: Option<usize>) -> Scripted {
        Scripted {
            calls: 0,
            queued: 0,
            terminate_at,
        }
    }

    #[test]
    fn a_kick_runs_the_first_callbacks_and_ops_in_the_starting_turn() {
        let h = running(begin("kicked", json!({ "done": false })));
        let mut js = scripted(None);
        kick(h, &mut js);
        assert_eq!(js.calls, 2, "both callbacks ran inside the start");
        let js_row = |id: i32| MaybeInteractReq::Req(button(id));
        assert_eq!(
            merge_ops(vec![js_row(1), js_row(2)]),
            vec![button(1), button(2), button(60)],
            "the kick's op lands after the rows its callbacks queued"
        );
        assert_eq!(take(h), Take::Pending);
        resume(&mut js);
        assert_eq!(js.calls, 2, "a kicked row not on a promise does not resume");
        step(&mut js);
        assert_eq!(take(h), Take::Settled(Outcome::Done(json!([0, 1]))));
    }

    #[test]
    fn a_row_that_ends_inside_its_kick_settles_at_once() {
        let h = running(begin("kicked", json!({ "done": true })));
        kick(h, &mut scripted(None));
        assert_eq!(live_rows(), 0);
        assert!(any_settled());
        assert_eq!(take(h), Take::Settled(Outcome::Done(json!([0, 1]))));
    }

    #[test]
    fn a_terminated_callback_in_a_kick_aborts_the_row_unseen() {
        // `keep` would record a throw and call on: the family must never
        // see the termination.
        let h = running(begin("burst", json!({ "calls": 3, "keep": true })));
        let mut js = scripted(Some(1));
        kick(h, &mut js);
        assert_eq!(js.calls, 1, "no callback runs after the terminated one");
        assert_eq!(live_rows(), 0);
        assert!(
            !any_settled(),
            "the unwound caller never awaits it: no orphaned settlement"
        );
        assert_eq!(
            take(h),
            Take::Settled(Outcome::Aborted(AbortReason::Unknown))
        );
    }

    #[test]
    fn a_terminated_callback_stops_the_whole_pass() {
        let a = running(begin("burst", json!({ "calls": 3, "keep": true })));
        let b = running(begin("burst", json!({ "calls": 1, "keep": true })));
        let mut js = scripted(Some(1));
        step(&mut js);
        assert_eq!(js.calls, 1, "the second row is not driven this pass");
        assert_eq!(
            take(a),
            Take::Settled(Outcome::Aborted(AbortReason::Terminated))
        );
        assert_eq!(take(b), Take::Pending);
        step(&mut Echo { calls: 0 });
        assert_eq!(take(b), Take::Settled(Outcome::Done(json!([0]))));
    }

    #[test]
    fn a_begin_that_starts_a_machine_is_refused_not_a_panic() {
        assert_eq!(
            begin("nested", json!({})),
            Started::Settled(Outcome::Done(json!(format!(
                "{:?}",
                Started::Refused("a machine begin may not start a machine".into())
            ))))
        );
        assert_eq!(live_rows(), 0);
        running(begin("probe", json!({ "button": 1, "steps": 1 })));
    }

    #[test]
    fn a_row_stepping_a_callback_stays_live() {
        let handle = running(begin("ask", json!({})));
        assert!(live("ask"));
        // Starts a `solo` row from inside the callback, as script code
        // does, and answers with what `live` saw during that call.
        struct StartsInside;
        impl Js for StartsInside {
            fn queue_len(&mut self) -> usize {
                0
            }

            fn call(&mut self, _hook: Option<&HeldCallback>, _args: &[Value]) -> Called {
                running(begin("solo", json!({ "button": 7, "steps": 5 })));
                Called::Settled(Reply::Value(json!({
                    "asking": live("ask"),
                    "started": live("solo"),
                })))
            }

            fn poll(&mut self, _pending: &Pending) -> Option<Reply> {
                unreachable!("the ask callback returns a value")
            }

            fn claimed(&mut self) -> bool {
                false
            }
        }
        step(&mut StartsInside);
        assert_eq!(
            take(handle),
            Take::Settled(Outcome::Done(json!({ "asking": true, "started": true }))),
            "a callback must see this pass's rows, not an empty map"
        );
        assert!(live("solo"), "the callback's own row is live");
        assert!(!live("ask"), "a settled row is not live");
        assert_eq!(drain(), vec![button(7)], "the inner row kept its ops");
    }

    #[test]
    fn machine_ops_merge_at_their_js_queue_position() {
        let js = |id: i32| MaybeInteractReq::Req(button(id));
        let placed = vec![
            PlacedOp {
                at: 0,
                request: button(1),
                lineage: CapturedLineage::default(),
            },
            PlacedOp {
                at: 2,
                request: button(3),
                lineage: CapturedLineage::default(),
            },
            PlacedOp {
                at: 2,
                request: button(4),
                lineage: CapturedLineage::default(),
            },
            PlacedOp {
                at: 9,
                request: button(6),
                lineage: CapturedLineage::default(),
            },
        ];
        HOST.with(|host| host.borrow_mut().ops = placed);
        let skipped = MaybeInteractReq::Skip(crate::shim::RejectedRow("a number".into()));
        let rows = vec![js(2), skipped, js(5)];
        assert_eq!(
            merge_ops(rows),
            vec![
                button(1),
                button(2),
                button(3),
                button(4),
                button(5),
                button(6)
            ]
        );
    }

    #[test]
    fn unadmitted_machine_events_wait_for_admission_and_replay_until_ack() {
        let phase_id = RequestId::isolate(1).expect("isolate id");
        let done_id = RequestId::isolate(2).expect("isolate id");
        let failed_id = RequestId::isolate(3).expect("isolate id");
        let manual_walk_id = RequestId::isolate(4).expect("isolate id");
        let phase = MachineEventKind::Phase(native::NativePhase::Working);
        let done = MachineEventKind::Done;
        let failed = MachineEventKind::Failed;
        let manual_walk = MachineEventKind::ManualWalk;
        let mut host = Host::new();
        host.machine_events.extend([
            MachineEvent {
                id: phase_id,
                kind: phase,
                admitted: false,
            },
            MachineEvent {
                id: done_id,
                kind: done,
                admitted: false,
            },
            MachineEvent {
                id: failed_id,
                kind: failed,
                admitted: false,
            },
            MachineEvent {
                id: manual_walk_id,
                kind: manual_walk,
                admitted: false,
            },
        ]);

        // A lost page or deferred start leaves every event in carrier custody.
        reconcile_machine_events(&mut host, &[], &[]);
        assert!(machine_events_for_batch(&host).is_empty());

        reconcile_machine_events(
            &mut host,
            &[
                (phase_id, native::NativePhase::Preparing),
                (done_id, native::NativePhase::Waiting),
                (failed_id, native::NativePhase::Waiting),
                (manual_walk_id, native::NativePhase::Waiting),
            ],
            &[],
        );
        let replayed = machine_events_for_batch(&host);
        assert_eq!(replayed.len(), 4);
        assert_eq!(
            replayed.iter().map(|event| event.kind).collect::<Vec<_>>(),
            vec![phase, done, failed, manual_walk]
        );
        assert!(replayed.iter().all(|event| event.admitted));

        reconcile_machine_events(&mut host, &[(phase_id, native::NativePhase::Working)], &[]);
        assert_eq!(
            machine_events_for_batch(&host),
            vec![
                MachineEvent {
                    id: done_id,
                    kind: done,
                    admitted: true,
                },
                MachineEvent {
                    id: failed_id,
                    kind: failed,
                    admitted: true,
                },
                MachineEvent {
                    id: manual_walk_id,
                    kind: manual_walk,
                    admitted: true,
                },
            ]
        );

        reconcile_machine_events(&mut host, &[], &[done_id, failed_id, manual_walk_id]);
        assert!(machine_events_for_batch(&host).is_empty());
    }

    #[test]
    fn every_production_family_callback_has_an_abi_row_matching_sync_hooks() {
        use crate::native::abi::{FrozenPathStatus, HookPath, HOOK_CHARACTERIZATIONS};
        use crate::native::divergence::{DivergenceStatus, DIVERGENCES};

        const TEST_FAMILY_NAMES: &[&str] = &[
            "probe",
            "solo",
            "walker",
            "hooked",
            "burst",
            "present",
            "nested",
            "solo-hooked",
            "inline",
            "solo-asker",
            "ask",
            "notification-order",
            "kicked",
        ];
        for family in super::FAMILIES {
            if TEST_FAMILY_NAMES.contains(&family.name) {
                continue;
            }
            for (index, member) in family.callbacks.iter().enumerate() {
                let rows: Vec<_> = HOOK_CHARACTERIZATIONS
                    .iter()
                    .filter(|row| row.families().contains(&family.name) && row.member() == *member)
                    .collect();
                assert!(
                    !rows.is_empty(),
                    "no ABI row for registered {} / {}",
                    family.name,
                    member
                );
                let sync = family.sync_hooks.contains(&index);
                for row in rows {
                    assert_eq!(
                        row.path() == HookPath::CallSync,
                        sync,
                        "{} / {} path {:?} vs SYNC_HOOKS {:?}",
                        family.name,
                        member,
                        row.path(),
                        family.sync_hooks
                    );
                }
            }
        }
        for descriptor in super::BANK_FAMILY_DESCRIPTORS {
            for member in descriptor.callbacks {
                let rows = HOOK_CHARACTERIZATIONS
                    .iter()
                    .filter(|row| {
                        row.families().contains(&descriptor.name) && row.member() == *member
                    })
                    .collect::<Vec<_>>();
                assert!(
                    !rows.is_empty(),
                    "no ABI row for bank descriptor {} / {member}",
                    descriptor.name
                );
            }
        }
        for row in HOOK_CHARACTERIZATIONS {
            for name in row.families() {
                let callbacks = super::FAMILIES
                    .iter()
                    .find(|family| {
                        family.name == *name && !TEST_FAMILY_NAMES.contains(&family.name)
                    })
                    .map(|family| family.callbacks)
                    .or_else(|| super::bank_family_descriptor(name).map(|family| family.callbacks))
                    .unwrap_or_else(|| panic!("ABI row names unregistered family {name}"));
                assert!(
                    callbacks.contains(&row.member()),
                    "ABI row {name} / {} is not a registered callback",
                    row.member()
                );
            }
        }
        for row in HOOK_CHARACTERIZATIONS {
            let FrozenPathStatus::ExistingDiscrepancy { q5_row, .. } = row.frozen_path() else {
                continue;
            };
            let id = q5_row.to_string();
            let found = DIVERGENCES
                .iter()
                .find(|divergence| divergence.id() == id)
                .unwrap_or_else(|| panic!("q5_row {q5_row} is not in DIVERGENCES"));
            assert_eq!(
                found.status(),
                DivergenceStatus::Existing,
                "q5_row {q5_row} must be an existing discrepancy"
            );
        }
    }
}
