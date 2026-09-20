//! Rust-owned `Modals.close` / `closeIfOpen` wait.
//!
//! JavaScript marshals the call and queues the one returned `close-modal`.
//! Capture, the 3000ms identity wait, pause/hold freeze, reset/token abort
//! and the bool-vs-void result stay here. A queued close is not accepted.

use crate::isolate_fb::SnapshotReader;
use crate::task_clock::InstantTaskClock;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::time::{Duration, Instant};

/// Frozen `Modals.close` wait for `main !== before`.
pub const CLOSE_TIMEOUT_MS: u64 = 3_000;

thread_local! {
    static RUNTIME: RefCell<ModalsRuntime> = const { RefCell::new(ModalsRuntime::new()) };
    static NATIVE_OBSERVATION: RefCell<NativeObservation> =
        const { RefCell::new(NativeObservation::new()) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Close,
    CloseIfOpen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    WaitChange,
}

struct NativeObservation {
    ingame: bool,
    main_modal_id: i32,
}

impl NativeObservation {
    const fn new() -> Self {
        Self {
            ingame: false,
            main_modal_id: -1,
        }
    }

    fn update(&mut self, snap: &SnapshotReader<'_>) {
        if snap.has_ingame() {
            if !snap.ingame() {
                *self = Self::new();
                return;
            }
            self.ingame = true;
        }
        if snap.has_main_modal_id() {
            self.main_modal_id = snap.main_modal_id();
        }
    }
}

struct ModalsRuntime {
    clock: InstantTaskClock,
    token: u64,
    phase: Phase,
    kind: Kind,
    before: i32,
}

impl ModalsRuntime {
    const fn new() -> Self {
        Self {
            clock: InstantTaskClock::new(),
            token: 0,
            phase: Phase::Idle,
            kind: Kind::Close,
            before: -1,
        }
    }

    fn frozen(&self) -> bool {
        self.clock.frozen()
    }

    fn now(&self) -> Instant {
        self.clock.now()
    }

    fn set_freeze(&mut self, paused: bool, held: bool) {
        self.clock.set_freeze(paused, held);
    }

    fn arm(&mut self, window: u64) {
        self.clock.arm(window);
    }

    fn bound_reached(&self) -> bool {
        self.clock.bound_reached()
    }

    fn abort_runtime(&mut self) {
        self.token = self.token.wrapping_add(1);
        self.phase = Phase::Idle;
        self.kind = Kind::Close;
        self.before = -1;
        self.clock.deadline = None;
    }

    fn done(&mut self, result: bool, reason: &str) -> Value {
        let token = self.token;
        let kind = self.kind;
        self.phase = Phase::Idle;
        self.clock.deadline = None;
        match kind {
            Kind::CloseIfOpen => json!({
                "kind": "done",
                "token": token,
                "reason": reason,
            }),
            Kind::Close => json!({
                "kind": "done",
                "token": token,
                "result": result,
                "reason": reason,
            }),
        }
    }

    fn wait(&self) -> Value {
        json!({ "kind": "wait", "token": self.token })
    }

    fn close_verb(&self) -> Value {
        json!({ "kind": "close-modal", "token": self.token })
    }
}

pub fn on_snapshot(snap: &SnapshotReader<'_>) {
    NATIVE_OBSERVATION.with(|obs| obs.borrow_mut().update(snap));
}

pub fn on_pause() {
    RUNTIME.with(|rt| {
        let held = rt.borrow().clock.held;
        rt.borrow_mut().set_freeze(true, held);
    });
}

pub fn on_resume() {
    RUNTIME.with(|rt| {
        let held = rt.borrow().clock.held;
        rt.borrow_mut().set_freeze(false, held);
    });
}

pub fn on_hold(held: bool) {
    RUNTIME.with(|rt| {
        let paused = rt.borrow().clock.paused;
        rt.borrow_mut().set_freeze(paused, held);
    });
}

pub fn on_reset() {
    RUNTIME.with(|rt| rt.borrow_mut().abort_runtime());
    NATIVE_OBSERVATION.with(|obs| *obs.borrow_mut() = NativeObservation::new());
}

pub fn dispatch(input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "begin" => begin(input),
        "next" => next(input.get("token").and_then(Value::as_u64).unwrap_or(0)),
        _ => json!({ "kind": "notImpl", "reason": "unknown op" }),
    }
}

