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
            batch: 0,
            effect: HostEffect::Interaction(request),
            observed_walk_outcome_seq: self.observed_walk_outcome_seq,
        });
        Ok(request_id.get())
    }
    /// Admit one ordered, wire-bounded interaction batch by taking ownership
    /// of its compact request prefix.
    pub fn emit_batch(&mut self, mut rows: [Option<InteractReq>; 5]) -> Result<u64, ActionError> {
        let owner = self.owner()?;
        let len = validate_batch(&rows, self.pin.revision)?;
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
                owner: Arc::clone(&owner),
                request_id,
                batch: first_id.get(),
                effect: HostEffect::Interaction(request),
                observed_walk_outcome_seq: self.observed_walk_outcome_seq,
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
            owner,
            request_id,
            batch: 0,
            effect: HostEffect::Interaction(request),
            observed_walk_outcome_seq: self.observed_walk_outcome_seq,
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
            batch: 0,
            effect: HostEffect::BankPick(request),
            observed_walk_outcome_seq: self.observed_walk_outcome_seq,
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
        ledger.walk_events.clear();
        ledger.outbox.push(HostAction {
            owner,
            request_id,
            batch: 0,
            effect: HostEffect::Walk(request),
            observed_walk_outcome_seq: self.observed_walk_outcome_seq,
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
            for receipt in &mut ledger.batch_receipts {
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

impl ActionHandle<crate::combat::Combat> {
    pub(crate) fn prayer_cleanup(&self) -> crate::combat::RaisedPrayers {
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
            observed_walk_outcome_seq: 0,
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
            observed_walk_outcome_seq: 0,
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
        }
    }

    fn drink_request() -> InteractReq {
        InteractReq::Held {
            name: "Prayer potion".into(),
            action: "Drink".into(),
            slot: None,
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
            },
            InteractReq::Obj {
                x: 2601,
                z: 3200,
                level: 0,
                name: Some("Ground item".into()),
                action: "Take".into(),
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
    fn batch_rejects_invalid_shapes_and_host_grammar_without_side_effects() {
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
                radius: 0,
                arrival: nav::arrival::ArrivalKind::Reach,
                options: FindOptions::default(),
                required_after: cx.evidence(),
                evidence: None,
                loc_id: None,
                cross: Box::new([]),
                protect: false,
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
                loc_id: None,
                radius: 0,
                arrival: nav::arrival::ArrivalKind::Reach,
                options: FindOptions::default(),
                required_after: cx.evidence(),
                evidence: None,
                cross: Vec::new().into_boxed_slice(),
                protect: false,
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
                            arrival: nav::arrival::ArrivalKind::Reach,
                            options: FindOptions::default(),
                            required_after: cx.evidence(),
                            evidence: None,
                            cross: Vec::new().into_boxed_slice(),
                            protect: false,
                            allow: Default::default(),
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
