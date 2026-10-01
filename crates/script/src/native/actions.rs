use super::ledger::{HostAction, HostEffect};
use super::owner::Owner;
use super::*;
use crate::native_bank::{BankPickReceipt, BankPickRequest};
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};

impl ActionContext<'_> {
    fn owner(&self) -> Result<Arc<Owner>, ActionError> {
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

    pub fn begin_quiet_read(&mut self, request_id: u64) -> Result<QuietReadLease, ActionError> {
        let owner = self.owner()?;
        let request_id = NonZeroU64::new(request_id).ok_or(ActionError::Stale)?;
        let ledger = self.ledger.as_mut().expect("owner checked");
        let lease_id = ledger.next_id()?;
        if !owner.acquire_quiet(lease_id) {
            return Err(ActionError::Busy);
        }
        ledger.quiet_since = Some((request_id, lease_id, self.wall_now));
        Ok(QuietReadLease { owner, lease_id })
    }

    pub fn end_quiet_read(&mut self, lease: QuietReadLease) {
        drop(lease);
    }

    pub fn charge_transition(&mut self) -> bool {
        self.eligible && self.budget.transition()
    }

    /// Queue one game interaction. Walks go through [`Self::walk`], which owns
    /// their follow and terminal receipt; route, channel, mouse, run-policy
    /// and lifecycle requests are host-owned and never native interactions.
    pub fn emit(&mut self, request: InteractReq) -> Result<u64, ActionError> {
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
            owner,
            request_id,
            effect: HostEffect::Interaction(request),
        });
        Ok(request_id.get())
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
        if !owner.disposal_available() {
            return Err(ActionError::BudgetExhausted);
        }
        if !self.budget.event(true) {
            return Err(ActionError::BudgetExhausted);
        }
        let ledger = self.ledger.as_mut().expect("owner checked");
        let request_id = ledger.next_id()?;
        if !owner.acquire_disposal(request_id) {
            return Err(ActionError::BudgetExhausted);
        }
        if ledger.outbox.capacity() < 5 {
            // A batch has five rows, not Vec's geometric eight-row capacity.
            ledger.outbox.reserve_exact(5 - ledger.outbox.len());
        }
        ledger.outbox.push(HostAction {
            owner,
            request_id,
            effect: HostEffect::Interaction(request),
        });
        Ok(request_id.get())
    }

    /// Queue one selector-owned bank pick. This computes a route choice; it
    /// does not consume a game-interaction event.
    pub fn bank_pick(&mut self, request: BankPickRequest) -> Result<u64, ActionError> {
        let owner = self.owner()?;
        let ledger = self.ledger.as_mut().expect("owner checked");
        let request_id = ledger.next_id()?;
        owner.set_interaction(request_id);
        ledger.interaction = None;
        ledger.interaction_request = None;
        ledger.bank_pick = None;
        ledger.bank_pick_request = Some(request_id);
        ledger.outbox.push(HostAction {
            owner,
            request_id,
            effect: HostEffect::BankPick(request),
        });
        Ok(request_id.get())
    }

    pub fn bank_pick_receipt(&self, request_id: u64) -> Option<&BankPickReceipt> {
        let ledger = self.ledger.as_ref()?;
        let owner = ledger.owner.as_ref()?;
        if owner.run != self.run() || owner.id.get() != self.action_id || !owner.live() {
            return None;
        }
        ledger.bank_pick.as_ref().filter(|receipt| {
            ledger
                .bank_pick_request
                .is_some_and(|id| id.get() == request_id)
                && receipt.request_id == request_id
                && receipt.evidence.run == self.run()
        })
    }

    pub fn walk(&mut self, request: WalkRequest) -> Result<u64, ActionError> {
        let owner = self.owner()?;
        if request.required_after.run != self.run() {
            return Err(ActionError::Stale);
        }
        if !self.budget.event(false) {
            return Err(ActionError::BudgetExhausted);
        }
        let ledger = self.ledger.as_mut().expect("owner checked");
        let request_id = ledger.next_id()?;
        owner.set_walk(request_id);
        ledger.walk = None;
        ledger.outbox.push(HostAction {
            owner,
            request_id,
            effect: HostEffect::Walk(request),
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
            .chain(ledger.disposal_receipts.iter().flatten())
            .find(|receipt| receipt.request_id == request_id && receipt.evidence.run == self.run())
    }

    pub fn cancel_request(&mut self, request_id: u64) {
        let Some(ledger) = self.ledger.as_mut() else {
            return;
        };
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
            if ledger.bank_pick_request == Some(request_id) {
                ledger.bank_pick_request = None;
                ledger.bank_pick = None;
            }
            for receipt in &mut ledger.disposal_receipts {
                if receipt
                    .as_ref()
                    .is_some_and(|receipt| receipt.request_id == request_id.get())
                {
                    *receipt = None;
                }
            }
            owner.cancel_walk(request_id);
            if let Some((request, lease, _)) = ledger.quiet_since {
                if request == request_id {
                    owner.release_quiet(lease);
                    ledger.quiet_since = None;
                }
            }
            ledger
                .outbox
                .retain(|action| action.request_id != request_id);
        }
    }
}

