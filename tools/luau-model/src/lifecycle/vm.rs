//! The VM: one script entry at a time (LD1 rules X12, C2, C4 and B5).
//!
//! An entry is a section. While it runs, the caller holds the slot mutex, so
//! the only steps are script operations and the clock. The script's choices
//! are the table of LD1 6.1: it returns, yields at a wait, fails, calls
//! `api.stop`, exceeds its budget, causes a cap event, loses an error at the
//! C-call limit, catches the trip error, or panics. The run answers each
//! with the check of rule C2 at its four check points.

use super::{
    Action, Alloc, Binding, Bounds, Clock, DPhase, Event, Fail, Inv, Kind, Latch, Lifecycle,
    LoadSec, Mutation, Op, Owner, Ret, RunState, Sec, Shapes, Site, State, Vm, Wait,
};
use crate::explorer::Violation;

/// How an entry leaves the VM.
#[derive(Clone, Copy, Debug)]
pub(super) enum How {
    Returned(Ret),
    Yielded(Wait),
    Errored,
    Panicked,
}

/// The four check points of rule C2, and the check of the entry itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Cp {
    Entry,
    Interrupt,
    Binding,
    Exit,
}

impl State {
    pub(super) fn vm(&self) -> Option<&Vm> {
        match &self.sec {
            Some(Sec::Load(load)) => Some(&load.vm),
            Some(Sec::Disp(disp)) => disp.vm.as_ref(),
            Some(Sec::Hook(hook)) => Some(&hook.vm),
            None => None,
        }
    }

    pub(super) fn vm_mut(&mut self) -> Option<&mut Vm> {
        match &mut self.sec {
            Some(Sec::Load(load)) => Some(&mut load.vm),
            Some(Sec::Disp(disp)) => disp.vm.as_mut(),
            Some(Sec::Hook(hook)) => Some(&mut hook.vm),
            None => None,
        }
    }

    /// The run whose VM executes now: the one a Load entry builds, or the
    /// slot's.
    pub(super) fn cur(&self) -> Option<&super::Run> {
        match &self.sec {
            Some(Sec::Load(load)) => Some(&load.run),
            _ => self.run.as_ref(),
        }
    }

    pub(super) fn cur_mut(&mut self) -> Option<&mut super::Run> {
        match &mut self.sec {
            Some(Sec::Load(load)) => Some(&mut load.run),
            _ => self.run.as_mut(),
        }
    }
}

impl Lifecycle {
    // ---- configuration of the checks, from the mutation ----

    /// The check at a binding (rule C2, check point 3).
    fn binding_check(&self) -> bool {
        !matches!(
            self.mutation,
            Mutation::GapEffect | Mutation::VerdictFromError | Mutation::EffectAfterTrip
        )
    }

    /// The check at the exit (check point 4).
    fn exit_check(&self) -> bool {
        !matches!(
            self.mutation,
            Mutation::NoExitCheck | Mutation::SampledClock | Mutation::VerdictFromError
        )
    }

    /// The check at an interrupt reads the clock.
    fn interrupt_reads_clock(&self) -> bool {
        !matches!(self.mutation, Mutation::SampledClock)
    }

    /// A check writes the latch where it finds the cause (rule C2). The
    /// earlier design waited for the error to reach the host.
    fn latch_on_detect(&self) -> bool {
        !matches!(self.mutation, Mutation::VerdictFromError)
    }

    /// The script's `pcall` can swallow the error of a failed check: the run
    /// has no guard (rule B5).
    fn script_catches_checks(&self) -> bool {
        matches!(
            self.mutation,
            Mutation::VerdictFromError | Mutation::EffectAfterTrip
        )
    }

    // ---- the clock ----

    pub(super) fn arm(&self, s: &mut State, owner: Owner, units: u8) {
        let real = !(owner == Owner::Load && self.is(Mutation::LoadUngated))
            && !(owner == Owner::Hook && self.is(Mutation::HookUngated));
        if real && !(self.is(Mutation::DeadlineCarries) && s.clock.armed.is_some()) {
            s.clock.armed = Some((owner, units));
        }
        s.clock.truth = Some((owner, units));
    }

