//! The slot: `crates/script/src/slot.rs` as far as the run lifecycle reads
//! it. Start, Pause, Resume, Stop, the observe step, the stop completion and
//! the progress watchdog's restart. LD1 section 2.2 items 3 and 11 to 14 and
//! 20 name the functions; each helper below says which one it follows.

use super::vm::Cp;
use super::{
    After, Card, Comp, Event, HookSec, Inv, Kind, Latch, Lifecycle, LoadHow, LoadSec, Msg,
    Mutation, Owner, Reason, Receipt, Run, RunState, Sec, State, StopReason, Violation, Vm, Wait,
    Who,
};

impl Lifecycle {
    // ---- Start ----

    /// `start_surface`. A Start on an Idle slot spawns at once; a Start while
    /// the slot is `Stopping` is queued (`AfterStop::Start`, 2.2 item 20); any
    /// other state refuses it.
    pub(super) fn start(
        &self,
        s: &mut State,
        who: Who,
        ev: &mut Vec<Event>,
    ) -> Result<(), Violation> {
        match s.slot.state {
            RunState::Idle => {
                s.n.starts += 1;
                self.admit(s, who);
                self.begin_load(s, LoadHow::Operator { who }, ev)
            }
            RunState::Stopping if s.slot.after == After::Idle => {
                s.n.starts += 1;
                self.admit(s, who);
                let slot = &mut s.slot;
                slot.after = After::Start;
                // The generation moves now, not when the reap finishes.
                slot.base = Some(slot.generation);
                slot.generation = slot.generation.wrapping_add(1);
                slot.want_run = true;
                slot.receipt = Receipt::None;
                slot.card = Card {
                    reason: Reason::None,
                    hook: false,
                };
                slot.diag = false;
                ev.push(Event::StartQueued);
                Ok(())
            }
            _ => {
                ev.push(Event::StartRefused);
                Ok(())
            }
        }
    }

    /// `spawn_surface`: the Load entry runs inside it (rule X9).
    pub(super) fn begin_load(
        &self,
        s: &mut State,
        how: LoadHow,
        ev: &mut Vec<Event>,
    ) -> Result<(), Violation> {
        s.n.runs += 1;
        let mut run = Run::new(s.n.runs, s.slot.serial);
        if !self.claim_dock(s, &mut run, how)? {
            // The claim fails before any script code runs: no VM exists.
            run.vm = false;
            return self.spawn_failed(s, run, how, ev);
        }
        s.sec = Some(Sec::Load(LoadSec {
            vm: Vm::new(Kind::Load),
            run,
            how,
        }));
        self.arm(s, Owner::Load, self.bounds.b_load);
        self.check(s, Cp::Entry, ev)?;
        Ok(())
    }

    /// `spawn` returned the run: it is `Starting` until its first
    /// `poll_ready`.
    pub(super) fn spawn_succeeded(
        &self,
        s: &mut State,
        run: Run,
        how: LoadHow,
        ev: &mut Vec<Event>,
    ) -> Result<(), Violation> {
        let slot = &mut s.slot;
        slot.state = RunState::Starting;
        slot.ready = false;
        slot.after = After::Idle;
        slot.receipt = Receipt::None;
        slot.card = Card {
            reason: Reason::None,
            hook: false,
        };
        slot.diag = false;
        match how {
            LoadHow::Operator { .. } => {
                slot.base = Some(slot.generation);
                slot.generation = slot.generation.wrapping_add(1);
                slot.want_run = true;
            }
            LoadHow::Restart => {
                slot.base = Some(slot.generation);
                slot.generation = slot.generation.wrapping_add(1);
            }
            LoadHow::Queued => {}
        }
        s.run = Some(run);
        ev.push(match how {
            LoadHow::Queued => Event::QueuedStartSpawned,
            LoadHow::Restart => Event::Restarted,
            LoadHow::Operator { .. } => Event::Spawned,
        });
        self.ledger_spawned(s);
        Ok(())
    }

