use super::owner::Owner;
use super::{ActionError, AssessReceipt, InteractionReceipt, WalkEvent, WalkReceipt, WalkRequest};
use crate::interact::InteractReq;
use crate::native_bank::{
    BankPickReceipt, BankPickRequest, ServicePickReceipt, ServicePickRequest,
};
use api::identity::ActiveClock;
use api::selected::RunKey;
use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(crate) struct Runtime {
    pub ledger: Option<Box<Ledger>>,
    pub budget: TickBudget,
    pub clock: ActiveClock,
    pub observed_walk_outcome: ObservedWalkOutcome,
    /// The latest manual takeover sequence consumed by Resume.
    pub takeover_consumed_seq: u64,
    /// One confirmed native gameplay change, fenced to its run and session.
    pub pending_note_progress: Option<RunKey>,
}

impl Runtime {
    pub fn revoke(&mut self) {
        if let Some(ledger) = &mut self.ledger {
            ledger.revoke();
        }
        self.clear_note_progress();
    }

    pub fn clear_note_progress(&mut self) {
        self.pending_note_progress = None;
    }
    /// Consume the currently observed manual takeover when the slot resumes.
    pub fn consume_manual_takeover(&mut self) {
        self.takeover_consumed_seq = self.observed_walk_outcome.seq();
    }

    /// Consume the one pending stamp, even when it belongs to an old
    /// run/session. A stale marker is never carried into the current run.
    pub fn take_note_progress(&mut self, run: RunKey) -> bool {
        self.pending_note_progress.take() == Some(run)
    }

    /// The live modal wait of this runtime's owner, sampled at `now` on the
    /// owner's own active clock: the clock its machines read as
    /// `ActionContext::active_now` (`slot/compiled.rs:411`).
    pub fn modal_wait_tick(&mut self, now: Instant, tick: u64) -> Option<(NonZeroU64, u64)> {
        let active_now = self.clock.now(now).as_duration();
        self.ledger.as_mut()?.modal_wait_tick(active_now, tick)
    }

    /// The live quiet read of this runtime's owner, sampled at `now` on the
    /// owner's own clocks, as [`Self::modal_wait_tick`].
    pub fn quiet_read_wait(&mut self, now: Instant, tick: u64) -> Option<(QuietReadOwner, u64)> {
        let active_now = self.clock.now(now).as_duration();
        self.ledger.as_mut()?.quiet_read_wait(now, active_now, tick)
    }
}

/// The host walk outcome the slot observed last: its sequence and its cancel
/// reason, packed in one word so the per-slot runtime keeps its size
/// (`s2b_admission_per_bot_layouts` pins `SlotScript`). The sequence uses
/// bits 0-62 (a 63-bit domain, `1..=WALK_OUTCOME_SEQ_MAX`) and the reason
/// uses bit 63. The host counts the sequence from 1 with a plain increment
/// and never wraps it, because readers compare sequences in order (the
/// manual takeover watermark uses `>=`). This relies on a lifetime
/// assumption: at one outcome per 600 ms tick, the count needs about 175
/// billion years to reach 2^63, so in practice the sequence never wraps and
/// never reaches bit 63. A sequence above the domain breaks the host
/// contract: debug builds panic, and release builds drop bit 63, so the
/// sequence never changes the reason.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ObservedWalkOutcome(u64);

impl ObservedWalkOutcome {
    const USER_INPUT: u64 = 1 << 63;

    pub fn new(seq: u64, cancel_reason: super::WalkCancelReason) -> Self {
        debug_assert!(
            seq <= super::WALK_OUTCOME_SEQ_MAX,
            "walk outcome sequence above WALK_OUTCOME_SEQ_MAX"
        );
        let reason = match cancel_reason {
            super::WalkCancelReason::None => 0,
            super::WalkCancelReason::UserInput => Self::USER_INPUT,
        };
        Self((seq & super::WALK_OUTCOME_SEQ_MAX) | reason)
    }

    pub fn seq(self) -> u64 {
        self.0 & !Self::USER_INPUT
    }

    pub fn cancel_reason(self) -> super::WalkCancelReason {
        if self.0 & Self::USER_INPUT == 0 {
            super::WalkCancelReason::None
        } else {
            super::WalkCancelReason::UserInput
        }
    }
}
/// Typed host work retains an owner, never a frame borrow. The host must check
/// `live` again immediately before applying the work or advancing its follower.
/// Immutable evidence carried with a native deposit click until host dispatch.
#[derive(Debug, Clone)]
pub struct BankDepositTrace {
    pub generation: u64,
    pub transfer: u8,
    pub click_tick: u64,
    pub sequence: u64,
    pub id: i32,
    pub slot: i32,
    pub component: i32,
    pub option: Arc<str>,
    pub operation: i32,
    pub main_before: i32,
    pub side_before: Option<i32>,
    pub bank_before: Option<i32>,
    pub main_revision: Option<u64>,
    pub side_revision: Option<u64>,
}

pub struct HostAction {
    pub(crate) custody: HostCustody,
    pub request_id: NonZeroU64,
    pub batch: u64,
    pub effect: HostEffect,
    pub observed_walk_outcome_seq: u64,
    pub bank_deposit: Option<Arc<BankDepositTrace>>,
    pub(crate) protected_items: Option<Arc<[i32]>>,
}

/// The owner of one queued host action, and the host's custody of a single
/// interaction or walk. The host takes queued work only through
/// `SlotScript::take_native_action`, which releases the custody, and it
/// records the interaction's receipt in that same observation pass. Every
/// other way out of the queue drops the action unsent: a held tick
/// (`SlotScript::seat_held_tick`, `GameplaySeat::held_tick`), a guardian
/// hold (`GameplaySeat::hold`), a seat refusal, a session reset or a
/// cancellation. Dropping armed custody marks the owner's current interaction
/// as unsent, so a machine can tell a request that still waits for an eligible
/// dispatch pass from one that no pass will ever send. A cancelled or
/// superseded request is no longer the owner's current interaction, so the
/// mark ignores it: `ActionContext::interaction_dispatch` already reports it
/// unsent from that ownership.
pub(crate) struct HostCustody {
    owner: Arc<Owner>,
    unsent: Option<NonZeroU64>,
}

impl HostCustody {
    /// Custody of a single interaction: the request is marked unsent if it
    /// leaves the queue other than through the host's take.
    pub(crate) fn interaction(owner: Arc<Owner>, request: NonZeroU64) -> Self {
        Self {
            owner,
            unsent: Some(request),
        }
    }

    /// Custody of a walk: the request is marked unsent if it leaves the queue
    /// other than through the host's take, so its machine can tell a route the
    /// host never armed from one that is still being followed.
    pub(crate) fn walk(owner: Arc<Owner>, request: NonZeroU64) -> Self {
        Self {
            owner,
            unsent: Some(request),
        }
    }

    /// Owner-only custody: batches, picks and assessments keep their own
    /// receipt rules.
    pub(crate) fn owner(owner: Arc<Owner>) -> Self {
        Self {
            owner,
            unsent: None,
        }
    }
}

impl Drop for HostCustody {
    fn drop(&mut self) {
        if let Some(request) = self.unsent {
            self.owner.note_unsent(request);
        }
    }
}

pub enum HostEffect {
    Interaction(InteractReq),
    Walk(WalkRequest),
    BankPick(BankPickRequest),
    /// Compute a typed service route choice under the same foreground fence.
    ServicePick(ServicePickRequest),
    AssessWalk(WalkRequest),
    ServerOrdered(crate::surface::ServerOrderedEnvelope),
}

/// A host continuation retains this fence after consuming the request payload.
/// It grants no authority after handle drop, cancellation or session rollover.
#[derive(Clone)]
pub struct HostAuthority {
    owner: Arc<Owner>,
    request: NonZeroU64,
    walk: bool,
    protected_items: Option<Arc<[i32]>>,
}

impl HostAuthority {
    pub fn live(&self) -> bool {
        if self.walk {
            self.owner.walk_live(self.request)
        } else {
            self.owner.interaction_live(self.request)
        }
    }

