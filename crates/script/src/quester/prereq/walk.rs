//! One logical walk, with bounded prerequisite resolution between native walks.
use super::run::{PrereqBudget, PrereqRun, RepairWalk, RunEntry};
use crate::local_state::SlotState;
use crate::native::walk::Walk;
use crate::native::{
    ActionContext, ActionError, ActionHandle, ActionLink, ActionRevoker, HandoverEntry,
    NativeActions, OuterIdentity, OuterWalk, PrereqAllow, PrereqRefusal, WalkAllow, WalkEnd,
    WalkHandover, WalkReceipt, WalkRefusal, WalkRequest,
};
use api::prereq_steps::ActionableGate;
use api::selected::QuestGate;
use std::sync::Arc;
use std::task::Poll;

enum Phase {
    Walking(ActionHandle<Walk>),
    Entry(HandoverEntry),
    Resolving(Box<PrereqRun>),
    Reissue,
    Cancelled,
    Complete,
}

pub struct PrereqWalk {
    request: Option<WalkRequest>,
    outer: Option<OuterWalk>,
    phase: Phase,
    budget: PrereqBudget,
    last_receipt: Option<WalkReceipt>,
    allow: PrereqAllow,
    revocation: Option<ActionLink>,
}

impl PrereqWalk {
    pub fn begin(
        request: WalkRequest,
        actions: &mut NativeActions,
        cx: &mut ActionContext<'_>,
    ) -> Result<Self, ActionError> {
        let handle = actions.begin::<Walk>(request.clone(), cx)?;
        Ok(Self {
            request: Some(request),
            outer: None,
            phase: Phase::Walking(handle),
            budget: PrereqBudget::default(),
            last_receipt: None,
            allow: PrereqAllow::default(),
            revocation: None,
        })
    }

    pub fn from_handover(handover: WalkHandover) -> Self {
        let allow = handover.outer.prereq_allow;
        Self {
            request: None,
            outer: Some(handover.outer),
            phase: Phase::Entry(handover.entry),
            budget: PrereqBudget::default(),
            last_receipt: None,
            allow,
            revocation: None,
        }
    }

    pub fn outer_identity(&self) -> Option<OuterIdentity> {
        self.outer.as_ref().map(|outer| outer.identity)
    }

    fn current_revoker(&self) -> Option<ActionRevoker> {
        match &self.phase {
            Phase::Walking(handle) => Some(handle.revoker()),
            Phase::Resolving(run) => run.revoker(),
            _ => None,
        }
    }

    /// Pair cancellation follows the logical action across native owners.
    /// Ordinary walks allocate no link unless a caller requests this capability.
    pub fn revoker(&mut self) -> Option<ActionRevoker> {
        if self.revocation.is_none() {
            self.revocation = Some(ActionLink::new(self.current_revoker()?));
        }
        self.revocation.as_ref().map(ActionLink::revoker)
    }

    pub fn walk_handle(&self) -> Option<&ActionHandle<Walk>> {
        match &self.phase {
            Phase::Walking(handle) => Some(handle),
            Phase::Resolving(run) => run.walk_handle(),
            _ => None,
        }
    }

    /// Dropping the only live handle revokes its queued requests. Do not click a page.
    pub fn cancel(&mut self) {
        if let Some(link) = &self.revocation {
            link.revoker().revoke();
        }
        self.phase = Phase::Cancelled;
    }

    fn receipt(&mut self, cx: &ActionContext<'_>, end: WalkEnd) -> WalkReceipt {
        let mut receipt = self.last_receipt.take().unwrap_or_else(|| WalkReceipt {
            request_id: match self.outer_identity() {
                Some(OuterIdentity::Compat { request_id }) => request_id,
                _ => 0,
            },
            evidence: cx.evidence(),
            goal: None,
            end: end.clone(),
            blocked: None,
            detail: None,
            refusal: None,
            assessment: None,
            escape: None,
            prereq_allow: Some(self.allow),
        });
        receipt.end = end;
        receipt.evidence = cx.evidence();
        receipt.prereq_allow = Some(self.allow);
        receipt
    }

    fn refused(&mut self, cx: &ActionContext<'_>, refusal: PrereqRefusal) -> WalkReceipt {
        let end = match refusal {
            PrereqRefusal::StepFailed { .. }
            | PrereqRefusal::StepUnwitnessed { .. }
            | PrereqRefusal::StepRepeated { .. }
            | PrereqRefusal::RefusedAfterStep { .. } => WalkEnd::Blocked,
            _ => WalkEnd::Refused,
        };
        let mut receipt = self.receipt(cx, end);
        receipt.detail = Some(Arc::from(refusal.to_string()));
        receipt.refusal = Some(WalkRefusal::Prerequisite(Arc::new(refusal)));
        receipt
    }

    fn unresolved(&mut self, cx: &ActionContext<'_>, gates: &[ActionableGate]) -> WalkReceipt {
        let gates: Arc<[QuestGate]> = gates.iter().map(|gate| gate.gate.clone()).collect();
        self.receipt(cx, WalkEnd::NeedsEvidence(gates))
    }

