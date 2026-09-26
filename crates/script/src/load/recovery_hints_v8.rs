//! Frozen `RecoveryHints` (`runtime/RecoveryHints.ts:3–18`) as a typed V8
//! helper over slot-owned state.
//!
//! Frozen keeps one module object for the page's life, so a StallGuard
//! restart (`StallGuard.ts:34–39`: `pendingRecovery = true`, then
//! `ScriptRunner.start`) finds the anchor the previous run latched. This
//! host recreates the isolate on a watchdog restart, so the slot owns the
//! hints ([`RecoveryHintsCell`]), marks the restart, and posts the same cell
//! to every isolate it spawns. After `onStart` settles, a recovery nobody
//! took clears the hints (`ScriptRunner.ts:221–223`). An isolate the slot
//! did not post a cell to keeps its own hints.
//!
//! JS keeps the frozen names (`takeAnchor`, `anchor`, `pendingRecovery`,
//! `clear`) and makes one native call per member.

use super::callback_v8::{self, JsResult};
use rustyscript::Runtime;
use std::cell::RefCell;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HintTile {
    pub x: i32,
    pub z: i32,
    pub level: i32,
}

#[derive(Default, Debug)]
struct Hints {
    pending: bool,
    anchor: Option<HintTile>,
}

/// The slot's recovery hints, shared with each isolate it spawns.
#[derive(Default, Debug)]
pub struct RecoveryHintsCell {
    hints: Mutex<Hints>,
}

impl RecoveryHintsCell {
    pub fn new() -> Self {
        Self::default()
    }

    fn with<R>(&self, f: impl FnOnce(&mut Hints) -> R) -> R {
        f(&mut self
            .hints
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()))
    }

    /// A watchdog restart is recreating the script: frozen StallGuard's
    /// `RecoveryHints.pendingRecovery = true` (`StallGuard.ts:37`).
    pub fn note_restart(&self) {
        self.with(|hints| hints.pending = true);
    }

    /// Frozen `takeAnchor()` (`RecoveryHints.ts:7–13`).
    pub fn take_anchor(&self) -> Option<HintTile> {
        self.with(|hints| {
            if !hints.pending {
                return None;
            }
            hints.pending = false;
            hints.anchor
        })
    }

    pub fn anchor(&self) -> Option<HintTile> {
        self.with(|hints| hints.anchor)
    }

    pub fn set_anchor(&self, anchor: Option<HintTile>) {
        self.with(|hints| hints.anchor = anchor);
    }

    pub fn pending(&self) -> bool {
        self.with(|hints| hints.pending)
    }

    pub fn set_pending(&self, pending: bool) {
        self.with(|hints| hints.pending = pending);
    }

    /// Frozen `clear()` (`RecoveryHints.ts:15–18`).
    pub fn clear(&self) {
        self.with(|hints| *hints = Hints::default());
    }

    /// Frozen `ScriptRunner.ts:221–223`, after a successful `onStart`.
    pub fn startup_complete(&self) {
        self.with(|hints| {
            if hints.pending {
                *hints = Hints::default();
            }
        });
    }
}

thread_local! {
    /// This isolate thread's hints: the slot's cell once posted.
    static CELL: RefCell<Arc<RecoveryHintsCell>> =
        RefCell::new(Arc::new(RecoveryHintsCell::new()));
}

/// Use the slot's cell for this isolate (`IsolateCmd::RecoveryHints`).
pub(crate) fn post(cell: Arc<RecoveryHintsCell>) {
    CELL.with(|slot| *slot.borrow_mut() = cell);
}

/// The compat runner's `onStart` settled without an error.
pub(crate) fn startup_complete() {
    CELL.with(|slot| slot.borrow().startup_complete());
}

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    callback_v8::install(runtime, "__rs2b0t_recovery_hints", recovery_hints_callback)
}

fn recovery_hints_callback<'s, 'cb>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'cb>,
) {
    let result = run(scope, &args);
    callback_v8::finish(scope, rv, result);
}

fn run<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let op = callback_v8::to_string(scope, args.get(0))?;
    let cell = CELL.with(|slot| Arc::clone(&slot.borrow()));
    let undefined = v8::undefined(scope).into();
    match op.as_str() {
        "take" => tile_value(scope, cell.take_anchor()),
        "anchor" => tile_value(scope, cell.anchor()),
        "set-anchor" => {
            let anchor = read_tile(scope, args.get(1))?;
            cell.set_anchor(anchor);
            Ok(undefined)
        }
        "pending" => Ok(v8::Boolean::new(scope, cell.pending()).into()),
        "set-pending" => {
            let pending = callback_v8::truthy(scope, args.get(1));
            cell.set_pending(pending);
            Ok(undefined)
        }
        "clear" => {
            cell.clear();
            Ok(undefined)
        }
        other => Err(callback_v8::type_error(
            scope,
            &format!("RecoveryHints: unknown op {other:?}"),
        )),
    }
}

fn tile_value<'s>(
    scope: &mut v8::HandleScope<'s>,
    tile: Option<HintTile>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let Some(tile) = tile else {
        return Ok(v8::null(scope).into());
    };
    let x = callback_v8::num(scope, f64::from(tile.x));
    let z = callback_v8::num(scope, f64::from(tile.z));
    let level = callback_v8::num(scope, f64::from(tile.level));
    callback_v8::object(scope, &[("x", x), ("z", z), ("level", level)])
}

/// A tile-shaped value (`x`, `z`, `level`), else no anchor.
fn read_tile<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
) -> JsResult<'s, Option<HintTile>> {
    if value.is_null_or_undefined() {
        return Ok(None);
    }
    let mut coord = |key: &str| -> JsResult<'s, Option<i32>> {
        let raw = callback_v8::get(scope, value, key)?;
        if raw.is_undefined() {
            return Ok(Some(0));
        }
        let n = callback_v8::number(scope, raw)?;
        Ok(
            (n.fract() == 0.0 && n >= f64::from(i32::MIN) && n <= f64::from(i32::MAX))
                .then_some(n as i32),
        )
    };
    let (Some(x), Some(z), Some(level)) = (coord("x")?, coord("z")?, coord("level")?) else {
        return Ok(None);
    };
    Ok(Some(HintTile { x, z, level }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: HintTile = HintTile {
        x: 3222,
        z: 3218,
        level: 0,
    };

    #[test]
    fn a_restart_hands_the_latched_anchor_to_the_next_start_once() {
        let cell = RecoveryHintsCell::new();
        cell.set_anchor(Some(HOME));
        assert_eq!(cell.take_anchor(), None, "no restart, no hint");
        cell.note_restart();
        assert_eq!(cell.take_anchor(), Some(HOME));
        assert_eq!(cell.take_anchor(), None, "taken once");
        cell.startup_complete();
        assert_eq!(
            cell.anchor(),
            Some(HOME),
            "a taken recovery keeps the latch"
        );
    }

    #[test]
    fn an_untaken_recovery_clears_the_hints_after_start() {
        let cell = RecoveryHintsCell::new();
        cell.set_anchor(Some(HOME));
        cell.note_restart();
        cell.startup_complete();
        assert!(!cell.pending());
        assert_eq!(cell.anchor(), None);
    }
}