    /// `spawn` returned an error. The VM is dropped inside it. An operator
    /// Start returns the error; a queued Start or a restart goes to
    /// `fail_setup`, which gives the generation back (2.2 item 20).
    pub(super) fn spawn_failed(
        &self,
        s: &mut State,
        mut run: Run,
        how: LoadHow,
        ev: &mut Vec<Event>,
    ) -> Result<(), Violation> {
        if run.vm && self.is(Mutation::FailedLoadLeak) {
            s.leaked += 1;
        }
        if run.dock.claimed {
            // A failed Load after the claim writes a terminal end (LD2 2.2).
            self.close_dock(s, &mut run, None)?;
        }
        self.ledger_failed_start(s, run.serial);
        ev.push(Event::SpawnFailed);
        match how {
            LoadHow::Operator { .. } => {}
            LoadHow::Queued | LoadHow::Restart => {
                let slot = &mut s.slot;
                if let Some(base) = slot.base.take() {
                    slot.generation = base;
                }
                slot.want_run = false;
                slot.state = RunState::Idle;
                slot.after = After::Idle;
                slot.diag = true;
            }
        }
        Ok(())
    }

    // ---- Pause and Resume ----

    /// `SlotScript::pause`.
    pub(super) fn pause(&self, s: &mut State, ev: &mut Vec<Event>) {
        s.n.ctl += 1;
        s.slot.want_run = false;
        if s.run.is_some() && matches!(s.slot.state, RunState::Running | RunState::Starting) {
            if let Some(run) = s.run.as_mut() {
                run.paused = true;
            }
            s.slot.state = RunState::Paused;
        }
        ev.push(Event::Paused);
    }

    /// `SlotScript::resume`. It does not enter the VM (I3, rule X7).
    pub(super) fn resume(&self, s: &mut State, ev: &mut Vec<Event>) {
        s.n.ctl += 1;
        s.slot.want_run = true;
        if s.run.is_some() && s.slot.state == RunState::Paused {
            if let Some(run) = s.run.as_mut() {
                run.paused = false;
            }
            s.slot.state = if s.slot.ready {
                RunState::Running
            } else {
                RunState::Starting
            };
        }
        ev.push(Event::Resumed);
    }

    // ---- Stop ----

    pub(super) fn operator_stop(
        &self,
        s: &mut State,
        ev: &mut Vec<Event>,
    ) -> Result<(), Violation> {
        s.n.ctl += 1;
        self.stop_call(s, StopReason::Operator, ev)
    }

    pub(super) fn remove(&self, s: &mut State, ev: &mut Vec<Event>) -> Result<(), Violation> {
        s.n.ctl += 1;
        self.stop_call(s, StopReason::Removed, ev)?;
        if s.sec.is_none() {
            s.gone = true;
        }
        Ok(())
    }

    /// Raw `Drop` of the slot: it stops the run and drops the receiver.
    pub(super) fn drop_slot(&self, s: &mut State, ev: &mut Vec<Event>) -> Result<(), Violation> {
        s.n.ctl += 1;
        if s.run.is_some() {
            self.begin_stop(s, StopReason::Drop, ev)?;
        }
        if s.sec.is_none() {
            s.gone = true;
        }
        Ok(())
    }

    /// `stop_with_reason`. A Stop while the slot is `Stopping` drops a queued
    /// Start or restart and the reap ends Idle (2.2 item 20); otherwise the
    /// run stops.
    pub(super) fn stop_call(
        &self,
        s: &mut State,
        reason: StopReason,
        ev: &mut Vec<Event>,
    ) -> Result<(), Violation> {
        if s.slot.state == RunState::Stopping {
            let dropped = s.slot.after != After::Idle;
            if !self.is(Mutation::StopKeepsQueuedStart) {
                s.slot.after = After::Idle;
            }
            s.slot.base = None;
            s.slot.want_run = false;
            s.slot.generation = s.slot.generation.wrapping_add(1);
            if dropped {
                ev.push(Event::QueuedStartDropped);
                self.ledger_dropped(s);
            }
            if s.slot.after != After::Idle {
                return Err(Self::violation(
                    "p1-stop-kept-queued-start",
                    format!(
                        "a Stop while the slot is Stopping left AfterStop::{:?}; the replacement will start",
                        s.slot.after
                    ),
                ));
            }
            return Ok(());
        }
        if s.run.is_some() {
            return self.begin_stop(s, reason, ev);
        }
        // `finish_idle_stop`: no run; the generation still moves.
        s.slot.want_run = false;
        s.slot.generation = s.slot.generation.wrapping_add(1);
        Ok(())
    }

    /// Whether the run's `stop` runs `onStop` (rule X8 step 2).
    fn hook_due(&self, run: &Run) -> bool {
        let entry = run.hook || self.is(Mutation::HookRunsWithoutEntry);
        let base = run.vm && entry && run.hook_fail.is_none();
        let latch_ok = matches!(run.latch, Latch::Empty | Latch::SelfStop)
            || self.is(Mutation::OnStopAfterTrip);
        base && latch_ok && !(self.is(Mutation::HookSkippedWhilePaused) && run.paused)
    }