/// Whether `emit` may queue `request`: a game interaction the host dispatches
/// once, not a route (the typed walk owns those), broker channel, script
/// mouse, inspect request/ack, SetCameraYaw, host run policy, GatherRun,
/// GatherStop, ProgressRead or isolate lifecycle marker.
fn native_interaction(request: &InteractReq) -> bool {
    request.is_game()
        && !matches!(
            request,
            InteractReq::Walk { .. }
                | InteractReq::WalkNear { .. }
                | InteractReq::WalkNearestBank
                | InteractReq::SelectBank { .. }
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
        ledger.revoke();
        let owner = Owner::new(run, ledger.next_id()?);
        ledger.owner = Some(Arc::clone(&owner));
        cx.action_id = owner.id.get();
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

    #[test]
    fn cancelling_a_drained_interaction_revokes_its_host_authority() {
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let pin = selected.selected_pin().unwrap();
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
            evidence,
            pin: &pin,
            snapshot: SnapshotView::new(None, evidence),
            retained: &mut retained,
            action_id: owner.id.get(),
            active_now: Duration::ZERO,
            wall_now: Instant::now(),
            ledger: &mut ledger,
            budget: &mut budget,
            eligible: true,
        };
        let request = cx.emit(InteractReq::CloseModal).unwrap();
        let drained = cx.ledger.as_mut().unwrap().outbox.pop().unwrap();
        let authority = drained.authority();
        assert!(drained.live());
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
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let pin = selected.selected_pin().unwrap();
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
            evidence,
            pin: &pin,
            snapshot: SnapshotView::new(snapshot, evidence),
            retained: &mut retained,
            action_id: 0,
            active_now,
            wall_now: Instant::now(),
            ledger,
            budget: &mut budget,
            eligible: true,
        };
        f(&mut cx)
    }

    fn drop_request(slot: i32) -> InteractReq {
        InteractReq::Held {
            name: "Logs".into(),
            action: "Drop".into(),
            slot: Some(slot),
        }
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
            let actions = std::mem::take(&mut cx.ledger.as_mut().unwrap().outbox);
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
            assert!(owner.acquire_disposal(reserved));

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
            let actions = std::mem::take(&mut cx.ledger.as_mut().unwrap().outbox);
            assert!(actions.iter().all(HostAction::live));
            assert_eq!(actions.len(), 5);
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
                radius: 0,
                options: FindOptions::default(),
                required_after: cx.evidence(),
                evidence: None,
                cross: Vec::new().into_boxed_slice(),
            };
            let handle = actions.begin::<super::walk::Walk>(request, cx).unwrap();
            let authority = cx.ledger.as_ref().unwrap().outbox[0].authority();
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
                actor: ActorView {
                    name: Some("alice".into()),
                    actions: Vec::new(),
                    tile,
                    distance: 0,
                    animation: -1,
                    pose_animation: -1,
                    orientation: 0,
                    target_orientation: 0,
                    overhead_text: None,
                    spot_animation: -1,
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
            },
            energy: 100,
            weight: 0,
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
                            loc_id: None,
                            radius,
                            options: FindOptions::default(),
                            required_after: cx.evidence(),
                            evidence: None,
                            cross: Vec::new().into_boxed_slice(),
                        };
                        let handle = actions.begin::<super::walk::Walk>(request, cx).unwrap();
                        let authority = cx.ledger.as_ref().unwrap().outbox[0].authority();
                        cx.ledger.as_mut().unwrap().walk = Some(WalkReceipt {
                            request_id: authority.request_id().get(),
                            evidence: cx.evidence(),
                            end: WalkEnd::RouteEnded,
                            blocked: None,
                            detail: None,
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
}
