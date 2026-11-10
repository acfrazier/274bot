use super::ledger::{
    BankDepositTrace, HostAction, HostCustody, HostEffect, PendingPick, PickChannel, PickReceipt,
};
use super::owner::Owner;
use super::*;
use crate::native_bank::{
    BankPickReceipt, BankPickRequest, ServicePickReceipt, ServicePickRequest,
};
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};

/// Add the stage's declared crossings to this step's. A non-empty set is the
/// script's request to cross danger zones on this walk. The host decides
/// permission (global override or this request) and keeps the survival check.
/// The food guard is armed for the whole walk.
fn apply_stage_cross(request: &mut WalkRequest, stage: Option<&[Arc<str>]>) {
    if let Some(stage) = stage {
        let duplicates = request
            .cross
            .iter()
            .enumerate()
            .filter(|(index, zone)| request.cross[..*index].contains(zone))
            .count();
        let missing = stage
            .iter()
            .filter(|zone| !request.cross.contains(zone))
            .count();
        if missing != 0 || duplicates != 0 {
            let mut cross = Vec::with_capacity(request.cross.len() - duplicates + missing);
            for zone in request.cross.iter().chain(stage) {
                if !cross.contains(zone) {
                    cross.push(Arc::clone(zone));
                }
            }
            request.cross = cross.into_boxed_slice();
        }
    }
    if !request.cross.is_empty() && !request.cure_guard {
        request.food_guard = true;
        request.allow.food = true;
    }
}

impl ActionContext<'_> {
    pub(super) fn owner(&self) -> Result<Arc<Owner>, ActionError> {
        let owner = self
            .ledger
            .as_ref()
            .and_then(|ledger| ledger.owner.as_ref())
            .ok_or(ActionError::Stale)?;
        if owner.run != self.run() || owner.id.get() != self.action_id {
            return Err(ActionError::Stale);
        }
        if !owner.live() {
            return Err(ActionError::Cancelled);
        }
        if !self.eligible {
            return Err(ActionError::Held);
        }
        Ok(Arc::clone(owner))
    }

    /// Begin the quiet read of a page this owner opens. `deadline` is the
    /// bound this owner enforces itself; renew it with
    /// [`Self::renew_quiet_read`] when the owner arms a new window.
    pub fn begin_quiet_read(
        &mut self,
        request_id: u64,
        deadline: WaitDeadline,
    ) -> Result<QuietReadLease, ActionError> {
        let owner = self.owner()?;
        let request_id = NonZeroU64::new(request_id).ok_or(ActionError::Stale)?;
        let wall_now = self.wall_now;
        let ledger = self.ledger.as_mut().expect("owner checked");
        let lease_id = ledger.next_id()?;
        if !owner.acquire_quiet(lease_id) {
            return Err(ActionError::Busy);
        }
        ledger.quiet_lease = Some(ledger::QuietRead {
            request: request_id,
            lease: lease_id,
            since: wall_now,
            hold_until: wall_now + ledger::QUIET_READ_WALL_BOUND,
        });
        ledger.quiet_deadline = Some((lease_id, deadline));
        Ok(QuietReadLease { owner, lease_id })
    }

    /// The owner armed a new window for its quiet read, as it sends its click
    /// or its close, or captures the page. The lease's wall bound runs from
    /// this poll, so the verb that the window follows never goes out against a
    /// hold that ends in the same frame. The paint-hide cap does not move.
    pub fn renew_quiet_read(&mut self, lease: &QuietReadLease, deadline: WaitDeadline) {
        let wall_now = self.wall_now;
        if let Some(ledger) = self.ledger.as_mut() {
            if let Some(quiet) = ledger
                .quiet_lease
                .as_mut()
                .filter(|quiet| quiet.lease == lease.lease_id)
            {
                quiet.hold_until = wall_now + ledger::QUIET_READ_WALL_BOUND;
                ledger.quiet_deadline = Some((lease.lease_id, deadline));
            }
        }
    }

    pub fn end_quiet_read(&mut self, lease: QuietReadLease) {
        drop(lease);
    }

    /// Begin one wait for the page this owner's own interaction opens. A newer
    /// lease of the same owner replaces the older one. Lease numbers come from
    /// their own sequence, so request ids are unchanged. `deadline` is the
    /// bound this owner enforces itself; renew it with
    /// [`Self::renew_modal_wait`] when the owner's bound moves.
    pub fn begin_modal_wait(
        &mut self,
        deadline: WaitDeadline,
    ) -> Result<ModalWaitLease, ActionError> {
        let owner = self.owner()?;
        let ledger = self.ledger.as_mut().expect("owner checked");
        ledger.modal_wait_seq = ledger.modal_wait_seq.wrapping_add(1);
        let lease_id = NonZeroU64::new(ledger.modal_wait_seq).ok_or(ActionError::Cancelled)?;
        ledger.modal_wait_deadline = Some((lease_id, deadline));
        if !owner.set_modal_wait(lease_id) {
            return Err(ActionError::Cancelled);
        }
        Ok(ModalWaitLease { owner, lease_id })
    }

    /// The owner's bound for its live modal wait moved.
    pub fn renew_modal_wait(&mut self, lease: &ModalWaitLease, deadline: WaitDeadline) {
        if let Some(ledger) = self.ledger.as_mut() {
            if ledger
                .modal_wait_deadline
                .is_some_and(|(id, _)| id == lease.lease_id)
            {
                ledger.modal_wait_deadline = Some((lease.lease_id, deadline));
            }
        }
    }

    pub fn charge_transition(&mut self) -> bool {
        self.eligible && self.budget.transition()
    }

    /// This tick already spent its interaction or walk event allowance.
    pub(crate) fn interaction_event_spent(&self) -> bool {
        self.budget.events != 0
    }

    /// The one same-tick continuation rule (TICK-FIX #4/#5/#6/#7): a caller
    /// that has just begun a machine, or whose machine or step has just
    /// completed, carries on within this observed tick (polls the new handle,
    /// evaluates the settle, begins the next one) only while the tick's single
    /// interaction event is unspent. A `begin` that already emitted (Walk,
    /// Reach, Talk-to, OneOp) leaves nothing a same-tick poll could add; one
    /// that did not (Select, Open, BankMachine, JournalMachine, a Dialogue
    /// adopting a live page, Buy, Make, Combat, Wear) acts now instead of a
    /// tick later. This never replenishes the per-tick budget and never
    /// dispatches another `on_game_tick`.
    ///
    /// Nor does it continue on the tick that first shows the player on a new
    /// level or in a rebuilt build area ([`Self::entered_scene`]); the work
    /// starts on the next tick instead.
    pub(crate) fn may_continue_this_tick(&self) -> bool {
        self.eligible && !self.interaction_event_spent() && !self.entered_scene()
    }

    /// A seat continuation keeps its owned arguments until admission can spend
    /// a transition. This check does not consume or renew the slot's budget.
    pub fn may_begin_this_tick(&self) -> bool {
        self.may_continue_this_tick() && self.budget.transition_available()
    }

    /// This observed tick is the first to show the player on a new level or
    /// in a rebuilt build area (`ledger::TickBudget::entered_scene`): its
    /// frame can still hold the old locs, so a loc op chosen from it can name
    /// a loc the server has since changed. Same-tick continuation and walk
    /// arrival wait for the next tick.
    pub(crate) fn entered_scene(&self) -> bool {
        self.budget.entered_scene()
    }

    /// The open bank session `generation` is known to withdraw as items:
    /// this account's native kernel opened it, and the server's open label
    /// resets `%bankcert` to 0 (`content/scripts/interface_bank/scripts/
    /// bank.rs2:18-19`), or it pressed Item in it. No native path presses Note.
    pub(crate) fn bank_item_mode(&self, generation: u64) -> bool {
        self.bank_mark(generation)
            .is_some_and(|mark| mark.item_mode)
    }

    /// Record that bank session `generation` withdraws as items. A machine
    /// that only pressed Item in a session it found open does not make that
    /// session its own open.
    pub(crate) fn note_bank_item_mode(&mut self, generation: u64) {
        let run = self.run();
        let opened_by_machine = self
            .bank_mark(generation)
            .is_some_and(|mark| mark.opened_by_machine);
        if let Some(ledger) = self.ledger.as_mut() {
            ledger.bank_session = Some(ledger::BankSessionMark {
                run,
                generation,
                item_mode: true,
                opened_by_machine,
            });
        }
    }

    /// Record that this account's native bank machine sent its own open
    /// request and bank session `generation` appeared after it. That session
    /// starts in Item mode (`bank.rs2:18-19`).
    pub(crate) fn note_bank_opened_by_machine(&mut self, generation: u64) {
        let run = self.run();
        if let Some(ledger) = self.ledger.as_mut() {
            ledger.bank_session = Some(ledger::BankSessionMark {
                run,
                generation,
                item_mode: true,
                opened_by_machine: true,
            });
        }
    }

    /// The open bank session `generation` of this login run is the one a
    /// native bank machine of this account opened with its own open request.
    pub(crate) fn bank_opened_by_machine(&self, generation: u64) -> bool {
        self.bank_mark(generation)
            .is_some_and(|mark| mark.opened_by_machine)
    }

    fn bank_mark(&self, generation: u64) -> Option<ledger::BankSessionMark> {
        let run = self.run();
        self.ledger
            .as_ref()?
            .bank_session
            .filter(|mark| mark.run == run && mark.generation == generation)
    }

    /// Queue one game interaction. Walks go through [`Self::walk`], which owns
    /// their follow and terminal receipt; route, channel, mouse, run-policy
    /// and lifecycle requests are host-owned and never native interactions.
    pub fn emit(&mut self, request: InteractReq) -> Result<u64, ActionError> {
        self.emit_with_bank_deposit(request, None)
    }

    pub(crate) fn emit_bank_deposit(
        &mut self,
        request: InteractReq,
        trace: &Arc<BankDepositTrace>,
    ) -> Result<u64, ActionError> {
        self.emit_with_bank_deposit(request, Some(Arc::clone(trace)))
    }

    fn emit_with_bank_deposit(
        &mut self,
        request: InteractReq,
        bank_deposit: Option<Arc<BankDepositTrace>>,
    ) -> Result<u64, ActionError> {
        let owner = self.owner()?;
        if !native_interaction(&request) {
            return Err(not_an_interaction());
        }
        if !self.budget.event(false) {
            return Err(ActionError::BudgetExhausted);
        }
        let ledger = self.ledger.as_mut().expect("owner checked");
        let request_id = ledger.next_id()?;
        owner.set_interaction(request_id);
        ledger.interaction = None;
        ledger.interaction_request = Some(request_id);
        ledger.outbox.push(HostAction {
            custody: HostCustody::interaction(owner, request_id),
            request_id,
            batch: 0,
            effect: HostEffect::Interaction(request),
            observed_walk_outcome_seq: self.observed_walk_outcome_seq,
            bank_deposit,
        });
        Ok(request_id.get())
    }
    /// Admit one ordered, wire-bounded interaction batch by taking ownership
    /// of its compact request prefix.
    pub fn emit_batch(&mut self, mut rows: [Option<InteractReq>; 5]) -> Result<u64, ActionError> {
        let owner = self.owner()?;
        let len = validate_batch(&rows, self.revision)?;
        let ledger = self.ledger.as_ref().expect("owner checked");
        if self.budget.events != 0
            || ledger.outbox.capacity().saturating_sub(ledger.outbox.len()) < len
            || owner.batch_free() < len
        {
            return Err(ActionError::BudgetExhausted);
        }
        let first_id = ledger.id_range(len)?;
        if !owner.acquire_batch(first_id, len) {
            return Err(ActionError::BudgetExhausted);
        }
        if !owner.live() {
            release_batch(&owner, first_id, len);
            return Err(ActionError::Cancelled);
        }
        if !self.budget.batch() {
            release_batch(&owner, first_id, len);
            return Err(ActionError::BudgetExhausted);
        }

        let ledger = self.ledger.as_mut().expect("owner checked");
        ledger.commit_id_range(first_id, len);
        for (offset, row) in rows.iter_mut().take(len).enumerate() {
            let request = row.take().expect("validated compact batch prefix");
            let request_id =
                NonZeroU64::new(first_id.get() + offset as u64).expect("validated id range");
            ledger.outbox.push(HostAction {
                custody: HostCustody::owner(Arc::clone(&owner)),
                request_id,
                batch: first_id.get(),
                effect: HostEffect::Interaction(request),
                observed_walk_outcome_seq: self.observed_walk_outcome_seq,
                bank_deposit: None,
            });
        }
        owner.set_latest_batch(first_id);
        Ok(first_id.get())
    }

    /// Queue a slot-exact drop without superseding another disposal request.
    /// Dispatch receipts release authority; inventory observation proves loss.
    pub fn emit_disposal(&mut self, request: InteractReq) -> Result<u64, ActionError> {
        let owner = self.owner()?;
        if !matches!(&request, InteractReq::Held { action, slot: Some(_), .. } if action == "Drop")
        {
            static REASON: LazyLock<Arc<str>> =
                LazyLock::new(|| Arc::from("disposal requires a slot-exact Drop"));
            return Err(ActionError::Unavailable(Arc::clone(&REASON)));
        }
        let ledger = self.ledger.as_ref().expect("owner checked");
        if self.budget.events >= 5
            || ledger.outbox.capacity() == ledger.outbox.len()
            || owner.batch_free() == 0
        {
            return Err(ActionError::BudgetExhausted);
        }
        let request_id = ledger.id_range(1)?;
        if !owner.acquire_batch(request_id, 1) {
            return Err(ActionError::BudgetExhausted);
        }
        if !owner.live() {
            owner.cancel_interaction(request_id);
            return Err(ActionError::Cancelled);
        }
        if !self.budget.event(true) {
            owner.cancel_interaction(request_id);
            return Err(ActionError::BudgetExhausted);
        }
        let ledger = self.ledger.as_mut().expect("owner checked");
        ledger.commit_id_range(request_id, 1);
        ledger.outbox.push(HostAction {
            custody: HostCustody::owner(owner),
            request_id,
            batch: 0,
            effect: HostEffect::Interaction(request),
            observed_walk_outcome_seq: self.observed_walk_outcome_seq,
            bank_deposit: None,
        });
        Ok(request_id.get())
    }

    /// Queue one selector-owned bank pick without a game-interaction event.
    pub fn bank_pick(&mut self, request: BankPickRequest) -> Result<u64, ActionError> {
        self.emit_pick(HostEffect::BankPick(request), PickChannel::Bank)
    }

    /// Queue a complete-domain service route pick without a game-interaction event.
    pub fn service_pick(&mut self, request: ServicePickRequest) -> Result<u64, ActionError> {
        self.emit_pick(HostEffect::ServicePick(request), PickChannel::Service)
    }

    fn emit_pick(&mut self, effect: HostEffect, channel: PickChannel) -> Result<u64, ActionError> {
        let owner = self.owner()?;
        let ledger = self.ledger.as_mut().expect("owner checked");
        let request_id = ledger.next_id()?;
        owner.set_interaction(request_id);
        ledger.interaction = None;
        ledger.interaction_request = None;
        ledger.pick = None;
        ledger.pick_request = Some(PendingPick {
            request: request_id,
            channel,
        });
        ledger.outbox.push(HostAction {
            custody: HostCustody::owner(owner),
            request_id,
            batch: 0,
            effect,
            observed_walk_outcome_seq: self.observed_walk_outcome_seq,
            bank_deposit: None,
        });
        Ok(request_id.get())
    }

    /// Read only the live matching bank-channel result.
    pub fn bank_pick_receipt(&self, request_id: u64) -> Option<&BankPickReceipt> {
        match self.pick_receipt(request_id)? {
            PickReceipt::Bank(receipt) => Some(receipt),
            PickReceipt::Service(_) => None,
        }
    }

    /// Read only the live matching typed service-channel result.
    pub fn service_pick_receipt(&self, request_id: u64) -> Option<&ServicePickReceipt> {
        match self.pick_receipt(request_id)? {
            PickReceipt::Service(receipt) => Some(receipt),
            PickReceipt::Bank(_) => None,
        }
    }

    fn pick_receipt(&self, request_id: u64) -> Option<&PickReceipt> {
        let ledger = self.ledger.as_ref()?;
        let owner = ledger.owner.as_ref()?;
        let pending = ledger.pick_request?;
        if owner.run != self.run()
            || owner.id.get() != self.action_id
            || !owner.interaction_live(pending.request)
            || pending.request.get() != request_id
        {
            return None;
        }
        ledger.pick.as_ref().filter(|receipt| {
            receipt.request_id() == request_id && receipt.evidence().run == self.run()
        })
    }

    /// Queue a compute-only walk assessment. It shares the owner's outbox but
    /// neither spends a game-event allowance nor claims movement authority.
    pub fn assess_walk(&mut self, mut request: WalkRequest) -> Result<u64, ActionError> {
        if !self.eligible {
            return Err(ActionError::Held);
        }
        let run = self.run();
        if request.required_after.run != run {
            return Err(ActionError::Stale);
        }
        if request.candidate_goals.is_some()
            && (request.arrival != nav_planner::contract::ArrivalKind::Area
                || request.loc_id.is_some()
                || request.radius != 0)
        {
            return Err(ActionError::WalkGoals(
                super::WalkGoalsError::IncompatibleArrival,
            ));
        }
        apply_stage_cross(&mut request, self.stage_cross.as_deref());
        let ledger = self.ledger.get_or_insert_with(Default::default);
        let owner = match ledger.assess_owner.as_ref() {
            Some(owner) if owner.run == run && owner.live() => Arc::clone(owner),
            _ => {
                if let Some(owner) = ledger.assess_owner.take() {
                    owner.revoke();
                }
                let owner = Owner::new(run, ledger.next_id()?);
                ledger.assess_owner = Some(Arc::clone(&owner));
                owner
            }
        };
        let request_id = ledger.next_id()?;
        owner.set_interaction(request_id);
        ledger.assess_receipt = None;
        ledger.assess_request = Some(request_id);
        ledger
            .outbox
            .retain(|action| !matches!(&action.effect, HostEffect::AssessWalk(_)));
        ledger.outbox.push(HostAction {
            custody: HostCustody::owner(owner),
            request_id,
            batch: 0,
            effect: HostEffect::AssessWalk(request),
            observed_walk_outcome_seq: self.observed_walk_outcome_seq,
            bank_deposit: None,
        });
        Ok(request_id.get())
    }

    pub fn assess_receipt(&self, request_id: u64) -> Option<&AssessReceipt> {
        let ledger = self.ledger.as_ref()?;
        let owner = ledger.assess_owner.as_ref()?;
        if owner.run != self.run() || !owner.live() {
            return None;
        }
        ledger.assess_receipt.as_ref().filter(|receipt| {
            ledger
                .assess_request
                .is_some_and(|id| id.get() == request_id)
                && receipt.request_id == request_id
                && receipt.evidence.run == self.run()
        })
    }

    /// Consume the retained advisory and revoke its host continuation so a
    /// duplicate or late publication cannot restore it.
    pub fn take_assess_receipt(&mut self, request_id: u64) -> Option<AssessReceipt> {
        let receipt = self.assess_receipt(request_id)?.clone();
        self.cancel_request(request_id);
        Some(receipt)
    }

    pub fn walk(&mut self, mut request: WalkRequest) -> Result<u64, ActionError> {
        let owner = self.owner()?;
        if request.required_after.run != self.run() {
            return Err(ActionError::Stale);
        }
        if request.candidate_goals.is_some()
            && (request.arrival != nav_planner::contract::ArrivalKind::Area
                || request.loc_id.is_some()
                || request.radius != 0)
        {
            return Err(ActionError::WalkGoals(
                super::WalkGoalsError::IncompatibleArrival,
            ));
        }
        apply_stage_cross(&mut request, self.stage_cross.as_deref());
        if !self.budget.event(false) {
            return Err(ActionError::BudgetExhausted);
        }
        self.walk_prereq.apply(&mut request.options);
        let ledger = self.ledger.as_mut().expect("owner checked");
        let request_id = ledger.next_id()?;
        owner.set_walk(request_id);
        ledger.walk = None;
        ledger.walk_events.clear();
        ledger.outbox.push(HostAction {
            custody: HostCustody::walk(owner, request_id),
            request_id,
            batch: 0,
            effect: HostEffect::Walk(request),
            observed_walk_outcome_seq: self.observed_walk_outcome_seq,
            bank_deposit: None,
        });
        Ok(request_id.get())
    }

    pub fn walk_receipt(&self, request_id: u64) -> Option<&WalkReceipt> {
        let ledger = self.ledger.as_ref()?;
        let owner = ledger.owner.as_ref()?;
        if owner.run != self.run() || owner.id.get() != self.action_id || !owner.live() {
            return None;
        }
        ledger.walk.as_ref().filter(|receipt| {
            receipt.request_id == request_id && receipt.evidence.run == self.run()
        })
    }

    pub fn interaction_receipt(&self, request_id: u64) -> Option<&InteractionReceipt> {
        let ledger = self.ledger.as_ref()?;
        let owner = ledger.owner.as_ref()?;
        if owner.run != self.run() || owner.id.get() != self.action_id || !owner.live() {
            return None;
        }
        ledger
            .interaction
            .iter()
            .chain(ledger.batch_receipts.iter().flatten())
            .find(|receipt| receipt.request_id == request_id && receipt.evidence.run == self.run())
    }

    /// What the host's dispatch lifecycle has done with this action's single
    /// interaction `request_id` ([`InteractionDispatch`]). The settled receipt
    /// comes first. Without one, the request is `Queued` only while it is
    /// still this action's current single interaction and the host has not
    /// discarded it: a cancelled request (`cancel_request` cleared it) or a
    /// superseded one (a later emit, pick or assessment replaced it) is
    /// `Unsent`: its queued action, if one remains, has lost its authority
    /// (`HostAction::live`), so the host drops it and no receipt will come.
    pub fn interaction_dispatch(&self, request_id: u64) -> InteractionDispatch<'_> {
        if let Some(receipt) = self.interaction_receipt(request_id) {
            return InteractionDispatch::Settled(receipt);
        }
        let queued = self
            .ledger
            .as_ref()
            .zip(NonZeroU64::new(request_id))
            .is_some_and(|(ledger, request)| {
                ledger.interaction_request == Some(request)
                    && ledger.owner.as_ref().is_some_and(|owner| {
                        owner.run == self.run()
                            && owner.id.get() == self.action_id
                            && owner.interaction_queued(request)
                    })
            });
        if queued {
            InteractionDispatch::Queued
        } else {
            InteractionDispatch::Unsent
        }
    }

    /// Whether the host discarded this action's walk `request_id` before it
    /// armed the route (a guardian hold, a held tick, a session reset dropped
    /// its queued action). No walk receipt will come for it: the machine that
    /// still wants the walk sends it again.
    pub fn walk_unsent(&self, request_id: u64) -> bool {
        let Some((ledger, request)) = self.ledger.as_ref().zip(NonZeroU64::new(request_id)) else {
            return false;
        };
        ledger.owner.as_ref().is_some_and(|owner| {
            owner.run == self.run()
                && owner.id.get() == self.action_id
                && owner.walk_unsent(request)
        })
    }

    /// Where this tick stands after a guardian hold ([`AfterHold`]). Only a
    /// machine that keeps a request in flight across a hold looks at it:
    /// [`NativeActions::poll`] does for every machine that sets
    /// [`NativeMachine::RESUMES_AFTER_HOLD`], and a step that waits on a
    /// finished request's effect with no live handle does itself.
    pub fn after_hold(&self) -> AfterHold {
        self.budget.after_hold()
    }

    /// Consume the one resume that follows a guardian hold: `true` on the
    /// first observed tick that is later than the hold's settle tick. The
    /// caller then re-checks its goal and re-issues its request at most once,
    /// and logs it ([`log_hold_resume`]).
    pub fn take_hold_resume(&mut self) -> bool {
        self.budget.take_hold_resume()
    }

    /// Consume the teleport that the last guardian hold observed: the player's
    /// plane changed, or a jump between two observed frames was beyond what
    /// running covers in their ticks ([`is_teleport`]). `true` once.
    pub fn take_hold_teleport(&mut self) -> bool {
        self.budget.take_hold_teleport()
    }

    pub fn cancel_request(&mut self, request_id: u64) {
        let Some(ledger) = self.ledger.as_mut() else {
            return;
        };
        if let Some(request) = NonZeroU64::new(request_id) {
            if ledger.assess_request == Some(request)
                && ledger
                    .assess_owner
                    .as_ref()
                    .is_some_and(|owner| owner.run == self.evidence.run)
            {
                if let Some(owner) = ledger.assess_owner.as_ref() {
                    owner.cancel_interaction(request);
                }
                ledger.assess_request = None;
                ledger.assess_receipt = None;
            }
        }
        let Some(owner) = ledger.owner.as_ref() else {
            return;
        };
        if owner.run != self.evidence.run || owner.id.get() != self.action_id {
            return;
        }
        if let Some(request_id) = NonZeroU64::new(request_id) {
            owner.cancel_interaction(request_id);
            if ledger.interaction_request == Some(request_id) {
                ledger.interaction_request = None;
                ledger.interaction = None;
            }
            if ledger
                .pick_request
                .is_some_and(|pending| pending.request == request_id)
            {
                ledger.pick_request = None;
                ledger.pick = None;
            }

            for receipt in &mut ledger.batch_receipts {
                if receipt
                    .as_ref()
                    .is_some_and(|receipt| receipt.request_id == request_id.get())
                {
                    *receipt = None;
                }
            }
            owner.cancel_walk(request_id);
            if let Some(quiet) = ledger.quiet_lease {
                if quiet.request == request_id {
                    owner.release_quiet(quiet.lease);
                    ledger.quiet_lease = None;
                }
            }
            ledger
                .outbox
                .retain(|action| action.request_id != request_id);
        }
    }
}

