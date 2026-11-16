//! The lent dispatch (LD1 4.2): one wake, one decision, at most one Tick
//! entry and one Paint entry, one deadline for both, and the outcome that the
//! slot reads in the same locked call (4.3).

use super::{
    Comp, DPhase, Disp, Event, Inv, Kind, Latch, Lifecycle, Mutation, Owner, Refusal, RunState,
    Sec, State, Violation, Wait, Wake,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TickPlan {
    Nothing,
    Start,
    Resume(Wait),
}

impl Lifecycle {
    /// The earlier designs whose path to the run does not consult the slot's
    /// gate (`Running && want_run`) for a wake: the run's own refusal is the
    /// only barrier, and a Pause reaches it.
    pub(super) fn gate_bypass(&self, wake: Wake) -> bool {
        match self.mutation {
            Mutation::SeatPageIgnoresPause
            | Mutation::AckOnRefusal
            | Mutation::RefusedDispatchLosesPayload
            | Mutation::ConsumeWhilePaused => wake == Wake::SeatPage,
            Mutation::PausedPaint => wake == Wake::Tick,
            Mutation::CheckPauseNoRefusal => true,
            _ => false,
        }
    }

    /// The rule of LD1 I6, I13 and P2 for one wake, stated without the
    /// mutations: the wait that this wake must end, if any.
    fn resume_due(run: &super::Run, wake: Wake, held: bool, carries: Option<Comp>) -> Option<Wait> {
        let Inv::Suspended(wait) = run.inv else {
            return None;
        };
        if held {
            return None;
        }
        let ready = match wait {
            Wait::Seat { .. } | Wait::Walk => {
                carries.is_some() || (run.stored.is_some() && !run.consumed)
            }
            Wait::Tick => wake == Wake::Tick,
            Wait::Ext => run.dock.call == super::Call::Completed && run.dock.payload,
        };
        ready.then_some(wait)
    }

    /// One wake from the slot: `on_game_tick_at`, `on_snapshot_change_at` or
    /// `on_seat_page_at` (4.2 rule 2). The call begins with
    /// `observe_lifecycle`, as the slot's own functions do.
    pub(super) fn dispatch(
        &self,
        s: &mut State,
        wake: Wake,
        ev: &mut Vec<Event>,
    ) -> Result<(), Violation> {
        self.observe_prefix(s, ev)?;
        let beside_hook =
            self.is(Mutation::HookOutsideMutex) && matches!(s.sec, Some(Sec::Hook(_)));
        if s.sec.is_some() && !beside_hook {
            return Ok(());
        }
        let open = s.slot.state == RunState::Running && s.slot.want_run;
        let bypass = self.gate_bypass(wake) && s.slot.state == RunState::Paused;
        if !(open || bypass) || s.run.is_none() {
            return Ok(());
        }
        let held = wake == Wake::Tick && s.host.hold;
        match wake {
            Wake::Tick => {
                s.n.ticks += 1;
                if held {
                    s.n.held += 1;
                }
            }
            Wake::Evidence | Wake::SeatPage => s.n.wakes += 1,
        }
        let carries = s.host.comp;
        let Some(run) = s.run.as_mut() else {
            return Ok(());
        };

        // The entry fence and the revocation token (LD2 5.1): a denied wake
        // does nothing and injects no abort.
        if (run.fenced || run.dock.revoked) && !self.is(Mutation::WakeBetweenFreezeAndStop) {
            ev.push(Event::Denied);
            return Ok(());
        }

        // The refusals of the run (4.2, the table of cases).
        let paused = run.paused
            && !(self.is(Mutation::SeatPageIgnoresPause) && wake == Wake::SeatPage)
            && !self.is(Mutation::CheckPauseNoRefusal)
            && !(self.is(Mutation::PausedPaint) && wake == Wake::Tick);
        let latch_refuses = run.latch != Latch::Empty
            && !(self.is(Mutation::SelfStopAcceptsDispatch) && run.latch == Latch::SelfStop);
        if paused || latch_refuses || (run.stopping && !beside_hook) {
            ev.push(Event::Refused(if paused {
                Refusal::Paused
            } else {
                Refusal::Latch
            }));
            if self.is(Mutation::AckOnRefusal) {
                s.host.comp = None;
            }
            if self.is(Mutation::RefusedDispatchLosesPayload) {
                run.dock.payload = false;
            }
            if self.is(Mutation::ConsumeWhilePaused)
                && run.paused
                && run.inv == Inv::Suspended(Wait::Ext)
                && run.dock.payload
            {
                return Err(Self::violation(
                    "d-consume-not-allowed",
                    "a paused run consumed the extension payload",
                ));
            }
            return Ok(());
        }

        // The slot gives the run the host's facts. An accepted call stores the
        // completion that the delivery carries (4.2 rule 5).
        let spec = Self::resume_due(run, wake, held, carries);
        let skip_store = (self.is(Mutation::LentSkipsSeatPage) && wake == Wake::SeatPage)
            || (self.is(Mutation::AckWithoutStore) && held);
        if let Some(comp) = carries
            && !skip_store
        {
            run.stored = Some(match comp {
                Comp::Terminal { .. } if self.is(Mutation::BareSeatEnd) => {
                    Comp::Terminal { typed: false }
                }
                other => other,
            });
            ev.push(Event::CompletionStored);
        }

        // The plan (rules X4 to X6).
        let tick_wake = wake == Wake::Tick;
        let mut plan = TickPlan::Nothing;
        match run.inv {
            Inv::Idle if tick_wake && !held => plan = TickPlan::Start,
            Inv::Suspended(wait) => {
                let ready = match wait {
                    Wait::Seat { .. } | Wait::Walk => run.stored.is_some(),
                    Wait::Tick => tick_wake,
                    Wait::Ext => run.dock.call == super::Call::Completed && run.dock.payload,
                };
                let wake_ok = !(wait == Wait::Tick && !tick_wake);
                let hold_ok = !held
                    || self.is(Mutation::HeldTickResumes)
                    || (wait == Wait::Ext && self.is(Mutation::HeldConsumes));
                if (ready || self.is(Mutation::ResumeWithoutCompletion)) && wake_ok && hold_ok {
                    plan = TickPlan::Resume(wait);
                } else if tick_wake && !held && self.is(Mutation::ReenterTick) {
                    plan = TickPlan::Start;
                    run.extra += 1;
                }
            }
            _ => {}
        }
        if self.is(Mutation::PausedPaint) && run.paused {
            plan = TickPlan::Nothing;
        }
        let paint = run.paint && tick_wake;

        // I6, I13 and P2.
        if let TickPlan::Resume(_) = plan {
            if run.consumed && matches!(run.inv, Inv::Suspended(Wait::Seat { .. } | Wait::Walk)) {
                return Err(Self::violation(
                    "i6-resume-twice",
                    "a completion that already resumed an invocation resumed another wait",
                ));
            }
            if spec.is_none() {
                let name = if held {
                    "i13-held-resumes"
                } else {
                    "i6-resume-without-completion"
                };
                return Err(Self::violation(
                    name,
                    format!("the run resumed its invocation for a {wake:?} wake, held {held}"),
                ));
            }
        } else if let Some(wait) = spec {
            return Err(Self::violation(
                "p2-completion-not-delivered",
                format!(
                    "the completion of the awaited {wait:?} was in the delivery of a {wake:?} wake and the run did not resume"
                ),
            ));
        }

        s.sec = Some(Sec::Disp(Disp {
            wake,
            held,
            phase: DPhase::InTick,
            vm: None,
            paint,
            ran: false,
            carries,
            consumed: false,
        }));
        if plan == TickPlan::Nothing && !paint {
            ev.push(Event::NothingDue);
            return self.finish_dispatch(s, ev);
        }
        // One deadline for the whole dispatch, armed before its first entry
        // (rule C2).
        self.arm(s, Owner::Dispatch, self.bounds.b_dispatch);
        match plan {
            TickPlan::Nothing => {
                if !self.begin_entry(s, Kind::Paint, ev)? {
                    return self.finish_dispatch(s, ev);
                }
                ev.push(Event::PaintRan);
            }
            TickPlan::Start | TickPlan::Resume(_) => {
                if !self.begin_entry(s, Kind::Tick, ev)? {
                    return self.finish_dispatch(s, ev);
                }
                if let TickPlan::Resume(wait) = plan {
                    self.consume(s, wait)?;
                    ev.push(Event::InvocationResumed);
                } else {
                    ev.push(Event::InvocationStarted);
                }
                if let Some(run) = s.run.as_mut() {
                    run.inv = Inv::Running;
                }
            }
        }
        Ok(())
    }

    /// The resume takes the completion once (4.2 rule 5, LD2 2.3).
    fn consume(&self, s: &mut State, wait: Wait) -> Result<(), Violation> {
        let Some(run) = s.run.as_mut() else {
            return Ok(());
        };
        match wait {
            Wait::Seat { typed } => {
                if typed && let Some(Comp::Terminal { typed: false }) = run.stored {
                    return Err(Self::violation(
                        "i6-bare-completion",
                        "the awaited family request resumed with the bare seat end and no typed outcome",
                    ));
                }
                if !self.is(Mutation::ResumeTwice) {
                    run.stored = None;
                }
                run.consumed = true;
            }
            Wait::Walk => {
                if !self.is(Mutation::ResumeTwice) {
                    run.stored = None;
                }
                run.consumed = true;
            }
            Wait::Tick => {}
            Wait::Ext => self.consume_dock(run),
        }
        if let Some(Sec::Disp(disp)) = s.sec.as_mut() {
            disp.consumed = true;
        }
        Ok(())
    }

    /// The dispatch ends: the outcome of 4.2 and 4.3.
    pub(super) fn finish_dispatch(
        &self,
        s: &mut State,
        ev: &mut Vec<Event>,
    ) -> Result<(), Violation> {
        let Some(Sec::Disp(disp)) = s.sec.take() else {
            return Err(Self::violation(
                "model",
                "a dispatch ended outside a dispatch",
            ));
        };
        self.disarm(s);
        let Some(run) = s.run.as_mut() else {
            return Err(Self::violation("model", "a dispatch ended with no run"));
        };
        let latch = run.latch;
        let sticky_rows = self.is(Mutation::RowsAfterStop) || self.is(Mutation::PostFreezeAction);

        if let Latch::Trip(fail) = latch {
            if self.is(Mutation::PendingFailure) {
                return Err(Self::violation(
                    "i11a-failure-pending",
                    "the dispatch returned Accepted and the run keeps the failure for a later read",
                ));
            }
            // `Failed(failure)`: the rows, the paint frame and the outcome are
            // discarded; the slot stops the run in the same locked call.
            if self.is(Mutation::DrainBeforeFailure) && run.rows > 0 {
                run.rows = 0;
                return Err(Self::violation(
                    "i7b-rows-of-failed-dispatch-admitted",
                    "the slot drained the rows of a dispatch before it handled `Failed`",
                ));
            }
            run.rows = 0;
            self.dock_dispatch_end(s, false)?;
            ev.push(Event::DispatchFailed(fail));
            return self.begin_stop(s, super::StopReason::Failure, ev);
        }

        if !sticky_rows {
            run.rows = 0;
        }
        self.dock_dispatch_end(s, true)?;
        let Some(run) = s.run.as_mut() else {
            return Ok(());
        };
        if disp.carries.is_some() {
            // The host acknowledges only an accepted dispatch (4.2 rule 6),
            // and the run holds the completion: stored, or consumed now.
            if run.stored.is_none() && !disp.consumed {
                let name = if disp.held {
                    "i13-held-not-stored"
                } else {
                    "i3-completion-lost"
                };
                return Err(Self::violation(
                    name,
                    "the host acknowledged a completion that the run neither stored nor consumed",
                ));
            }
            s.host.comp = None;
            ev.push(Event::CompletionAcked);
        }
        ev.push(if latch == Latch::SelfStop {
            Event::DispatchSelfStopped
        } else {
            Event::DispatchAccepted
        });
        Ok(())
    }
}