    pub(super) fn disarm(&self, s: &mut State) {
        s.clock.truth = None;
        if !self.is(Mutation::DeadlineCarries) {
            s.clock.armed = None;
        }
    }

    pub(super) fn advance(&self, s: &mut State) {
        for (_, left) in [&mut s.clock.armed, &mut s.clock.truth]
            .into_iter()
            .flatten()
        {
            *left = left.saturating_sub(1);
        }
    }

    fn truth_expired(s: &State) -> bool {
        s.clock.truth.is_some_and(|(_, left)| left == 0)
    }

    fn armed_expired(s: &State) -> bool {
        s.clock.armed.is_some_and(|(_, left)| left == 0)
    }

    // ---- the check of rule C2 ----

    /// Write a trip where the check found its cause (rule C2). A trip in a
    /// Load entry is the error of `spawn`; in a Dispatch it is the latch; in
    /// the Hook entry it is the hook failure.
    pub(super) fn write_trip(
        &self,
        s: &mut State,
        kind: Kind,
        fail: Fail,
        ev: &mut Vec<Event>,
    ) -> Result<(), Violation> {
        if !self.latch_on_detect() {
            if let Some(vm) = s.vm_mut() {
                vm.pending = Some(fail);
            }
            return Ok(());
        }
        let Some(run) = s.cur_mut() else {
            return Ok(());
        };
        if kind == Kind::Hook {
            if run.hook_fail.is_none() {
                run.hook_fail = Some(fail);
                ev.push(Event::HookTripped(fail));
            }
            return Ok(());
        }
        if run.latch == Latch::Empty {
            run.latch = Latch::Trip(fail);
            ev.push(Event::Tripped(fail));
        } else if run.latch != Latch::Trip(fail) && self.is(Mutation::LatchOverwritten) {
            let before = run.latch;
            run.latch = Latch::Trip(fail);
            return Err(Self::violation(
                "i11b-latch-overwritten",
                format!("the latch was {before:?} and a {fail:?} trip replaced it"),
            ));
        }
        Ok(())
    }

    /// One check, at one check point. `Ok(true)` is an error that the VM
    /// returns to the script: the latch has a trip, or the script stopped
    /// itself, or this check tripped.
    pub(super) fn check(
        &self,
        s: &mut State,
        cp: Cp,
        ev: &mut Vec<Event>,
    ) -> Result<bool, Violation> {
        let Some(vm) = s.vm().copied() else {
            return Ok(false);
        };
        let kind = vm.kind;
        let skipped = match cp {
            Cp::Binding => !self.binding_check(),
            Cp::Exit => !self.exit_check(),
            Cp::Entry => kind == Kind::Load && self.is(Mutation::LoadUngated),
            Cp::Interrupt => false,
        };
        if skipped {
            return Ok(false);
        }
        let Some(run) = s.cur() else {
            return Ok(false);
        };
        let (latch, hook_fail, over) = (run.latch, run.hook_fail, run.over);
        // 1. A cause that is already written.
        if latch.is_trip() || (kind == Kind::Hook && hook_fail.is_some()) {
            return Ok(true);
        }
        if latch == Latch::SelfStop
            && kind != Kind::Hook
            && !self.is(Mutation::LatchOverwritten)
            && !self.is(Mutation::StopContinues)
        {
            return Ok(true);
        }
        // 2. Used bytes at or above `S`.
        if over && !(kind == Kind::Load && self.is(Mutation::LoadUngated)) {
            self.write_trip(s, kind, Fail::Memory, ev)?;
            return Ok(true);
        }
        // 3. The clock at or past the deadline.
        let reads_clock = cp != Cp::Interrupt || self.interrupt_reads_clock();
        if reads_clock && Self::armed_expired(s) {
            self.write_trip(s, kind, Fail::Watchdog, ev)?;
            return Ok(true);
        }
        Ok(false)
    }

    /// An error propagates through the script: a failed check, `api.stop` or
    /// a refusal. The entry unwinds. Time can still pass before the exit
    /// check (LD1 4.3, trace 3), and only a mutation lets the script catch
    /// the error.
    fn raise(&self, s: &mut State, _ev: &mut Vec<Event>) -> Result<(), Violation> {
        if let Some(vm) = s.vm_mut() {
            vm.err = true;
        }
        Ok(())
    }