    /// Observe the action owner's lifetime independently of this request.
    /// This is not dispatch authority: request continuations must use `live`.
    pub fn owner_live(&self) -> bool {
        self.owner.live()
    }

    pub fn run(&self) -> RunKey {
        self.owner.run
    }
    pub fn action_id(&self) -> NonZeroU64 {
        self.owner.id
    }
    /// Path items reserved from food selection for this authorized walk.
    pub fn protected_items(&self) -> Option<&[i32]> {
        self.protected_items.as_deref()
    }
    /// Share the compiled Path allocation with a persistent route guard.
    pub fn clone_protected_items(&self) -> Option<Arc<[i32]>> {
        self.protected_items.as_ref().map(Arc::clone)
    }
    pub fn request_id(&self) -> NonZeroU64 {
        self.request
    }

    /// Revoke only this request at a real dispatch boundary in integration tests.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn cancel_request_for_test(&self) {
        self.owner.cancel_interaction(self.request);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuietReadOwner {
    pub run: RunKey,
    pub action_id: NonZeroU64,
    pub request_id: NonZeroU64,
}

impl HostAction {
    pub fn live(&self) -> bool {
        let owner = &self.custody.owner;
        match &self.effect {
            HostEffect::Interaction(_)
            | HostEffect::BankPick(_)
            | HostEffect::ServicePick(_)
            | HostEffect::AssessWalk(_) => owner.interaction_live(self.request_id),
            HostEffect::Walk(_) => owner.walk_live(self.request_id),
            HostEffect::ServerOrdered(_) => owner.batch_live(self.request_id),
        }
    }
    pub fn run(&self) -> RunKey {
        self.custody.owner.run
    }
    pub fn action_id(&self) -> NonZeroU64 {
        self.custody.owner.id
    }
    pub fn authority(&self) -> HostAuthority {
        HostAuthority {
            owner: Arc::clone(&self.custody.owner),
            request: self.request_id,
            walk: matches!(self.effect, HostEffect::Walk(_)),
            protected_items: self.protected_items.as_ref().map(Arc::clone),
        }
    }
    /// The host has taken this action for dispatch in an eligible pass.
    pub(crate) fn release_custody(&mut self) {
        self.custody.unsent = None;
    }
}

/// The quiet-read lease's wall bound. It is the paint-hide cap from the
/// begin, and the hold's own bound from the begin and from each window the
/// owner arms: a paused or stranded reader stops hiding the page, and stops
/// holding the host, after this, whatever its owner does.
pub(super) const QUIET_READ_WALL_BOUND: Duration = Duration::from_secs(10);

/// The ledger's record of the live quiet read.
#[derive(Debug, Clone, Copy)]
pub(crate) struct QuietRead {
    pub request: NonZeroU64,
    pub lease: NonZeroU64,
    /// Wall time of the begin. The paint-hide cap runs from here and never
    /// moves.
    pub since: Instant,
    /// The lease's wall bound for the hold. It starts at the begin and each
    /// window the owner arms (its click, its capture, its close) takes a
    /// fresh bound from that poll, so a click never goes out against a lease
    /// whose bound passes in the frame that sends it. The same rule as the
    /// compatibility read's `renew_hold`.
    pub hold_until: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickChannel {
    Bank,
    Service,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PendingPick {
    pub request: NonZeroU64,
    pub channel: PickChannel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PickReceipt {
    Bank(BankPickReceipt),
    Service(ServicePickReceipt),
}

impl PickReceipt {
    fn channel(&self) -> PickChannel {
        match self {
            Self::Bank(_) => PickChannel::Bank,
            Self::Service(_) => PickChannel::Service,
        }
    }

    pub fn request_id(&self) -> u64 {
        match self {
            Self::Bank(receipt) => receipt.request_id,
            Self::Service(receipt) => receipt.request_id,
        }
    }

    pub fn evidence(&self) -> api::quest_progress::EvidenceStamp {
        match self {
            Self::Bank(receipt) => receipt.evidence,
            Self::Service(receipt) => receipt.evidence,
        }
    }
}

/// Allocated only on first native admission. There is one foreground owner,
/// one outstanding walk and a bounded outbox, not a second navigation runtime.
pub struct Ledger {
    pub(crate) owner: Option<Arc<Owner>>,
    next_id: u64,
    pub(crate) outbox: Vec<HostAction>,
    pub(crate) walk: Option<WalkReceipt>,
    pub(crate) walk_prereq_allow: Option<(NonZeroU64, super::PrereqAllow)>,
    pub(crate) walk_events: Vec<WalkEvent>,
    pub(crate) interaction: Option<InteractionReceipt>,
    pub(crate) interaction_request: Option<NonZeroU64>,
    pub(crate) pick: Option<PickReceipt>,
    pub(crate) pick_request: Option<PendingPick>,
    // Keep the 72-byte terminal envelope lazy instead of widening every native ledger.
    pub(crate) server_ordered: Option<Box<super::ServerOrderedCompletion>>,
    pub(crate) server_ordered_request: Option<NonZeroU64>,
    pub(crate) assess_owner: Option<Arc<Owner>>,
    pub(crate) assess_receipt: Option<AssessReceipt>,
    pub(crate) assess_request: Option<NonZeroU64>,
    pub(crate) batch_receipts: [Option<InteractionReceipt>; 5],
    next_batch_receipt: usize,
    pub(crate) quiet_lease: Option<QuietRead>,
    /// The quiet read's lease and the window its owner enforces.
    pub(crate) quiet_deadline: Option<(NonZeroU64, super::WaitDeadline)>,
    /// Last modal-wait lease number. A separate sequence, so a lease does not
    /// shift the request ids that receipts and traces carry.
    pub(crate) modal_wait_seq: u64,
    /// The latest modal-wait lease and the bound its owner enforces.
    pub(crate) modal_wait_deadline: Option<(NonZeroU64, super::WaitDeadline)>,
    /// The latest open bank session a native machine of this account touched.
    /// It outlives one machine, so a later verb in the same session skips the
    /// Item press and a later reader can tell whose open it was.
    pub(crate) bank_session: Option<BankSessionMark>,
}

/// What a native machine knows about one open bank session: the login run and
/// the snapshot session generation it names, and two separate facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BankSessionMark {
    pub(crate) run: RunKey,
    pub(crate) generation: u64,
    /// The session withdraws as items: a native machine opened it (the server
    /// resets `%bankcert` on open) or pressed Item in it.
    pub(crate) item_mode: bool,
    /// A native bank machine of this account sent its own open request, and
    /// this is the session that appeared after it. A session that a player
    /// opened, or that a machine only found already open, never has this.
    pub(crate) opened_by_machine: bool,
}

impl Default for Ledger {
    fn default() -> Self {
        Self {
            owner: None,
            next_id: 1,
            outbox: Vec::with_capacity(6),
            walk: None,
            walk_prereq_allow: None,
            walk_events: Vec::new(),
            interaction: None,
            interaction_request: None,
            pick: None,
            pick_request: None,
            server_ordered: None,
            server_ordered_request: None,
            assess_owner: None,
            assess_receipt: None,
            assess_request: None,
            batch_receipts: std::array::from_fn(|_| None),
            next_batch_receipt: 0,
            quiet_lease: None,
            quiet_deadline: None,
            modal_wait_seq: 0,
            modal_wait_deadline: None,
            bank_session: None,
        }
    }
}

impl Ledger {
    /// Freeze consent on this live walk request, not its later terminal frame.
    pub(crate) fn capture_walk_prereq_allow(
        &mut self,
        authority: &HostAuthority,
        allow: super::PrereqAllow,
    ) -> bool {
        if !authority.walk
            || !authority.live()
            || !self.owner.as_ref().is_some_and(|owner| {
                owner.run == authority.run() && owner.id == authority.action_id() && owner.live()
            })
        {
            return false;
        }
        if let Some((request, captured)) = self.walk_prereq_allow {
            if request == authority.request_id() {
                return captured == allow;
            }
        }
        self.walk_prereq_allow = Some((authority.request_id(), allow));
        true
    }

    /// Test-only borrow of the real queued host actions.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn test_outbox(&self) -> &[HostAction] {
        &self.outbox
    }

    /// Test-only mutable access for existing receipt-driving fixtures.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn test_outbox_mut(&mut self) -> &mut Vec<HostAction> {
        &mut self.outbox
    }

