use super::ledger::{HostAction, HostEffect};
use super::owner::Owner;
use super::*;
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

    pub fn emit(&mut self, request: InteractReq) -> Result<u64, ActionError> {
        let owner = self.owner()?;
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
        ledger.interaction.as_ref().filter(|receipt| {
            receipt.request_id == request_id && receipt.evidence.run == self.run()
        })
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
}