    /// A binding with an effect (a row, a log line, a started request, a
    /// staged call) after a trip, a self-stop, the deadline or the cap is a
    /// violation of I7 b or I11 c.
    fn effect(&self, s: &State, kind: Kind) -> Result<(), Violation> {
        let Some(run) = s.cur() else {
            return Ok(());
        };
        if run.latch == Latch::SelfStop && kind != Kind::Hook {
            return Err(Self::violation(
                "i11c-effect-after-stop",
                format!("a {kind:?} binding had an effect after `api.stop`"),
            ));
        }
        let tripped = run.latch.is_trip() || (kind == Kind::Hook && run.hook_fail.is_some());
        if tripped || run.over || Self::truth_expired(s) {
            return Err(Self::violation(
                "i7b-effect-after-trip",
                format!(
                    "a {kind:?} binding had an effect with latch {:?}, over {}, deadline passed {}",
                    run.latch,
                    run.over,
                    Self::truth_expired(s)
                ),
            ));
        }
        Ok(())
    }

    // ---- the operations of an entry ----

    pub(super) fn vm_op(
        &self,
        s: &mut State,
        op: Op,
        ev: &mut Vec<Event>,
    ) -> Result<(), Violation> {
        let kind = s
            .vm()
            .map(|vm| vm.kind)
            .ok_or_else(|| Self::violation("model", "a script operation outside an entry"))?;
        match op {
            Op::Gap => {
                let vm = s
                    .vm_mut()
                    .ok_or_else(|| Self::violation("model", "no vm"))?;
                vm.gap += 1;
                if vm.gap > self.bounds.r_gap {
                    return Err(Self::violation(
                        "i7c-gap-bound",
                        format!(
                            "{} operations ran with no check point; the residual R_gap is {}",
                            vm.gap, self.bounds.r_gap
                        ),
                    ));
                }
            }
            Op::Point => {
                if self.check(s, Cp::Interrupt, ev)? {
                    return self.raise(s, ev);
                }
                if let Some(vm) = s.vm_mut() {
                    vm.gap = 0;
                }
            }
            Op::Call(binding) => {
                if self.check(s, Cp::Binding, ev)? {
                    return self.raise(s, ev);
                }
                if self.binding_check()
                    && let Some(vm) = s.vm_mut()
                {
                    vm.gap = 0;
                }
                self.binding_body(s, kind, binding, ev)?;
            }
            Op::Return(ret) => {
                if self.check(s, Cp::Interrupt, ev)? {
                    return self.raise(s, ev);
                }
                self.end_entry(s, How::Returned(ret), ev)?;
            }
            Op::Fail => self.end_entry(s, How::Errored, ev)?,
            Op::Catch => {
                let swallows = self.script_catches_checks();
                if let Some(vm) = s.vm_mut() {
                    vm.caught += 1;
                    if swallows {
                        vm.err = false;
                        vm.pending = None;
                    }
                }
            }
            Op::Alloc(alloc) => {
                s.n.caps += 1;
                match alloc {
                    Alloc::OverS => {
                        if let Some(run) = s.cur_mut() {
                            run.over = true;
                        }
                    }
                    Alloc::Refuse(site) => self.refusal(s, kind, site, ev)?,
                    Alloc::Lost => self.lost(s, kind, ev)?,
                }
            }
            Op::Free => {
                if let Some(run) = s.cur_mut() {
                    run.over = false;
                }
            }
            Op::Panic => {
                s.n.panics += 1;
                self.panic(s, kind, ev)?;
            }
        }
        Ok(())
    }

