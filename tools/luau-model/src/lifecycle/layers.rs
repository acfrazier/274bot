//! The runner side that the lifecycle must stay compatible with (LD2
//! `design-r7.md`): the scenario dock, the entry fence, the execution ledger
//! with its retirement tickets, and the runner's final gate. Each layer is off
//! unless the profile turns it on (`Bounds::dock`, `fence`, `runner`).
//!
//! The runner never runs script code and never holds the slot while the VM
//! runs: it posts an owner intent, and the role pump executes it on the slot
//! side (LD2 3.5 and 5.1 phase D). A model step that begins a VM section
//! under the runner lock is the violation of the mutations that LD2 rejects.

use super::{
    Action, Call, End, Event, Exec, ExecState, Intent, Kind, Latch, Lifecycle, LoadHow, Mutation,
    Run, RunState, RunnerPhase, Sec, State, StopReason, Ticket, Verdict, Violation, Wait, Who,
};

impl Lifecycle {
    // ---- the execution ledger (LD2 3.2) ----

    /// An admitted Start, queued Start or restart: the slot's Start serial
    /// advances before the spawn, and the ledger records the admission. A
    /// setup that fails later does not give the serial back.
    pub(super) fn admit(&self, s: &mut State, who: Who) {
        s.slot.serial = s.slot.serial.wrapping_add(1);
        if !self.bounds.runner {
            return;
        }
        let serial = s.slot.serial;
        let planned = who == Who::Runner;
        if !planned {
            s.runner.foreign_start = true;
        }
        let idx = usize::from(serial).saturating_sub(1);
        if self.is(Mutation::ForgetEarlierExecution) {
            // One mutable field: the later admission replaces the earlier.
            s.runner.ledger.fill(None);
        }
        if idx < s.runner.ledger.len() && !self.is(Mutation::LoadingMissingFromLedger) {
            s.runner.ledger[idx] = Some(Exec {
                serial,
                planned,
                state: ExecState::Live,
                clean: true,
                natural: false,
                generation: 0,
            });
        }
    }

    pub(super) fn ledger_spawned(&self, s: &mut State) {
        if !self.bounds.runner {
            return;
        }
        let (serial, planned) = match s.run.as_ref() {
            Some(run) => (run.serial, s.runner.pending_planned),
            None => return,
        };
        let generation = s.slot.generation;
        let idx = usize::from(serial).saturating_sub(1);
        if idx >= s.runner.ledger.len() {
            return;
        }
        match &mut s.runner.ledger[idx] {
            Some(exec) => exec.generation = generation,
            None => {
                // The earlier design records the admission at Ready.
                s.runner.ledger[idx] = Some(Exec {
                    serial,
                    planned,
                    state: ExecState::Live,
                    clean: true,
                    natural: false,
                    generation,
                });
            }
        }
    }

    pub(super) fn ledger_failed_start(&self, s: &mut State, serial: u8) {
        if !self.bounds.runner {
            return;
        }
        let idx = usize::from(serial).saturating_sub(1);
        if let Some(Some(exec)) = s.runner.ledger.get_mut(idx) {
            exec.state = ExecState::FailedStart;
        }
    }

    pub(super) fn ledger_dropped(&self, s: &mut State) {
        if !self.bounds.runner {
            return;
        }
        let idx = usize::from(s.slot.serial).saturating_sub(1);
        if let Some(Some(exec)) = s.runner.ledger.get_mut(idx)
            && exec.state == ExecState::Live
        {
            exec.state = ExecState::Dropped;
        }
    }