/// Whether `emit` may queue `request`: a game interaction the host dispatches
/// once, not a route (the typed walk owns those), bank selection, broker
/// channel, script mouse, inspect request/ack, SetCameraYaw, host run policy,
/// GatherRun, GatherStop, ProgressRead or isolate lifecycle marker.
fn native_interaction(request: &InteractReq) -> bool {
    request.is_game()
        && !matches!(
            request,
            InteractReq::Walk { .. }
                | InteractReq::WalkNear { .. }
                | InteractReq::WalkNearestBank
                | InteractReq::AbortWalk { .. }
                | InteractReq::Mouse { .. }
        )
}

fn not_an_interaction() -> ActionError {
    // One process allocation, never a new error string per refusal.
    static REASON: LazyLock<Arc<str>> =
        LazyLock::new(|| Arc::from("not a native interaction: walks use ActionContext::walk"));
    ActionError::Unavailable(Arc::clone(&REASON))
}
const BATCH_ROWS: usize = 5;

fn batch_shape_error() -> ActionError {
    static REASON: LazyLock<Arc<str>> = LazyLock::new(|| Arc::from("batch shape"));
    ActionError::Unavailable(Arc::clone(&REASON))
}

fn selected_protect_component(revision: api::selected::ClientRevision, component_id: i32) -> bool {
    const PROTECT_NAMES: [&str; 3] = [
        "Protect from Magic",
        "Protect from Missiles",
        "Protect from Melee",
    ];
    api::game_data::for_revision(revision)
        .ok()
        .is_some_and(|data| {
            data.prayers().iter().any(|prayer| {
                PROTECT_NAMES.contains(&prayer.name.as_str()) && prayer.button_com == component_id
            })
        })
}