    /// What the binding does after its check passed (rules B3, B4, 4.3).
    fn binding_body(
        &self,
        s: &mut State,
        kind: Kind,
        binding: Binding,
        ev: &mut Vec<Event>,
    ) -> Result<(), Violation> {
        match (kind, binding) {
            (Kind::Tick | Kind::Paint, Binding::Row) => {
                self.effect(s, kind)?;
                let cap = self.bounds.rows;
                if let Some(run) = s.cur_mut()
                    && run.rows < cap
                {
                    run.rows += 1;
                }
            }
            (Kind::Hook, Binding::Row) if self.is(Mutation::HookActionLegal) => {
                self.effect(s, kind)?;
                return Err(Self::violation(
                    "i4-row-after-stop",
                    "an action binding queued a row inside onStop, after the Stop began",
                ));
            }
            (_, Binding::Log) => self.effect(s, kind)?,
            (Kind::Tick, Binding::Wait(wait)) => {
                self.effect(s, kind)?;
                if matches!(wait, Wait::Seat { .. } | Wait::Walk) {
                    s.n.waits += 1;
                }
                if let Some(run) = s.cur_mut() {
                    run.arrived = false;
                }
                self.end_entry(s, How::Yielded(wait), ev)?;
            }
            (Kind::Paint, Binding::Wait(_)) if self.is(Mutation::F1PaintWaitSuspends) => {
                return Err(Self::violation(
                    "i6-paint-suspends",
                    "a waiting binding in a Paint entry started a request and yielded; no rule resumes that coroutine",
                ));
            }
            (Kind::Tick | Kind::Paint, Binding::Stop) => {
                if self.is(Mutation::OnStopInDispatch) {
                    return Err(Self::violation(
                        "i5-hook-in-dispatch",
                        "`api.stop` ran onStop inside the dispatch",
                    ));
                }
                self.effect(s, kind)?;
                if let Some(run) = s.cur_mut()
                    && run.latch == Latch::Empty
                {
                    run.latch = Latch::SelfStop;
                    ev.push(Event::SelfStop);
                }
                if !self.is(Mutation::StopContinues) {
                    self.raise(s, ev)?;
                }
            }
            (_, Binding::Ext { awaited }) => self.ext_call(s, kind, awaited, ev)?,
            _ => {}
        }
        Ok(())
    }

    /// Event E2: the allocator refuses a request at a site that sees it
    /// (rule C4). Every site writes the latch; an earlier design left the
    /// verdict to the error value.
    fn refusal(
        &self,
        s: &mut State,
        kind: Kind,
        site: Site,
        ev: &mut Vec<Event>,
    ) -> Result<(), Violation> {
        if let Some(run) = s.cur_mut() {
            run.cap_event = true;
        }
        let swallowed = match site {
            Site::Host => false,
            Site::Binding => self.is(Mutation::MemoryVerdictFromError),
            Site::Guard => {
                self.is(Mutation::MemoryVerdictFromError) || self.is(Mutation::LostRefusal)
            }
        };
        if swallowed {
            return Ok(());
        }
        if site == Site::Host && kind == Kind::Paint && self.is(Mutation::EntryThroughCall) {
            // The refusal reaches the run as an ordinary error.
            return self.raise(s, ev);
        }
        self.write_trip(s, kind, Fail::Memory, ev)?;
        self.raise(s, ev)
    }

    /// Event E3: an `mlua` call inside a binding returns the replaced error.
    /// The run cannot know that it was a refusal, so it fails closed.
    fn lost(&self, s: &mut State, kind: Kind, ev: &mut Vec<Event>) -> Result<(), Violation> {
        if let Some(run) = s.cur_mut() {
            run.cap_event = true;
        }
        if self.is(Mutation::LostErrorIgnored) {
            return Ok(());
        }
        self.write_trip(s, kind, Fail::Internal, ev)?;
        self.raise(s, ev)
    }

    /// An internal failure (rule X11): the run catches the panic and drops the
    /// VM. After `api.stop` the latch stays `SelfStop` and the failure goes to
    /// the completion (4.3).
    fn panic(&self, s: &mut State, kind: Kind, ev: &mut Vec<Event>) -> Result<(), Violation> {
        if let Some(run) = s.cur_mut() {
            run.vm = false;
            match kind {
                Kind::Hook => {
                    if run.hook_fail.is_none() {
                        run.hook_fail = Some(Fail::Internal);
                        ev.push(Event::HookTripped(Fail::Internal));
                    }
                }
                Kind::Load | Kind::Tick | Kind::Paint => match run.latch {
                    Latch::Empty => {
                        run.latch = Latch::Trip(Fail::Internal);
                        ev.push(Event::Tripped(Fail::Internal));
                    }
                    Latch::SelfStop => {
                        if run.hook_fail.is_none() {
                            run.hook_fail = Some(Fail::Internal);
                            ev.push(Event::InternalAfterSelfStop);
                        }
                    }
                    Latch::Trip(_) => {}
                },
            }
        }
        self.end_entry(s, How::Panicked, ev)
    }