    /// The slot's receipt and card text follow from the latch of the run it
    /// stops: the first terminal cause decides (4.3, I11 b). LD1 4.3 gives
    /// the rows for a dispatch failure and for `api.stop` seen by the log
    /// drain; this model applies the same rule to every Stop, because an
    /// operator Stop can overtake the drain (finding F2).
    fn write_receipt(&self, s: &mut State, reason: StopReason) {
        let Some(run) = s.run.as_ref() else {
            return;
        };
        let latch = run.latch;
        match latch {
            Latch::SelfStop => {
                if reason == StopReason::ScriptStop || !self.is(Mutation::F2StopIgnoresSelfStop) {
                    s.slot.receipt = Receipt::Stopped;
                    s.slot.card.reason = Reason::SelfStop;
                } else {
                    s.slot.card = Card {
                        reason: Reason::None,
                        hook: false,
                    };
                }
            }
            Latch::Trip(_) => {
                s.slot.receipt = Receipt::Failed;
                s.slot.card.reason = Reason::Trip;
            }
            Latch::Empty => {
                if reason != StopReason::Restart {
                    s.slot.card = Card {
                        reason: Reason::None,
                        hook: false,
                    };
                }
            }
        }
    }

    /// `begin_async_stop` and the run's `stop` (rule X8): the receipt, then
    /// the hook if it is due, then the drop of the VM.
    pub(super) fn begin_stop(
        &self,
        s: &mut State,
        reason: StopReason,
        ev: &mut Vec<Event>,
    ) -> Result<(), Violation> {
        let Some(run) = s.run.as_ref() else {
            return Err(Self::violation("model", "a stop with no run"));
        };
        if reason == StopReason::Restart && run.latch == Latch::SelfStop {
            return Err(Self::violation(
                "i11b-selfstop-overridden",
                "the run called `api.stop` and the progress watchdog restarted it: the replacement runs",
            ));
        }
        self.write_receipt(s, reason);
        let Some(run) = s.run.as_mut() else {
            return Err(Self::violation("model", "a stop with no run"));
        };
        // I11 b: the first terminal cause decides the receipt.
        let wanted = match run.latch {
            Latch::SelfStop => Some(Receipt::Stopped),
            Latch::Trip(_) => Some(Receipt::Failed),
            Latch::Empty => None,
        };
        if let Some(wanted) = wanted
            && s.slot.receipt != wanted
        {
            return Err(Self::violation(
                "i11b-wrong-receipt",
                format!(
                    "the run's first cause is {:?}, so the receipt is {wanted:?}; the slot wrote {:?} (reason {reason:?})",
                    run.latch, s.slot.receipt
                ),
            ));
        }
        run.stopping = true;
        let expected = run.vm
            && run.hook
            && matches!(run.latch, Latch::Empty | Latch::SelfStop)
            && run.hook_fail.is_none();
        let due = self.hook_due(run);
        if expected && !due {
            return Err(Self::violation(
                "i5-hook-skipped",
                format!(
                    "onStop did not run for a loaded script that has it, latch {:?}, paused {}",
                    run.latch, run.paused
                ),
            ));
        }
        if due && !expected {
            let name = if run.latch.is_trip() {
                "i5-hook-after-trip"
            } else {
                "i5-hook-unexpected"
            };
            return Err(Self::violation(
                name,
                format!("onStop ran with latch {:?}, vm {}", run.latch, run.vm),
            ));
        }
        if !due {
            ev.push(Event::HookSkipped);
            return self.finish_stop(s, reason, ev);
        }
        run.hook_runs += 1;
        ev.push(Event::HookRan);
        s.sec = Some(Sec::Hook(HookSec {
            vm: Vm::new(Kind::Hook),
            reason,
        }));
        self.arm(s, Owner::Hook, self.bounds.b_hook);
        if self.check(s, Cp::Entry, ev)? {
            // The check at the entry failed (rule C2, check point 1): the run
            // does not enter, and the failure is the hook's.
            self.disarm(s);
            s.sec = None;
            return self.finish_stop(s, reason, ev);
        }
        Ok(())
    }

