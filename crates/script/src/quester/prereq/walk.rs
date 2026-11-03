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

/// A reissue gets the current evidence stamp. Retaining the old 40-byte stamp
/// would enlarge every ordinary native walk without preserving any policy.
struct SavedNativeWalk {
    target: api::WorldTile,
    candidate_goals: Option<crate::native::WalkGoals>,
    loc_id: Option<i32>,
    radius: u16,
    arrival: nav_planner::contract::ArrivalKind,
    options: crate::native::WalkOptions,
    evidence: Option<Arc<dyn api::quest_progress::EvidenceProvider>>,
    cross: Box<[Arc<str>]>,
    avoid: Box<[Arc<str>]>,
    protect: bool,
    food_guard: bool,
    cure_guard: bool,
    allow: WalkAllow,
}

impl From<WalkRequest> for SavedNativeWalk {
    fn from(request: WalkRequest) -> Self {
        let WalkRequest {
            target,
            candidate_goals,
            loc_id,
            radius,
            arrival,
            options,
            required_after: _,
            evidence,
            cross,
            avoid,
            protect,
            food_guard,
            cure_guard,
            allow,
        } = request;
        Self {
            target,
            candidate_goals,
            loc_id,
            radius,
            arrival,
            options,
            evidence,
            cross,
            avoid,
            protect,
            food_guard,
            cure_guard,
            allow,
        }
    }
}

impl SavedNativeWalk {
    fn request(&self, required_after: api::quest_progress::EvidenceStamp) -> WalkRequest {
        WalkRequest {
            target: self.target,
            candidate_goals: self.candidate_goals.clone(),
            loc_id: self.loc_id,
            radius: self.radius,
            arrival: self.arrival,
            options: self.options,
            required_after,
            evidence: self.evidence.clone(),
            cross: self.cross.clone(),
            avoid: self.avoid.clone(),
            protect: self.protect,
            food_guard: self.food_guard,
            cure_guard: self.cure_guard,
            allow: self.allow,
        }
    }

    fn repair(&self) -> RepairWalk<'_> {
        RepairWalk {
            options: self.options,
            protect: self.protect,
            food_guard: self.food_guard,
            cure_guard: self.cure_guard,
            allow: self.allow,
            cross: &self.cross,
            avoid: &self.avoid,
            evidence: self.evidence.as_ref(),
        }
    }
}

enum OriginalWalk {
    Native(SavedNativeWalk),
    Handover(OuterWalk),
}

enum Phase {
    Walking(ActionHandle<Walk>),
    Entry(HandoverEntry),
    Resolving(Box<PrereqRun>),
    Reissue,
    Cancelled,
    Complete,
}

pub struct PrereqWalk {
    original: OriginalWalk,
    phase: Phase,
    budget: PrereqBudget,
    last_receipt: Option<Box<WalkReceipt>>,
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
            original: OriginalWalk::Native(request.into()),
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
            original: OriginalWalk::Handover(handover.outer),
            phase: Phase::Entry(handover.entry),
            budget: PrereqBudget::default(),
            last_receipt: None,
            allow,
            revocation: None,
        }
    }

    pub fn outer_identity(&self) -> Option<OuterIdentity> {
        match &self.original {
            OriginalWalk::Native(_) => None,
            OriginalWalk::Handover(outer) => Some(outer.identity),
        }
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
        if matches!(self.phase, Phase::Complete) {
            return;
        }
        if let Some(link) = &self.revocation {
            link.revoker().revoke();
        }
        self.phase = Phase::Cancelled;
    }

    fn receipt(&mut self, cx: &ActionContext<'_>, end: WalkEnd) -> WalkReceipt {
        let mut receipt = self
            .last_receipt
            .take()
            .map(|receipt| *receipt)
            .unwrap_or_else(|| WalkReceipt {
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
        match &self.original {
            OriginalWalk::Native(request) => request.request(cx.evidence()),
            OriginalWalk::Handover(outer) => WalkRequest {
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
                cure_guard: false,
                allow: WalkAllow::default(),
            },
        }
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
                    if self.budget.re_issues == 0
                        && matches!(&self.original, OriginalWalk::Native(_))
                    {
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
                        if let Some(retained) = self.last_receipt.as_mut() {
                            **retained = receipt;
                        } else {
                            self.last_receipt = Some(Box::new(receipt));
                        }
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
                    let evidence = match &self.original {
                        OriginalWalk::Native(request) => request.evidence.clone(),
                        OriginalWalk::Handover(_) => None,
                    };
                    self.phase = Phase::Resolving(Box::new(
                        PrereqRun::begin(gate, run_entry, self.allow).with_evidence(evidence),
                    ));
                }
                Phase::Resolving(run) => {
                    let repair = match &self.original {
                        OriginalWalk::Native(request) => request.repair(),
                        OriginalWalk::Handover(outer) => RepairWalk::from_outer(outer),
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

#[cfg(test)]
mod tests {
    use super::*;
    use api::quest_progress::EvidenceStamp;
    use api::selected::RunKey;

    #[test]
    fn saving_native_policy_allocates_nothing_and_does_not_retain_the_old_stamp() {
        let original = EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 2,
                session: 3,
            },
            tick: 4,
            sequence: 5,
        };
        let next = EvidenceStamp {
            tick: 6,
            sequence: 7,
            ..original
        };
        let target = api::WorldTile {
            x: 10,
            z: 20,
            level: 0,
        };
        let mut request =
            crate::quester::families::reach::walk_request(target, 2, Some(42), original);
        assert!(
            !request.cure_guard,
            "ordinary requests keep the false default"
        );
        request.options.allow_teleports = crate::native::WalkBit::Forbid;
        request.protect = true;
        request.food_guard = true;
        request.cure_guard = true;
        request.allow.food = false;
        let mut saved = None;
        let measured = allocation_counter::measure(|| {
            saved = Some(SavedNativeWalk::from(request));
        });
        let saved = saved.unwrap();
        let reissued = saved.request(next);
        assert_eq!(measured.count_total, 0);
        assert_eq!(reissued.required_after, next);
        assert_eq!(reissued.target, target);
        assert_eq!(reissued.radius, 2);
        assert_eq!(reissued.loc_id, Some(42));
        assert_eq!(
            reissued.options.allow_teleports,
            crate::native::WalkBit::Forbid
        );
        assert!(reissued.protect && reissued.food_guard);
        assert!(!reissued.allow.food);
        assert!(reissued.cure_guard);
        assert!(saved.repair().cure_guard);
        assert!(
            std::mem::size_of::<SavedNativeWalk>() + std::mem::size_of::<EvidenceStamp>()
                <= std::mem::size_of::<WalkRequest>()
        );
    }
}