    /// Test-only observation of the real foreground owner's live state.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn test_owner_live(&self) -> bool {
        self.owner.as_ref().is_some_and(|owner| owner.live())
    }

    pub fn next_id(&mut self) -> Result<NonZeroU64, ActionError> {
        let id = NonZeroU64::new(self.next_id).ok_or(ActionError::Cancelled)?;
        self.next_id = self.next_id.checked_add(1).unwrap_or(0);
        Ok(id)
    }
    /// Check that `count` consecutive nonzero ids can be consumed without
    /// changing the cursor. Batches use this before reserving owner slots.
    pub fn id_range(&self, count: usize) -> Result<NonZeroU64, ActionError> {
        let first = NonZeroU64::new(self.next_id).ok_or(ActionError::Cancelled)?;
        let last_offset = u64::try_from(count.checked_sub(1).ok_or(ActionError::Cancelled)?)
            .map_err(|_| ActionError::Cancelled)?;
        first
            .get()
            .checked_add(last_offset)
            .ok_or(ActionError::Cancelled)?;
        Ok(first)
    }

    /// Commit a previously checked consecutive id range.
    pub fn commit_id_range(&mut self, first: NonZeroU64, count: usize) {
        debug_assert_eq!(self.next_id, first.get());
        self.next_id = first.get().checked_add(count as u64).unwrap_or(0);
    }

    /// The live quiet read. The lease ends at its wall bound: the begin's, or
    /// the last window the owner armed.
    ///
    /// An expired hold makes its queued interaction stale (finding N1 of
    /// `HOLD-MODEL-AB`). The owner renews the bound from its poll, but the
    /// host reads the wall time later in the same frame, and a stall of the
    /// process (a suspended or sleeping machine) can put the whole bound
    /// between the two. The click that the poll queued is then still live in
    /// the interaction authority, which this lease does not own. The sample
    /// that releases the lease by expiry therefore also cancels the click or
    /// close that the owner queued and the host has not yet dispatched: it
    /// never goes out without a hold. A dispatched one is unchanged, and so is
    /// its receipt. The owner learns of it at its next poll, where the lease
    /// is no longer live ([`QuietReadLease::live`]): it fails its read, and
    /// its own retry and timeout apply.
    pub fn quiet_read(&mut self, now: Instant) -> Option<QuietReadOwner> {
        let quiet = self.quiet_lease?;
        let owner = Arc::clone(self.owner.as_ref()?);
        // Independent of the active action clock: a paused or stranded
        // reader must not keep hiding a modal indefinitely.
        if now >= quiet.hold_until && owner.quiet(quiet.lease) {
            owner.release_quiet(quiet.lease);
            self.cancel_undispatched_interaction(&owner);
        }
        if !owner.quiet(quiet.lease) {
            self.quiet_lease = None;
            self.quiet_deadline = None;
            return None;
        }
        Some(QuietReadOwner {
            run: owner.run,
            action_id: owner.id,
            request_id: quiet.request,
        })
    }

    /// Cancel the owner's interaction that is queued and not yet dispatched:
    /// the host's fence ([`HostAction::live`], [`HostAuthority::live`]) then
    /// drops it, wherever it waits. A dispatched interaction has its receipt
    /// (`complete_interaction`) and is not touched.
    fn cancel_undispatched_interaction(&mut self, owner: &Owner) {
        let Some(request) = self.interaction_request else {
            return;
        };
        if self.interaction.is_none() && owner.interaction_live(request) {
            owner.cancel_interaction(request);
            self.outbox.retain(|action| action.request_id != request);
        }
    }

    /// Whether the live quiet read still hides the journal page: the lease is
    /// live and its paint-hide cap, [`QUIET_READ_WALL_BOUND`] from the begin,
    /// has not passed. A later window of the owner does not move the cap.
    pub fn quiet_paint_hidden(&mut self, now: Instant) -> bool {
        self.quiet_read(now).is_some()
            && self.quiet_lease.is_some_and(|quiet| {
                now.saturating_duration_since(quiet.since) < QUIET_READ_WALL_BOUND
            })
    }

    /// The live quiet read and the observed tick by which its owner ends it:
    /// the window the owner armed last, or else the lease's own wall bound,
    /// which [`Self::quiet_read`] enforces itself. `active_now` is the
    /// owner's active clock at `now` ([`Runtime::quiet_read_wait`]).
    pub(crate) fn quiet_read_wait(
        &mut self,
        now: Instant,
        active_now: Duration,
        tick: u64,
    ) -> Option<(QuietReadOwner, u64)> {
        let quiet = self.quiet_read(now)?;
        let lease = self.quiet_lease?;
        let deadline_tick = match self.quiet_deadline.as_mut() {
            Some((id, deadline)) if *id == lease.lease => deadline.owner_tick(active_now, tick),
            _ => super::WaitDeadline::self_ending_tick(lease.hold_until, now, tick),
        };
        Some((quiet, deadline_tick))
    }

    /// The live foreground owner's modal-wait lease id: it waits for the page
    /// its own interaction opens. Revocation clears it.
    pub fn modal_wait(&self) -> Option<NonZeroU64> {
        self.owner.as_ref()?.modal_wait()
    }

    /// The live modal wait and the observed tick by which its owner ends it.
    /// Every lease is taken with its deadline
    /// (`ActionContext::begin_modal_wait`). `active_now` is the owner's
    /// active clock at the sample ([`Runtime::modal_wait_tick`]).
    pub(crate) fn modal_wait_tick(
        &mut self,
        active_now: Duration,
        tick: u64,
    ) -> Option<(NonZeroU64, u64)> {
        let lease = self.modal_wait()?;
        let deadline_tick = match self.modal_wait_deadline.as_mut() {
            Some((id, deadline)) if *id == lease => deadline.owner_tick(active_now, tick),
            // Unreachable: a lease without its deadline ends at the host's margin.
            _ => 0,
        };
        Some((lease, deadline_tick))
    }

    pub fn revoke_foreground(&mut self) {
        if let Some(owner) = self.owner.take() {
            owner.revoke();
        }
        self.outbox
            .retain(|action| matches!(action.effect, HostEffect::AssessWalk(_)));
        self.walk = None;
        self.walk_prereq_allow = None;
        self.walk_events.clear();
        self.interaction = None;
        self.interaction_request = None;
        self.pick = None;
        self.pick_request = None;
        self.server_ordered = None;
        self.server_ordered_request = None;
        self.batch_receipts.fill(None);
        self.next_batch_receipt = 0;
        self.quiet_lease = None;
    }

    pub fn revoke(&mut self) {
        self.revoke_foreground();
        if let Some(owner) = self.assess_owner.take() {
            owner.revoke();
        }
        self.outbox.clear();
        self.assess_receipt = None;
        self.assess_request = None;
        self.bank_session = None;
    }

    pub fn complete_assess_walk(&mut self, authority: &HostAuthority, receipt: AssessReceipt) {
        let request = authority.request_id();
        if !authority.live()
            || request.get() != receipt.request_id
            || authority.run() != receipt.evidence.run
            || self.assess_request != Some(request)
            || self.assess_receipt.is_some()
            || !self.assess_owner.as_ref().is_some_and(|owner| {
                owner.run == authority.run() && owner.id == authority.action_id() && owner.live()
            })
        {
            return;
        }
        self.assess_receipt = Some(receipt);
    }

    pub fn complete_interaction(&mut self, authority: &HostAuthority, receipt: InteractionReceipt) {
        if !authority.live()
            || authority.request_id().get() != receipt.request_id
            || authority.run() != receipt.evidence.run
            || !self.owner.as_ref().is_some_and(|owner| {
                owner.run == authority.run() && owner.id == authority.action_id() && owner.live()
            })
        {
            return;
        }
        let owner = self.owner.as_ref().expect("owner checked");
        if owner.batch_live(authority.request_id()) {
            owner.record_batch_receipt(authority.request_id(), receipt.accepted);
            self.batch_receipts[self.next_batch_receipt] = Some(receipt);
            self.next_batch_receipt = (self.next_batch_receipt + 1) % self.batch_receipts.len();
            owner.cancel_interaction(authority.request_id());
        } else if self.interaction_request == Some(authority.request_id())
            && self.interaction.is_none()
        {
            self.interaction = Some(receipt);
        }
    }