    // ---- the end of an entry ----

    pub(super) fn end_entry(
        &self,
        s: &mut State,
        how: How,
        ev: &mut Vec<Event>,
    ) -> Result<(), Violation> {
        let kind = s
            .vm()
            .map(|vm| vm.kind)
            .ok_or_else(|| Self::violation("model", "an entry ended outside an entry"))?;
        if !matches!(how, How::Panicked) {
            self.check(s, Cp::Exit, ev)?;
            // The verdict that waited for the error reaches the host.
            if matches!(how, How::Errored) {
                let pending = s.vm().and_then(|vm| vm.pending);
                if let Some(fail) = pending {
                    let latch_on = self.latch_on_detect();
                    // The earlier design writes the latch here, and only here.
                    if !latch_on {
                        self.write_trip_now(s, kind, fail, ev);
                    }
                }
            }
        }
        // I8 b: a refusal or a lost error always trips the run.
        let tripped = s.cur().is_some_and(|run| {
            run.latch != Latch::Empty
                || run.hook_fail.is_some()
                || s.vm().is_some_and(|vm| vm.pending.is_some())
        });
        let cap_event = s.cur().is_some_and(|run| run.cap_event);
        if cap_event && !tripped && !matches!(how, How::Panicked) {
            return Err(Self::violation(
                "i8b-cap-event-untripped",
                format!(
                    "a {kind:?} entry saw a refused allocation or a lost error and the latch stayed empty"
                ),
            ));
        }
        // I7 a: no entry that passed its deadline is accepted, unless the
        // script stopped itself first (LD1 4.3, "first terminal cause").
        let latch = s.cur().map_or(Latch::Empty, |run| run.latch);
        let hook_fail = s.cur().and_then(|run| run.hook_fail);
        if Self::truth_expired(s)
            && !matches!(how, How::Panicked)
            && match kind {
                Kind::Hook => hook_fail.is_none(),
                _ => latch == Latch::Empty,
            }
        {
            let name = match kind {
                Kind::Load => "i7a-late-load",
                Kind::Hook => "i7a-late-hook",
                _ => "i7a-late-accept",
            };
            return Err(Self::violation(
                name,
                format!("a {kind:?} entry that passed its deadline was accepted"),
            ));
        }
        match kind {
            Kind::Load => self.finish_load(s, how, ev),
            Kind::Tick => self.finish_tick(s, how, ev),
            Kind::Paint => self.finish_paint(s, ev),
            Kind::Hook => self.finish_hook(s, ev),
        }
    }

    /// The pending verdict reaches the latch (mutation `VerdictFromError`).
    fn write_trip_now(&self, s: &mut State, kind: Kind, fail: Fail, ev: &mut Vec<Event>) {
        if let Some(run) = s.cur_mut() {
            if kind == Kind::Hook {
                if run.hook_fail.is_none() {
                    run.hook_fail = Some(fail);
                }
            } else if run.latch == Latch::Empty {
                run.latch = Latch::Trip(fail);
                ev.push(Event::Tripped(fail));
            }
        }
        if let Some(vm) = s.vm_mut() {
            vm.pending = None;
        }
    }

    fn finish_tick(&self, s: &mut State, how: How, ev: &mut Vec<Event>) -> Result<(), Violation> {
        let Some(run) = s.run.as_mut() else {
            return Err(Self::violation("model", "a Tick entry ended with no run"));
        };
        match how {
            How::Yielded(wait) => {
                run.inv = Inv::Suspended(wait);
                ev.push(Event::InvocationYielded);
            }
            _ => run.inv = Inv::Idle,
        }
        let to_paint = run.latch == Latch::Empty && run.vm;
        let Some(Sec::Disp(disp)) = s.sec.as_mut() else {
            return Err(Self::violation(
                "model",
                "a Tick entry ended outside a dispatch",
            ));
        };
        disp.vm = None;
        if disp.paint && to_paint {
            disp.phase = DPhase::Between;
            return Ok(());
        }
        self.finish_dispatch(s, ev)
    }