fn begin(input: &Value) -> Value {
    let kind = match input.get("kind").and_then(Value::as_str).unwrap_or("") {
        "close" => Kind::Close,
        "closeIfOpen" => Kind::CloseIfOpen,
        _ => return json!({ "kind": "notImpl", "reason": "unknown modals op" }),
    };
    NATIVE_OBSERVATION.with(|o| {
        let obs = o.borrow();
        RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            rt.abort_runtime();
            rt.kind = kind;
            if !obs.ingame {
                return json!({ "kind": "aborted", "reason": "not ingame" });
            }
            let before = obs.main_modal_id;
            if before == -1 {
                return rt.done(true, "absent");
            }
            rt.before = before;
            rt.phase = Phase::WaitChange;
            rt.arm(CLOSE_TIMEOUT_MS);
            rt.close_verb()
        })
    })
}

fn next(token: u64) -> Value {
    NATIVE_OBSERVATION.with(|o| {
        let obs = o.borrow();
        RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            if token != rt.token || rt.phase == Phase::Idle {
                return json!({ "kind": "aborted", "token": rt.token });
            }
            if rt.frozen() {
                return rt.wait();
            }
            if !obs.ingame {
                return json!({ "kind": "aborted", "token": rt.token });
            }
            if obs.main_modal_id != rt.before {
                return rt.done(true, "changed");
            }
            if rt.bound_reached() {
                return rt.done(false, "timeout");
            }
            rt.wait()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observe(ingame: bool, main_modal_id: i32) {
        NATIVE_OBSERVATION.with(|slot| {
            *slot.borrow_mut() = NativeObservation {
                ingame,
                main_modal_id,
            };
        });
    }

    #[test]
    fn frozen_close_timeout_is_3000ms() {
        assert_eq!(CLOSE_TIMEOUT_MS, 3_000);
    }

    #[test]
    fn absent_close_is_true_without_a_verb() {
        on_reset();
        observe(true, -1);
        let step = dispatch(&json!({ "op": "begin", "kind": "close" }));
        assert_eq!(step["kind"], "done");
        assert_eq!(step["result"], true);
        assert_eq!(step["reason"], "absent");
    }

    #[test]
    fn present_close_emits_one_close_modal_and_settles_on_identity_change() {
        on_reset();
        observe(true, 6675);
        let begin = dispatch(&json!({ "op": "begin", "kind": "close" }));
        assert_eq!(begin["kind"], "close-modal");
        let token = begin["token"].as_u64().unwrap();

        observe(true, 6675);
        assert_eq!(
            dispatch(&json!({ "op": "next", "token": token }))["kind"],
            "wait"
        );

        observe(true, -1);
        let closed = dispatch(&json!({ "op": "next", "token": token }));
        assert_eq!(closed["kind"], "done");
        assert_eq!(closed["result"], true);

        on_reset();
        observe(true, 6675);
        let again = dispatch(&json!({ "op": "begin", "kind": "close" }));
        let token = again["token"].as_u64().unwrap();
        observe(true, 3824);
        let swapped = dispatch(&json!({ "op": "next", "token": token }));
        assert_eq!(swapped["result"], true);
        assert_eq!(
            dispatch(&json!({ "op": "next", "token": token }))["kind"],
            "aborted",
            "settled token must not emit another close"
        );
    }

    #[test]
    fn unchanged_root_times_out_false_without_reclose() {
        on_reset();
        observe(true, 6675);
        let begin = dispatch(&json!({ "op": "begin", "kind": "close" }));
        let token = begin["token"].as_u64().unwrap();
        RUNTIME.with(|rt| {
            let now = rt.borrow().now();
            rt.borrow_mut().clock.deadline = Some(now - Duration::from_millis(1));
        });
        let timed = dispatch(&json!({ "op": "next", "token": token }));
        assert_eq!(timed["kind"], "done");
        assert_eq!(timed["result"], false);
        assert_eq!(timed["reason"], "timeout");
        assert_eq!(
            dispatch(&json!({ "op": "next", "token": token }))["kind"],
            "aborted"
        );
    }

    #[test]
    fn close_if_open_absent_is_void_and_pause_reset_do_not_close() {
        on_reset();
        observe(true, -1);
        let absent = dispatch(&json!({ "op": "begin", "kind": "closeIfOpen" }));
        assert_eq!(absent["kind"], "done");
        assert!(absent.get("result").is_none());

        observe(true, 6675);
        let begin = dispatch(&json!({ "op": "begin", "kind": "closeIfOpen" }));
        assert_eq!(begin["kind"], "close-modal");
        let token = begin["token"].as_u64().unwrap();
        on_pause();
        observe(true, -1);
        assert_eq!(
            dispatch(&json!({ "op": "next", "token": token }))["kind"],
            "wait"
        );
        on_resume();
        on_hold(true);
        assert_eq!(
            dispatch(&json!({ "op": "next", "token": token }))["kind"],
            "wait"
        );
        on_hold(false);
        on_reset();
        assert_eq!(
            dispatch(&json!({ "op": "next", "token": token }))["kind"],
            "aborted"
        );
    }
}
