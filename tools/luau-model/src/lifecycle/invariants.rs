//! The invariants that hold after every step. The step functions check the
//! rest at the step where they decide (the check names the LD1 invariant:
//! `i1` to `i13`, `p1`, `p2`; `d-`, `f-` and `l-` are the runner side).

use super::{Call, Inv, Lifecycle, RunState, Runner, RunnerPhase, Sec, State, Violation, Wait};

impl Lifecycle {
    pub(super) fn check_state(&self, s: &State) -> Result<(), Violation> {
        // I1: one VM, only after Start.
        if s.vms() > 1 {
            return Err(Self::violation(
                "i1-two-vms",
                format!(
                    "{} VMs exist: the slot's run, the Load entry in progress and {} kept by a failed spawn",
                    s.vms(),
                    s.leaked
                ),
            ));
        }
        if s.leaked > 0 {
            return Err(Self::violation(
                "i1-vm-outlives-run",
                "a failed spawn returned its error and kept the VM",
            ));
        }
        if s.gone && (s.run.is_some() || s.sec.is_some() || s.vms() > 0) {
            return Err(Self::violation(
                "i4-vm-after-stop",
                "the slot is gone and a run or a VM is left",
            ));
        }
        // I7 d: a deadline is cleared when its owner ends.
        if s.sec.is_none() && s.clock.armed.is_some() {
            return Err(Self::violation(
                "i7d-deadline-outlives-owner",
                format!(
                    "the deadline {:?} is armed and no entry runs",
                    s.clock.armed
                ),
            ));
        }
        // P1: a Stopping slot has a completion to read, or a spawn in progress.
        if s.slot.state == RunState::Stopping
            && s.slot.chan.is_none()
            && !matches!(s.sec, Some(Sec::Load(_)))
            && !s.gone
        {
            return Err(Self::violation(
                "p1-stuck-stopping",
                "the slot is Stopping and no completion is waiting for the observe step",
            ));
        }
        if s.slot.state == RunState::Idle && s.run.is_some() {
            return Err(Self::violation(
                "i9-idle-with-run",
                "the slot is Idle and still owns a run",
            ));
        }
        if let Some(run) = &s.run {
            if run.hook_runs > 1 {
                return Err(Self::violation(
                    "i5-hook-twice",
                    format!("onStop started {} times", run.hook_runs),
                ));
            }
            if run.extra > 0 {
                return Err(Self::violation(
                    "i6-two-invocations",
                    "a game tick started `tick` while an invocation is suspended",
                ));
            }
            // I3 and I13: a completion that arrived is never lost.
            if matches!(run.inv, Inv::Suspended(Wait::Seat { .. } | Wait::Walk))
                && run.arrived
                && !run.consumed
                && s.host.comp.is_none()
                && run.stored.is_none()
            {
                return Err(Self::violation(
                    "i3-completion-lost",
                    "the completion of the awaited request arrived and neither the host nor the run holds it",
                ));
            }
            if run.dock.revoked && run.dock.payload {
                return Err(Self::violation(
                    "d-revoke-keeps-payload",
                    "the dock was revoked and its completion payload is still there",
                ));
            }
            if run.dock.call == Call::Completed && run.dock.delivered {
                return Err(Self::violation(
                    "d-consume-twice",
                    "the payload was delivered and the dock still offers it",
                ));
            }
            if run.inv == Inv::Suspended(Wait::Ext)
                && run.dock.call == Call::Completed
                && !run.dock.payload
            {
                return Err(Self::violation(
                    "d-payload-lost",
                    "the call is Completed and the payload is gone before the resume",
                ));
            }
        }
        // LD2 3.2: the ledger holds every admission.
        if self.bounds.runner {
            let Runner { ledger, .. } = &s.runner;
            for serial in 1..=s.slot.serial.min(3) {
                if ledger[usize::from(serial) - 1].is_none() {
                    return Err(Self::violation(
                        "l-ledger-incomplete",
                        format!("the admission #{serial} has no ledger entry"),
                    ));
                }
            }
            if let RunnerPhase::Done(_) = s.runner.phase {
                // The verdict was written; nothing more is checked here.
            }
        }
        Ok(())
    }
}