    /// A run ended. A Stop that the runner issued against this execution is
    /// ticketed; any other end is an end that the runner did not order.
    pub(super) fn ledger_end(
        &self,
        s: &mut State,
        run: &Run,
        reason: StopReason,
        ev: &mut Vec<Event>,
    ) -> Result<(), Violation> {
        if !self.bounds.runner {
            return Ok(());
        }
        let idx = usize::from(run.serial).saturating_sub(1);
        let clean = run.latch == Latch::Empty && run.hook_fail.is_none() && !run.dock.fault;
        // A clean natural self-stop (LD2 4.1, T30): the script's own `api.stop`,
        // read by the slot, with no failure of `onStop` and no first fault.
        let natural = self.bounds.natural
            && reason == StopReason::ScriptStop
            && run.latch == Latch::SelfStop
            && run.hook_fail.is_none()
            && !run.dock.fault;
        let issued = s.runner.issuing == Some(run.serial) && reason == StopReason::Operator;
        s.runner.issuing = None;
        let gen_before = s.slot.generation;
        let Some(Some(exec)) = s.runner.ledger.get_mut(idx) else {
            return Ok(());
        };
        exec.clean = clean;
        exec.natural = natural;
        if issued {
            s.slot.stops = s.slot.stops.wrapping_add(1);
            exec.state = ExecState::Ticketed(Ticket {
                serial: run.serial,
                ordinal: s.slot.stops,
                gen_before,
                gen_after: gen_before.wrapping_add(1),
                start_serial: s.slot.serial,
            });
            ev.push(Event::Ticketed);
        } else {
            exec.state = ExecState::Ended;
            if exec.planned && !natural {
                s.runner.foreign_end = true;
            }
        }
        Ok(())
    }

    /// A naturally ended last execution that the idle slot has not retired
    /// (LD2 3.2): the runner still owes it the one ordinary housekeeping Stop.
    fn housekeeping_due(&self, s: &State, exec: &Exec) -> bool {
        self.bounds.natural
            && exec.planned
            && exec.natural
            && exec.state == ExecState::Ended
            && exec.serial == s.slot.serial
            && s.run.is_none()
            && s.sec.is_none()
            && s.slot.state == RunState::Idle
    }

    fn housekeeping_target(&self, s: &State, serial: u8) -> bool {
        let idx = usize::from(serial).saturating_sub(1);
        matches!(s.runner.ledger.get(idx), Some(Some(exec)) if self.housekeeping_due(s, exec))
    }

    /// With no run bound the entry fence has nothing to close. After a natural
    /// end the runner still publishes it before its final gate.
    fn natural_fence_target(&self, s: &State) -> bool {
        self.bounds.natural
            && s.runner
                .ledger
                .iter()
                .flatten()
                .any(|exec| exec.planned && exec.natural && exec.state == ExecState::Ended)
    }

    /// The one ordinary Stop of the idle slot after a natural self-stop. The
    /// generation moves, and the slot issues the ticket. A forced Stop never
    /// makes this witness; this Stop only retires it.
    fn housekeeping_stop(
        &self,
        s: &mut State,
        serial: u8,
        ev: &mut Vec<Event>,
    ) -> Result<(), Violation> {
        let gen_before = s.slot.generation;
        self.stop_call(s, StopReason::Operator, ev)?;
        s.slot.stops = s.slot.stops.wrapping_add(1);
        let ticket = Ticket {
            serial,
            ordinal: s.slot.stops,
            gen_before,
            gen_after: s.slot.generation,
            start_serial: s.slot.serial,
        };
        let idx = usize::from(serial).saturating_sub(1);
        if let Some(Some(exec)) = s.runner.ledger.get_mut(idx) {
            exec.state = ExecState::Ticketed(ticket);
            ev.push(Event::Ticketed);
        }
        Ok(())
    }

    // ---- the dock (LD2 2.2 to 2.4) ----

    /// `claim` succeeds once, before any script code runs (LD2 2.2, the X9
    /// amendment). A fresh admitted Start gets another dock. A retained Start
    /// that a watchdog restart reuses cannot claim: the spawn fails with
    /// "this Start already ran" (`Ok(false)`).
    pub(super) fn claim_dock(
        &self,
        s: &State,
        run: &mut Run,
        how: LoadHow,
    ) -> Result<bool, Violation> {
        if !self.bounds.dock {
            return Ok(true);
        }
        if how == LoadHow::Restart {
            if self.is(Mutation::SharedRestartDock) {
                return Err(Self::violation(
                    "d-dock-shared",
                    format!(
                        "the restart claimed dock #{} again; the old run's requests reach the new VM",
                        s.n.runs.saturating_sub(1)
                    ),
                ));
            }
            return Ok(false);
        }
        run.dock.id = run.id;
        run.dock.claimed = true;
        Ok(true)
    }