    fn finish_paint(&self, s: &mut State, ev: &mut Vec<Event>) -> Result<(), Violation> {
        let Some(Sec::Disp(disp)) = s.sec.as_mut() else {
            return Err(Self::violation(
                "model",
                "a Paint entry ended outside a dispatch",
            ));
        };
        disp.vm = None;
        self.finish_dispatch(s, ev)
    }

    fn finish_hook(&self, s: &mut State, ev: &mut Vec<Event>) -> Result<(), Violation> {
        let Some(Sec::Hook(hook)) = s.sec.take() else {
            return Err(Self::violation(
                "model",
                "a Hook entry ended outside a stop",
            ));
        };
        self.disarm(s);
        self.finish_stop(s, hook.reason, ev)
    }

    /// The Load entry ended: `spawn` returns the run or the error (rule X9).
    fn finish_load(&self, s: &mut State, how: How, ev: &mut Vec<Event>) -> Result<(), Violation> {
        let Some(Sec::Load(load)) = s.sec.take() else {
            return Err(Self::violation(
                "model",
                "a Load entry ended outside a spawn",
            ));
        };
        self.disarm(s);
        let LoadSec {
            mut run,
            how: spawn,
            ..
        } = load;
        let shape = match how {
            How::Returned(Ret::Table { hook, paint }) if run.latch == Latch::Empty => {
                Some((hook, paint))
            }
            _ => None,
        };
        match shape {
            Some((hook, paint)) => {
                run.hook = hook;
                run.paint = paint;
                self.spawn_succeeded(s, run, spawn, ev)
            }
            None => self.spawn_failed(s, run, spawn, ev),
        }
    }

    // ---- the actions of a section ----

    pub(super) fn next_entry(&self, s: &mut State, ev: &mut Vec<Event>) -> Result<(), Violation> {
        if self.is(Mutation::PaintRearmsDeadline) {
            s.clock.armed = Some((Owner::Dispatch, self.bounds.b_dispatch));
        }
        if !self.begin_entry(s, Kind::Paint, ev)? {
            return self.finish_dispatch(s, ev);
        }
        ev.push(Event::PaintRan);
        Ok(())
    }

    /// Begin a Tick or a Paint entry of the dispatch in progress. The checks
    /// of I2, I4, I10 and the fence come first. `Ok(false)`: the entry check
    /// failed, and the run does not enter (rule C2, check point 1).
    pub(super) fn begin_entry(
        &self,
        s: &mut State,
        kind: Kind,
        ev: &mut Vec<Event>,
    ) -> Result<bool, Violation> {
        let Some(run) = s.run.as_ref() else {
            return Err(Self::violation("model", "an entry with no run"));
        };
        if run.paused {
            return Err(Self::violation(
                "i2-entry-while-paused",
                format!("a {kind:?} entry started while the run is paused"),
            ));
        }
        if run.stopping {
            return Err(Self::violation(
                "i4-entry-after-stop",
                format!("a {kind:?} entry started after `stop` began"),
            ));
        }
        if run.fenced || run.dock.revoked {
            return Err(Self::violation(
                "f-entry-after-fence",
                format!("a {kind:?} entry started after the entry fence was published"),
            ));
        }
        if run.latch != Latch::Empty {
            return Err(Self::violation(
                "i10-entry-not-allowed",
                format!("a {kind:?} entry started with latch {:?}", run.latch),
            ));
        }
        if s.slot.state != RunState::Running {
            return Err(Self::violation(
                "i10-entry-not-allowed",
                format!("a {kind:?} entry started in slot state {:?}", s.slot.state),
            ));
        }
        let Some(Sec::Disp(disp)) = s.sec.as_mut() else {
            return Err(Self::violation("model", "an entry outside a dispatch"));
        };
        disp.vm = Some(Vm::new(kind));
        disp.phase = if kind == Kind::Tick {
            DPhase::InTick
        } else {
            DPhase::InPaint
        };
        disp.ran = true;
        if self.check(s, Cp::Entry, ev)? {
            if let Some(Sec::Disp(disp)) = s.sec.as_mut() {
                disp.vm = None;
            }
            return Ok(false);
        }
        Ok(true)
    }