    pub fn complete_server_ordered(
        &mut self,
        authority: &HostAuthority,
        completion: super::ServerOrderedCompletion,
    ) {
        let request = authority.request_id();
        // Cancellation can revoke authority after a committed prefix. Retain
        // its terminal evidence, but never settle a replacement owner or run.
        if completion.dispatch.request_id != request.get()
            || completion.evidence.run != authority.run()
            || self.server_ordered_request != Some(request)
            || self.server_ordered.is_some()
            || !self.owner.as_ref().is_some_and(|owner| {
                owner.run == authority.run() && owner.id == authority.action_id()
            })
        {
            return;
        }
        authority.owner.cancel_interaction(request);
        self.server_ordered = Some(Box::new(completion));
        self.server_ordered_request = None;
    }

    pub fn complete_bank_pick(&mut self, authority: &HostAuthority, receipt: BankPickReceipt) {
        self.complete_pick(authority, PickReceipt::Bank(receipt));
    }

    pub fn complete_service_pick(
        &mut self,
        authority: &HostAuthority,
        receipt: ServicePickReceipt,
    ) {
        self.complete_pick(authority, PickReceipt::Service(receipt));
    }

    fn complete_pick(&mut self, authority: &HostAuthority, receipt: PickReceipt) {
        let request = authority.request_id();
        if !authority.live()
            || request.get() != receipt.request_id()
            || authority.run() != receipt.evidence().run
            || self.pick_request
                != Some(PendingPick {
                    request,
                    channel: receipt.channel(),
                })
            || self.pick.is_some()
            || !self.owner.as_ref().is_some_and(|owner| {
                owner.run == authority.run() && owner.id == authority.action_id() && owner.live()
            })
        {
            return;
        }
        self.pick = Some(receipt);
    }
}

impl Drop for Ledger {
    fn drop(&mut self) {
        self.revoke();
    }
}

/// The observed tick, not the caller's polling rate, replenishes the allowance.
/// This value is slot-owned even when there is no native action ledger.
///
/// It also holds the slot's view of a guardian hold, in the spare bytes of its
/// 24-byte layout (`scene_fence_uses_four_bytes_and_tick_budget_stays_compact`):
/// where the hold stands, whether the player was teleported while it lasted,
/// and the last observed player position that the teleport rule compares.
pub(crate) struct TickBudget {
    /// The last observed tick, or [`NO_TICK`] before the first one.
    tick: u64,
    scene: SceneFence,
    /// The tick (low 32 bits) and packed tile of the last host frame's player
    /// fix ([`pack_fix`]); `fix == 0` means none was observed.
    fix_tick: u32,
    fix: u32,
    transitions: u8,
    pub(super) events: u8,
    /// [`HOLD_PHASE_MASK`] bits: where the guardian hold stands; and
    /// [`HOLD_TELEPORTED`].
    hold: u8,
}

const NO_TICK: u64 = u64::MAX;

impl Default for TickBudget {
    fn default() -> Self {
        Self {
            tick: NO_TICK,
            scene: SceneFence::default(),
            fix_tick: 0,
            fix: 0,
            transitions: 0,
            events: 0,
            hold: 0,
        }
    }
}

/// No hold is pending for the actions.
const HOLD_NONE: u8 = 0;
/// The guardian holds the slot: no script tick runs.
const HOLD_HELD: u8 = 1;
/// The hold ended and no script tick has run since.
const HOLD_ENDED: u8 = 2;
/// The first observed tick after the hold. An in-flight server effect of the
/// guardian's last packet can still land in it, so an interrupted action
/// waits ([`AfterHold::Settling`]).
const HOLD_SETTLE: u8 = 3;
/// A later observed tick: the interrupted action re-checks its goal and
/// resumes once ([`AfterHold::Resume`]).
const HOLD_RESUME: u8 = 4;
const HOLD_PHASE_MASK: u8 = 0b111;
/// The player's position changed by a teleport while the hold lasted.
const HOLD_TELEPORTED: u8 = 0x80;

/// Where an action's own tick stands after a guardian hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterHold {
    /// No guardian hold is pending for the actions.
    None,
    /// The first observed tick after the hold: the guardian's last request may
    /// still be taking effect, so an interrupted action only waits.
    Settling,
    /// A later observed tick: an interrupted action re-checks its goal and
    /// re-issues its request at most once (consume with
    /// [`ActionContext::take_hold_resume`](super::ActionContext)).
    Resume,
}

/// Most tiles a player walks in one engine cycle: running advances two
/// (`PathingEntity.ts:135-149`).
pub const WALK_TILES_PER_TICK: u32 = 2;

/// Whether the player's observed position changed by a teleport instead of by
/// walking, between two observations `ticks` engine cycles apart. A plane
/// change is always one (stairs and ladders are the player's own click, and
/// the guardian climbs none). On a plane, a jump beyond what running covers in
/// the observed cycles is one, plus one cycle of slack: the position reaches
/// the client in the PLAYER_INFO that follows the tick marker (`World.ts:
/// 1108-1123`, `dialogue.rs` `DIALOGUE_APPROACH_SLACK_TICKS`).
///
/// Content that moves the player this way: the Maze and Mime events
/// (`macro_event_maze.rs2:15-18` `p_telejump`, `macro_event_mime.rs2:9-11`
/// `p_teleport`), the teleport of an ignored Genie or Mysterious Old Man
/// (`macro_events.rs2:120-137` `macro_event_fail_teleport`), and the return
/// to Lumbridge when the tile the player left is blocked or gone
/// (`macro_events.rs2:235-241`). A guardian walk away from a hostile event
/// covers a few tiles at running speed, which this rule never calls a
/// teleport.
pub fn is_teleport(from: api::WorldTile, to: api::WorldTile, ticks: u32) -> bool {
    if from.level != to.level {
        return true;
    }
    let distance = (from.x - to.x)
        .unsigned_abs()
        .max((from.z - to.z).unsigned_abs());
    distance > WALK_TILES_PER_TICK.saturating_mul(ticks.saturating_add(1))
}

const FIX_COORD_BITS: u32 = 14;
const FIX_COORD_MASK: u32 = (1 << FIX_COORD_BITS) - 1;
const FIX_VALID: u32 = 1 << 31;

/// A tile in 31 bits: x and z in 14 bits each, the plane in 2. A tile outside
/// that range packs to `None`.
fn pack_fix(tile: api::WorldTile) -> Option<u32> {
    let coord = |value: i32| u32::try_from(value).ok().filter(|v| *v <= FIX_COORD_MASK);
    let level = u32::try_from(tile.level).ok().filter(|level| *level <= 3)?;
    Some(
        FIX_VALID
            | coord(tile.x)?
            | (coord(tile.z)? << FIX_COORD_BITS)
            | (level << (2 * FIX_COORD_BITS)),
    )
}

fn unpack_fix(fix: u32) -> api::WorldTile {
    api::WorldTile {
        x: (fix & FIX_COORD_MASK) as i32,
        z: ((fix >> FIX_COORD_BITS) & FIX_COORD_MASK) as i32,
        level: ((fix >> (2 * FIX_COORD_BITS)) & 0b11) as i32,
    }
}

const SCENE_KEY_BITS: u32 = 25;
const SCENE_KEY_MASK: u32 = (1 << SCENE_KEY_BITS) - 1;
const SCENE_SEEN_SHIFT: u32 = SCENE_KEY_BITS;
const SCENE_SEEN_MASK: u32 = 0b11 << SCENE_SEEN_SHIFT;
const SCENE_ENTERED_BIT: u32 = 1 << 27;

/// What the last observed frame showed of the player's scene.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
#[repr(u8)]
enum Seen {
    /// The slot has observed no frame: a run that starts in a scene has not
    /// just entered it.
    #[default]
    Nothing = 0,
    /// Out of game, or the scene was not built.
    Outside = 1,
    /// In a particular observed scene key.
    Scene = 2,
}

