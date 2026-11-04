//! Reissue a native operation after the existing bounded prerequisite owner resolves its gate.
use super::run::{PrereqBudget, PrereqRun, RunEntry};
use crate::bank::seat_open::{Opening, OpeningArgs};
use crate::local_state::SlotState;
use crate::native::{
    ActionContext, ActionError, ActionHandle, NativeActions, NativeMachine, PrereqAllow,
    PrereqRefusal,
};
use crate::native_bank::{Select, SelectArgs};
use crate::quester::families::reach::{GroundTake, Reach, ReachArgs};
use std::sync::Arc;
use std::task::Poll;

enum Phase<M: PrereqMachine> {
    Reaching(ActionHandle<M>),
    Resolving(Box<PrereqRun>),
    Reissue,
    Cancelled,
    Complete,
}

/// Reach, pickup, bank selection and bank opening use one prerequisite loop.
pub(crate) trait PrereqMachine: NativeMachine {
    fn clone_args(args: &Self::Args) -> Self::Args;
    fn grants(&self, cx: &ActionContext<'_>) -> Option<PrereqAllow>;
}
impl PrereqMachine for Reach {
    fn clone_args(args: &ReachArgs) -> ReachArgs {
        args.clone()
    }
    fn grants(&self, cx: &ActionContext<'_>) -> Option<PrereqAllow> {
        self.prereq_allow(cx)
    }
}
impl PrereqMachine for GroundTake {
    fn clone_args(args: &ReachArgs) -> ReachArgs {
        args.clone()
    }
    fn grants(&self, cx: &ActionContext<'_>) -> Option<PrereqAllow> {
        self.prereq_allow(cx)
    }
}
impl PrereqMachine for Select {
    fn clone_args(args: &SelectArgs) -> SelectArgs {
        args.clone()
    }
    fn grants(&self, cx: &ActionContext<'_>) -> Option<PrereqAllow> {
        self.prereq_allow(cx)
    }
}
impl PrereqMachine for Opening {
    fn clone_args(args: &OpeningArgs) -> OpeningArgs {
        args.clone()
    }
    fn grants(&self, cx: &ActionContext<'_>) -> Option<PrereqAllow> {
        self.prereq_allow(cx)
    }
}

pub(crate) type PrereqSelect = PrereqAction<Select>;
pub(crate) type PrereqOpening = PrereqAction<Opening>;
pub(crate) type PrereqReach = PrereqAction<Reach>;
pub(crate) type PrereqGroundTake = PrereqAction<GroundTake>;

pub(crate) struct PrereqAction<M: PrereqMachine> {
    args: M::Args,
    phase: Phase<M>,
    budget: PrereqBudget,
    resolve: bool,
    allow: Option<PrereqAllow>,
}
impl<M: PrereqMachine> PrereqAction<M> {
    pub fn begin(
        args: M::Args,
        actions: &mut NativeActions,
        cx: &mut ActionContext<'_>,
    ) -> Result<Self, ActionError> {
        let handle = actions.begin::<M>(M::clone_args(&args), cx)?;
        Ok(Self {
            args,
            phase: Phase::Reaching(handle),
            budget: PrereqBudget::default(),
            resolve: true,
            allow: None,
        })
    }
    pub(crate) fn resolve_after(
        args: M::Args,
        error: ActionError,
        allow: PrereqAllow,
        cx: &ActionContext<'_>,
    ) -> Result<Self, (M::Args, ActionError)> {
        let budget = PrereqBudget::default();
        let run = match resolution(error, allow, cx, &budget) {
            Ok(run) => run,
            Err(error) => return Err((args, error)),
        };
        Ok(Self {
            args,
            phase: Phase::Resolving(Box::new(run)),
            budget,
            resolve: true,
            allow: Some(allow),
        })
    }

    /// Recover the original arguments after a terminal prerequisite refusal.
    pub(crate) fn into_args(self) -> M::Args {
        self.args
    }

    #[cfg(test)]
    pub(crate) fn original_allow(&self) -> Option<PrereqAllow> {
        self.allow
    }