    /// Stop builds the end record after the VM is dropped and before the
    /// completion is published (LD2 2.4, X8 amendment).
    pub(super) fn close_dock(
        &self,
        s: &mut State,
        run: &mut Run,
        hook_fail: Option<super::Fail>,
    ) -> Result<(), Violation> {
        if !self.bounds.dock {
            return Ok(());
        }
        let dock = &mut run.dock;
        dock.closed = true;
        if !dock.call.is_terminal() {
            dock.call = Call::Cancelled;
        }
        if !self.is(Mutation::RevokeKeepsPayload) {
            dock.payload = false;
        }
        let end_hook = if self.is(Mutation::HookFailureNotInEnd) {
            None
        } else {
            hook_fail
        };
        // LD2 2.4: the end record retains the first fault. The retained
        // history is the truth here, not the latch: a Revoke that cleared the
        // latch must not pass for a dock that never had a fault.
        let had_fault = dock.faults > 0;
        let fault = if self.is(Mutation::UnobservedRefusalLost) {
            false
        } else {
            dock.fault
        };
        if end_hook != hook_fail {
            return Err(Self::violation(
                "d-end-lacks-hook-failure",
                "the dock's end record does not carry the failure of onStop",
            ));
        }
        if fault != had_fault {
            return Err(Self::violation(
                "d-first-fault-lost",
                "the dock's end record lost the first fault that an entry latched",
            ));
        }
        dock.end = Some(End {
            hook_fail: end_hook,
            fault,
        });
        s.closed_dock = Some(*dock);
        Ok(())
    }

    /// The resume takes the payload once.
    pub(super) fn consume_dock(&self, run: &mut Run) {
        run.dock.delivered = true;
        if !self.is(Mutation::ConsumeTwice) {
            run.dock.call = Call::Consumed;
            run.dock.payload = false;
        }
    }

    /// The end of a dispatch moves a staged call: an accepted exit queues it,
    /// a failed or self-stopped exit discards it (LD2 2.3).
    pub(super) fn dock_dispatch_end(&self, s: &mut State, accepted: bool) -> Result<(), Violation> {
        if !self.bounds.dock {
            return Ok(());
        }
        let Some(run) = s.run.as_mut() else {
            return Ok(());
        };
        if let Call::Staged { awaited } = run.dock.call {
            if accepted && run.latch == Latch::Empty {
                run.dock.call = Call::Queued { awaited };
            } else if !accepted && self.is(Mutation::CommitFailedDispatch) {
                run.dock.call = Call::Queued { awaited };
                return Err(Self::violation(
                    "d-failed-dispatch-committed",
                    "a staged call of a failed dispatch was committed to the dock queue",
                ));
            } else {
                run.dock.call = Call::Discarded;
            }
        }
        Ok(())
    }

    /// An extension call from an entry. Only a Tick entry can issue one; a
    /// refused call latches the first fault before the error reaches Luau
    /// (LD2 2.4).
    pub(super) fn ext_call(
        &self,
        s: &mut State,
        kind: Kind,
        awaited: bool,
        ev: &mut Vec<Event>,
    ) -> Result<(), Violation> {
        if !self.bounds.dock {
            return Ok(());
        }
        let Some(run) = s.cur_mut() else {
            return Ok(());
        };
        if kind != Kind::Tick || run.dock.closed || run.dock.revoked {
            // The refused call enters the retained history first, before any
            // code of the dock acts: no later step can erase the record.
            run.dock.faults = run.dock.faults.saturating_add(1).min(2);
            if !(kind == Kind::Hook && self.is(Mutation::CaughtHookRefusalLost)) {
                run.dock.fault = true;
            } else {
                return Err(Self::violation(
                    "d-refusal-not-latched",
                    "an extension call in onStop was refused and left no first fault",
                ));
            }
            return Ok(());
        }
        if !run.dock.call.is_terminal() {
            // A second awaited operation while one owns the invocation.
            run.dock.faults = run.dock.faults.saturating_add(1).min(2);
            run.dock.fault = true;
            return Ok(());
        }
        run.dock.call = Call::Staged { awaited };
        if awaited {
            s.n.waits += 1;
            if let Some(run) = s.cur_mut() {
                run.arrived = false;
            }
            return self.end_entry_yielded(s, ev);
        }
        Ok(())
    }