fn validate_batch(
    rows: &[Option<InteractReq>; BATCH_ROWS],
    revision: api::selected::ClientRevision,
) -> Result<usize, ActionError> {
    let mut len = 0;
    let mut ended = false;
    for row in rows {
        match row {
            Some(_) if ended => return Err(batch_shape_error()),
            Some(_) => len += 1,
            None => ended = true,
        }
    }
    if len == 0 {
        return Err(batch_shape_error());
    }

    let mut wire_cost = 0;
    let mut eats = 0;
    let mut terminal_seen = false;
    for (index, row) in rows.iter().take(len).enumerate() {
        let request = row.as_ref().expect("compact prefix was counted");
        if !native_interaction(request) {
            return Err(batch_shape_error());
        }
        let (cost, terminal) = match request {
            InteractReq::SideTab { tab: 0 } if len == 1 => (0, true),
            InteractReq::IfButton { .. }
            | InteractReq::Wear { .. }
            | InteractReq::SetRetaliate { .. } => (1, false),
            InteractReq::Held { action, .. } if action == "Eat" => {
                eats += 1;
                if eats > 2 {
                    return Err(batch_shape_error());
                }
                (1, eats == 2)
            }
            InteractReq::Held { action, .. } if action == "Drink" => (1, true),
            InteractReq::Npc { .. }
            | InteractReq::Player { .. }
            | InteractReq::UseWidgetOn { .. }
            | InteractReq::Obj { .. } => (2, true),
            _ => return Err(batch_shape_error()),
        };
        wire_cost += cost;
        if terminal {
            if terminal_seen || index + 1 != len {
                return Err(batch_shape_error());
            }
            terminal_seen = true;
        }
    }
    if wire_cost > 5 {
        return Err(batch_shape_error());
    }

    let mut repeated_component = None;
    for index in 0..len {
        let Some(InteractReq::IfButton { component_id }) = rows[index].as_ref() else {
            continue;
        };
        let repeated = (0..index).find(|&previous| {
            matches!(
                rows[previous].as_ref(),
                Some(InteractReq::IfButton {
                    component_id: prior
                }) if prior == component_id
            )
        });
        if let Some(previous) = repeated {
            if repeated_component.is_some()
                || index != previous + 1
                || !selected_protect_component(revision, *component_id)
            {
                return Err(batch_shape_error());
            }
            repeated_component = Some(*component_id);
        }
    }
    if repeated_component.is_some() && wire_cost > 4 {
        return Err(batch_shape_error());
    }
    Ok(len)
}

fn release_batch(owner: &Owner, first_id: NonZeroU64, len: usize) {
    for offset in 0..len {
        let request_id =
            NonZeroU64::new(first_id.get() + offset as u64).expect("validated id range");
        owner.cancel_interaction(request_id);
    }
}

impl QuietReadLease {
    /// Revalidate after a wall-clock safety expiry, including on resume while
    /// the machine's active clock has been frozen.
    pub fn live(&self) -> bool {
        self.owner.quiet(self.lease_id)
    }
}

impl Drop for QuietReadLease {
    fn drop(&mut self) {
        self.owner.release_quiet(self.lease_id);
    }
}

impl Drop for ModalWaitLease {
    fn drop(&mut self) {
        self.owner.release_modal_wait(self.lease_id);
    }
}

impl<M: NativeMachine> Drop for ActionHandle<M> {
    fn drop(&mut self) {
        // No user code runs until the host authority is invalidated. Contain a
        // second panic from cleanup during unwinding as well as ordinary Drop.
        self.owner.revoke();
        if let Some(mut machine) = self.machine.get_mut().take() {
            if let Err(payload) = catch_unwind(AssertUnwindSafe(|| machine.cancel())) {
                std::mem::forget(payload);
            }
            if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(machine))) {
                std::mem::forget(payload);
            }
        }
    }
}

impl<M: NativeMachine> ActionHandle<M> {
    pub(crate) fn machine_prayer_cleanup(&self) -> crate::combat::RaisedPrayers {
        let machine = self.machine.borrow();
        match machine.as_ref() {
            Some(machine) => machine.prayer_cleanup(),
            None => crate::combat::RaisedPrayers::empty(),
        }
    }

    /// Read the live machine without polling it; `None` once it completed. A
    /// live proof reads a machine's own state with it (the PvP V2 switch trace).
    pub fn inspect<R>(&self, read: impl FnOnce(&M) -> R) -> Option<R> {
        self.machine.borrow().as_ref().map(read)
    }

    /// Observe a live machine's receipts under its existing owner. This does
    /// not poll, send, or spend an observed-tick allowance.
    pub(crate) fn inspect_in<R>(
        &self,
        cx: &mut ActionContext<'_>,
        read: impl FnOnce(&M, &ActionContext<'_>) -> R,
    ) -> Option<R> {
        if self.owner.run != cx.run() || !self.owner.live() {
            return None;
        }
        cx.action_id = self.owner.id.get();
        cx.owner().ok()?;
        self.machine
            .borrow()
            .as_ref()
            .map(|machine| read(machine, cx))
    }
}

impl ActionHandle<crate::combat::Combat> {
    /// Give this attempt the stand-approach walks its quest step already spent.
    /// The step owns the count: a kill-budget end rebegins the fight inside it.
    pub(crate) fn seed_stand_walk_failures(&self, failures: u8) {
        if let Some(machine) = self.machine.borrow_mut().as_mut() {
            machine.seed_stand_walk_failures(failures);
        }
    }

    /// Read accepted combat prayer-cleanup debt without polling the machine.
    pub fn prayer_cleanup(&self) -> crate::combat::RaisedPrayers {
        let machine = self.machine.borrow();
        let Some(combat) = machine.as_ref() else {
            return crate::combat::RaisedPrayers::empty();
        };
        let plan_first_id = combat.prayer_plan_first_id();
        let accepted_prefix = self
            .owner
            .latest_batch_receipt()
            .filter(|(first_id, _)| *first_id == plan_first_id)
            .map(|(_, accepted_prefix)| accepted_prefix)
            .unwrap_or(0);
        combat.prayer_cleanup(accepted_prefix)
    }
}

impl NativeActions {
    pub fn begin<M: NativeMachine>(
        &mut self,
        args: M::Args,
        cx: &mut ActionContext<'_>,
    ) -> Result<ActionHandle<M>, ActionError> {
        if !cx.eligible {
            return Err(ActionError::Held);
        }
        let run = cx.run();
        let ledger = cx.ledger.get_or_insert_with(Default::default);
        if ledger.owner.as_ref().is_some_and(|owner| owner.live()) {
            return Err(ActionError::Busy);
        }
        if !cx.budget.transition() {
            return Err(ActionError::BudgetExhausted);
        }
        ledger.revoke_foreground();
        let owner = Owner::new(run, ledger.next_id()?);
        ledger.owner = Some(Arc::clone(&owner));
        cx.action_id = owner.id.get();
        // A machine that begins now was not interrupted by an earlier hold.
        cx.budget.clear_hold();
        let result = catch_unwind(AssertUnwindSafe(|| M::begin(args, cx)));
        match result {
            Ok(Ok(machine)) => Ok(ActionHandle {
                owner,
                machine: RefCell::new(Some(machine)),
            }),
            Ok(Err(error)) => {
                owner.revoke();
                Err(error)
            }
            Err(panic) => {
                owner.revoke();
                resume_unwind(panic)
            }
        }
    }

    /// Resume a retained request cursor after a kept session boundary.
    /// The caller selects the request from the closed reconnect-policy table;
    /// interrupted sub-operations and non-kept resets must not call this.
    pub fn resume_kept<M: NativeMachine>(
        &mut self,
        kind: super::request::NativeKind,
        handle: &mut ActionHandle<M>,
        cx: &mut ActionContext<'_>,
    ) -> Result<(), ActionError> {
        if kind.policy().reconnect() != api::script_contract::ReconnectPolicy::Resume {
            return Err(ActionError::Stale);
        }
        let previous = handle.owner.run;
        let run = cx.run();
        if handle.owner.live()
            || previous.slot != run.slot
            || previous.run != run.run
            || previous.session == run.session
            || handle.machine.get_mut().is_none()
        {
            return Err(ActionError::Stale);
        }
        if !cx.eligible {
            return Err(ActionError::Held);
        }
        let ledger = cx.ledger.get_or_insert_with(Default::default);
        if ledger.owner.as_ref().is_some_and(|owner| owner.live()) {
            return Err(ActionError::Busy);
        }
        if !cx.budget.transition() {
            return Err(ActionError::BudgetExhausted);
        }
        ledger.revoke_foreground();
        let owner = Owner::new(run, ledger.next_id()?);
        ledger.owner = Some(Arc::clone(&owner));
        cx.action_id = owner.id.get();
        cx.budget.clear_hold();
        handle.owner = owner;
        Ok(())
    }