    fn request(&self, cx: &ActionContext<'_>) -> WalkRequest {
        let mut request = if let Some(request) = &self.request {
            request.clone()
        } else {
            let outer = self.outer.as_ref().expect("handover retains original walk");
            WalkRequest {
                target: outer.target,
                candidate_goals: None,
                loc_id: None,
                radius: outer.radius,
                arrival: outer.arrival,
                options: outer.options,
                required_after: cx.evidence(),
                evidence: None,
                cross: outer.cross.clone(),
                avoid: outer.avoid.clone(),
                protect: false,
                food_guard: false,
                allow: WalkAllow::default(),
            }
        };
        request.required_after = cx.evidence();
        request
    }

    pub fn poll(
        &mut self,
        actions: &mut NativeActions,
        cx: &mut ActionContext<'_>,
        state: Option<&SlotState>,
    ) -> Poll<Result<WalkReceipt, ActionError>> {
        if self.revocation.as_ref().is_some_and(ActionLink::revoked) {
            self.cancel();
        }
        let result = self.poll_inner(actions, cx, state);
        if let Some(link) = &self.revocation {
            link.replace(self.current_revoker());
        }
        result
    }

    fn poll_inner(
        &mut self,
        actions: &mut NativeActions,
        cx: &mut ActionContext<'_>,
        state: Option<&SlotState>,
    ) -> Poll<Result<WalkReceipt, ActionError>> {
        if !cx.eligible() {
            return Poll::Pending;
        }
        loop {
            match &mut self.phase {
                Phase::Walking(handle) => {
                    let mut receipt = std::task::ready!(actions.poll(handle, cx))?;
                    // The original admission fixes consent for the whole logical walk.
                    if self.budget.re_issues == 0 && self.outer.is_none() {
                        self.allow = receipt.prereq_allow.unwrap_or_default();
                    }
                    receipt.prereq_allow = Some(self.allow);
                    let entry = match &receipt.end {
                        WalkEnd::NeedsStep(gates) => {
                            Some(HandoverEntry::NeedsStep(Arc::clone(gates)))
                        }
                        WalkEnd::GateRefused(observation) => {
                            Some(HandoverEntry::GateRefused(Arc::clone(observation)))
                        }
                        _ => None,
                    };
                    if let Some(entry) = entry {
                        self.last_receipt = Some(receipt);
                        self.phase = Phase::Entry(entry);
                    } else {
                        self.phase = Phase::Complete;
                        return Poll::Ready(Ok(receipt));
                    }
                }
                Phase::Entry(entry) => {
                    let (gates, run_entry) = match entry {
                        HandoverEntry::NeedsStep(gates) => (Arc::clone(gates), RunEntry::NeedsStep),
                        HandoverEntry::GateRefused(observation) => (
                            Arc::clone(&observation.gates),
                            RunEntry::GateRefused(Arc::clone(observation)),
                        ),
                    };
                    let Some(gate) = PrereqRun::first_supported(&gates, &run_entry, cx) else {
                        let receipt = self.unresolved(cx, &gates);
                        self.phase = Phase::Complete;
                        return Poll::Ready(Ok(receipt));
                    };
                    if self.budget.re_issues >= PrereqBudget::MAX_RE_ISSUES {
                        let refusal = match run_entry {
                            RunEntry::GateRefused(observation) => PrereqRefusal::RefusedAfterStep {
                                step: PrereqRun::identity(&gate, cx)?,
                                observation,
                            },
                            RunEntry::NeedsStep => PrereqRefusal::StepRepeated { gate: gate.gate },
                        };
                        let receipt = self.refused(cx, refusal);
                        self.phase = Phase::Complete;
                        return Poll::Ready(Ok(receipt));
                    }
                    let evidence = self
                        .request
                        .as_ref()
                        .and_then(|request| request.evidence.clone());
                    self.phase = Phase::Resolving(Box::new(
                        PrereqRun::begin(gate, run_entry, self.allow).with_evidence(evidence),
                    ));
                }
                Phase::Resolving(run) => {
                    let repair = if let Some(request) = self.request.as_ref() {
                        RepairWalk::from_request(request)
                    } else {
                        RepairWalk::from_outer(
                            self.outer.as_ref().expect("handover retains caller policy"),
                        )
                    };
                    match std::task::ready!(run.poll_with_walk(
                        actions,
                        cx,
                        state,
                        &mut self.budget,
                        repair
                    )) {
                        Ok(_) => self.phase = Phase::Reissue,
                        Err(refusal) => {
                            let receipt = self.refused(cx, refusal);
                            self.phase = Phase::Complete;
                            return Poll::Ready(Ok(receipt));
                        }
                    }
                }
                Phase::Reissue => {
                    if !cx.may_begin_this_tick() {
                        return Poll::Pending;
                    }
                    let request = self.request(cx);
                    let handle = std::task::ready!(crate::native::defer_budget(
                        actions.begin::<Walk>(request, cx)
                    ))?;
                    self.budget.re_issues += 1;
                    self.phase = Phase::Walking(handle);
                    return Poll::Pending;
                }
                Phase::Cancelled => {
                    let receipt = self.receipt(cx, WalkEnd::Cancelled);
                    self.phase = Phase::Complete;
                    return Poll::Ready(Ok(receipt));
                }
                Phase::Complete => return Poll::Ready(Err(ActionError::Stale)),
            }
        }
    }
}