/// The engine tracks the zones around the player per level inside the build
/// area (`BuildArea.ts:31-55`): a level change or a rebuilt build area (a
/// new origin) makes every zone around the player newly tracked, and
/// `NetworkPlayer.ts:294-315` then resets each one and resends its loc
/// changes. The scene key uses 25 bits: a known-origin bit, origin zone x and
/// z (11 bits each), and the level (2 bits). Its spare high bits hold the
/// shared scene-entry latch.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct SceneKey(u32);

impl SceneKey {
    fn observe(snapshot: Option<&api::snapshot::GameSnapshot>) -> Option<Self> {
        let snapshot = snapshot?;
        if !snapshot.ingame() || snapshot.scene_state() != 2 {
            return None;
        }
        Some(Self::from_parts(
            snapshot.local_player()?.player.network.level,
            snapshot.base(),
        ))
    }

    pub fn from_parts(level: i32, origin: Option<(i32, i32)>) -> Self {
        let level = (level as u32) & 0b11;
        let origin = origin.map_or(0, |(x, z)| {
            (1 << 22) | ((((x as u32) >> 3) & 0x7ff) << 11) | (((z as u32) >> 3) & 0x7ff)
        });
        Self((origin << 2) | level)
    }

    fn seen(self) -> Seen {
        match (self.0 & SCENE_SEEN_MASK) >> SCENE_SEEN_SHIFT {
            0 => Seen::Nothing,
            1 => Seen::Outside,
            2 => Seen::Scene,
            _ => unreachable!("SceneFence only stores the three Seen states"),
        }
    }

    fn scene(self) -> Self {
        Self(self.0 & SCENE_KEY_MASK)
    }
}

/// Shared native/compat latch packed into SceneKey's spare high bits, so
/// evidence wakes cannot clear scene entry without growing TickBudget.
#[derive(Default)]
pub struct SceneFence(SceneKey);

impl SceneFence {
    pub fn next_tick(&mut self) {
        self.0 .0 &= !SCENE_ENTERED_BIT;
    }

    pub fn observe(&mut self, scene: Option<SceneKey>) {
        let (seen, scene) = match scene {
            Some(scene) => (Seen::Scene, scene),
            None => (Seen::Outside, SceneKey::default()),
        };
        let previous_seen = self.0.seen();
        let previous_scene = self.0.scene();
        let mut entered = self.0 .0 & SCENE_ENTERED_BIT;
        if previous_seen != Seen::Nothing && (previous_seen, previous_scene) != (seen, scene) {
            entered = SCENE_ENTERED_BIT;
        }
        self.0 =
            SceneKey((scene.0 & SCENE_KEY_MASK) | ((seen as u32) << SCENE_SEEN_SHIFT) | entered);
    }

    pub fn entered_scene(&self) -> bool {
        self.0 .0 & SCENE_ENTERED_BIT != 0
    }
}

impl TickBudget {
    const MAX_TRANSITIONS: u8 = 32;

    pub fn observe(&mut self, tick: u64) {
        if self.tick != tick {
            self.tick = tick;
            self.transitions = 0;
            self.events = 0;
            self.scene.next_tick();
            // Each new observed tick moves a finished hold one step on: the
            // first is the settle tick, the next one resumes.
            match self.hold & HOLD_PHASE_MASK {
                HOLD_ENDED => self.set_hold_phase(HOLD_SETTLE),
                HOLD_SETTLE => self.set_hold_phase(HOLD_RESUME),
                _ => {}
            }
        }
    }

    fn set_hold_phase(&mut self, phase: u8) {
        self.hold = (self.hold & !HOLD_PHASE_MASK) | phase;
    }

    /// The guardian took the slot (the seat's rising edge).
    pub fn hold_begin(&mut self) {
        self.set_hold_phase(HOLD_HELD);
    }

    /// The guardian released the slot (the seat's falling edge).
    pub fn hold_end(&mut self) {
        if self.hold & HOLD_PHASE_MASK == HOLD_HELD {
            self.set_hold_phase(HOLD_ENDED);
        }
    }

    /// Forget a pending hold: a machine that begins now has not been
    /// interrupted by it.
    pub fn clear_hold(&mut self) {
        self.hold = HOLD_NONE;
    }

    pub fn after_hold(&self) -> AfterHold {
        match self.hold & HOLD_PHASE_MASK {
            HOLD_SETTLE => AfterHold::Settling,
            HOLD_RESUME => AfterHold::Resume,
            _ => AfterHold::None,
        }
    }

    /// Consume the one resume that follows a hold.
    pub fn take_hold_resume(&mut self) -> bool {
        let resume = self.hold & HOLD_PHASE_MASK == HOLD_RESUME;
        if resume {
            self.set_hold_phase(HOLD_NONE);
        }
        resume
    }

    /// Consume the teleport that the finished hold observed, once.
    pub fn take_hold_teleport(&mut self) -> bool {
        let finished = self.hold & HOLD_PHASE_MASK != HOLD_HELD;
        let teleported = self.hold & HOLD_TELEPORTED != 0;
        if finished && teleported {
            self.hold &= !HOLD_TELEPORTED;
            return true;
        }
        false
    }

    /// Whether a teleport is latched and not yet taken; reading it takes
    /// nothing.
    #[cfg(test)]
    pub(crate) fn hold_teleport_latched(&self) -> bool {
        self.hold & HOLD_TELEPORTED != 0
    }

    /// One host frame's player position. The frame before a hold, every frame
    /// of it and the first after it are compared pairwise: a teleport between
    /// two of them is remembered for [`Self::take_hold_teleport`]. The
    /// position is kept on every frame, held or not, and stays as it was on a
    /// frame with no usable position.
    pub fn observe_fix(&mut self, tick: u64, tile: Option<api::WorldTile>) {
        let Some(fix) = tile.and_then(pack_fix) else {
            return;
        };
        let tick = tick as u32;
        let watching = matches!(self.hold & HOLD_PHASE_MASK, HOLD_HELD | HOLD_ENDED);
        if self.fix != 0 && watching {
            let elapsed = tick.wrapping_sub(self.fix_tick);
            // A wrapped (negative) elapsed count is a restarted tick clock,
            // not time that passed.
            if elapsed < u32::MAX / 2 && is_teleport(unpack_fix(self.fix), unpack_fix(fix), elapsed)
            {
                self.hold |= HOLD_TELEPORTED;
            }
        }
        self.fix_tick = tick;
        self.fix = fix;
    }

    /// One host frame: replenish on a new tick, and mark the tick that first
    /// shows the player on a new level or in a rebuilt build area.
    pub fn observe_frame(&mut self, tick: u64, snapshot: Option<&api::snapshot::GameSnapshot>) {
        self.observe(tick);
        self.scene.observe(SceneKey::observe(snapshot));
    }

    /// This observed tick is the first to show the player on a new level or
    /// in a rebuilt build area. The engine writes the PLAYER_INFO that moves
    /// the player before the resets and loc changes of the zones it now
    /// tracks (`World.ts:1108-1114`, `Zone.ts:157-189`), and the client reads
    /// at most five packets a frame (`client.rs:12497-12501`), so this tick's
    /// locs can still be the old ones. The next observed tick's PLAYER_INFO
    /// arrives after all of them.
    pub fn entered_scene(&self) -> bool {
        self.scene.entered_scene()
    }

    pub(super) fn transition_available(&self) -> bool {
        self.transitions < Self::MAX_TRANSITIONS
    }

    pub fn transition(&mut self) -> bool {
        if self.transitions == Self::MAX_TRANSITIONS {
            return false;
        }
        self.transitions += 1;
        true
    }