    /// The VM is dropped, the completion is sent once, the slot's bookkeeping
    /// follows (`begin_async_stop`, I4, I9).
    pub(super) fn finish_stop(
        &self,
        s: &mut State,
        reason: StopReason,
        ev: &mut Vec<Event>,
    ) -> Result<(), Violation> {
        let Some(mut run) = s.run.take() else {
            return Err(Self::violation("model", "a stop finished with no run"));
        };
        if self.is(Mutation::CheckHookTwice) && run.hook_runs == 1 {
            run.hook_runs += 1;
        }
        if run.hook_runs > 1 {
            return Err(Self::violation(
                "i5-hook-twice",
                format!("onStop started {} times", run.hook_runs),
            ));
        }
        run.vm = false;
        let hook_fail = if self.is(Mutation::HookFailureLost) {
            None
        } else {
            run.hook_fail
        };
        if hook_fail != run.hook_fail {
            return Err(Self::violation(
                "i12-failure-lost",
                format!(
                    "the failure of onStop ({:?}) is not in the completion that `stop` sent",
                    run.hook_fail
                ),
            ));
        }
        run.sends += if self.is(Mutation::DoneTwice) { 2 } else { 1 };
        if run.sends != 1 {
            return Err(Self::violation(
                "i4-done-count",
                format!("`stop` sent {} messages on `done`", run.sends),
            ));
        }
        if !reason.receiver_gone() && !self.is(Mutation::CheckDoneDropped) {
            s.slot.chan = Some(Msg { hook_fail });
        }
        if self.is(Mutation::RowsAfterStop) {
            s.orphan_rows = run.rows;
        }
        self.close_dock(s, &mut run, hook_fail)?;
        self.ledger_end(s, &run, reason, ev)?;
        s.host.comp = None;
        ev.push(Event::Stopped(reason));
        if reason.receiver_gone() {
            s.gone = true;
            if self.is(Mutation::CheckRemoveKeepsVm) {
                // The slot is gone and its run, with the VM, is not dropped.
                run.vm = true;
                s.run = Some(run);
            }
            return Ok(());
        }
        if self.is(Mutation::CheckStopKeepsRun) {
            s.run = Some(run);
        }
        let slot = &mut s.slot;
        slot.state = RunState::Stopping;
        slot.ready = false;
        slot.base = None;
        match reason {
            StopReason::Restart => slot.after = After::Restart,
            _ => {
                slot.after = After::Idle;
                slot.generation = slot.generation.wrapping_add(1);
                slot.want_run = false;
            }
        }
        if reason == StopReason::ScriptStop {
            // `drain_logs` calls `observe_lifecycle` again after `stop`.
            self.observe_prefix(s, ev)?;
        }
        Ok(())
    }

    // ---- the observe step ----

    /// `observe_surface_lifecycle`: the first `poll_ready`, then the stop
    /// completion and `complete_stop`.
    pub(super) fn observe_prefix(
        &self,
        s: &mut State,
        ev: &mut Vec<Event>,
    ) -> Result<(), Violation> {
        if s.run.is_some() && !s.slot.ready {
            s.slot.ready = true;
            s.slot.base = None;
            if s.slot.state == RunState::Starting {
                if s.slot.want_run {
                    s.slot.state = RunState::Running;
                } else {
                    if let Some(run) = s.run.as_mut() {
                        run.paused = true;
                    }
                    s.slot.state = RunState::Paused;
                }
            }
        }
        if let Some(msg) = s.slot.chan.take() {
            let receipt = s.slot.receipt;
            if msg.hook_fail.is_some() {
                s.slot.hook_logs += 1;
                s.slot.card.hook = true;
                if self.is(Mutation::HookFailureReplacesReceipt) {
                    s.slot.receipt = Receipt::Failed;
                }
            }
            if s.slot.receipt != receipt {
                return Err(Self::violation(
                    "i12-receipt-replaced",
                    format!(
                        "the failure of onStop changed the receipt from {receipt:?} to {:?}",
                        s.slot.receipt
                    ),
                ));
            }
            self.complete_stop(s, ev)?;
        }
        Ok(())
    }

    /// `complete_stop`: the next state is the one that `AfterStop` selects
    /// (P1).
    fn complete_stop(&self, s: &mut State, ev: &mut Vec<Event>) -> Result<(), Violation> {
        let after = s.slot.after;
        s.slot.chan = None;
        s.slot.after = After::Idle;
        ev.push(Event::Retired(after));
        match after {
            After::Idle => {
                s.slot.state = RunState::Idle;
                s.slot.want_run = false;
                Ok(())
            }
            After::Start if self.is(Mutation::QueuedStartLost) => {
                s.slot.state = RunState::Idle;
                s.slot.want_run = false;
                Err(Self::violation(
                    "p1-queued-start-lost",
                    "complete_stop ended Idle with no diagnostic; a Start was queued behind the Stop",
                ))
            }
            After::Start => self.begin_load(s, LoadHow::Queued, ev),
            After::Restart => self.begin_load(s, LoadHow::Restart, ev),
        }
    }