    /// The offer of extension calls in an entry.
    pub(super) fn ext_actions(&self, s: &State, kind: Kind, run: &Run, out: &mut Vec<Action>) {
        if !self.bounds.dock {
            return;
        }
        let b = &self.bounds;
        if kind == Kind::Tick
            && !run.dock.closed
            && !run.dock.revoked
            && run.dock.call.is_terminal()
        {
            out.push(Action::Vm(super::Op::Call(super::Binding::Ext {
                awaited: false,
            })));
            if s.n.waits < b.waits {
                out.push(Action::Vm(super::Op::Call(super::Binding::Ext {
                    awaited: true,
                })));
            }
        } else if kind != Kind::Tick && !run.dock.fault {
            out.push(Action::Vm(super::Op::Call(super::Binding::Ext {
                awaited: false,
            })));
        }
    }

    pub(super) fn take(&self, s: &mut State) {
        if let Some(run) = s.run.as_mut()
            && let Call::Queued { awaited } = run.dock.call
        {
            run.dock.call = Call::Taken { awaited };
        }
    }

    /// The runner completes the operation that it took.
    pub(super) fn complete(&self, s: &mut State) -> Result<(), Violation> {
        if s.run.is_none() && self.is(Mutation::CallsAfterClose) && s.closed_dock.is_some() {
            return Err(Self::violation(
                "d-call-after-close",
                "the runner completed a call on a dock that already closed",
            ));
        }
        let Some(run) = s.run.as_mut() else {
            return Ok(());
        };
        match run.dock.call {
            Call::Taken { awaited: true } => {
                run.dock.call = Call::Completed;
                run.dock.payload = true;
                run.dock.delivered = false;
            }
            Call::Taken { awaited: false } => run.dock.call = Call::Done,
            Call::Completed if self.is(Mutation::CompletionOverwrite) && run.dock.payload => {
                return Err(Self::violation(
                    "d-completion-overwritten",
                    "`complete` wrote over a payload that was in the dock",
                ));
            }
            _ => {}
        }
        Ok(())
    }

    /// Revocation closes admissions, cancels completions and disposes queued
    /// requests. It keeps the first fault (LD2 2.4).
    pub(super) fn revoke(&self, s: &mut State, ev: &mut Vec<Event>) {
        if let Some(run) = s.run.as_mut() {
            run.dock.revoked = true;
            if matches!(
                run.dock.call,
                Call::Queued { .. } | Call::Taken { .. } | Call::Completed
            ) {
                run.dock.call = Call::Cancelled;
            }
            if !self.is(Mutation::RevokeKeepsPayload) {
                run.dock.payload = false;
            }
            if self.is(Mutation::RevokeClearsFirstFault) {
                run.dock.fault = false;
            }
        }
        s.runner.phase = RunnerPhase::Fenced;
        s.runner.failed = true;
        ev.push(Event::Revoked);
    }

    // ---- the fence and the runner (LD2 5.1) ----

    /// Phase A: one try-lock attempt publishes the entry fence of a bound
    /// run, or defers. It never waits for a busy slot and never rolls back.
    pub(super) fn fence(&self, s: &mut State, ev: &mut Vec<Event>) -> Result<(), Violation> {
        if let Some(run) = s.run.as_mut() {
            run.fenced = true;
        }
        s.runner.phase = RunnerPhase::Fenced;
        ev.push(Event::Fenced);
        Ok(())
    }