    pub fn poll<M: NativeMachine>(
        &mut self,
        handle: &ActionHandle<M>,
        cx: &mut ActionContext<'_>,
    ) -> Poll<Result<M::Output, ActionError>> {
        if handle.owner.run != cx.run() {
            return Poll::Ready(Err(ActionError::Stale));
        }
        if !handle.owner.live() {
            return Poll::Ready(Err(ActionError::Cancelled));
        }
        cx.action_id = handle.owner.id.get();
        if !cx.eligible {
            return Poll::Pending;
        }
        if let Err(error) = cx.owner() {
            return Poll::Ready(Err(error));
        }
        if !cx.budget.transition() {
            return Poll::Pending;
        }
        let mut machine = handle.machine.borrow_mut();
        let Some(active) = machine.as_mut() else {
            return Poll::Ready(Err(ActionError::Cancelled));
        };
        if M::RESUMES_AFTER_HOLD {
            match cx.after_hold() {
                // The guardian's last request can still be taking effect.
                AfterHold::Settling => return Poll::Pending,
                AfterHold::Resume => {
                    let resumed = catch_unwind(AssertUnwindSafe(|| active.after_hold(cx)));
                    match resumed {
                        Ok(Ok(resume)) => {
                            cx.take_hold_resume();
                            log_hold_resume(&handle.owner.id.to_string(), resume);
                        }
                        // The tick's event allowance is spent: try next tick.
                        Ok(Err(ActionError::BudgetExhausted)) => return Poll::Pending,
                        Ok(Err(error)) => {
                            cx.take_hold_resume();
                            return Poll::Ready(Err(error));
                        }
                        Err(panic) => {
                            handle.owner.revoke();
                            resume_unwind(panic)
                        }
                    }
                }
                AfterHold::None => {}
            }
        }
        let result = catch_unwind(AssertUnwindSafe(|| active.poll(cx)));
        match result {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(result)) => {
                handle.owner.revoke();
                let completed = machine.take();
                drop(machine);
                drop(completed);
                Poll::Ready(result)
            }
            Err(panic) => {
                handle.owner.revoke();
                resume_unwind(panic)
            }
        }
    }
    /// Interrupt a live machine without completing or revoking its handle.
    pub fn interrupt<M: NativeMachine>(&mut self, handle: &ActionHandle<M>) {
        if !handle.owner.live() {
            return;
        }
        let mut machine = handle.machine.borrow_mut();
        let Some(active) = machine.as_mut() else {
            return;
        };
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| active.interrupt())) {
            resume_unwind(payload);
        }
    }

    /// Consume one warning from this walk's live request without completing
    /// its machine or spending an interaction/transition allowance.
    pub fn take_walk_event(
        &mut self,
        handle: &ActionHandle<super::walk::Walk>,
        cx: &mut ActionContext<'_>,
    ) -> Option<super::WalkEvent> {
        if handle.owner.run != cx.run() || !handle.owner.live() {
            return None;
        }
        let request = handle.owner.active_walk()?;
        let ledger = cx.ledger.as_mut()?;
        if !ledger
            .owner
            .as_ref()
            .is_some_and(|owner| Arc::ptr_eq(owner, &handle.owner))
        {
            return None;
        }
        let index = ledger.walk_events.iter().position(|event| {
            event.request_id == request.get() && event.evidence.run == handle.owner.run
        })?;
        Some(ledger.walk_events.remove(index))
    }
    /// Queue and observe advisory route work through the same frame authority
    /// used by the native action context.
    pub fn assess_walk(
        &mut self,
        request: WalkRequest,
        cx: &mut ActionContext<'_>,
    ) -> Result<u64, ActionError> {
        cx.assess_walk(request)
    }

    pub fn assess_receipt<'a>(
        &self,
        cx: &'a ActionContext<'_>,
        request_id: u64,
    ) -> Option<&'a AssessReceipt> {
        cx.assess_receipt(request_id)
    }

    pub fn take_assess_receipt(
        &mut self,
        cx: &mut ActionContext<'_>,
        request_id: u64,
    ) -> Option<AssessReceipt> {
        cx.take_assess_receipt(request_id)
    }

    pub fn cancel<M: NativeMachine>(&mut self, handle: ActionHandle<M>) {
        drop(handle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::selected::RunKey;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct PanickingCleanup {
        owner: Arc<Owner>,
        request: NonZeroU64,
        observations: Arc<AtomicUsize>,
    }

    impl NativeMachine for PanickingCleanup {
        type Args = Self;
        type Output = ();

        fn begin(args: Self, _: &mut ActionContext<'_>) -> Result<Self, ActionError> {
            Ok(args)
        }

        fn poll(&mut self, _: &mut ActionContext<'_>) -> Poll<Result<(), ActionError>> {
            Poll::Pending
        }

        fn cancel(&mut self) {
            if !self.owner.live()
                && !self.owner.walk_live(self.request)
                && !self.owner.quiet(self.request)
            {
                self.observations.fetch_add(1, Ordering::SeqCst);
            }
            panic!("machine cancellation failed");
        }
    }

    impl Drop for PanickingCleanup {
        fn drop(&mut self) {
            if !self.owner.live() {
                self.observations.fetch_add(1, Ordering::SeqCst);
            }
            panic!("machine destruction failed");
        }
    }

    #[test]
    fn handle_drop_revokes_walk_and_quiet_before_panicking_cleanup() {
        let request = NonZeroU64::new(2).unwrap();
        let owner = Owner::new(
            RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            NonZeroU64::new(1).unwrap(),
        );
        owner.set_walk(request);
        assert!(owner.acquire_quiet(request));
        let observations = Arc::new(AtomicUsize::new(0));
        let handle = ActionHandle {
            owner: Arc::clone(&owner),
            machine: RefCell::new(Some(PanickingCleanup {
                owner: Arc::clone(&owner),
                request,
                observations: Arc::clone(&observations),
            })),
        };
        drop(handle);
        assert_eq!(observations.load(Ordering::SeqCst), 2);
        assert!(!owner.walk_live(request));
        assert!(!owner.quiet(request));
    }

    /// One live machine owner and its action context at tick 1.
    fn with_live_owner(f: impl FnOnce(&mut ActionContext<'_>, &Arc<Owner>)) {
        let evidence = EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 1,
        };
        let mut ledger = Some(Box::new(ledger::Ledger::default()));
        let owner = Owner::new(evidence.run, ledger.as_mut().unwrap().next_id().unwrap());
        ledger.as_mut().unwrap().owner = Some(Arc::clone(&owner));
        let mut retained = RetainedMemory::default();
        let mut budget = ledger::TickBudget::default();
        budget.observe(1);
        let mut cx = ActionContext {
            stage_cross: None,
            walk_prereq: Default::default(),
            condition: None,
            poison_unknown_pulse: u8::MAX,
            cure_poison: crate::survival::CurePoison::WhenNeeded,
            evidence,
            revision: api::selected::ClientRevision::R289,
            snapshot: SnapshotView::new(None, evidence),
            retained: &mut retained,
            action_id: owner.id.get(),
            observed_walk_outcome_seq: 0,
            observed_walk_cancel_reason: crate::native::WalkCancelReason::None,
            shop_keepers: None,
            wilderness: None,
            active_now: Duration::ZERO,
            wall_now: Instant::now(),
            ledger: &mut ledger,
            pending_note_progress: None,
            budget: &mut budget,
            eligible: true,
        };
        f(&mut cx, &owner);
    }

    #[test]
    fn cancelling_a_drained_interaction_revokes_its_host_authority() {
        with_live_owner(|cx, owner| {
            let request = cx.emit(InteractReq::CloseModal).unwrap();
            let drained = cx.ledger.as_mut().unwrap().outbox.pop().unwrap();
            let authority = drained.authority();
            assert!(drained.live());
            assert_eq!(
                cx.interaction_dispatch(request),
                InteractionDispatch::Queued
            );
            cx.cancel_request(request);
            assert!(!drained.live(), "cancelled input must not reach the host");
            assert!(
                !authority.live(),
                "a captured continuation must lose authority"
            );
            assert!(
                owner.live(),
                "request cancellation must not cancel the machine"
            );
            assert_eq!(
                cx.interaction_dispatch(request),
                InteractionDispatch::Unsent,
                "a cancelled request is no longer queued while its action still exists"
            );
            drop(drained);
            assert_eq!(
                cx.interaction_dispatch(request),
                InteractionDispatch::Unsent
            );
        });
    }

    #[test]
    fn a_cancelled_queued_interaction_is_unsent_while_its_owner_stays_live() {
        with_live_owner(|cx, owner| {
            let request = cx.emit(InteractReq::CloseModal).unwrap();
            assert_eq!(
                cx.interaction_dispatch(request),
                InteractionDispatch::Queued
            );
            // Cancelling clears the owner's current interaction before the
            // queued action (and its armed custody) leaves the outbox.
            cx.cancel_request(request);
            assert!(cx.ledger.as_ref().unwrap().outbox.is_empty());
            assert!(owner.live());
            assert_eq!(
                cx.interaction_dispatch(request),
                InteractionDispatch::Unsent,
                "no action and no receipt remain for a cancelled request"
            );
        });
    }

    #[test]
    fn an_old_guard_dropping_after_a_new_request_starts_leaves_each_state_exact() {
        with_live_owner(|cx, owner| {
            let old = cx.emit(InteractReq::CloseModal).unwrap();
            let old_action = cx.ledger.as_mut().unwrap().outbox.pop().unwrap();
            cx.budget.observe(2);
            let new = cx.emit(InteractReq::CloseModal).unwrap();
            assert!(owner.live());
            assert!(
                !old_action.live(),
                "the host must not send a superseded request"
            );
            assert_eq!(
                cx.interaction_dispatch(old),
                InteractionDispatch::Unsent,
                "a superseded request that was never sent is not queued"
            );
            assert_eq!(cx.interaction_dispatch(new), InteractionDispatch::Queued);
            // The old action leaves the host unsent after the new one started:
            // its custody must not touch the new request.
            drop(old_action);
            assert_eq!(cx.interaction_dispatch(old), InteractionDispatch::Unsent);
            assert_eq!(cx.interaction_dispatch(new), InteractionDispatch::Queued);
            // The host takes the new request and settles it in the same pass.
            let mut taken = cx.ledger.as_mut().unwrap().outbox.pop().unwrap();
            assert!(taken.live());
            taken.release_custody();
            let authority = taken.authority();
            drop(taken);
            assert_eq!(cx.interaction_dispatch(new), InteractionDispatch::Queued);
            let receipt = InteractionReceipt {
                request_id: new,
                evidence: cx.evidence,
                accepted: true,
                chat_since: 0,
            };
            cx.ledger
                .as_mut()
                .unwrap()
                .complete_interaction(&authority, receipt);
            assert_eq!(
                cx.interaction_dispatch(new),
                InteractionDispatch::Settled(&receipt),
                "the receipt settles the request first"
            );
            assert_eq!(cx.interaction_dispatch(old), InteractionDispatch::Unsent);
        });
    }

    #[test]
    fn script_services_borrow_real_native_actions_and_recheck_owner_authority() {
        use crate::native::services::{ScriptServices, ServiceAuthority, ServiceError};
        use api::identity::{ExecutionId, Fence, FenceError};
        use api::lifecycle::{HoldReason, SessionEventOut, SessionGate};
        use api::selected::RunKey;

        let mut ledger = Some(Box::new(ledger::Ledger::default()));
        with_frame(&mut ledger, Duration::from_secs(3), |cx| {
            let owner = install_owner(cx);
            let execution = ExecutionId::first(cx.run());
            let session = execution.run().session_key().unwrap();
            let mut fence = Fence::new();
            fence.arm(execution);
            let mut gate = SessionGate::new(session.slot());
            gate.begin(session).unwrap();
            gate.observe(session, &SessionEventOut::SceneReady).unwrap();
            let mut actions = NativeActions { _private: () };
            let id = api::identity::RequestId::from_raw(api::identity::RequestId::HOST_FIRST + 500)
                .expect("host request id");
            let point = crate::native::PromptPoint::from_contract_row(
                crate::native::abi::HOOK_CHARACTERIZATIONS
                    .iter()
                    .find(|row| row.member() == "sustain")
                    .expect("awaited sustain contract row"),
            )
            .expect("awaited hook point");
            {
                let mut services = ScriptServices::new(
                    ServiceAuthority::new(execution, &fence, &gate),
                    &mut actions,
                    cx,
                    None,
                )
                .unwrap();
                assert_eq!(services.active_time().as_duration(), Duration::from_secs(3));
                services
                    .with_actions(|_, cx| cx.emit(InteractReq::CloseModal))
                    .unwrap()
                    .unwrap();
                assert!(services.state_identity().is_none());
                assert!(matches!(
                    services.with_state(|_, _, _| panic!("no state port was injected")),
                    Err(ServiceError::StateUnavailable)
                ));
                assert!(matches!(
                    services.prompt(id, point, &[1]),
                    Err(ServiceError::PromptPortUnavailable)
                ));
            }
            // The same real owner frame binds bounded native prompt custody.
            let mut seat = crate::native::seat::GameplaySeat::<u8>::new(cx.run());
            assert_eq!(
                seat.start(
                    api::script_contract::RequestStamp::new(id, None, None, execution).unwrap(),
                    crate::native::request::NativeKind::Hunt,
                    false,
                )
                .unwrap(),
                crate::native::seat::Admission::Admitted
            );
            {
                let mut services = ScriptServices::new(
                    ServiceAuthority::new(execution, &fence, &gate),
                    &mut actions,
                    cx,
                    None,
                )
                .unwrap()
                .with_prompt_port(&mut seat)
                .unwrap();
                assert!(matches!(
                    services.prompt(
                        id,
                        point,
                        &[0; api::script_contract::PROMPT_ARGS_MAX_BYTES + 1],
                    ),
                    Err(ServiceError::Seat(
                        crate::native::seat::SeatError::PromptTooLarge
                    ))
                ));
                assert_eq!(
                    services
                        .prompt(id, point, &[1; api::script_contract::PROMPT_ARGS_MAX_BYTES])
                        .unwrap(),
                    api::script_contract::DecisionSequence::FIRST
                );
            }
            assert_eq!(
                seat.prompt_payload(id).unwrap().args,
                [1; api::script_contract::PROMPT_ARGS_MAX_BYTES]
            );
            assert_eq!(
                seat.decide_with_answer(
                    id,
                    api::script_contract::DecisionSequence::FIRST,
                    vec![9],
                ),
                crate::native::seat::SeatDecisionStatus::Consumed
            );
            let mut services = ScriptServices::new(
                ServiceAuthority::new(execution, &fence, &gate),
                &mut actions,
                cx,
                None,
            )
            .unwrap()
            .with_prompt_port(&mut seat)
            .unwrap();
            assert_eq!(services.take_decision(id).unwrap().unwrap().answer, [9]);
            let queued = cx.ledger.as_mut().unwrap().outbox.pop().unwrap();
            assert!(
                queued.live(),
                "the real native queue owns the admitted action"
            );
            owner.revoke();
            assert!(
                !queued.live(),
                "existing host authority still fences dispatch"
            );

            let other_session = session.next().unwrap();
            let mut other_gate = SessionGate::new(session.slot());
            other_gate.begin(other_session).unwrap();
            other_gate
                .observe(other_session, &SessionEventOut::SceneReady)
                .unwrap();
            {
                let mut services = ScriptServices::new(
                    ServiceAuthority::new(execution, &fence, &other_gate),
                    &mut actions,
                    cx,
                    None,
                )
                .unwrap();
                assert!(
                    matches!(
                        services.with_actions(|_, _| panic!("foreign session admitted an action")),
                        Err(ServiceError::SessionHeld)
                    ),
                    "a ready gate on another session is not this execution's session"
                );
            }

            let other_run = ExecutionId::first(RunKey {
                slot: 1,
                run: 99,
                session: 1,
            });
            assert!(
                matches!(
                    ScriptServices::new(
                        ServiceAuthority::new(other_run, &fence, &gate),
                        &mut actions,
                        cx,
                        None,
                    ),
                    Err(ServiceError::ContextMismatch)
                ),
                "an ExecutionId for another run is a context mismatch"
            );
            let mut foreign_seat = crate::native::seat::GameplaySeat::<u8>::new(other_run.run());
            let services = ScriptServices::new(
                ServiceAuthority::new(execution, &fence, &gate),
                &mut actions,
                cx,
                None,
            )
            .unwrap();
            assert!(
                matches!(
                    services.with_prompt_port(&mut foreign_seat),
                    Err(ServiceError::ContextMismatch)
                ),
                "a seat from another run cannot be bound to this authority"
            );

            let mut stale_fence = Fence::new();
            stale_fence.arm(execution.next_generation().unwrap());
            {
                let mut services = ScriptServices::new(
                    ServiceAuthority::new(execution, &stale_fence, &gate),
                    &mut actions,
                    cx,
                    None,
                )
                .unwrap();
                assert!(
                    matches!(
                        services.with_actions(|_, _| panic!("stale fence admitted an action")),
                        Err(ServiceError::Fence(FenceError::Stale { .. }))
                    ),
                    "a fence armed for a newer execution is stale, not revoked"
                );
            }

            gate.observe(
                session,
                &SessionEventOut::Held {
                    why: HoldReason::Guardian,
                },
            )
            .unwrap();
            let mut services = ScriptServices::new(
                ServiceAuthority::new(execution, &fence, &gate),
                &mut actions,
                cx,
                None,
            )
            .unwrap();
            assert!(matches!(
                services.with_actions(|_, _| panic!("held frame admitted an action")),
                Err(ServiceError::SessionHeld)
            ));
            fence.revoke();
            let mut services = ScriptServices::new(
                ServiceAuthority::new(execution, &fence, &gate),
                &mut actions,
                cx,
                None,
            )
            .unwrap();
            assert!(matches!(
                services.with_actions(|_, _| panic!("stale frame admitted an action")),
                Err(ServiceError::Fence(_))
            ));
        });
    }

    #[test]
    fn shared_action_revoker_fences_already_queued_input_before_slot_cleanup() {
        struct Pending;
        impl NativeMachine for Pending {
            type Args = ();
            type Output = ();
            fn begin(_: (), cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
                cx.emit(InteractReq::CloseModal)?;
                Ok(Self)
            }
            fn poll(&mut self, _: &mut ActionContext<'_>) -> Poll<Result<(), ActionError>> {
                Poll::Pending
            }
            fn cancel(&mut self) {}
        }
        let mut ledger = None;
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            let mut actions = NativeActions { _private: () };
            let handle = actions.begin::<Pending>((), cx).unwrap();
            let queued = cx.ledger.as_mut().unwrap().outbox.pop().unwrap();
            assert!(queued.live());
            let revoker = handle.revoker();
            assert_eq!(revoker.run(), cx.run());
            std::thread::spawn(move || revoker.revoke()).join().unwrap();
            assert!(
                !queued.live(),
                "the pair cancellation fences the drain without a slot lock"
            );
            assert!(matches!(
                actions.poll(&handle, cx),
                Poll::Ready(Err(ActionError::Cancelled))
            ));
        });
    }
    #[test]
    fn native_machine_default_interrupt_is_noop() {
        struct Passive(u8);
        impl NativeMachine for Passive {
            type Args = ();
            type Output = ();

            fn begin(_: (), _: &mut ActionContext<'_>) -> Result<Self, ActionError> {
                Ok(Self(7))
            }

            fn poll(&mut self, _: &mut ActionContext<'_>) -> Poll<Result<(), ActionError>> {
                Poll::Pending
            }
        }

        let mut actions = NativeActions { _private: () };
        let mut ledger = None;
        let handle = with_frame(&mut ledger, Duration::ZERO, |cx| {
            actions.begin::<Passive>((), cx).unwrap()
        });
        assert_eq!(handle.inspect(|machine| machine.0), Some(7));
        with_frame(&mut ledger, Duration::ZERO, |_| actions.interrupt(&handle));
        assert_eq!(
            handle.inspect(|machine| machine.0),
            Some(7),
            "the trait's default interrupt must preserve machines that do not opt in"
        );
    }

    #[test]
    fn action_handle_inspect_stays_shared() {
        struct Passive;
        impl NativeMachine for Passive {
            type Args = ();
            type Output = ();

            fn begin(_: (), _: &mut ActionContext<'_>) -> Result<Self, ActionError> {
                Ok(Self)
            }

            fn poll(&mut self, _: &mut ActionContext<'_>) -> Poll<Result<(), ActionError>> {
                Poll::Pending
            }
        }
        fn inspect_shared(handle: &ActionHandle<Passive>) -> Option<()> {
            handle.inspect(|_| ())
        }

        let mut ledger = None;
        let handle = with_frame(&mut ledger, Duration::ZERO, |cx| {
            NativeActions { _private: () }
                .begin::<Passive>((), cx)
                .unwrap()
        });
        assert_eq!(inspect_shared(&handle), Some(()));
    }

    #[test]
    fn handle_drop_contains_panicking_cancellation_payload_destructor() {
        struct Payload;
        impl Drop for Payload {
            fn drop(&mut self) {
                panic!("panic payload destructor");
            }
        }
        struct Machine;
        impl NativeMachine for Machine {
            type Args = ();
            type Output = ();
            fn begin(_: (), _: &mut ActionContext<'_>) -> Result<Self, ActionError> {
                Ok(Self)
            }
            fn poll(&mut self, _: &mut ActionContext<'_>) -> Poll<Result<(), ActionError>> {
                Poll::Pending
            }
            fn cancel(&mut self) {
                std::panic::panic_any(Payload);
            }
        }
        let owner = Owner::new(
            RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            NonZeroU64::new(1).unwrap(),
        );
        let handle = ActionHandle {
            owner: Arc::clone(&owner),
            machine: RefCell::new(Some(Machine)),
        };
        assert!(catch_unwind(AssertUnwindSafe(|| drop(handle))).is_ok());
        assert!(!owner.live());
    }

    /// An eligible frame on tick 1 over the slot's persistent ledger.
    fn with_frame<R>(
        ledger: &mut Option<Box<ledger::Ledger>>,
        active_now: Duration,
        f: impl FnOnce(&mut ActionContext<'_>) -> R,
    ) -> R {
        with_snapshot_frame(ledger, active_now, None, f)
    }

    fn with_snapshot_frame<R>(
        ledger: &mut Option<Box<ledger::Ledger>>,
        active_now: Duration,
        snapshot: Option<&api::snapshot::GameSnapshot>,
        f: impl FnOnce(&mut ActionContext<'_>) -> R,
    ) -> R {
        let evidence = EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 1,
        };
        let mut retained = RetainedMemory::default();
        let mut budget = ledger::TickBudget::default();
        budget.observe(1);
        let mut cx = ActionContext {
            stage_cross: None,
            walk_prereq: Default::default(),
            condition: None,
            poison_unknown_pulse: u8::MAX,
            evidence,
            cure_poison: crate::survival::CurePoison::WhenNeeded,
            revision: api::selected::ClientRevision::R289,
            snapshot: SnapshotView::new(snapshot, evidence),
            retained: &mut retained,
            action_id: 0,
            observed_walk_outcome_seq: 0,
            observed_walk_cancel_reason: crate::native::WalkCancelReason::None,
            shop_keepers: None,
            wilderness: None,
            active_now,
            wall_now: Instant::now(),
            ledger,
            pending_note_progress: None,
            budget: &mut budget,
            eligible: true,
        };
        f(&mut cx)
    }

    #[test]
    fn kept_resume_preserves_cursor_revokes_old_authority_and_spends_transition() {
        use crate::native::request::NativeKind;

        struct Cursor(u8);
        impl NativeMachine for Cursor {
            type Args = ();
            type Output = ();
            fn begin(_: (), cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
                cx.emit_disposal(drop_request(0))?;
                Ok(Self(0))
            }
            fn poll(&mut self, _: &mut ActionContext<'_>) -> Poll<Result<(), ActionError>> {
                self.0 += 1;
                Poll::Pending
            }
        }

        let mut actions = NativeActions { _private: () };
        let mut ledger = None;
        let (mut handle, old) = with_frame(&mut ledger, Duration::ZERO, |cx| {
            let handle = actions.begin::<Cursor>((), cx).unwrap();
            assert!(actions.poll(&handle, cx).is_pending());
            let old = cx.ledger.as_ref().unwrap().outbox[0].authority();
            (handle, old)
        });
        assert_eq!(handle.inspect(|cursor| cursor.0), Some(1));
        ledger.as_mut().unwrap().revoke();
        assert!(!old.live(), "old queued effects lose dispatch authority");

        with_frame(&mut ledger, Duration::ZERO, |cx| {
            cx.evidence.run.session = 2;
            assert_eq!(
                actions.resume_kept(NativeKind::QuestProgress, &mut handle, cx),
                Err(ActionError::Stale),
                "the closed Carry row cannot use Resume"
            );
            for _ in 0..32 {
                assert!(cx.budget.transition());
            }
            assert_eq!(
                actions.resume_kept(NativeKind::BankPlan, &mut handle, cx),
                Err(ActionError::BudgetExhausted)
            );
            assert!(!handle.owner.live());
            cx.budget.observe(2);
            cx.evidence.tick = 2;
            actions
                .resume_kept(NativeKind::BankPlan, &mut handle, cx)
                .unwrap();
            assert_eq!(handle.owner.run.session, 2);
            assert!(handle.owner.live());
            assert!(!old.live());
            assert!(
                cx.ledger.as_ref().unwrap().outbox.is_empty(),
                "no verb is replayed"
            );
            let mut remaining = 0;
            while cx.budget.transition() {
                remaining += 1;
            }
            assert_eq!(remaining, 31, "owner renewal spends one transition");
        });
        assert_eq!(handle.inspect(|cursor| cursor.0), Some(1));
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            cx.evidence.run.session = 2;
            assert!(actions.poll(&handle, cx).is_pending());
            assert!(cx.ledger.as_ref().unwrap().outbox.is_empty());
        });
        assert_eq!(handle.inspect(|cursor| cursor.0), Some(2));
    }

    fn drop_request(slot: i32) -> InteractReq {
        InteractReq::Held {
            name: "Logs".into(),
            action: "Drop".into(),
            slot: Some(slot),
            target_item_id: None,
        }
    }
    fn install_owner(cx: &mut ActionContext<'_>) -> Arc<Owner> {
        let run = cx.run();
        let owner = {
            let ledger = cx.ledger.as_mut().expect("test ledger is prepared");
            let owner = Owner::new(run, ledger.next_id().unwrap());
            ledger.owner = Some(Arc::clone(&owner));
            owner
        };
        cx.action_id = owner.id.get();
        owner
    }

    fn one_row(request: InteractReq) -> [Option<InteractReq>; BATCH_ROWS] {
        [Some(request), None, None, None, None]
    }

    fn eat_request(name: &str) -> InteractReq {
        InteractReq::Held {
            name: name.into(),
            action: "Eat".into(),
            slot: None,
            target_item_id: None,
        }
    }

    fn drink_request() -> InteractReq {
        InteractReq::Held {
            name: "Prayer potion".into(),
            action: "Drink".into(),
            slot: None,
            target_item_id: None,
        }
    }

    fn wear_request() -> InteractReq {
        InteractReq::Wear {
            name: "Bronze sword".into(),
        }
    }

    fn retaliate_request() -> InteractReq {
        InteractReq::SetRetaliate { on: true }
    }

    fn npc_attack() -> InteractReq {
        InteractReq::Npc {
            name: "Goblin".into(),
            action: "Attack".into(),
            index: Some(4),
        }
    }

    fn protect_component() -> i32 {
        api::game_data::for_revision(api::selected::ClientRevision::R289)
            .unwrap()
            .prayers()
            .iter()
            .find(|prayer| prayer.name == "Protect from Melee")
            .unwrap()
            .button_com
    }

    fn assert_no_batch_change(
        cx: &ActionContext<'_>,
        owner: &Owner,
        next_id: u64,
        free_slots: usize,
        outbox_len: usize,
    ) {
        let ledger = cx.ledger.as_ref().unwrap();
        assert_eq!(ledger.id_range(1).unwrap().get(), next_id);
        assert_eq!(owner.batch_free(), free_slots);
        assert_eq!(ledger.outbox.len(), outbox_len);
    }

    fn assert_shape_refusal(
        cx: &mut ActionContext<'_>,
        owner: &Owner,
        rows: [Option<InteractReq>; BATCH_ROWS],
    ) {
        let next_id = cx.ledger.as_ref().unwrap().id_range(1).unwrap().get();
        let free_slots = owner.batch_free();
        let outbox_len = cx.ledger.as_ref().unwrap().outbox.len();
        assert!(matches!(
            cx.emit_batch(rows),
            Err(ActionError::Unavailable(reason)) if reason.as_ref() == "batch shape"
        ));
        assert_no_batch_change(cx, owner, next_id, free_slots, outbox_len);
    }

    #[test]
    fn batch_accepts_each_native_combat_interaction_variant() {
        let requests = [
            InteractReq::SideTab { tab: 0 },
            InteractReq::IfButton { component_id: 900 },
            eat_request("Shrimp"),
            drink_request(),
            wear_request(),
            retaliate_request(),
            npc_attack(),
            InteractReq::Player {
                name: "alice".into(),
                action: "Attack".into(),
            },
            InteractReq::UseWidgetOn {
                component_id: 1234,
                kind: "npc".into(),
                target_name: Some("Goblin".into()),
                x: 2601,
                z: 3200,
                level: 0,
                index: Some(4),
                target_id: None,
            },
            InteractReq::Obj {
                x: 2601,
                z: 3200,
                level: 0,
                name: Some("Ground item".into()),
                action: "Take".into(),
                id: Some(1053),
            },
        ];
        for request in requests {
            let mut ledger = Some(Box::new(ledger::Ledger::default()));
            with_frame(&mut ledger, Duration::ZERO, |cx| {
                let owner = install_owner(cx);
                let first = cx.emit_batch(one_row(request)).unwrap();
                let action = &cx.ledger.as_ref().unwrap().outbox[0];
                assert_eq!(first, 2);
                assert_eq!(action.batch, first);
                assert_eq!(action.request_id.get(), first);
                assert!(action.live());
                assert!(action.authority().live());
                assert_eq!(owner.batch_free(), 4);
            });
        }
    }

    #[test]
    fn autocast_side_tab_is_one_serial_zero_wire_event_batch() {
        let revision = api::selected::ClientRevision::R289;
        assert_eq!(
            validate_batch(&one_row(InteractReq::SideTab { tab: 0 }), revision),
            Ok(1)
        );
        let mut ledger = Some(Box::new(ledger::Ledger::default()));
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            let owner = install_owner(cx);
            for rows in [
                [
                    Some(InteractReq::SideTab { tab: 0 }),
                    Some(InteractReq::IfButton { component_id: 353 }),
                    None,
                    None,
                    None,
                ],
                [
                    Some(InteractReq::IfButton { component_id: 353 }),
                    Some(InteractReq::SideTab { tab: 0 }),
                    None,
                    None,
                    None,
                ],
                one_row(InteractReq::SideTab { tab: 1 }),
            ] {
                assert_shape_refusal(cx, &owner, rows);
            }
            cx.emit_batch(one_row(InteractReq::SideTab { tab: 0 }))
                .unwrap();
            assert_eq!(cx.emit(npc_attack()), Err(ActionError::BudgetExhausted));
            assert_eq!(cx.ledger.as_ref().unwrap().outbox.len(), 1);
        });
    }

    #[test]
    fn batch_rejects_invalid_shapes_and_host_grammar_without_side_effects() {
        if !api::test_support::game_data_available() {
            return;
        }
        let mut ledger = Some(Box::new(ledger::Ledger::default()));
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            let owner = install_owner(cx);
            let protect = protect_component();
            let invalid = [
                [None, None, None, None, None],
                [
                    Some(InteractReq::IfButton { component_id: 900 }),
                    None,
                    Some(wear_request()),
                    None,
                    None,
                ],
                [Some(drop_request(1)), None, None, None, None],
                [
                    Some(InteractReq::Held {
                        name: "Bones".into(),
                        action: "Bury".into(),
                        slot: None,
                        target_item_id: None,
                    }),
                    None,
                    None,
                    None,
                    None,
                ],
                [Some(InteractReq::CloseModal), None, None, None, None],
                [Some(InteractReq::WalkNearestBank), None, None, None, None],
                [
                    Some(drink_request()),
                    Some(wear_request()),
                    None,
                    None,
                    None,
                ],
                [
                    Some(eat_request("Shrimp")),
                    Some(eat_request("Lobster")),
                    Some(wear_request()),
                    None,
                    None,
                ],
                [Some(npc_attack()), Some(wear_request()), None, None, None],
                [Some(drink_request()), Some(npc_attack()), None, None, None],
                [
                    Some(npc_attack()),
                    Some(InteractReq::Player {
                        name: "alice".into(),
                        action: "Attack".into(),
                    }),
                    None,
                    None,
                    None,
                ],
                [
                    Some(eat_request("Shrimp")),
                    Some(eat_request("Lobster")),
                    Some(eat_request("Swordfish")),
                    None,
                    None,
                ],
                [
                    Some(InteractReq::IfButton { component_id: 900 }),
                    Some(wear_request()),
                    Some(retaliate_request()),
                    Some(eat_request("Shrimp")),
                    Some(npc_attack()),
                ],
                [
                    Some(InteractReq::IfButton {
                        component_id: protect,
                    }),
                    Some(InteractReq::IfButton {
                        component_id: protect,
                    }),
                    Some(InteractReq::IfButton { component_id: 900 }),
                    Some(retaliate_request()),
                    Some(wear_request()),
                ],
                [
                    Some(InteractReq::IfButton { component_id: 5609 }),
                    Some(InteractReq::IfButton { component_id: 5609 }),
                    None,
                    None,
                    None,
                ],
                [
                    Some(InteractReq::IfButton {
                        component_id: protect,
                    }),
                    Some(wear_request()),
                    Some(InteractReq::IfButton {
                        component_id: protect,
                    }),
                    None,
                    None,
                ],
                [
                    Some(InteractReq::IfButton {
                        component_id: protect,
                    }),
                    Some(InteractReq::IfButton {
                        component_id: protect,
                    }),
                    Some(InteractReq::IfButton { component_id: 5609 }),
                    Some(InteractReq::IfButton { component_id: 5609 }),
                    None,
                ],
            ];
            for rows in invalid {
                assert_shape_refusal(cx, &owner, rows);
            }
            assert_eq!(cx.emit_batch(one_row(wear_request())).unwrap(), 2);
        });
    }

    #[test]
    fn selected_protect_pair_is_adjacent_and_capped_at_four_wire_events() {
        if !api::test_support::game_data_available() {
            return;
        }
        let mut ledger = Some(Box::new(ledger::Ledger::default()));
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            install_owner(cx);
            let protect = protect_component();
            let first = cx
                .emit_batch([
                    Some(InteractReq::IfButton {
                        component_id: protect,
                    }),
                    Some(InteractReq::IfButton {
                        component_id: protect,
                    }),
                    Some(npc_attack()),
                    None,
                    None,
                ])
                .unwrap();
            let actions = &cx.ledger.as_ref().unwrap().outbox;
            assert_eq!(actions.len(), 3);
            assert_eq!(actions[0].batch, first);
            assert_eq!(actions[1].batch, first);
            assert_eq!(actions[2].batch, first);
        });
    }

    #[test]
    fn stale_held_and_cancelled_batch_admission_is_side_effect_free() {
        let mut ledger = Some(Box::new(ledger::Ledger::default()));
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            let owner = install_owner(cx);
            cx.action_id += 1;
            assert_eq!(
                cx.emit_batch(one_row(wear_request())),
                Err(ActionError::Stale)
            );
            assert_no_batch_change(cx, &owner, 2, 5, 0);

            cx.action_id = owner.id.get();
            cx.eligible = false;
            assert_eq!(
                cx.emit_batch(one_row(wear_request())),
                Err(ActionError::Held)
            );
            assert_no_batch_change(cx, &owner, 2, 5, 0);

            cx.eligible = true;
            assert_eq!(cx.emit_batch(one_row(wear_request())).unwrap(), 2);
            owner.revoke();
            assert_eq!(
                cx.emit_batch(one_row(wear_request())),
                Err(ActionError::Cancelled)
            );
            assert_no_batch_change(cx, &owner, 3, 0, 1);
        });
    }

    #[test]
    fn insufficient_slots_and_reservation_races_leave_no_partial_batch_state() {
        let mut ledger = Some(Box::new(ledger::Ledger::default()));
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            let owner = install_owner(cx);
            let occupied = NonZeroU64::new(100).unwrap();
            assert!(owner.acquire_batch(occupied, 4));
            assert_eq!(owner.batch_free(), 1);
            let rows = [Some(wear_request()), Some(npc_attack()), None, None, None];
            assert_eq!(
                cx.emit_batch(rows),
                Err(ActionError::BudgetExhausted),
                "two rows cannot fit in the single free reservation"
            );
            assert_no_batch_change(cx, &owner, 2, 1, 0);
            for offset in 0..4 {
                owner.cancel_interaction(NonZeroU64::new(occupied.get() + offset).unwrap());
            }
            assert_eq!(cx.emit_batch(one_row(wear_request())).unwrap(), 2);
        });

        let mut ledger = Some(Box::new(ledger::Ledger::default()));
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            let owner = install_owner(cx);
            let occupied = NonZeroU64::new(3).unwrap();
            assert!(owner.acquire_batch(occupied, 1));
            let rows = [
                Some(InteractReq::IfButton { component_id: 900 }),
                Some(wear_request()),
                Some(retaliate_request()),
                Some(npc_attack()),
                None,
            ];
            assert_eq!(
                cx.emit_batch(rows),
                Err(ActionError::BudgetExhausted),
                "the second id conflict must roll back the first reservation"
            );
            assert_no_batch_change(cx, &owner, 2, 4, 0);
            assert!(!owner.batch_live(NonZeroU64::new(2).unwrap()));
            assert!(owner.batch_live(occupied));
            owner.cancel_interaction(occupied);
            assert_eq!(cx.emit_batch(one_row(wear_request())).unwrap(), 2);
        });
    }

    #[test]
    fn batch_excludes_other_native_actions_and_resets_on_the_next_tick() {
        let mut ledger = Some(Box::new(ledger::Ledger::default()));
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            let owner = install_owner(cx);
            let first = cx.emit_batch(one_row(wear_request())).unwrap();
            assert_eq!(
                cx.emit_batch(one_row(wear_request())),
                Err(ActionError::BudgetExhausted)
            );
            assert_eq!(
                cx.emit(InteractReq::CloseModal),
                Err(ActionError::BudgetExhausted)
            );
            assert_eq!(
                cx.emit_disposal(drop_request(1)),
                Err(ActionError::BudgetExhausted)
            );
            let walk = WalkRequest {
                target: api::WorldTile {
                    x: 9,
                    z: 9,
                    level: 0,
                },
                candidate_goals: None,
                radius: 0,
                arrival: nav_planner::contract::ArrivalKind::Reach,
                options: WalkOptions::default(),
                required_after: cx.evidence(),
                evidence: None,
                loc_id: None,
                cross: Box::new([]),
                avoid: Box::new([]),
                protect: false,
                food_guard: false,
                cure_guard: false,
                allow: Default::default(),
            };
            assert_eq!(cx.walk(walk), Err(ActionError::BudgetExhausted));
            assert_no_batch_change(cx, &owner, first + 1, 4, 1);

            let actions = cx.ledger.as_mut().unwrap().outbox.split_off(0);
            for action in actions {
                let receipt = InteractionReceipt {
                    request_id: action.request_id.get(),
                    evidence: cx.evidence(),
                    accepted: true,
                    chat_since: 0,
                };
                cx.ledger
                    .as_mut()
                    .unwrap()
                    .complete_interaction(&action.authority(), receipt);
            }
            assert_eq!(owner.batch_free(), 5);
            cx.budget.observe(2);
            assert_eq!(cx.emit_batch(one_row(wear_request())).unwrap(), first + 1);
        });

        let mut ledger = Some(Box::new(ledger::Ledger::default()));
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            let owner = install_owner(cx);
            let ordinary = cx.emit(InteractReq::CloseModal).unwrap();
            assert_eq!(
                cx.emit_batch(one_row(wear_request())),
                Err(ActionError::BudgetExhausted)
            );
            assert_no_batch_change(cx, &owner, ordinary + 1, 5, 1);
            assert_eq!(cx.ledger.as_ref().unwrap().outbox[0].batch, 0);
        });
    }

    #[test]
    fn batch_and_disposal_share_reservations_and_event_allowance() {
        let mut ledger = Some(Box::new(ledger::Ledger::default()));
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            let owner = install_owner(cx);
            let first = cx
                .emit_batch([
                    Some(InteractReq::IfButton { component_id: 900 }),
                    Some(wear_request()),
                    Some(retaliate_request()),
                    Some(eat_request("Shrimp")),
                    Some(InteractReq::IfButton { component_id: 901 }),
                ])
                .unwrap();
            let batch_actions = cx.ledger.as_mut().unwrap().outbox.split_off(0);
            assert_eq!(batch_actions.len(), 5);
            assert!(batch_actions
                .iter()
                .all(|action| action.batch == first && action.live()));
            assert_eq!(owner.batch_free(), 0);

            cx.budget.observe(2);
            assert_eq!(
                cx.emit_disposal(drop_request(0)),
                Err(ActionError::BudgetExhausted),
                "a full batch leaves no disposal reservation"
            );
            cx.cancel_request(first);
            assert!(!batch_actions[0].live());
            assert_eq!(cx.emit_disposal(drop_request(0)).unwrap(), first + 5);
            assert_eq!(owner.batch_free(), 0);
            assert_eq!(
                cx.emit_disposal(drop_request(1)),
                Err(ActionError::BudgetExhausted)
            );
            assert_eq!(
                cx.ledger.as_ref().unwrap().id_range(1).unwrap().get(),
                first + 6
            );

            cx.cancel_request(first + 1);
            assert!(!batch_actions[1].live());
            assert_eq!(owner.batch_free(), 1);
            assert_eq!(cx.emit_disposal(drop_request(1)).unwrap(), first + 6);
            for offset in 2..5 {
                cx.cancel_request(first + offset);
            }
            assert!(batch_actions.iter().all(|action| !action.live()));
            assert_eq!(owner.batch_free(), 3);
            for slot in 2..5 {
                cx.emit_disposal(drop_request(slot)).unwrap();
            }
            assert_eq!(owner.batch_free(), 0);
            assert_eq!(
                cx.emit_disposal(drop_request(5)),
                Err(ActionError::BudgetExhausted)
            );
            assert_eq!(
                cx.ledger.as_ref().unwrap().id_range(1).unwrap().get(),
                first + 10
            );
            assert_eq!(cx.ledger.as_ref().unwrap().outbox.len(), 5);
        });
    }

    #[test]
    fn batch_ids_order_authorities_and_receipts_are_independent() {
        let mut ledger = Some(Box::new(ledger::Ledger::default()));
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            let owner = install_owner(cx);
            let rows = [
                Some(InteractReq::IfButton { component_id: 900 }),
                Some(eat_request("Shrimp")),
                Some(wear_request()),
                Some(retaliate_request()),
                Some(InteractReq::IfButton { component_id: 901 }),
            ];
            let first = cx.emit_batch(rows).unwrap();
            let actions = cx.ledger.as_mut().unwrap().outbox.split_off(0);
            assert_eq!(actions.len(), 5);
            assert!(actions
                .iter()
                .all(|action| action.batch == first && action.live() && action.authority().live()));
            assert!(actions
                .windows(2)
                .all(|pair| pair[1].request_id.get() == pair[0].request_id.get() + 1));
            assert_eq!(actions[0].request_id.get(), first);
            assert!(matches!(
                &actions[0].effect,
                HostEffect::Interaction(InteractReq::IfButton { component_id: 900 })
            ));
            assert!(matches!(
                &actions[1].effect,
                HostEffect::Interaction(InteractReq::Held { action, .. }) if action == "Eat"
            ));
            assert!(matches!(
                &actions[2].effect,
                HostEffect::Interaction(InteractReq::Wear { .. })
            ));
            assert!(matches!(
                &actions[3].effect,
                HostEffect::Interaction(InteractReq::SetRetaliate { on: true })
            ));
            assert!(matches!(
                &actions[4].effect,
                HostEffect::Interaction(InteractReq::IfButton { component_id: 901 })
            ));

            for (index, action) in actions.iter().enumerate() {
                let authority = action.authority();
                let receipt = InteractionReceipt {
                    request_id: action.request_id.get(),
                    evidence: cx.evidence(),
                    accepted: index != 2,
                    chat_since: 0,
                };
                cx.ledger
                    .as_mut()
                    .unwrap()
                    .complete_interaction(&authority, receipt);
                assert!(!authority.live());
                assert!(
                    authority.owner_live(),
                    "settling one request must not retire its action owner"
                );
                assert!(!action.live());
                assert_eq!(owner.batch_free(), index + 1);
                assert_eq!(cx.interaction_receipt(receipt.request_id), Some(&receipt));
                assert!(actions.iter().skip(index + 1).all(HostAction::live));
            }
            assert_eq!(owner.batch_free(), 5);

            cx.budget.observe(2);
            let second = cx
                .emit_batch([
                    Some(InteractReq::IfButton { component_id: 902 }),
                    Some(wear_request()),
                    Some(retaliate_request()),
                    None,
                    None,
                ])
                .unwrap();
            assert_eq!(second, first + 5);
            let second_actions = cx.ledger.as_mut().unwrap().outbox.split_off(0);
            let authorities =
                std::array::from_fn::<_, 3, _>(|index| second_actions[index].authority());
            assert!(second_actions
                .iter()
                .all(|action| action.batch == second && action.live()));
            owner.revoke();
            assert!(second_actions.iter().all(|action| !action.live()));
            assert!(authorities.iter().all(|authority| !authority.live()));
            assert!(authorities.iter().all(|authority| !authority.owner_live()));
        });
    }

    #[test]
    fn batch_moves_owned_strings_without_allocating_during_admission() {
        let mut ledger = Some(Box::new(ledger::Ledger::default()));
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            install_owner(cx);
            let food_name = String::from("Shrimp");
            let food_action = String::from("Eat");
            let npc_name = String::from("Goblin");
            let npc_action = String::from("Attack");
            let food_name_ptr = food_name.as_ptr();
            let food_action_ptr = food_action.as_ptr();
            let npc_name_ptr = npc_name.as_ptr();
            let npc_action_ptr = npc_action.as_ptr();
            let rows = [
                Some(InteractReq::Held {
                    name: food_name,
                    action: food_action,
                    slot: None,
                    target_item_id: None,
                }),
                Some(InteractReq::Npc {
                    name: npc_name,
                    action: npc_action,
                    index: Some(4),
                }),
                None,
                None,
                None,
            ];
            let mut admitted = None;
            let allocations = allocation_counter::measure(|| {
                admitted = Some(cx.emit_batch(rows));
            });
            assert_eq!(allocations.count_total, 0);
            let first = admitted.unwrap().unwrap();
            let outbox = &cx.ledger.as_ref().unwrap().outbox;
            assert_eq!(outbox.len(), 2);
            match &outbox[0].effect {
                HostEffect::Interaction(InteractReq::Held { name, action, .. }) => {
                    assert_eq!(name.as_ptr(), food_name_ptr);
                    assert_eq!(action.as_ptr(), food_action_ptr);
                }
                _ => panic!("expected moved Eat row"),
            }
            match &outbox[1].effect {
                HostEffect::Interaction(InteractReq::Npc { name, action, .. }) => {
                    assert_eq!(name.as_ptr(), npc_name_ptr);
                    assert_eq!(action.as_ptr(), npc_action_ptr);
                }
                _ => panic!("expected moved target row"),
            }
            assert!(outbox.iter().all(|action| action.batch == first));
        });
    }

    #[test]
    fn disposal_batch_keeps_five_authorities_and_correlates_receipts() {
        eprintln!(
            "G1 native memory: Owner={} Ledger={} HostAction={} InteractionReceipt={}",
            std::mem::size_of::<Owner>(),
            std::mem::size_of::<ledger::Ledger>(),
            std::mem::size_of::<HostAction>(),
            std::mem::size_of::<InteractionReceipt>(),
        );
        let mut ledger = Some(Box::new(ledger::Ledger::default()));
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            let owner = Owner::new(cx.run(), cx.ledger.as_mut().unwrap().next_id().unwrap());
            cx.ledger.as_mut().unwrap().owner = Some(Arc::clone(&owner));
            cx.action_id = owner.id.get();
            let requests: Vec<_> = (0..5)
                .map(|slot| cx.emit_disposal(drop_request(slot)).unwrap())
                .collect();
            assert_eq!(
                cx.emit_disposal(drop_request(5)),
                Err(ActionError::BudgetExhausted)
            );
            assert_eq!(cx.ledger.as_ref().unwrap().outbox.len(), 5);
            assert!(cx
                .ledger
                .as_ref()
                .unwrap()
                .outbox
                .iter()
                .all(|action| action.batch == 0));
            eprintln!(
                "G1 disposal outbox retained bytes={}",
                cx.ledger.as_ref().unwrap().outbox.capacity() * std::mem::size_of::<HostAction>(),
            );
            assert!(cx
                .ledger
                .as_ref()
                .unwrap()
                .outbox
                .iter()
                .all(HostAction::live));
            assert!(requests.windows(2).all(|pair| pair[0] != pair[1]));
            cx.budget.observe(2);
            assert_eq!(
                cx.emit_disposal(drop_request(5)),
                Err(ActionError::BudgetExhausted)
            );
            assert_eq!(cx.ledger.as_ref().unwrap().outbox.len(), 5);
            cx.cancel_request(requests[2]);
            assert_eq!(cx.ledger.as_ref().unwrap().outbox.len(), 4);
            let actions = cx.ledger.as_mut().unwrap().outbox.split_off(0);
            for action in actions.into_iter().rev() {
                let receipt = InteractionReceipt {
                    request_id: action.request_id.get(),
                    evidence: cx.evidence(),
                    accepted: action.request_id.get() != requests[1],
                    chat_since: 0,
                };
                cx.ledger
                    .as_mut()
                    .unwrap()
                    .complete_interaction(&action.authority(), receipt);
                assert!(!action.live(), "dispatch must release the disposal slot");
                assert_eq!(cx.interaction_receipt(receipt.request_id), Some(&receipt));
            }
            assert!(cx.interaction_receipt(requests[2]).is_none());
            for request in requests.iter().filter(|&&request| request != requests[2]) {
                assert!(cx.interaction_receipt(*request).is_some());
            }
            cx.budget.observe(3);
            for slot in 0..5 {
                cx.emit_disposal(drop_request(slot)).unwrap();
            }
            cx.ledger.as_mut().unwrap().revoke();
            assert!(cx.ledger.as_ref().unwrap().outbox.is_empty());
            assert!(!owner.live());
        });
    }

    #[test]
    fn full_disposal_authority_refusal_preserves_budget_and_ids() {
        let mut ledger = Some(Box::new(ledger::Ledger::default()));
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            let owner = Owner::new(cx.run(), cx.ledger.as_mut().unwrap().next_id().unwrap());
            cx.ledger.as_mut().unwrap().owner = Some(Arc::clone(&owner));
            cx.action_id = owner.id.get();

            let requests: Vec<_> = (0..4)
                .map(|slot| cx.emit_disposal(drop_request(slot)).unwrap())
                .collect();
            let reserved = cx.ledger.as_mut().unwrap().next_id().unwrap();
            assert!(owner.acquire_batch(reserved, 1));

            assert_eq!(
                cx.emit_disposal(drop_request(4)),
                Err(ActionError::BudgetExhausted),
                "a full authority set must not consume its remaining event allowance"
            );
            owner.cancel_interaction(reserved);

            let fifth = cx
                .emit_disposal(drop_request(4))
                .expect("releasing an authority leaves the allowance available");
            assert_eq!(
                fifth,
                reserved.get() + 1,
                "a refused full-set reservation must not consume an ID"
            );

            cx.cancel_request(requests[0]);
            assert_eq!(
                cx.emit_disposal(drop_request(5)),
                Err(ActionError::BudgetExhausted),
                "an exhausted event allowance must not reserve a disposal authority"
            );

            cx.budget.observe(2);
            let next = cx
                .emit_disposal(drop_request(5))
                .expect("the released authority remains available on the next tick");
            assert_eq!(next, fifth + 1);
        });
    }

    #[test]
    fn modal_close_shares_disposal_allowance_and_repoll_does_not_refill_it() {
        let mut ledger = Some(Box::new(ledger::Ledger::default()));
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            let owner = Owner::new(cx.run(), cx.ledger.as_mut().unwrap().next_id().unwrap());
            cx.ledger.as_mut().unwrap().owner = Some(owner.clone());
            cx.action_id = owner.id.get();
            assert!(matches!(
                cx.emit_disposal(InteractReq::CloseModal),
                Err(ActionError::Unavailable(_))
            ));
            cx.emit(InteractReq::CloseModal).unwrap();
            for slot in 0..4 {
                cx.emit_disposal(drop_request(slot)).unwrap();
            }
            cx.budget.observe(1);
            assert_eq!(
                cx.emit_disposal(drop_request(4)),
                Err(ActionError::BudgetExhausted)
            );
            let actions = cx.ledger.as_mut().unwrap().outbox.split_off(0);
            assert!(actions.iter().all(HostAction::live));
            assert_eq!(actions.len(), 5);
            assert!(actions.iter().all(|action| action.batch == 0));
            for action in &actions {
                let receipt = InteractionReceipt {
                    request_id: action.request_id.get(),
                    evidence: cx.evidence(),
                    accepted: true,
                    chat_since: 0,
                };
                cx.ledger
                    .as_mut()
                    .unwrap()
                    .complete_interaction(&action.authority(), receipt);
            }
            cx.budget.observe(2);
            cx.emit_disposal(drop_request(4)).unwrap();
            let pending = cx.ledger.as_ref().unwrap().outbox[0].authority();
            owner.revoke();
            assert!(
                !pending.live(),
                "revocation before drain must fence the batch"
            );
        });
    }

    #[test]
    fn cancelling_native_disposal_handle_revokes_every_queued_drop_before_drain() {
        struct Batch;
        impl NativeMachine for Batch {
            type Args = ();
            type Output = ();
            fn begin(_: (), cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
                for slot in 0..5 {
                    cx.emit_disposal(drop_request(slot))?;
                }
                Ok(Self)
            }
            fn poll(&mut self, _: &mut ActionContext<'_>) -> Poll<Result<(), ActionError>> {
                Poll::Pending
            }
            fn cancel(&mut self) {}
        }
        let mut actions = NativeActions { _private: () };
        let mut ledger = None;
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            let handle = actions.begin::<Batch>((), cx).unwrap();
            let queued = &cx.ledger.as_ref().unwrap().outbox;
            assert_eq!(queued.len(), 5);
            assert!(queued.iter().all(HostAction::live));
            actions.cancel(handle);
            assert!(cx
                .ledger
                .as_ref()
                .unwrap()
                .outbox
                .iter()
                .all(|action| !action.live()));
            cx.ledger.as_mut().unwrap().revoke();
            assert!(cx.ledger.as_ref().unwrap().outbox.is_empty());
        });
    }

    #[test]
    fn emit_refuses_walk_family_and_host_control_without_queueing_or_charging() {
        let mut ledger = Some(Box::new(ledger::Ledger::default()));
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            let owner = Owner::new(cx.run(), cx.ledger.as_mut().unwrap().next_id().unwrap());
            cx.ledger.as_mut().unwrap().owner = Some(Arc::clone(&owner));
            cx.action_id = owner.id.get();
            for request in [
                InteractReq::Walk {
                    x: 1,
                    z: 1,
                    level: 0,
                    allow_teleports: false,
                    allow_wilderness: false,
                    allow_bank_fetch: false,
                    allow_prereq_dialogue: None,
                    allow_quest_start: None,
                    request_id: 0,
                    avoid: Vec::new(),
                    cross: Vec::new(),
                },
                InteractReq::WalkNearestBank,
                InteractReq::AbortWalk { request_id: 0 },
                InteractReq::LoopSettled,
            ] {
                assert!(matches!(cx.emit(request), Err(ActionError::Unavailable(_))));
            }
            assert!(cx.ledger.as_ref().unwrap().outbox.is_empty());
            // A refused request spends no event: the tick's one dispatch is
            // still available, and exhaustion leaves the outbox unchanged.
            cx.emit(InteractReq::CloseModal).unwrap();
            assert_eq!(
                cx.emit(InteractReq::CloseModal),
                Err(ActionError::BudgetExhausted)
            );
            assert_eq!(cx.ledger.as_ref().unwrap().outbox.len(), 1);
        });
    }

    #[test]
    fn a_walk_without_a_host_terminal_fails_at_its_active_deadline() {
        let mut actions = NativeActions { _private: () };
        let mut ledger = None;
        let (handle, authority) = with_frame(&mut ledger, Duration::ZERO, |cx| {
            let request = WalkRequest {
                target: api::WorldTile {
                    x: 9,
                    z: 9,
                    level: 0,
                },
                candidate_goals: None,
                loc_id: None,
                radius: 0,
                arrival: nav_planner::contract::ArrivalKind::Reach,
                options: WalkOptions::default(),
                required_after: cx.evidence(),
                evidence: None,
                cross: Vec::new().into_boxed_slice(),
                avoid: Box::new([]),
                protect: false,
                food_guard: false,
                cure_guard: false,
                allow: Default::default(),
            };
            let handle = actions.begin::<super::walk::Walk>(request, cx).unwrap();
            let authority = cx.ledger.as_ref().unwrap().outbox[0].authority();
            assert_eq!(cx.ledger.as_ref().unwrap().outbox[0].batch, 0);
            assert!(actions.poll(&handle, cx).is_pending());
            (handle, authority)
        });
        assert!(authority.live());
        // A day of eligible time: far past any walk deadline, and no host
        // receipt ever arrived.
        with_frame(&mut ledger, Duration::from_secs(24 * 60 * 60), |cx| {
            let ended = actions.poll(&handle, cx);
            assert!(
                matches!(
                    &ended,
                    Poll::Ready(Ok(WalkReceipt {
                        end: WalkEnd::Failed,
                        ..
                    }))
                ),
                "{ended:?}"
            );
        });
        assert!(!authority.live(), "the expired walk's follow is revoked");
    }
    fn walking_snapshot(tile: api::WorldTile, moving: bool) -> api::snapshot::GameSnapshot {
        use api::snapshot::{ActorView, GameSnapshot, LocalPlayerView, PlayerView};
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(LocalPlayerView {
            player: PlayerView {
                index: 0,
                network: tile,
                actor: ActorView {
                    name: Some("alice".into()),
                    actions: Vec::new(),
                    tile,
                    distance: 0,
                    animation: -1,
                    animation_frame: 0,
                    pose_animation: -1,
                    orientation: 0,
                    target_orientation: 0,
                    overhead_text: None,
                    spot_animation: -1,
                    spot_animation_stamp: 0,
                    health: 10,
                    total_health: 10,
                    face_entity: -1,
                    target: None,
                    moving,
                    running: false,
                    in_combat: false,
                },
                combat_level: 3,
                skill_level: 3,
                headicons: 0,
                weapon: None,
            },
            energy: 100,
            weight: 0,
            animation_update: None,
        });
        snapshot
    }

    #[test]
    fn route_ended_waits_for_final_movement_without_widening_radius() {
        let target = api::WorldTile {
            x: 3185,
            z: 3440,
            level: 0,
        };
        for radius in [0, 1] {
            for final_end in [WalkEnd::Arrived, WalkEnd::RouteEnded, WalkEnd::Failed] {
                let mut actions = NativeActions { _private: () };
                let mut ledger = None;
                let short = api::WorldTile {
                    x: target.x - i32::from(radius) - 1,
                    ..target
                };
                let moving = walking_snapshot(short, true);
                let (handle, authority) = with_snapshot_frame(
                    &mut ledger,
                    Duration::ZERO,
                    Some(&moving),
                    |cx| {
                        let request = WalkRequest {
                            target,
                            candidate_goals: None,
                            loc_id: None,
                            radius,
                            arrival: nav_planner::contract::ArrivalKind::Reach,
                            options: WalkOptions::default(),
                            required_after: cx.evidence(),
                            evidence: None,
                            cross: Vec::new().into_boxed_slice(),
                            avoid: Box::new([]),
                            protect: false,
                            food_guard: false,
                            cure_guard: false,
                            allow: Default::default(),
                        };
                        let handle = actions.begin::<super::walk::Walk>(request, cx).unwrap();
                        let authority = cx.ledger.as_ref().unwrap().outbox[0].authority();
                        cx.ledger.as_mut().unwrap().walk = Some(WalkReceipt {
                            goal: None,
                            request_id: authority.request_id().get(),
                            evidence: cx.evidence(),
                            end: WalkEnd::RouteEnded,
                            blocked: None,
                            detail: None,
                            refusal: None,
                            assessment: None,
                            escape: None,
                            prereq_allow: None,
                        });
                        assert!(actions.poll(&handle, cx).is_pending(),
                        "radius {radius}: a still-moving final segment is not a stationary route end");
                        (handle, authority)
                    },
                );
                assert!(
                    authority.live(),
                    "pending movement retains its action authority"
                );
                let (tile, still_moving, now) = match final_end {
                    WalkEnd::Arrived => (target, true, Duration::from_secs(1)),
                    WalkEnd::RouteEnded => (short, false, Duration::from_secs(1)),
                    WalkEnd::Failed => (short, true, super::walk::WALK_DEADLINE),
                    _ => unreachable!(),
                };
                let final_snapshot = walking_snapshot(tile, still_moving);
                with_snapshot_frame(&mut ledger, now, Some(&final_snapshot), |cx| {
                    let result = actions.poll(&handle, cx);
                    assert!(
                        matches!(&result, Poll::Ready(Ok(receipt)) if receipt.end == final_end),
                        "radius {radius}, expected {final_end:?}, got {result:?}"
                    );
                });
                assert!(
                    !authority.live(),
                    "settled or expired movement revokes its authority"
                );
            }
        }
    }
    fn assessment_walk_request(evidence: EvidenceStamp) -> WalkRequest {
        WalkRequest {
            target: api::WorldTile {
                x: 9,
                z: 9,
                level: 0,
            },
            candidate_goals: None,
            loc_id: None,
            radius: 0,
            arrival: nav_planner::contract::ArrivalKind::Reach,
            options: WalkOptions::default(),
            required_after: evidence,
            evidence: None,
            cross: Vec::new().into_boxed_slice(),
            avoid: Box::new([]),
            protect: false,
            food_guard: false,
            cure_guard: false,
            allow: WalkAllow::default(),
        }
    }

    fn advisory_failure(request_id: u64, evidence: EvidenceStamp) -> AssessReceipt {
        AssessReceipt {
            request_id,
            evidence,
            assessment: None,
            refusal: Some(WalkRefusal::Unknown(
                crate::combat::risk::UnknownWhy::MissingFacts,
            )),
            detail: Some(Arc::from("risk input unavailable")),
            route_ticks: 0,
        }
    }

    #[test]
    fn advisory_has_its_own_run_owner_without_a_foreground_machine() {
        let mut actions = NativeActions { _private: () };
        let mut ledger = None;
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            let evidence = cx.evidence();
            let request = actions
                .assess_walk(assessment_walk_request(evidence), cx)
                .unwrap();
            let ledger = cx.ledger.as_mut().unwrap();
            assert!(ledger.owner.is_none());
            let authority = ledger.outbox.last().unwrap().authority();
            ledger.revoke_foreground();
            assert!(
                authority.live(),
                "foreground replacement cannot cancel advice"
            );
            assert_eq!(ledger.outbox.len(), 1);
            ledger.complete_assess_walk(&authority, advisory_failure(request, evidence));
            assert!(actions.take_assess_receipt(cx, request).is_some());
            assert!(!authority.live());
        });
    }

    #[test]
    fn assess_walk_is_compute_only_and_does_not_spend_event_budget_or_own_movement() {
        let mut actions = NativeActions { _private: () };
        let mut ledger = Some(Box::new(ledger::Ledger::default()));
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            let owner = install_owner(cx);
            let evidence = cx.evidence();
            let request = assessment_walk_request(evidence);
            let request_id = actions.assess_walk(request, cx).unwrap();
            let ledger = cx.ledger.as_ref().unwrap();
            let action = ledger.outbox.last().expect("assessment is queued");
            assert_eq!(action.request_id.get(), request_id);
            assert!(matches!(&action.effect, HostEffect::AssessWalk(_)));
            assert!(action.live());
            assert_eq!(cx.budget.events, 0, "assessment spends no event budget");
            assert_eq!(owner.active_walk(), None, "assessment owns no movement");
            assert!(
                ledger.walk.is_none(),
                "assessment does not create a walk receipt"
            );
            assert!(ledger.assess_receipt.is_none());
            let active_walk = NonZeroU64::new(request_id + 100).unwrap();
            owner.set_walk(active_walk);
            let newer = actions
                .assess_walk(assessment_walk_request(evidence), cx)
                .unwrap();
            assert!(newer > request_id);
            assert_eq!(owner.active_walk(), Some(active_walk));
            assert_eq!(cx.ledger.as_ref().unwrap().outbox.len(), 1);
        });
    }

    #[test]
    fn assess_receipts_replace_cancel_and_reject_stale_publications() {
        let mut actions = NativeActions { _private: () };
        let mut ledger = Some(Box::new(ledger::Ledger::default()));
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            install_owner(cx);
            let evidence = cx.evidence();
            let first = actions
                .assess_walk(assessment_walk_request(evidence), cx)
                .unwrap();
            let old_authority = cx
                .ledger
                .as_ref()
                .unwrap()
                .outbox
                .last()
                .unwrap()
                .authority();

            let second = actions
                .assess_walk(assessment_walk_request(evidence), cx)
                .unwrap();
            let current_authority = cx
                .ledger
                .as_ref()
                .unwrap()
                .outbox
                .last()
                .unwrap()
                .authority();
            assert!(second > first);
            assert!(!old_authority.live());
            assert_eq!(cx.ledger.as_ref().unwrap().outbox.len(), 1);
            cx.ledger
                .as_mut()
                .unwrap()
                .complete_assess_walk(&old_authority, advisory_failure(first, evidence));
            assert!(cx.assess_receipt(second).is_none());

            let receipt = advisory_failure(second, evidence);
            cx.ledger
                .as_mut()
                .unwrap()
                .complete_assess_walk(&current_authority, receipt.clone());
            assert_eq!(cx.assess_receipt(second).unwrap().request_id, second);
            assert_eq!(
                actions
                    .take_assess_receipt(cx, second)
                    .unwrap()
                    .detail
                    .as_deref(),
                Some("risk input unavailable")
            );
            assert!(cx.assess_receipt(second).is_none());
            assert!(!current_authority.live());
            cx.ledger
                .as_mut()
                .unwrap()
                .complete_assess_walk(&current_authority, receipt);
            assert!(cx.assess_receipt(second).is_none());

            let cancelled = actions
                .assess_walk(assessment_walk_request(evidence), cx)
                .unwrap();
            let cancelled_authority = cx
                .ledger
                .as_ref()
                .unwrap()
                .outbox
                .last()
                .unwrap()
                .authority();
            cx.cancel_request(cancelled);
            assert!(cx.ledger.as_ref().unwrap().assess_receipt.is_none());
            assert!(cx.ledger.as_ref().unwrap().assess_request.is_none());
            cx.ledger
                .as_mut()
                .unwrap()
                .complete_assess_walk(&cancelled_authority, advisory_failure(cancelled, evidence));
            assert!(cx.assess_receipt(cancelled).is_none());

            let revoked = actions
                .assess_walk(assessment_walk_request(evidence), cx)
                .unwrap();
            let revoked_authority = cx
                .ledger
                .as_ref()
                .unwrap()
                .outbox
                .last()
                .unwrap()
                .authority();
            cx.ledger.as_mut().unwrap().revoke();
            assert!(cx.ledger.as_ref().unwrap().assess_receipt.is_none());
            cx.ledger
                .as_mut()
                .unwrap()
                .complete_assess_walk(&revoked_authority, advisory_failure(revoked, evidence));
            assert!(cx.assess_receipt(revoked).is_none());
        });
    }

    #[test]
    fn plain_walk_under_stage_grant_arms_food_guard() {
        let mut ledger = None;
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            cx.stage_cross = Some(Arc::from([Arc::from("declared-zone")]));
            let mut actions = NativeActions { _private: () };
            let handle = actions
                .begin::<walk::Walk>(assessment_walk_request(cx.evidence()), cx)
                .unwrap();
            let HostEffect::Walk(request) = &cx.ledger.as_ref().unwrap().outbox[0].effect else {
                panic!("the stage-granted walk must use the native walk path");
            };
            assert_eq!(request.cross.as_ref(), &[Arc::<str>::from("declared-zone")]);
            assert!(
                request.food_guard,
                "a declared crossing must arm the food guard"
            );
            assert!(request.allow.food);
            assert!(!request.protect);
            drop(handle);
        });
    }
    #[test]
    fn an_unscoped_walk_keeps_the_inherited_danger_policy() {
        let mut ledger = None;
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            assert!(cx.stage_cross.is_none());
            let mut actions = NativeActions { _private: () };
            let handle = actions
                .begin::<walk::Walk>(assessment_walk_request(cx.evidence()), cx)
                .unwrap();
            let HostEffect::Walk(request) = &cx.ledger.as_ref().unwrap().outbox[0].effect else {
                panic!("the unscoped walk must use the native walk path");
            };
            assert_eq!(
                request.options.allow_danger_zones,
                WalkBit::Inherit,
                "an unscoped Quester walk retains the pre-consent admission policy"
            );
            drop(handle);
        });
    }
    #[test]
    fn plain_walk_under_step_grant_arms_food_guard_without_a_stage() {
        let mut ledger = None;
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            let mut request = assessment_walk_request(cx.evidence());
            request.cross = vec![Arc::from("declared-step-zone")].into_boxed_slice();
            let mut actions = NativeActions { _private: () };
            let handle = actions.begin::<walk::Walk>(request, cx).unwrap();
            let HostEffect::Walk(request) = &cx.ledger.as_ref().unwrap().outbox[0].effect else {
                panic!("the step-granted walk must use the native walk path");
            };
            assert_eq!(
                request.cross.as_ref(),
                &[Arc::<str>::from("declared-step-zone")]
            );
            assert!(
                request.food_guard,
                "a declared crossing must arm the food guard"
            );
            assert!(request.allow.food);
            assert_eq!(
                request.options.allow_danger_zones,
                WalkBit::Inherit,
                "a step crossing is a request carried by `cross`; the option stays inherited"
            );
            assert!(!request.protect);
            drop(handle);
        });
    }

    #[test]
    fn explicit_cure_guard_keeps_food_disabled_across_step_and_stage_crossings() {
        for stage_cross in [false, true] {
            for step_cross in [false, true] {
                let mut ledger = None;
                with_frame(&mut ledger, Duration::ZERO, |cx| {
                    cx.stage_cross = stage_cross.then(|| Arc::from([Arc::from("stage-zone")]));
                    let mut request = assessment_walk_request(cx.evidence());
                    assert!(!request.cure_guard);
                    request.cure_guard = true;
                    request.allow.food = false;
                    request.allow.prayer = false;
                    if step_cross {
                        request.cross = Box::from([Arc::from("step-zone")]);
                    }
                    let mut actions = NativeActions { _private: () };
                    let handle = actions.begin::<walk::Walk>(request, cx).unwrap();
                    let HostEffect::Walk(request) =
                        &cx.ledger.as_ref().unwrap().outbox.last().unwrap().effect
                    else {
                        panic!("the explicit cure request must remain a walk");
                    };
                    assert!(request.cure_guard);
                    assert!(!request.protect && !request.food_guard && !request.allow.food);
                    assert_eq!(
                        request.cross.len(),
                        usize::from(stage_cross) + usize::from(step_cross)
                    );
                    drop(handle);
                });
            }
        }
    }

    #[test]
    fn stage_cross_unions_step_grants_without_disabling_survival() {
        let mut ledger = None;
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            cx.stage_cross = Some(Arc::from([Arc::from("declared-zone")]));
            let mut request = assessment_walk_request(cx.evidence());
            request.cross = vec![Arc::from("step-zone")].into_boxed_slice();
            request.options.allow_danger_zones = WalkBit::Allow;
            request.protect = true;
            let mut actions = NativeActions { _private: () };
            let handle = actions.begin::<walk::Walk>(request, cx).unwrap();
            let HostEffect::Walk(request) = &cx.ledger.as_ref().unwrap().outbox[0].effect else {
                panic!("the native walk must reach the existing host route path");
            };
            assert_eq!(
                request
                    .cross
                    .iter()
                    .map(AsRef::as_ref)
                    .collect::<Vec<&str>>(),
                ["step-zone", "declared-zone"]
            );
            assert_eq!(request.options.allow_danger_zones, WalkBit::Allow);
            assert!(request.protect && request.food_guard);
            let expected_allow = WalkAllow {
                food: true,
                ..WalkAllow::default()
            };
            assert_eq!(request.allow, expected_allow);
            drop(handle);
            actions
                .assess_walk(assessment_walk_request(cx.evidence()), cx)
                .unwrap();
            let HostEffect::AssessWalk(request) =
                &cx.ledger.as_ref().unwrap().outbox.last().unwrap().effect
            else {
                panic!("the advisory must use the same stage declaration");
            };
            assert_eq!(request.cross.as_ref(), &[Arc::<str>::from("declared-zone")]);
            assert_eq!(request.options.allow_danger_zones, WalkBit::Inherit);
        });
    }

    #[test]
    fn stage_cross_deduplicates_grants_and_preserves_prelude_step_consent() {
        let mut ledger = None;
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            let stage = [Arc::<str>::from("declared-zone")];
            let mut request = assessment_walk_request(cx.evidence());
            request.cross = vec![Arc::clone(&stage[0]), Arc::clone(&stage[0])].into_boxed_slice();
            apply_stage_cross(&mut request, Some(&stage));
            assert_eq!(request.cross.as_ref(), &stage);
            let mut prelude = assessment_walk_request(cx.evidence());
            prelude.cross = vec![Arc::from("prelude-zone")].into_boxed_slice();
            apply_stage_cross(&mut prelude, Some(&[]));
            assert_eq!(prelude.cross.as_ref(), &[Arc::<str>::from("prelude-zone")]);
            assert_eq!(prelude.options.allow_danger_zones, WalkBit::Inherit);
            let mut next_step = assessment_walk_request(cx.evidence());
            apply_stage_cross(&mut next_step, Some(&[]));
            assert!(
                next_step.cross.is_empty(),
                "a fresh step inherits no earlier grants"
            );
            assert_eq!(request.options.allow_danger_zones, WalkBit::Inherit);
            let mut ordinary = assessment_walk_request(cx.evidence());
            ordinary.options.allow_danger_zones = WalkBit::Allow;
            apply_stage_cross(&mut ordinary, None);
            assert_eq!(ordinary.options.allow_danger_zones, WalkBit::Allow);
        });
    }

    #[test]
    fn empty_declared_stage_leaves_the_walk_option_alone() {
        let mut ledger = None;
        with_frame(&mut ledger, Duration::ZERO, |cx| {
            cx.stage_cross = Some(Arc::<[Arc<str>]>::from([]));
            let mut request = assessment_walk_request(cx.evidence());
            request.options.allow_danger_zones = WalkBit::Allow;
            let mut actions = NativeActions { _private: () };
            let handle = actions.begin::<walk::Walk>(request, cx).unwrap();
            let HostEffect::Walk(request) = &cx.ledger.as_ref().unwrap().outbox[0].effect else {
                panic!("the scoped walk must use the native walk path");
            };
            assert!(request.cross.is_empty(), "the stage declares no zones");
            assert_eq!(
                request.options.allow_danger_zones,
                WalkBit::Allow,
                "a stage scope neither grants nor forbids danger zones"
            );
            assert!(!request.food_guard, "an empty declaration arms no guard");
            drop(handle);
        });
    }
}