    pub fn event(&mut self, disposal: bool) -> bool {
        if self.events >= if disposal { 5 } else { 1 } {
            return false;
        }
        self.events += 1;
        true
    }
    pub fn batch(&mut self) -> bool {
        if self.events != 0 {
            return false;
        }
        self.events = 5;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::QuietReadLease;

    #[test]
    fn walk_consent_capture_is_frozen_and_fenced_to_the_live_request_and_run() {
        let run = RunKey {
            slot: 1,
            run: 2,
            session: 3,
        };
        let mut ledger = Ledger::default();
        assert!(ledger.walk_prereq_allow.is_none());
        let owner = Owner::new(run, ledger.next_id().unwrap());
        ledger.owner = Some(Arc::clone(&owner));
        let request = ledger.next_id().unwrap();
        owner.set_walk(request);
        let current = HostAuthority {
            owner: Arc::clone(&owner),
            request,
            walk: true,
            protected_items: None,
        };
        let allowed = super::super::PrereqAllow {
            dialogue: true,
            quest_start: false,
        };
        assert!(ledger.capture_walk_prereq_allow(&current, allowed));
        assert!(ledger.capture_walk_prereq_allow(&current, allowed));
        assert!(!ledger.capture_walk_prereq_allow(&current, super::super::PrereqAllow::default(),));
        assert_eq!(ledger.walk_prereq_allow, Some((request, allowed)));

        owner.set_interaction(request);
        let interaction = HostAuthority {
            walk: false,
            ..current.clone()
        };
        assert!(interaction.live());
        assert!(!ledger.capture_walk_prereq_allow(&interaction, allowed));

        let next_request = ledger.next_id().unwrap();
        owner.set_walk(next_request);
        let next = HostAuthority {
            request: next_request,
            ..current.clone()
        };
        assert!(!ledger.capture_walk_prereq_allow(&current, allowed));
        let denied = super::super::PrereqAllow::default();
        assert!(ledger.capture_walk_prereq_allow(&next, denied));
        for foreign_run in [
            RunKey {
                slot: run.slot + 1,
                ..run
            },
            RunKey {
                run: run.run + 1,
                ..run
            },
            RunKey {
                session: run.session + 1,
                ..run
            },
        ] {
            let foreign_owner = Owner::new(foreign_run, owner.id);
            foreign_owner.set_walk(next_request);
            let foreign = HostAuthority {
                owner: foreign_owner,
                request: next_request,
                walk: true,
                protected_items: None,
            };
            assert!(foreign.live());
            assert!(!ledger.capture_walk_prereq_allow(&foreign, allowed));
            assert_eq!(ledger.walk_prereq_allow, Some((next_request, denied)));
        }

        let capture_bytes = std::mem::size_of_val(&ledger.walk_prereq_allow);
        println!(
            "native_walk_consent_layout ledger={} capture={capture_bytes}",
            std::mem::size_of::<Ledger>(),
        );
        assert!(capture_bytes <= 16);
        ledger.revoke();
        assert!(ledger.walk_prereq_allow.is_none());
        assert!(!ledger.capture_walk_prereq_allow(&next, allowed));
    }

    #[test]
    fn interaction_receipts_reject_old_requests_sessions_and_revoked_owners() {
        let run = RunKey {
            slot: 1,
            run: 2,
            session: 3,
        };
        let mut ledger = Ledger::default();
        let owner = Owner::new(run, ledger.next_id().unwrap());
        ledger.owner = Some(Arc::clone(&owner));
        let old = HostAuthority {
            owner: Arc::clone(&owner),
            request: ledger.next_id().unwrap(),
            walk: false,
            protected_items: None,
        };
        let current = HostAuthority {
            owner: Arc::clone(&owner),
            request: ledger.next_id().unwrap(),
            walk: false,
            protected_items: None,
        };
        ledger.interaction_request = Some(current.request);
        owner.set_interaction(current.request);
        let receipt = InteractionReceipt {
            request_id: current.request.get(),
            evidence: api::quest_progress::EvidenceStamp {
                run,
                tick: 4,
                sequence: 4,
            },
            accepted: true,
            chat_since: 0,
        };
        ledger.complete_interaction(
            &old,
            InteractionReceipt {
                request_id: old.request.get(),
                ..receipt
            },
        );
        assert!(ledger.interaction.is_none());
        ledger.complete_interaction(
            &current,
            InteractionReceipt {
                evidence: api::quest_progress::EvidenceStamp {
                    run: run.next_session().unwrap(),
                    ..receipt.evidence
                },
                ..receipt
            },
        );
        assert!(ledger.interaction.is_none());
        ledger.complete_interaction(&current, receipt);
        assert_eq!(ledger.interaction, Some(receipt));
        ledger.complete_interaction(
            &current,
            InteractionReceipt {
                accepted: false,
                ..receipt
            },
        );
        assert_eq!(
            ledger.interaction,
            Some(receipt),
            "a duplicate cannot replace settlement"
        );
        ledger.revoke();
        ledger.complete_interaction(&current, receipt);
        assert!(
            ledger.interaction.is_none(),
            "late receipt revived cancelled work"
        );
    }
    #[test]
    fn id_ranges_check_the_last_id_without_consuming_the_cursor() {
        let mut ledger = Ledger::default();
        ledger.next_id = u64::MAX - 1;
        let first = ledger.id_range(2).unwrap();
        assert_eq!(first.get(), u64::MAX - 1);
        assert_eq!(ledger.id_range(3), Err(ActionError::Cancelled));
        assert_eq!(ledger.id_range(0), Err(ActionError::Cancelled));
        assert_eq!(ledger.next_id, u64::MAX - 1);

        ledger.commit_id_range(first, 2);
        assert_eq!(ledger.next_id, 0, "the last nonzero id may be consumed");
        assert_eq!(ledger.id_range(1), Err(ActionError::Cancelled));
    }

    #[test]
    fn bank_pick_receipts_require_the_live_matching_owner_run_and_request() {
        let run = RunKey {
            slot: 1,
            run: 2,
            session: 3,
        };
        let mut ledger = Ledger::default();
        let owner = Owner::new(run, ledger.next_id().unwrap());
        ledger.owner = Some(Arc::clone(&owner));
        let old = HostAuthority {
            owner: Arc::clone(&owner),
            request: ledger.next_id().unwrap(),
            walk: false,
            protected_items: None,
        };
        let current = HostAuthority {
            owner: Arc::clone(&owner),
            request: ledger.next_id().unwrap(),
            walk: false,
            protected_items: None,
        };
        ledger.pick_request = Some(PendingPick {
            request: current.request,
            channel: PickChannel::Bank,
        });
        owner.set_interaction(current.request);
        let receipt = BankPickReceipt {
            request_id: current.request.get(),
            evidence: api::quest_progress::EvidenceStamp {
                run,
                tick: 4,
                sequence: 4,
            },
            outcome: crate::native_bank::BankPickOutcome::Selected(
                crate::native_bank::SelectedBank {
                    bank_index: 7,
                    access_tile: api::snapshot::WorldTile {
                        x: 100,
                        z: 200,
                        level: 0,
                    },
                    kind: crate::native_bank::PickKind::Reachable,
                    access: None,
                },
            ),
        };
        ledger.complete_bank_pick(
            &old,
            BankPickReceipt {
                request_id: old.request.get(),
                ..receipt.clone()
            },
        );
        assert!(ledger.pick.is_none());
        ledger.complete_bank_pick(
            &current,
            BankPickReceipt {
                evidence: api::quest_progress::EvidenceStamp {
                    run: run.next_session().unwrap(),
                    ..receipt.evidence
                },
                ..receipt.clone()
            },
        );
        assert!(ledger.pick.is_none());
        ledger.complete_bank_pick(&current, receipt.clone());
        assert_eq!(ledger.pick, Some(PickReceipt::Bank(receipt.clone())));
        ledger.complete_bank_pick(
            &current,
            BankPickReceipt {
                outcome: match &receipt.outcome {
                    crate::native_bank::BankPickOutcome::Selected(selected) => {
                        crate::native_bank::BankPickOutcome::Selected(
                            crate::native_bank::SelectedBank {
                                bank_index: 8,
                                ..selected.clone()
                            },
                        )
                    }
                    _ => unreachable!("the fixture receipt is a selected bank"),
                },
                ..receipt.clone()
            },
        );
        assert_eq!(
            ledger.pick,
            Some(PickReceipt::Bank(receipt.clone())),
            "a duplicate cannot replace the selected stand"
        );
        ledger.revoke();
        ledger.complete_bank_pick(&current, receipt);
        assert!(ledger.pick.is_none(), "a late pick revived cancelled work");
    }

    #[test]
    fn service_pick_receipts_require_matching_channel_owner_run_and_request() {
        use crate::native_bank::ServicePickOutcome;

        let run = RunKey {
            slot: 1,
            run: 2,
            session: 3,
        };
        let mut ledger = Ledger::default();
        let owner = Owner::new(run, ledger.next_id().unwrap());
        ledger.owner = Some(Arc::clone(&owner));
        let old = HostAuthority {
            owner: Arc::clone(&owner),
            request: ledger.next_id().unwrap(),
            walk: false,
        };
        let current = HostAuthority {
            owner: Arc::clone(&owner),
            request: ledger.next_id().unwrap(),
            walk: false,
        };
        ledger.pick_request = Some(PendingPick {
            request: current.request,
            channel: PickChannel::Service,
        });
        owner.set_interaction(current.request);
        let receipt = ServicePickReceipt {
            request_id: current.request.get(),
            evidence: api::quest_progress::EvidenceStamp {
                run,
                tick: 4,
                sequence: 4,
            },
            outcome: ServicePickOutcome::Routed {
                index: 7,
                access_tile: api::snapshot::WorldTile {
                    x: 100,
                    z: 200,
                    level: 0,
                },
                cost: 11,
            },
        };
        ledger.complete_bank_pick(
            &current,
            BankPickReceipt {
                request_id: receipt.request_id,
                evidence: receipt.evidence,
                outcome: crate::native_bank::BankPickOutcome::Selected(
                    crate::native_bank::SelectedBank {
                        bank_index: 7,
                        access_tile: api::snapshot::WorldTile {
                            x: 100,
                            z: 200,
                            level: 0,
                        },
                        kind: crate::native_bank::PickKind::Reachable,
                        access: None,
                    },
                ),
            },
        );
        assert!(
            ledger.pick.is_none(),
            "a bank receipt cannot answer a service request"
        );
        ledger.complete_service_pick(
            &old,
            ServicePickReceipt {
                request_id: old.request.get(),
                ..receipt.clone()
            },
        );
        assert!(ledger.pick.is_none(), "the replaced request cannot answer");
        ledger.complete_service_pick(
            &current,
            ServicePickReceipt {
                evidence: api::quest_progress::EvidenceStamp {
                    run: run.next_session().unwrap(),
                    ..receipt.evidence
                },
                ..receipt.clone()
            },
        );
        assert!(ledger.pick.is_none(), "the old session cannot answer");
        ledger.complete_service_pick(&current, receipt.clone());
        ledger.complete_service_pick(
            &current,
            ServicePickReceipt {
                outcome: ServicePickOutcome::NotSettled,
                ..receipt.clone()
            },
        );
        assert_eq!(ledger.pick, Some(PickReceipt::Service(receipt.clone())));
        ledger.revoke();
        ledger.complete_service_pick(&current, receipt);
        assert!(
            ledger.pick.is_none(),
            "a late receipt cannot revive cancelled work"
        );
    }

    #[test]
    fn expired_guard_cannot_clear_a_replacement_for_the_same_request() {
        let run = RunKey {
            slot: 1,
            run: 2,
            session: 3,
        };
        let mut ledger = Ledger::default();
        let owner = Owner::new(run, ledger.next_id().unwrap());
        ledger.owner = Some(Arc::clone(&owner));
        let request = owner.id;
        let first = ledger.next_id().unwrap();
        let now = Instant::now();
        assert!(owner.acquire_quiet(first));
        ledger.quiet_lease = Some(QuietRead {
            request,
            lease: first,
            since: now,
            hold_until: now + QUIET_READ_WALL_BOUND,
        });
        let old_guard = QuietReadLease {
            owner: Arc::clone(&owner),
            lease_id: first,
        };
        assert_eq!(ledger.quiet_read(now).unwrap().request_id, request);
        // No active-clock advance or machine poll is required for safety expiry.
        assert!(ledger.quiet_read(now + Duration::from_secs(10)).is_none());
        let replacement = ledger.next_id().unwrap();
        assert!(owner.acquire_quiet(replacement));
        let replaced = now + Duration::from_secs(10);
        ledger.quiet_lease = Some(QuietRead {
            request,
            lease: replacement,
            since: replaced,
            hold_until: replaced + QUIET_READ_WALL_BOUND,
        });
        drop(old_guard);
        assert_eq!(
            ledger
                .quiet_read(now + Duration::from_secs(11))
                .unwrap()
                .request_id,
            request
        );
        ledger.revoke();
        assert!(ledger.quiet_read(now + Duration::from_secs(11)).is_none());
    }

    #[test]
    fn same_tick_repoll_does_not_replenish_shared_budget() {
        let mut budget = TickBudget::default();
        budget.observe(7);
        assert!(budget.event(false));
        for _ in 0..32 {
            budget.observe(7);
            assert!(!budget.event(false));
            assert!(budget.transition());
        }
        assert!(!budget.transition());
        for _ in 0..4 {
            assert!(budget.event(true));
        }
        assert!(!budget.event(true));
        budget.observe(8);
        assert!(budget.event(false));
        assert!(budget.transition());
    }
    #[test]
    fn batch_saturates_shared_events_until_the_next_observed_tick() {
        let mut budget = TickBudget::default();
        budget.observe(7);
        assert!(budget.batch());
        assert_eq!(budget.events, 5);
        assert!(!budget.batch());
        assert!(!budget.event(false));
        assert!(!budget.event(true));

        budget.observe(7);
        assert_eq!(
            budget.events, 5,
            "same-tick re-observation cannot refill it"
        );
        assert!(!budget.batch());
        budget.observe(8);
        assert!(budget.batch(), "the next observed tick starts uncharged");
        assert_eq!(budget.events, 5);
    }
    #[test]
    fn a_finished_guardian_hold_settles_for_one_observed_tick_then_resumes_once() {
        let mut budget = TickBudget::default();
        budget.observe(5);
        assert_eq!(budget.after_hold(), AfterHold::None);
        budget.hold_begin();
        // Ticks that pass while the guardian holds move nothing on.
        budget.observe(6);
        budget.observe(7);
        assert_eq!(budget.after_hold(), AfterHold::None);
        budget.hold_end();
        assert_eq!(budget.after_hold(), AfterHold::None, "no tick has run yet");
        budget.observe(8);
        assert_eq!(budget.after_hold(), AfterHold::Settling);
        budget.observe(8);
        assert_eq!(
            budget.after_hold(),
            AfterHold::Settling,
            "a second poll of the same tick does not move it"
        );
        assert!(!budget.take_hold_resume());
        budget.observe(9);
        assert_eq!(budget.after_hold(), AfterHold::Resume);
        assert!(budget.take_hold_resume());
        assert!(!budget.take_hold_resume(), "the resume is consumed once");
        assert_eq!(budget.after_hold(), AfterHold::None);
    }

    #[test]
    fn a_new_hold_restarts_the_settle_and_an_action_begun_after_one_is_not_resumed() {
        let mut budget = TickBudget::default();
        budget.observe(1);
        budget.hold_begin();
        budget.hold_end();
        budget.observe(2);
        budget.hold_begin();
        budget.observe(3);
        assert_eq!(budget.after_hold(), AfterHold::None, "held again");
        budget.hold_end();
        budget.observe(4);
        assert_eq!(budget.after_hold(), AfterHold::Settling);
        budget.clear_hold();
        budget.observe(5);
        assert_eq!(budget.after_hold(), AfterHold::None);
    }

    #[test]
    fn the_teleport_rule_separates_a_walk_from_a_jump() {
        let at = |x, z, level| api::WorldTile { x, z, level };
        // Running covers two tiles a cycle; one cycle of slack on the count.
        assert!(!is_teleport(at(3200, 3200, 0), at(3204, 3200, 0), 1));
        assert!(is_teleport(at(3200, 3200, 0), at(3205, 3200, 0), 1));
        assert!(!is_teleport(at(3200, 3200, 0), at(3220, 3200, 0), 9));
        assert!(is_teleport(at(3200, 3200, 0), at(3231, 3200, 0), 9));
        // A plane change is a teleport at any distance.
        assert!(is_teleport(at(3200, 3200, 0), at(3200, 3200, 1), 50));
        // The Maze and Mime squares are far from anywhere a player is sent from.
        assert!(is_teleport(at(3200, 3200, 0), at(2900, 4555, 0), 30));
    }

    #[test]
    fn only_frames_of_a_hold_can_report_a_teleport() {
        let at = |x, z| Some(api::WorldTile { x, z, level: 0 });
        let mut budget = TickBudget::default();
        // A teleport the player makes on their own (a spell, a lodestone) with
        // no guardian hold is not reported.
        budget.observe_fix(1, at(3200, 3200));
        budget.observe_fix(2, at(3000, 3500));
        assert!(!budget.take_hold_teleport());
        budget.hold_begin();
        budget.observe_fix(3, at(3002, 3500));
        budget.observe_fix(4, at(3004, 3500));
        budget.hold_end();
        budget.observe_fix(5, at(3005, 3500));
        assert!(!budget.take_hold_teleport(), "a few tiles of walking");
        budget.hold_begin();
        budget.observe_fix(6, at(2900, 4555));
        assert!(!budget.take_hold_teleport(), "not before the hold is over");
        budget.observe_fix(7, at(3005, 3500));
        budget.hold_end();
        assert!(budget.take_hold_teleport(), "out and back is a teleport");
        assert!(!budget.take_hold_teleport(), "reported once");
        // A frame with no player keeps the last position.
        budget.hold_begin();
        budget.observe_fix(8, None);
        budget.observe_fix(9, at(3500, 3500));
        budget.hold_end();
        assert!(budget.take_hold_teleport());
    }

    /// Two frames with the same observed tick number are zero ticks apart,
    /// not an invalid pair: a jump out and back inside one tick is latched,
    /// the return frame does not clear it, and it is taken once after the
    /// hold. The start, the jump and the return all carry tick 11, so only
    /// the zero-elapsed comparison can see the jump.
    #[test]
    fn a_jump_out_and_back_inside_one_observed_tick_is_latched_and_taken_once() {
        let at = |x, z| Some(api::WorldTile { x, z, level: 0 });
        let mut budget = TickBudget::default();
        budget.observe_fix(11, at(3209, 3215));
        budget.hold_begin();
        budget.observe_fix(11, at(2893, 4555));
        budget.observe_fix(11, at(3209, 3215));
        assert!(budget.hold_teleport_latched(), "latched inside the tick");
        assert!(!budget.take_hold_teleport(), "not before the hold is over");
        budget.observe_fix(12, at(3209, 3215));
        budget.hold_end();
        budget.observe_fix(12, at(3209, 3215));
        assert!(
            budget.hold_teleport_latched(),
            "the return did not clear it"
        );
        assert!(budget.take_hold_teleport());
        assert!(!budget.take_hold_teleport(), "taken once");
        assert!(!budget.hold_teleport_latched());
        // Inside one tick, two tiles of running is still a walk.
        budget.hold_begin();
        budget.observe_fix(13, at(3211, 3215));
        budget.observe_fix(13, at(3209, 3215));
        budget.hold_end();
        assert!(!budget.take_hold_teleport());
    }

    #[test]
    fn scene_fence_uses_four_bytes_and_tick_budget_stays_compact() {
        assert_eq!(std::mem::size_of::<SceneFence>(), 4);
        assert_eq!(std::mem::size_of::<TickBudget>(), 24);
    }

    #[test]
    fn service_pick_shares_the_bank_slot_with_bounded_control_metadata() {
        use std::mem::size_of;
        assert_eq!(size_of::<PendingPick>(), 16);
        assert_eq!(size_of::<Option<PendingPick>>(), 16);
        let largest = size_of::<BankPickReceipt>().max(size_of::<ServicePickReceipt>());
        assert!(size_of::<Option<PickReceipt>>() <= largest + 8);
        println!(
            "pick owner bytes: ledger={} pending={} bank_receipt={} service_receipt={} shared_receipt={}",
            size_of::<Ledger>(),
            size_of::<Option<PendingPick>>(),
            size_of::<BankPickReceipt>(),
            size_of::<ServicePickReceipt>(),
            size_of::<Option<PickReceipt>>(),
        );
    }

    fn scene_frame(level: i32, x: i32, z: i32, origin: (i32, i32)) -> api::snapshot::GameSnapshot {
        let mut snapshot = api::snapshot::GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_world(api::snapshot::WorldStateView {
            map_base_x: origin.0,
            map_base_z: origin.1,
            ..Default::default()
        });
        snapshot.seed_local_player(crate::quester::families::tests::local_player(
            api::WorldTile { x, z, level },
        ));
        snapshot
    }

    #[test]
    fn only_a_level_change_or_rebuilt_build_area_enters_a_scene_for_its_tick() {
        let origin = (3112, 3256);
        let mut budget = TickBudget::default();
        budget.observe_frame(1, Some(&scene_frame(1, 3165, 3307, origin)));
        assert!(
            !budget.entered_scene(),
            "a run that starts in a scene has not just entered it"
        );
        budget.observe_frame(2, Some(&scene_frame(1, 3165, 3307, origin)));
        assert!(!budget.entered_scene());
        // Walking inside the build area keeps the tracked zones.
        budget.observe_frame(3, Some(&scene_frame(1, 3175, 3311, origin)));
        assert!(!budget.entered_scene());

        // Down the ladder: the new level's loc state follows PLAYER_INFO.
        budget.observe_frame(4, Some(&scene_frame(0, 3175, 3311, origin)));
        assert!(budget.entered_scene());
        budget.observe_frame(4, Some(&scene_frame(0, 3175, 3311, origin)));
        assert!(
            budget.entered_scene(),
            "a same-tick re-observation keeps the entry"
        );
        budget.observe_frame(5, Some(&scene_frame(0, 3175, 3311, origin)));
        assert!(!budget.entered_scene(), "the next tick holds the new locs");

        // A rebuilt build area resets every tracked zone.
        budget.observe_frame(6, Some(&scene_frame(0, 3184, 3311, (3120, 3256))));
        assert!(budget.entered_scene());
        budget.observe_frame(7, None);
        assert!(budget.entered_scene(), "leaving the scene is a change");
        budget.observe_frame(8, None);
        assert!(!budget.entered_scene());
        // A relog's first in-game frame enters its scene.
        budget.observe_frame(9, Some(&scene_frame(0, 3184, 3311, (3120, 3256))));
        assert!(budget.entered_scene());
    }
}

#[cfg(test)]
mod observed_walk_outcome_tests {
    use super::super::{WalkCancelReason, WALK_OUTCOME_SEQ_MAX};
    use super::ObservedWalkOutcome;