    /// The earlier design: the runner waits for the slot mutex while it holds
    /// its own lock.
    pub(super) fn fence_wait(&self, s: &State) -> Violation {
        Self::violation(
            "f-lock-wait",
            format!(
                "the fence waited for a busy slot ({}) while the runner lock was held",
                s.sec.as_ref().map_or("idle".to_string(), super::sec_name)
            ),
        )
    }

    pub(super) fn post(&self, s: &mut State, intent: Intent) {
        match intent {
            Intent::Start => s.runner.plan = s.runner.plan.saturating_sub(1),
            Intent::Stop { .. } => {}
        }
        s.runner.intent = Some(intent);
    }

    /// The role pump executes the posted intent on the slot side, outside the
    /// runner lock (LD2 3.5, 5.1 phase D). The earlier designs ran it under
    /// the runner lock or under the shared admission guard.
    pub(super) fn pump(&self, s: &mut State, ev: &mut Vec<Event>) -> Result<(), Violation> {
        let Some(intent) = s.runner.intent.take() else {
            return Ok(());
        };
        let under_runner = matches!(
            (intent, self.mutation),
            (Intent::Start, Mutation::FileCompileUnderRunner)
                | (Intent::Stop { .. }, Mutation::StopUnderRunnerLock)
        );
        match intent {
            Intent::Start => {
                s.runner.pending_planned = true;
                self.start(s, Who::Runner, ev)?;
                s.runner.pending_planned = false;
                if s.sec.is_some() && self.is(Mutation::FileCompileHoldsSharedAdmission) {
                    return Err(Self::violation(
                        "l-load-under-admission",
                        "the Load entry ran under the shared admission guard",
                    ));
                }
            }
            Intent::Stop { serial } => {
                let live = s
                    .run
                    .as_ref()
                    .is_some_and(|r| r.serial == serial && !r.stopping);
                if !live {
                    if self.housekeeping_target(s, serial) {
                        s.n.ctl += 1;
                        return self.housekeeping_stop(s, serial, ev);
                    }
                    if self.is(Mutation::RepeatLastTableStop) && s.run.is_some() {
                        s.runner.issuing = None;
                        self.stop_call(s, StopReason::Operator, ev)?;
                        return Err(Self::violation(
                            "l-stop-unowned",
                            format!(
                                "the runner's Stop for execution #{serial} ended execution #{}",
                                s.run.as_ref().map_or(0, |r| r.serial)
                            ),
                        ));
                    }
                    return Ok(());
                }
                s.runner.issuing = Some(serial);
                s.n.ctl += 1;
                self.stop_call(s, StopReason::Operator, ev)?;
            }
        }
        if under_runner && s.sec.is_some() {
            return Err(Self::violation(
                "l-vm-under-runner-lock",
                "a VM section began inside the runner's closure, under the runner lock",
            ));
        }
        Ok(())
    }

    /// LD2 3.2: at the final gate the live Idle slot is compared with the
    /// role's final ticket and its final ledger frontier. The last admitted
    /// execution's ticket names the generation and the Start serial that the
    /// slot must still show. A later Stop of the idle slot moves the
    /// generation, and a later admission moves the serial, so a stale ticket
    /// does not authorise the end. An empty ledger has no ticket: the
    /// frontier check covers the NoExecution case.
    fn final_ticket_matches(s: &State) -> bool {
        match s
            .runner
            .ledger
            .iter()
            .flatten()
            .max_by_key(|exec| exec.serial)
        {
            None => true,
            Some(exec) => matches!(
                exec.state,
                ExecState::Ticketed(t)
                    if t.serial == exec.serial
                        && t.gen_after == s.slot.generation
                        && t.start_serial == s.slot.serial
            ),
        }
    }