    /// The prerequisite runner already owns the shared resolution budget.
    pub(crate) fn begin_unresolved(
        args: M::Args,
        actions: &mut NativeActions,
        cx: &mut ActionContext<'_>,
    ) -> Result<Self, ActionError> {
        let mut action = Self::begin(args, actions, cx)?;
        action.resolve = false;
        Ok(action)
    }
    pub(crate) fn inspect<R>(&self, read: impl FnOnce(&M) -> R) -> Option<R> {
        match &self.phase {
            Phase::Reaching(handle) => handle.inspect(read),
            _ => None,
        }
    }
    pub(crate) fn inspect_in<R>(
        &self,
        cx: &mut ActionContext<'_>,
        read: impl FnOnce(&M, &ActionContext<'_>) -> R,
    ) -> Option<R> {
        match &self.phase {
            Phase::Reaching(handle) => handle.inspect_in(cx, read),
            _ => None,
        }
    }
    pub(crate) fn current_revoker(&self) -> Option<crate::native::ActionRevoker> {
        match &self.phase {
            Phase::Reaching(handle) => Some(handle.revoker()),
            Phase::Resolving(run) => run.revoker(),
            _ => None,
        }
    }
    pub(crate) fn resume_after_session(
        &mut self,
        actions: &mut NativeActions,
        kind: crate::native::request::NativeKind,
        cx: &mut ActionContext<'_>,
    ) -> Result<(), ActionError> {
        if kind.policy().reconnect() != api::script_contract::ReconnectPolicy::Resume {
            return Err(ActionError::Stale);
        }
        match &mut self.phase {
            Phase::Reaching(handle) => actions.resume_kept(kind, handle, cx),
            Phase::Resolving(_) => {
                // The journal owner belongs to the old session. Revalidate
                // the original operation through the existing reissue budget,
                // rather than restart a reader outside its timeout bound.
                self.phase = Phase::Reissue;
                Ok(())
            }
            Phase::Reissue => Ok(()),
            Phase::Cancelled | Phase::Complete => Err(ActionError::Stale),
        }
    }

    pub fn cancel(&mut self) {
        self.phase = Phase::Cancelled;
    }
    pub fn poll(
        &mut self,
        actions: &mut NativeActions,
        cx: &mut ActionContext<'_>,
        state: Option<&SlotState>,
    ) -> Poll<Result<M::Output, ActionError>> {
        if !cx.eligible() {
            return Poll::Pending;
        }
        loop {
            match &mut self.phase {
                Phase::Reaching(handle) => {
                    // Capture the terminal's grants before polling retires its native owner.
                    let allow = self
                        .allow
                        .or_else(|| handle.inspect_in(cx, M::grants).flatten())
                        .unwrap_or_default();
                    match actions.poll(handle, cx) {
                        Poll::Pending => return Poll::Pending,
                        Poll::Ready(Ok(result)) => {
                            self.phase = Phase::Complete;
                            return Poll::Ready(Ok(result));
                        }
                        Poll::Ready(Err(error)) => {
                            if !self.resolve {
                                self.phase = Phase::Complete;
                                return Poll::Ready(Err(error));
                            }
                            let run = resolution(error, allow, cx, &self.budget)?;
                            // A new host receipt must not broaden the consent
                            // captured by this logical operation's first refusal.
                            self.allow = Some(allow);
                            self.phase = Phase::Resolving(Box::new(run));
                        }
                    }
                }
                Phase::Resolving(run) => {
                    match std::task::ready!(run.poll(actions, cx, state, &mut self.budget)) {
                        Ok(_) => self.phase = Phase::Reissue,
                        Err(refusal) => {
                            self.phase = Phase::Complete;
                            return Poll::Ready(Err(ActionError::Prerequisite(Arc::new(refusal))));
                        }
                    }
                }
                Phase::Reissue => {
                    if !cx.may_begin_this_tick() {
                        return Poll::Pending;
                    }
                    let handle = std::task::ready!(crate::native::defer_budget(
                        actions.begin::<M>(M::clone_args(&self.args), cx)
                    ))?;
                    self.budget.re_issues += 1;
                    self.phase = Phase::Reaching(handle);
                    return Poll::Pending;
                }
                Phase::Cancelled => {
                    self.phase = Phase::Complete;
                    return Poll::Ready(Err(ActionError::Cancelled));
                }
                Phase::Complete => return Poll::Ready(Err(ActionError::Stale)),
            }
        }
    }
}

fn resolution(
    error: ActionError,
    allow: PrereqAllow,
    cx: &ActionContext<'_>,
    budget: &PrereqBudget,
) -> Result<PrereqRun, ActionError> {
    let (gates, entry) = match error {
        ActionError::NeedsStep(gates) => (gates, RunEntry::NeedsStep),
        ActionError::GateRefused(observation) => (
            Arc::clone(&observation.gates),
            RunEntry::GateRefused(observation),
        ),
        error => return Err(error),
    };
    let Some(gate) = PrereqRun::first_supported(&gates, &entry, cx) else {
        return Err(ActionError::NeedsEvidence(
            gates.iter().map(|gate| gate.gate.clone()).collect(),
        ));
    };
    if budget.re_issues >= PrereqBudget::MAX_RE_ISSUES {
        let refusal = match entry {
            RunEntry::NeedsStep => PrereqRefusal::StepRepeated { gate: gate.gate },
            RunEntry::GateRefused(observation) => PrereqRefusal::RefusedAfterStep {
                step: PrereqRun::identity(&gate, cx)?,
                observation,
            },
        };
        return Err(ActionError::Prerequisite(Arc::new(refusal)));
    }
    Ok(PrereqRun::begin(gate, entry, allow))
}
