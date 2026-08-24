//! Per-uid script runner. `SlotScript` owns at most one compiled `Script`
//! and gates it on operator intent (`want_run`) and client presence
//! (`on_is_up`). `tick` runs on the caller's pump at a game-tick edge and
//! must return; panics are caught, never abort the process.

use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::ctx::{Script, ScriptCtx};

/// Lifecycle of the script slot. `paused` covers both operator Pause and
/// the not-`is_up` gate; `stopping` is the Load-join window (later task).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Idle,
    Running,
    Paused,
    Stopping,
    Error,
}

/// Per-uid runner. Compiled XOR (later) Load. This task: compiled only.
pub struct SlotScript {
    pub want_run: bool,
    state: RunState,
    compiled: Option<Box<dyn Script>>,
    last_error: Option<String>,
    /// Dispatched game ticks since the last Start.
    ticks: u64,
}

impl SlotScript {
    pub fn new() -> Self {
        SlotScript {
            want_run: false,
            state: RunState::Idle,
            compiled: None,
            last_error: None,
            ticks: 0,
        }
    }

    /// Install a compiled script and start it. Refuses (no silent replace)
    /// while Running, Paused, or Stopping; allowed from Idle and Error
    /// (a fresh Start clears the previous error).
    pub fn start_compiled(&mut self, script: Box<dyn Script>) -> Result<(), String> {
        match self.state {
            RunState::Running | RunState::Paused | RunState::Stopping => {
                Err("script already active: stop it first".to_string())
            }
            RunState::Idle | RunState::Error => {
                self.compiled = Some(script);
                self.want_run = true;
                self.last_error = None;
                self.ticks = 0;
                self.state = RunState::Running;
                Ok(())
            }
        }
    }

    /// Operator Pause: `want_run` stays false (survives login) until
    /// Resume. Instance kept. No-op when there is no instance.
    pub fn pause(&mut self) {
        self.want_run = false;
        if self.compiled.is_some() && self.state == RunState::Running {
            self.state = RunState::Paused;
        }
    }

    /// Operator Resume: `want_run` back on. Assumes the client is up; the
    /// next `on_is_up(false)` re-gates if it is not. No-op when there is
    /// no instance or the slot errored.
    pub fn resume(&mut self) {
        self.want_run = true;
        if self.compiled.is_some() && self.state == RunState::Paused {
            self.state = RunState::Running;
        }
    }

    /// Operator Stop: run the teardown hook, drop the instance, Idle.
    pub fn stop(&mut self) {
        if let Some(mut script) = self.compiled.take() {
            script.on_stop();
        }
        self.want_run = false;
        self.state = RunState::Idle;
    }

    /// Recompute the gate from client presence. With an instance, the slot
    /// is Running only when `up && want_run`; every other combination is
    /// Paused. Without an instance the state is untouched (Idle, or Error
    /// after a panic — `is_up` must not resurrect or wipe an error).
    pub fn on_is_up(&mut self, up: bool) {
        if self.compiled.is_none() {
            return;
        }
        self.state = if up && self.want_run {
            RunState::Running
        } else {
            RunState::Paused
        };
    }

    /// Call only on observed server tick. Dispatches the script's `tick`
    /// only while Running && want_run. A panic is caught: the slot goes
    /// Error with the message, the instance is dropped, the run is over.
    pub fn on_game_tick(&mut self, ctx: &mut ScriptCtx<'_>) {
        if self.state != RunState::Running || !self.want_run {
            return;
        }
        let Some(script) = self.compiled.as_mut() else {
            return;
        };
        self.ticks += 1;
        let result = catch_unwind(AssertUnwindSafe(|| script.tick(ctx)));
        if let Err(payload) = result {
            self.last_error = Some(format!("script panic: {}", panic_message(&payload)));
            self.state = RunState::Error;
            self.want_run = false;
            self.compiled = None;
        }
    }

    pub fn state(&self) -> RunState {
        self.state
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
}

/// Best-effort panic payload to string. Downcasts the usual `&str` and
/// `String` payloads; also unwraps a re-boxed `Box<dyn Any + Send>` payload,
/// the shape `catch_unwind` can re-arm in some unwind runtimes.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        return (*s).to_string();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    if let Some(inner) = payload.downcast_ref::<Box<dyn std::any::Any + Send>>() {
        if let Some(s) = inner.downcast_ref::<&str>() {
            return (*s).to_string();
        }
        if let Some(s) = inner.downcast_ref::<String>() {
            return s.clone();
        }
    }
    "(no message)".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::test_support::NullDriver;

    struct Noop;

    impl Script for Noop {
        fn name(&self) -> &str {
            "noop"
        }
        fn tick(&mut self, _ctx: &mut ScriptCtx<'_>) {}
    }

    #[test]
    fn ticks_counts_dispatched_ticks_since_start() {
        let mut s = SlotScript::new();
        s.start_compiled(Box::new(Noop)).unwrap();
        let mut d = NullDriver::default();
        s.on_game_tick(&mut ScriptCtx { driver: &mut d, tick: 1, here: None, walk: None });
        s.on_game_tick(&mut ScriptCtx { driver: &mut d, tick: 2, here: None, walk: None });
        assert_eq!(s.ticks, 2);

        // Paused ticks do not count.
        s.pause();
        s.on_game_tick(&mut ScriptCtx { driver: &mut d, tick: 3, here: None, walk: None });
        assert_eq!(s.ticks, 2);

        // A fresh Start resets the counter.
        s.stop();
        s.start_compiled(Box::new(Noop)).unwrap();
        s.on_game_tick(&mut ScriptCtx { driver: &mut d, tick: 4, here: None, walk: None });
        assert_eq!(s.ticks, 1);
    }
}