    /// LD2 3.2: an earlier ticket is not compared with the final live slot.
    /// It is linked to the next authorised admission of the chain: that
    /// admission moved the generation by one from the ticket's
    /// after-generation. A Stop of the idle slot between two planned
    /// executions moves it by two, and the chain is broken.
    fn tickets_link(s: &State) -> bool {
        let ledger = &s.runner.ledger;
        ledger
            .iter()
            .flatten()
            .zip(ledger.iter().flatten().skip(1))
            .all(|(earlier, next)| match earlier.state {
                ExecState::Ticketed(t) => next.generation == t.gen_after.wrapping_add(1),
                _ => true,
            })
    }

    /// The truth that the final gate must establish (LD2 5.4, items 3 and
    /// 4): every planned execution was retired by the runner's own ticketed
    /// Stop and ended clean, or, when the plan requires the natural self-stop,
    /// ended by its own clean `api.stop` before that Stop. The live Idle slot
    /// still matches the final ticket. No admission outside the plan
    /// happened, and the slot is Idle with nothing pending.
    fn pass_is_sound(&self, s: &State) -> bool {
        let slot_idle = s.slot.state == RunState::Idle
            && s.run.is_none()
            && s.sec.is_none()
            && s.slot.after == super::After::Idle
            && s.slot.chan.is_none();
        let planned_ok = s.runner.ledger.iter().flatten().all(|exec| {
            !exec.planned
                || (matches!(exec.state, ExecState::Ticketed(t) if t.serial == exec.serial)
                    && if self.bounds.natural {
                        exec.natural
                    } else {
                        exec.clean
                    })
        });
        slot_idle
            && planned_ok
            && Self::final_ticket_matches(s)
            && Self::tickets_link(s)
            && !s.runner.foreign_start
            && !s.runner.foreign_end
            && !s.runner.failed
    }

    /// The gate's own rule: the ledger and the tickets, then the slot's
    /// frontier. The earlier designs decided from the generation or from the
    /// old run's end.
    fn gate_accepts(&self, s: &State) -> bool {
        if s.runner.failed {
            return false;
        }
        let ledger = &s.runner.ledger;
        let ticket_ok =
            |exec: &Exec| matches!(exec.state, ExecState::Ticketed(t) if t.serial == exec.serial);
        let ticketed = |exec: &Exec| ticket_ok(exec) && exec.clean;
        let slot_idle = s.slot.state == RunState::Idle
            && s.run.is_none()
            && s.sec.is_none()
            && s.slot.after == super::After::Idle
            && s.slot.chan.is_none();
        let frontier =
            ledger.iter().flatten().map(|e| e.serial).max().unwrap_or(0) == s.slot.serial;
        let last_ticket = ledger
            .iter()
            .flatten()
            .filter_map(|exec| match exec.state {
                ExecState::Ticketed(t) => Some(t),
                _ => None,
            })
            .next_back();
        match self.mutation {
            Mutation::GuessedStopGeneration => {
                // A Stop is the runner's own when the generation went up by
                // one since the Start.
                ledger.iter().flatten().filter(|e| e.planned).all(|exec| {
                    ticketed(exec)
                        || (exec.state == ExecState::Ended
                            && s.slot.generation == exec.generation.wrapping_add(1))
                }) && s.slot.state == RunState::Idle
            }
            Mutation::ReplacementWithOldEnd => {
                ledger.iter().flatten().filter(|e| e.planned).all(ticketed)
            }
            Mutation::FailedReplacementRestoresAuthorization => {
                s.slot.state == RunState::Idle
                    && ledger.iter().flatten().filter(|e| e.planned).all(ticketed)
                    && last_ticket.is_some_and(|t| t.gen_after == s.slot.generation)
            }
            Mutation::ForcedStopSatisfiesNaturalStop => {
                // A ticket and a clean end are enough: the Stop that the
                // runner forced stands for the natural self-stop.
                slot_idle
                    && frontier
                    && Self::final_ticket_matches(s)
                    && Self::tickets_link(s)
                    && ledger
                        .iter()
                        .flatten()
                        .all(|exec| exec.planned && ticket_ok(exec) && (exec.clean || exec.natural))
            }
            _ => {
                // The ticket is compared with the live slot, unless the
                // earlier design takes it as proof.
                let ticket_current =
                    self.is(Mutation::StaleFinalTicket) || Self::final_ticket_matches(s);
                let chain_linked =
                    self.is(Mutation::UnlinkedEarlierTicket) || Self::tickets_link(s);
                slot_idle
                    && frontier
                    && ticket_current
                    && chain_linked
                    && ledger.iter().flatten().all(|exec| {
                        exec.planned
                            && ticket_ok(exec)
                            && if self.bounds.natural {
                                exec.natural
                            } else {
                                exec.clean
                            }
                    })
            }
        }
    }