    /// Time passes only while a deadline is armed.
    pub(super) fn time_runs(clock: &Clock) -> bool {
        clock.armed.is_some_and(|(_, left)| left > 0) || clock.truth.is_some_and(|(_, l)| l > 0)
    }

    // ---- which operations a script may do (the enumeration) ----

    pub(super) fn vm_actions(&self, s: &State, out: &mut Vec<Action>) {
        let Some(vm) = s.vm() else {
            return;
        };
        let Some(run) = s.cur() else {
            return;
        };
        let b: &Bounds = &self.bounds;
        if vm.err {
            // The script's `pcall` catches the error and goes on, up to
            // `R_prop` times. The guard raises it again (rule B5).
            if vm.caught < b.r_prop {
                out.push(Action::Vm(Op::Catch));
            }
            // The unwinding can meet a memory event or an internal failure
            // (LD1 4.3: neither changes the latch of a self-stop).
            if s.n.caps < b.caps {
                if !run.over {
                    out.push(Action::Vm(Op::Alloc(Alloc::OverS)));
                }
                out.push(Action::Vm(Op::Alloc(Alloc::Refuse(Site::Host))));
            }
            if s.n.panics < b.panics {
                out.push(Action::Vm(Op::Panic));
            }
            out.push(Action::Vm(Op::Fail));
            return;
        }
        let kind = vm.kind;
        if vm.gap < b.r_gap || (self.is(Mutation::CheckGapUnbounded) && vm.gap <= b.r_gap) {
            out.push(Action::Vm(Op::Gap));
        }
        out.push(Action::Vm(Op::Point));
        // Bindings. A refused binding is an error to the script and no step.
        let acts = matches!(kind, Kind::Tick | Kind::Paint);
        if acts || (kind == Kind::Hook && self.is(Mutation::HookActionLegal)) {
            out.push(Action::Vm(Op::Call(Binding::Row)));
        }
        if self.is(Mutation::GapEffect)
            || self.is(Mutation::EffectAfterTrip)
            || self.is(Mutation::VerdictFromError)
            || self.is(Mutation::StopContinues)
        {
            out.push(Action::Vm(Op::Call(Binding::Log)));
        }
        if kind == Kind::Tick || (kind == Kind::Paint && self.is(Mutation::F1PaintWaitSuspends)) {
            if b.seat_waits && s.n.waits < b.waits {
                out.push(Action::Vm(Op::Call(Binding::Wait(Wait::Seat {
                    typed: false,
                }))));
                if b.typed_waits {
                    out.push(Action::Vm(Op::Call(Binding::Wait(Wait::Seat {
                        typed: true,
                    }))));
                }
            }
            if b.walk_waits && s.n.waits < b.waits {
                out.push(Action::Vm(Op::Call(Binding::Wait(Wait::Walk))));
            }
            if b.tick_waits {
                out.push(Action::Vm(Op::Call(Binding::Wait(Wait::Tick))));
            }
        }
        if acts && b.self_stop {
            out.push(Action::Vm(Op::Call(Binding::Stop)));
        }
        self.ext_actions(s, kind, run, out);
        // Memory events and panics.
        if s.n.caps < b.caps {
            if !run.over {
                out.push(Action::Vm(Op::Alloc(Alloc::OverS)));
            }
            for site in [Site::Host, Site::Binding, Site::Guard] {
                out.push(Action::Vm(Op::Alloc(Alloc::Refuse(site))));
            }
            out.push(Action::Vm(Op::Alloc(Alloc::Lost)));
        }
        if run.over {
            out.push(Action::Vm(Op::Free));
        }
        if s.n.panics < b.panics {
            out.push(Action::Vm(Op::Panic));
        }
        // Exits.
        match kind {
            Kind::Load => {
                let shapes = |shape: Shapes| match shape {
                    Shapes::Without => vec![false],
                    Shapes::With => vec![true],
                    Shapes::Either => vec![false, true],
                };
                for hook in shapes(b.hook) {
                    for paint in shapes(b.paint) {
                        out.push(Action::Vm(Op::Return(Ret::Table { hook, paint })));
                    }
                }
                out.push(Action::Vm(Op::Return(Ret::Other)));
            }
            _ => out.push(Action::Vm(Op::Return(Ret::Plain))),
        }
        out.push(Action::Vm(Op::Fail));
    }
}