    /// `observe_lifecycle` and `drain_logs` (2.2 items 12 and 13): the
    /// stop request of the script starts the Stop (4.3).
    pub(super) fn observe(&self, s: &mut State, ev: &mut Vec<Event>) -> Result<(), Violation> {
        self.observe_prefix(s, ev)?;
        if s.sec.is_some() {
            return Ok(());
        }
        let stopped = s
            .run
            .as_ref()
            .is_some_and(|run| run.latch == Latch::SelfStop && !run.stopping);
        if stopped {
            self.begin_stop(s, StopReason::ScriptStop, ev)?;
        }
        Ok(())
    }

    /// The progress watchdog's restart (`restart_from_identity`).
    pub(super) fn restart(&self, s: &mut State, ev: &mut Vec<Event>) -> Result<(), Violation> {
        s.n.restarts += 1;
        s.n.starts += 1;
        self.admit(s, Who::Operator);
        if self.is(Mutation::SpawnBeforeRetire) {
            // The earlier design: spawn the replacement, then retire the old
            // run.
            return self.begin_load(s, LoadHow::Restart, ev);
        }
        self.begin_stop(s, StopReason::Restart, ev)
    }

    // ---- the client, the host and the seat ----

    /// `on_is_up`: with a run, the slot is Running only when the client is up
    /// and the operator wants it to run (2.2 item 7).
    pub(super) fn client_gate(&self, s: &mut State, up: bool, ev: &mut Vec<Event>) {
        s.n.gates += 1;
        s.slot.up = up;
        if s.run.is_some() && !matches!(s.slot.state, RunState::Starting | RunState::Stopping) {
            s.slot.state = if up && s.slot.want_run {
                RunState::Running
            } else {
                RunState::Paused
            };
        }
        ev.push(Event::Gated);
    }

    /// The completion of the awaited request reaches the host's facts.
    pub(super) fn arrive(&self, s: &mut State) {
        let Some(run) = s.run.as_mut() else {
            return;
        };
        let comp = match run.inv {
            Inv::Suspended(Wait::Seat { typed }) => Comp::Terminal { typed },
            Inv::Suspended(Wait::Walk) => Comp::Walk,
            _ => return,
        };
        if self.is(Mutation::LostWake) && run.paused {
            // The completion arrives during Pause and the host drops it.
            run.arrived = true;
            run.consumed = false;
            return;
        }
        s.host.comp = Some(comp);
        run.arrived = true;
        run.consumed = false;
    }

    /// `reset_session_work`: the awaited seat request ends with a typed abort;
    /// an extension wait is not session work (LD2 4.4 item 7).
    pub(super) fn session_reset(&self, s: &mut State) -> Result<(), Violation> {
        s.n.resets += 1;
        let Some(run) = s.run.as_mut() else {
            return Ok(());
        };
        match run.inv {
            Inv::Suspended(Wait::Seat { .. } | Wait::Walk) if !run.arrived => {
                run.arrived = true;
                run.consumed = false;
                s.host.comp = Some(Comp::Abort);
            }
            Inv::Suspended(Wait::Ext) if self.is(Mutation::SessionCancelsExtensionWait) => {
                return Err(Self::violation(
                    "d-session-cancels-extension-wait",
                    "a session reset ended the extension wait of the run",
                ));
            }
            _ => {}
        }
        Ok(())
    }

    /// The seat drains the rows that the run queued. The head admits them in
    /// the dispatch (LD1 4.2 rule 2), so this step exists only for the
    /// designs that did not.
    pub(super) fn admit_rows(&self, s: &mut State) -> Result<(), Violation> {
        if s.orphan_rows > 0 {
            s.orphan_rows = 0;
            return Err(Self::violation(
                "i4-row-after-stop",
                "a row of the last dispatch reached the seat after the Stop began",
            ));
        }
        if let Some(run) = s.run.as_mut()
            && run.rows > 0
        {
            if run.fenced {
                return Err(Self::violation(
                    "f-row-after-fence",
                    "the seat admitted a row of a run whose entry fence is published",
                ));
            }
            run.rows = 0;
        }
        Ok(())
    }
}