    pub(super) fn final_gate(&self, s: &mut State, ev: &mut Vec<Event>) -> Result<(), Violation> {
        let accepts = self.gate_accepts(s);
        if accepts && !self.pass_is_sound(s) {
            return Err(Self::violation(
                "l-pass-unsound",
                format!(
                    "the final gate wrote PASS; foreign Start {}, foreign end {}, slot {:?}, generation {}",
                    s.runner.foreign_start, s.runner.foreign_end, s.slot.state, s.slot.generation
                ),
            ));
        }
        let verdict = if accepts {
            Verdict::Pass
        } else {
            Verdict::Fail
        };
        s.runner.phase = RunnerPhase::Done(verdict);
        ev.push(Event::Verdict(verdict));
        Ok(())
    }

    // ---- the offer of the layers' steps ----

    pub(super) fn layer_actions(&self, s: &State, out: &mut Vec<Action>) {
        let b = &self.bounds;
        let phase = s.runner.phase;
        if b.fence
            && phase == RunnerPhase::Running
            && s.run
                .as_ref()
                .map_or(self.natural_fence_target(s), |run| !run.fenced)
            && s.sec.is_none()
        {
            out.push(Action::Fence);
        }
        if b.dock {
            if let Some(run) = &s.run {
                match run.dock.call {
                    Call::Queued { .. } if !run.dock.revoked => out.push(Action::Take),
                    Call::Taken { .. } if !run.dock.revoked => out.push(Action::Complete),
                    Call::Completed
                        if self.is(Mutation::CompletionOverwrite) && run.dock.payload =>
                    {
                        out.push(Action::Complete);
                    }
                    _ => {}
                }
                if run.dock.claimed && !run.dock.revoked {
                    out.push(Action::Revoke);
                }
            } else if self.is(Mutation::CallsAfterClose) && s.closed_dock.is_some() {
                out.push(Action::Complete);
            }
        }
        if b.runner && !matches!(phase, RunnerPhase::Done(_)) {
            if s.runner.intent.is_none() {
                if s.runner.plan > 0 && s.sec.is_none() {
                    out.push(Action::Post(Intent::Start));
                }
                for exec in s.runner.ledger.iter().flatten() {
                    if exec.planned
                        && (exec.state == ExecState::Live || self.housekeeping_due(s, exec))
                    {
                        out.push(Action::Post(Intent::Stop {
                            serial: exec.serial,
                        }));
                    }
                }
                if self.is(Mutation::RepeatLastTableStop) {
                    for exec in s.runner.ledger.iter().flatten() {
                        if exec.planned && matches!(exec.state, ExecState::Ticketed(_)) {
                            out.push(Action::Post(Intent::Stop {
                                serial: exec.serial,
                            }));
                        }
                    }
                }
            } else {
                out.push(Action::Pump);
            }
            if phase == RunnerPhase::Fenced && s.runner.intent.is_none() {
                out.push(Action::FinalGate);
            }
        }
    }

    /// An entry of a section that ended by a yield at an awaited call.
    pub(super) fn end_entry_yielded(
        &self,
        s: &mut State,
        ev: &mut Vec<Event>,
    ) -> Result<(), Violation> {
        if !matches!(s.sec, Some(Sec::Disp(_))) {
            return Ok(());
        }
        self.end_entry(s, super::vm::How::Yielded(Wait::Ext), ev)
    }
}