    #[test]
    fn packed_outcome_keeps_sequence_and_reason() {
        assert_eq!(ObservedWalkOutcome::default().seq(), 0);
        assert_eq!(
            ObservedWalkOutcome::default().cancel_reason(),
            WalkCancelReason::None
        );
        for seq in [1, 41, WALK_OUTCOME_SEQ_MAX - 1, WALK_OUTCOME_SEQ_MAX] {
            for reason in [WalkCancelReason::None, WalkCancelReason::UserInput] {
                let outcome = ObservedWalkOutcome::new(seq, reason);
                assert_eq!((outcome.seq(), outcome.cancel_reason()), (seq, reason));
            }
        }
    }

    /// A sequence above the host range breaks the host contract: debug
    /// builds stop at the producer boundary.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "walk outcome sequence above WALK_OUTCOME_SEQ_MAX")]
    fn sequence_above_the_host_range_panics_in_debug() {
        let _ = ObservedWalkOutcome::new(WALK_OUTCOME_SEQ_MAX + 1, WalkCancelReason::None);
    }

    /// Release builds drop bit 63: the sequence never sets the reason.
    #[cfg(not(debug_assertions))]
    #[test]
    fn sequence_above_the_host_range_never_sets_the_reason_in_release() {
        let outcome = ObservedWalkOutcome::new(u64::MAX, WalkCancelReason::None);
        assert_eq!(
            (outcome.seq(), outcome.cancel_reason()),
            (WALK_OUTCOME_SEQ_MAX, WalkCancelReason::None)
        );
    }
}
