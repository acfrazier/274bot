//! The steps that are enabled in a state. The explorer offers only enabled
//! steps; a conformance trace must use only enabled steps too.

use super::{
    Action, After, DPhase, Inv, Latch, Lifecycle, Mutation, RunState, Sec, State, Wait, Wake, Who,
};

impl Lifecycle {
    pub(super) fn actions(&self, s: &State, out: &mut Vec<Action>) {
        if s.gone {
            return;
        }
        if let Some(sec) = &s.sec {
            // A call that holds the slot mutex: only the script and the
            // clock move (rule X2).
            match sec {
                Sec::Disp(disp) if disp.vm.is_none() => {
                    if disp.phase == DPhase::Between {
                        out.push(Action::Next);
                    }
                }
                _ => self.vm_actions(s, out),
            }
            if self.bounds.clock && Self::time_runs(&s.clock) {
                out.push(Action::Advance);
            }
            if self.is(Mutation::HookOutsideMutex)
                && matches!(sec, Sec::Hook(_))
                && s.n.ticks < self.bounds.ticks
            {
                out.push(Action::Dispatch(Wake::Tick));
            }
            if self.is(Mutation::BlockingFleetLock) && self.bounds.fence {
                out.push(Action::FenceWait);
            }
            return;
        }
        let b = &self.bounds;
        let n = &s.n;
        let slot = &s.slot;
        let run = s.run.as_ref();

        // The control thread.
        if n.starts < b.starts
            && (slot.state == RunState::Idle
                || (slot.state == RunState::Stopping && slot.after == After::Idle))
        {
            out.push(Action::Start(Who::Operator));
        }
        if n.ctl < b.ctl {
            let live = run.is_some();
            let stopping = slot.state == RunState::Stopping;
            let can_pause = slot.want_run
                || run.is_some_and(|r| !r.paused)
                    && matches!(slot.state, RunState::Running | RunState::Starting);
            if (live || stopping) && can_pause {
                out.push(Action::Pause);
            }
            let can_resume =
                !slot.want_run || run.is_some_and(|r| r.paused) || slot.state == RunState::Paused;
            if (live || stopping) && can_resume {
                out.push(Action::Resume);
            }
            if live
                || (stopping && (slot.after != After::Idle || b.runner))
                || (b.runner && slot.state == RunState::Idle)
            {
                out.push(Action::Stop);
            }
            if b.removal && live {
                out.push(Action::Remove);
                out.push(Action::DropSlot);
            }
        }

        // The slot thread.
        if let Some(run) = run {
            if slot.up {
                let open = slot.want_run
                    && (slot.state == RunState::Running
                        || (slot.state == RunState::Starting && !slot.ready));
                let paused = slot.state == RunState::Paused;
                let tick = open || (paused && self.gate_bypass(Wake::Tick));
                if tick && n.ticks < b.ticks && (!s.host.hold || n.held < b.held) {
                    out.push(Action::Dispatch(Wake::Tick));
                }
                if !s.host.hold && n.wakes < b.wakes {
                    if open || (paused && self.gate_bypass(Wake::Evidence)) {
                        out.push(Action::Dispatch(Wake::Evidence));
                    }
                    if open || (paused && self.gate_bypass(Wake::SeatPage)) {
                        out.push(Action::Dispatch(Wake::SeatPage));
                    }
                }
            }
            if !slot.ready || (run.latch == Latch::SelfStop && !run.stopping) {
                out.push(Action::Observe);
            }
            if n.restarts < b.restarts
                && n.starts < b.starts
                && slot.want_run
                && matches!(slot.state, RunState::Starting | RunState::Running)
                && (run.latch != Latch::SelfStop || self.is(Mutation::F3RestartIgnoresSelfStop))
            {
                out.push(Action::Restart);
            }
        }
        if slot.chan.is_some() {
            out.push(Action::Observe);
        }

        // The client, the host and the seat.
        if n.gates < b.gates {
            out.push(if slot.up {
                Action::ClientDown
            } else {
                Action::ClientUp
            });
        }
        if n.holds < b.holds {
            out.push(if s.host.hold {
                Action::HoldOff
            } else {
                Action::HoldOn
            });
        }
        if let Some(run) = run {
            if matches!(run.inv, Inv::Suspended(Wait::Seat { .. } | Wait::Walk))
                && !run.arrived
                && s.host.comp.is_none()
            {
                out.push(Action::Arrive);
            }
            if n.resets < b.resets
                && matches!(
                    run.inv,
                    Inv::Suspended(Wait::Seat { .. } | Wait::Walk | Wait::Ext)
                )
            {
                out.push(Action::SessionReset);
            }
            if run.rows > 0
                && (self.is(Mutation::PostFreezeAction) || self.is(Mutation::RowsAfterStop))
            {
                out.push(Action::Admit);
            }
        }
        if s.orphan_rows > 0 {
            out.push(Action::Admit);
        }
        if self.bounds.clock && Self::time_runs(&s.clock) {
            out.push(Action::Advance);
        }
        self.layer_actions(s, out);
    }
}
