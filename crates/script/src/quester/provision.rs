//! Inventory-first preparation and selective capacity deposits for one Path.
//!
//! Needs are planned from the frame's [`Stock`]: the pack, the worn table and
//! the account's bank memory (design-bank-snapshot §4 N2). An `Unknown` bank
//! or a `Hint`-predicted shortage costs one scan trip that observes the whole
//! bank; only a `Session`-known shortage blocks in place (§2.4).
//!
//! A bank session is the run of bank trips this provisioner starts. It ends
//! when provisioning returns to the Path (`Ready`), parks (`Blocked`) or starts
//! a recipe. Then the provisioner closes the bank if its own trips opened it
//! and it is still open, because a Path step cannot act under the bank
//! interface. It never closes a bank that was already open when its first trip
//! began (the player's, or an earlier step's): that session is not its own.
//!
//! Ownership is a [`BankLease`]. It is recorded only when a frame shows the
//! bank open after a trip of this provisioner began with the bank shut. It
//! names the login session and the bank session generation it covers, so a
//! relog, or a bank that was closed and opened again by someone else, makes it
//! void. A trip that ends by error or cancel keeps an observed lease: the
//! bank is still open, and the queue runner closes it through [`BankCloser`]
//! when input is allowed again. An operator Stop sends nothing and discards
//! the lease with the run. A Path step that needs the bank opens it again
//! through the bank kernel.
use super::compile::{
    CompiledAcquireRecipe, CompiledItemKind, CompiledProvisioning, StepContext, StepPlan, StepRun,
};
use super::families::AcquirePlan;
use crate::acquire::plan::{NeedObservation, Planner, UnmetCause, UnmetNeed};
use crate::acquire::run::{bank_open, AcquireRun, MAX_CLOSE_ATTEMPTS};
use crate::acquire::{
    AcquireOutcome, AcquireRequest, NeedKind, OpenGate, Presence, SourceSet, WorldFacts,
};
use crate::bank::{self, ops};
use crate::native::{ActionContext, ActionError, ActionHandle, NativeActions, NativeTick};
use crate::native_bank::{BankAction, BankReceipt};
use api::bank_memory::Origin;
use api::selected::{FactKey, Truth};
use api::snapshot::ItemView;
use api::stock::Stock;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

/// Whether the frame's bank memory is known (`Hint` or `Session`).
fn bank_known(cx: &StepContext<'_, '_>) -> bool {
    cx.tick.cx.snapshot().stock().banked_origin() != Origin::Unknown
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvisionPhase {
    Idle,
    Scanning,
    Spillover,
    Withdrawing,
    Closing,
    Acquiring,
    Ready,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Status {
    phase: ProvisionPhase,
    item: Option<Arc<str>>,
    need: i32,
    pack: i32,
    bank: Option<i32>,
    bank_known: bool,
    attempts: u32,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            phase: ProvisionPhase::Idle,
            item: None,
            need: 0,
            pack: 0,
            bank: None,
            bank_known: false,
            attempts: 0,
        }
    }
}

pub struct ProvisionStatus<'a> {
    pub phase: ProvisionPhase,
    pub item: Option<&'a str>,
    pub need: i32,
    pub pack: i32,
    pub bank: Option<i32>,
    pub bank_known: bool,
    pub attempts: u32,
}

#[derive(Debug)]
pub enum ProvisionEvent {
    Ready,
    BankReceipt(BankReceipt),
    Acquired,
    Blocked { item: Arc<str> },
}

/// The bank this Quester's own trips opened. It names the login session and
/// the bank session generation of the frame that showed the bank open. Any
/// other session or generation is somebody else's bank.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BankLease {
    session: u64,
    generation: u64,
}

impl BankLease {
    pub(super) fn new(session: u64, generation: u64) -> Self {
        Self {
            session,
            generation,
        }
    }
}

/// What the current frame says about a lease.
enum LeaseView {
    /// No lease, or the frame voided it (relog, bank shut, another session).
    Absent,
    /// The frame has no bank observation, so the lease is neither proved nor voided.
    Unobserved,
    /// The bank is open in the leased session.
    Owned,
}

/// Reads the lease against the frame and drops it when the frame voids it.
fn lease_view(cx: &ActionContext<'_>, lease: &mut Option<BankLease>) -> LeaseView {
    let Some(held) = *lease else {
        return LeaseView::Absent;
    };
    if held.session != cx.evidence().run.session {
        *lease = None;
        return LeaseView::Absent;
    }
    match cx.snapshot().bank_session() {
        None => LeaseView::Unobserved,
        Some(view) if view.value.open && view.value.generation == held.generation => {
            LeaseView::Owned
        }
        Some(_) => {
            *lease = None;
            LeaseView::Absent
        }
    }
}

/// How one drive of a [`BankCloser`] ended.
pub(super) enum Closed {
    /// No bank is owed a close (none leased, voided, or no frame to judge by).
    Done,
    Pending,
    /// The close is given up and the lease dropped, so the failure is final.
    Failed(ActionError),
}

/// Closes the bank a [`BankLease`] covers, through the bank kernel's own
/// close. Its wait counts observed game ticks, with an active-time floor. A
/// close that times out is begun again, up to `MAX_CLOSE_ATTEMPTS`. It sends
/// nothing without a lease the frame still proves.
#[derive(Default)]
pub(super) struct BankCloser {
    handle: Option<Box<ActionHandle<bank::Close>>>,
    attempts: u8,
}

impl BankCloser {
    pub(super) fn active(&self) -> bool {
        self.handle.is_some()
    }

    /// Drops the close in flight. Dropping revokes it before the host can dispatch it.
    pub(super) fn cancel(&mut self) {
        self.handle = None;
        self.attempts = 0;
    }

    /// Begins the close when none is in flight and the frame still proves the
    /// lease, then polls it, so the click goes out on the tick it begins.
    pub(super) fn drive(
        &mut self,
        tick: &mut NativeTick<'_>,
        lease: &mut Option<BankLease>,
    ) -> Closed {
        if self.handle.is_none() {
            match lease_view(&tick.cx, lease) {
                LeaseView::Absent => {
                    self.attempts = 0;
                    return Closed::Done;
                }
                LeaseView::Unobserved => return Closed::Done,
                LeaseView::Owned => {}
            }
            if !tick.cx.may_continue_this_tick() {
                return Closed::Pending;
            }
            if self.attempts >= MAX_CLOSE_ATTEMPTS {
                self.attempts = 0;
                *lease = None;
                return Closed::Failed(ActionError::Failed(Arc::from(format!(
                    "bank-close-failed: bank did not close after {MAX_CLOSE_ATTEMPTS} attempts"
                ))));
            }
            match tick.actions.begin::<bank::Close>((), &mut tick.cx) {
                Ok(handle) => {
                    self.attempts += 1;
                    self.handle = Some(Box::new(handle));
                }
                Err(
                    ActionError::Busy
                    | ActionError::Held
                    | ActionError::Stale
                    | ActionError::Cancelled
                    | ActionError::BudgetExhausted,
                ) => return Closed::Pending,
                Err(error) => {
                    self.attempts = 0;
                    *lease = None;
                    return Closed::Failed(error);
                }
            }
        }
        let handle = self.handle.take().expect("a close is in flight");
        match tick.actions.poll(&handle, &mut tick.cx) {
            Poll::Pending => {
                self.handle = Some(handle);
                Closed::Pending
            }
            Poll::Ready(Ok(())) => {
                *lease = None;
                self.attempts = 0;
                Closed::Done
            }
            // The bank did not shut in time, its session was replaced, or the
            // action owner refused: the next drive judges the frame again.
            Poll::Ready(Err(
                ActionError::Busy
                | ActionError::Held
                | ActionError::Stale
                | ActionError::Cancelled
                | ActionError::BudgetExhausted,
            )) => Closed::Pending,
            Poll::Ready(Err(ActionError::Failed(message)))
                if message.as_ref() == "bank did not close"
                    || message.starts_with("bank session replaced") =>
            {
                Closed::Pending
            }
            Poll::Ready(Err(error)) => {
                *lease = None;
                self.attempts = 0;
                Closed::Failed(error)
            }
        }
    }
}

pub struct Provisioner {
    path: Option<api::selected::FactKey>,
    bank_run: Option<AcquireRun>,
    acquire_run: Option<Box<dyn StepRun>>,
    acquire_finished: bool,
    coin_drawn: bool,
    carry_drawn: u64,
    /// `usize::MAX` is outside every compiled stage slice.
    survival_stage: usize,
    gear_stage: usize,
    gear_run: Option<Box<dyn StepRun>>,
    attempts: u32,
    status: Status,
    revision: u64,
    /// Bank trips started so far (wrapping), so the runner can trace a trip
    /// that began and ended within one poll.
    bank_starts: u32,
    /// The provisioner has started bank trips and has not reached the end of
    /// that session, the close included. A lease outside a session is owed to
    /// the queue runner, which closes it.
    in_session: bool,
    /// A trip began with the bank shut and no frame has shown it open yet.
    opening: bool,
    /// The bank this provisioner's trips opened, once a frame showed it open.
    lease: Option<BankLease>,
    closer: BankCloser,
    /// Optional needs already logged as unmet since the last path reset.
    unmet_logged: Vec<UnmetNeed>,
}

const _: () = assert!(std::mem::size_of::<Provisioner>() <= 912);

impl Default for Provisioner {
    fn default() -> Self {
        Self::new()
    }
}

impl Provisioner {
    pub fn new() -> Self {
        Self {
            path: None,
            bank_run: None,
            acquire_run: None,
            acquire_finished: false,
            coin_drawn: false,
            carry_drawn: 0,
            survival_stage: usize::MAX,
            gear_stage: usize::MAX,
            gear_run: None,
            attempts: 0,
            status: Status::default(),
            revision: 0,
            bank_starts: 0,
            in_session: false,
            opening: false,
            lease: None,
            closer: BankCloser::default(),
            unmet_logged: Vec::new(),
        }
    }

    pub fn poll(
        &mut self,
        cx: &mut StepContext<'_, '_>,
        plan: &CompiledProvisioning,
        active_loadout: Option<&str>,
        current_stage: Option<&FactKey>,
    ) -> Poll<Result<ProvisionEvent, ActionError>> {
        let result = self.poll_inner(cx, plan, active_loadout, current_stage);
        // Read after the poll, so a bank that this poll's own kernel opened is
        // recorded on the tick it appears, even when the poll ends the session.
        self.observe_bank(cx.tick);
        // An error, or a return to the Path, ends the session. The lease stays
        // with this provisioner for the queue runner to close.
        if matches!(
            result,
            Poll::Ready(Err(_) | Ok(ProvisionEvent::Ready | ProvisionEvent::Blocked { .. }))
        ) {
            self.in_session = false;
            self.opening = false;
        }
        result
    }

    fn poll_inner(
        &mut self,
        cx: &mut StepContext<'_, '_>,
        plan: &CompiledProvisioning,
        active_loadout: Option<&str>,
        current_stage: Option<&FactKey>,
    ) -> Poll<Result<ProvisionEvent, ActionError>> {
        if self.acquire_finished {
            self.acquire_run = None;
            self.acquire_finished = false;
        }
        if self.path.as_ref().is_some_and(|path| path != &plan.path) {
            self.reset(cx.tick.actions);
        }
        if self.path.is_none() {
            self.path = Some(plan.path.clone());
            self.set_coin_drawn(plan.coin_float <= 0);
        }

        if self.closer.active() {
            match self.closer.drive(cx.tick, &mut self.lease) {
                Closed::Pending => return Poll::Pending,
                Closed::Failed(error) => return Poll::Ready(Err(error)),
                Closed::Done => {}
            }
        }
        if let Some(run) = self.gear_run.as_mut() {
            match run.poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(result) => {
                    self.gear_run = None;
                    result?;
                    self.gear_stage = stage_index(plan, current_stage).unwrap_or(usize::MAX);
                }
            }
        }
        if self.bank_run.is_some() {
            return self.poll_bank(cx);
        }
        if self.acquire_run.is_some() {
            return self.poll_acquire(cx);
        }
        if plan.owns_inventory {
            if let Some(ended) = self.end_session(cx) {
                return ended;
            }
            self.set_status(ProvisionPhase::Ready, None, 0, 0, None, bank_known(cx));
            return Poll::Ready(Ok(ProvisionEvent::Ready));
        }

        let prepared = self.poll_prepare(cx, plan, active_loadout, current_stage);
        // A bank trip or recipe started by this plan acts on this tick when
        // starting it spent nothing (TICK-FIX #4/#7, E-Q7/E-Q8a).
        if prepared.is_pending() && cx.tick.cx.may_continue_this_tick() {
            if self.bank_run.is_some() {
                return self.poll_bank(cx);
            }
            if self.closer.active() {
                return match self.closer.drive(cx.tick, &mut self.lease) {
                    Closed::Failed(error) => {
                        self.in_session = false;
                        Poll::Ready(Err(error))
                    }
                    _ => Poll::Pending,
                };
            }
            if self.acquire_run.is_some() {
                return self.poll_acquire(cx);
            }
        }
        prepared
    }

    /// Stops this provisioner's runs. It sends nothing. A lease that a frame
    /// showed open stays: that bank is still open and still ours. A trip that
    /// was cancelled before any frame showed its bank open owns nothing.
    pub fn cancel(&mut self) {
        self.bank_run = None;
        self.acquire_run = None;
        self.gear_run = None;
        self.closer.cancel();
        self.in_session = false;
        self.opening = false;
        self.acquire_finished = false;
        self.set_status(
            ProvisionPhase::Idle,
            None,
            0,
            0,
            None,
            self.status.bank_known,
        );
    }

    pub fn reset(&mut self, actions: &mut NativeActions) {
        if let Some(mut run) = self.bank_run.take() {
            run.cancel(actions);
        }
        if let Some(mut run) = self.acquire_run.take() {
            run.cancel(actions);
        }
        if let Some(mut run) = self.gear_run.take() {
            run.cancel(actions);
        }
        self.gear_stage = usize::MAX;
        self.survival_stage = usize::MAX;
        self.closer.cancel();
        self.in_session = false;
        self.opening = false;
        self.acquire_finished = false;
        self.path = None;
        self.set_coin_drawn(false);
        self.set_carry_drawn(0);
        self.attempts = 0;
        self.unmet_logged.clear();
        self.set_status(ProvisionPhase::Idle, None, 0, 0, None, false);
    }

    pub fn status(&self) -> ProvisionStatus<'_> {
        ProvisionStatus {
            phase: self.status.phase,
            item: self.status.item.as_deref(),
            need: self.status.need,
            pack: self.status.pack,
            bank: self.status.bank,
            bank_known: self.status.bank_known,
            attempts: self.status.attempts,
        }
    }

    pub(super) fn bank_phase(&self) -> Option<ProvisionPhase> {
        (self.bank_run.is_some() || self.gear_run.is_some() || self.closer.active())
            .then_some(self.status.phase)
    }

    pub(super) fn bank_starts(&self) -> u32 {
        self.bank_starts
    }

    /// This provisioner's trips are running, or its close of their bank is.
    pub(super) fn session_open(&self) -> bool {
        self.in_session
    }

    /// The bank this provisioner's trips opened, while a frame last showed it open.
    pub(super) fn bank_lease(&self) -> Option<BankLease> {
        self.lease
    }

    /// Hands the lease of an ended session to the queue runner, which closes
    /// it. `None` while a session is running: the provisioner closes that one.
    pub(super) fn take_unowned_lease(&mut self) -> Option<BankLease> {
        if self.in_session {
            None
        } else {
            self.lease.take()
        }
    }

    /// Test fixture for a session that left its bank open before the test begins.
    #[cfg(test)]
    pub(super) fn set_lease_for_test(&mut self, lease: BankLease) {
        self.lease = Some(lease);
    }

    pub(super) fn take_trace_event(&mut self) -> Option<super::compile::StepTraceEvent> {
        self.acquire_run.as_mut()?.take_trace_event()
    }

    /// Prayers the acquisition run raised, so a survival cancel can still
    /// clear them after it drops the run.
    pub(super) fn prayer_cleanup(&self) -> crate::combat::RaisedPrayers {
        self.acquire_run
            .as_ref()
            .map_or_else(crate::combat::RaisedPrayers::empty, |run| {
                run.prayer_cleanup()
            })
    }

    pub fn status_revision(&self) -> u64 {
        self.revision
    }

    pub fn coin_drawn(&self) -> bool {
        self.coin_drawn
    }

    pub fn carry_drawn(&self) -> u64 {
        self.carry_drawn
    }

    /// A bank or acquisition run is live and may own an open dialogue.
    pub(super) fn run_live(&self) -> bool {
        self.bank_run.is_some()
            || self.closer.active()
            || self.gear_run.is_some()
            || (self.acquire_run.is_some() && !self.acquire_finished)
    }
    pub(super) fn has_run(&self) -> bool {
        self.bank_run.is_some() || self.acquire_run.is_some() || self.gear_run.is_some()
    }

    pub(super) fn transfer_in_flight(&self) -> bool {
        self.bank_run
            .as_ref()
            .is_some_and(AcquireRun::transfer_in_flight)
            || self
                .acquire_run
                .as_ref()
                .is_some_and(|run| run.transfer_in_flight())
            || self
                .gear_run
                .as_ref()
                .is_some_and(|run| run.transfer_in_flight())
    }

    pub(super) fn interrupt_transfer(&mut self, actions: &mut NativeActions) {
        if let Some(run) = &mut self.bank_run {
            run.interrupt_transfer(actions);
        } else if let Some(run) = &mut self.acquire_run {
            run.interrupt_transfer(actions);
        } else if let Some(run) = &mut self.gear_run {
            run.interrupt_transfer(actions);
        }
    }

    pub fn needs_progress_read(&self) -> bool {
        self.acquire_run
            .as_ref()
            .is_some_and(|run| run.needs_progress_read())
    }

    pub fn progress_read_completed(&mut self, now: Duration) {
        if let Some(run) = self.acquire_run.as_mut() {
            run.progress_read_completed(now);
        }
    }

    fn poll_prepare(
        &mut self,
        cx: &mut StepContext<'_, '_>,
        plan: &CompiledProvisioning,
        active_loadout: Option<&str>,
        current_stage: Option<&FactKey>,
    ) -> Poll<Result<ProvisionEvent, ActionError>> {
        let snapshot = cx.tick.cx.snapshot();
        let stock = snapshot.stock();
        let known = stock.banked_origin() != Origin::Unknown;
        let Some(inventory) = snapshot.inventory() else {
            self.set_status(ProvisionPhase::Scanning, None, 0, 0, None, known);
            return Poll::Pending;
        };
        let Some(capacity) = snapshot.inventory_capacity() else {
            self.set_status(ProvisionPhase::Scanning, None, 0, 0, None, known);
            return Poll::Pending;
        };
        let inventory = inventory.value;
        let capacity = i32::from(capacity.value);
        let mut needs = Planner::new(&stock, WorldFacts::default());
        let current = stage_index(plan, current_stage);
        let survival = if let (Some(survival), Some(stage)) = (&plan.survival, current) {
            if self.survival_stage != stage && survival.stage_has_risk(stage) {
                let Some(frame) = crate::combat::frame::Frame::borrow(snapshot) else {
                    return Poll::Pending;
                };
                let poison = cx.tick.cx.condition().map_or(
                    api::condition::PoisonState::Unknown { since: frame.tick },
                    |condition| condition.poison,
                );
                let input = crate::combat::risk::RiskInput::capture(
                    &frame,
                    survival.tables(),
                    &crate::combat::ThreatSet::default(),
                    poison,
                    false,
                );
                let target = survival.plan(stage, stock, input).map_err(|why| {
                    ActionError::Unavailable(Arc::from(format!(
                        "stage survival bound refused: {why:?}"
                    )))
                })?;
                if self.gear_stage != stage {
                    if target.gear.iter().any(|item| {
                        stock.carried(item.id).unwrap_or(0) == 0 && stock.banked(item.id).is_none()
                    }) {
                        self.start_bank(
                            cx,
                            plan,
                            BankAction::Scan,
                            ProvisionPhase::Scanning,
                            target.gear.first().map(|item| Arc::clone(&item.name)),
                            1,
                            0,
                            None,
                        );
                        return Poll::Pending;
                    }
                    let gear: Vec<_> = target
                        .gear
                        .iter()
                        .filter(|item| stock.total(item.id).is_some_and(|count| count > 0))
                        .cloned()
                        .collect();
                    if gear
                        .iter()
                        .any(|item| stock.worn(item.id).unwrap_or(0) == 0)
                        && (stock.free_slots().unwrap_or(0) > 0
                            || gear.iter().any(|item| stock.held(item.id).unwrap_or(0) > 0)
                            || inventory
                                .iter()
                                .any(|row| !plan.base_spillover_keep.contains(&row.def.id)))
                    {
                        self.gear_run = Some(super::families::begin_survival_loadout(
                            &gear,
                            plan.bank,
                            plan.bank_required,
                            Arc::clone(&plan.base_spillover_keep),
                            cx,
                        )?);
                        self.in_session = true;
                        self.opening = !bank_open(&cx.tick.cx);
                        self.bank_starts = self.bank_starts.wrapping_add(1);
                        self.set_status(ProvisionPhase::Withdrawing, None, 0, 0, None, known);
                        return Poll::Pending;
                    }
                    self.gear_stage = stage;
                }
                for item in &target.gear {
                    if stock.carried(item.id).unwrap_or(0) == 0 {
                        needs.note_unmet(UnmetNeed {
                            item: Arc::clone(&item.name),
                            need: 1,
                            have: 0,
                            cause: if stock.banked(item.id).unwrap_or(0) > 0 {
                                UnmetCause::NoRoom
                            } else {
                                UnmetCause::NoSupply
                            },
                        });
                    }
                }
                Some(target)
            } else {
                None
            }
        } else {
            None
        };

        for item in plan.items.iter() {
            if !item_due(item, current) {
                // The quest has not reached this item's stage yet: no need,
                // and no Unknown-bank scan, until it is due.
                continue;
            }
            if item.kind == CompiledItemKind::Acquirable && item.acquire.is_none() {
                // Authored steps may gather or transform these goals in stages.
                // Consider their bank hints only after the immediate needs.
                continue;
            }
            let target = i32::try_from(item.qty).map_err(|_| {
                ActionError::Unavailable(Arc::from("compiled item quantity overflow"))
            })?;
            let mut need = bank_need(
                item.id,
                &item.name,
                target,
                NeedKind::Required,
                Presence::Carried,
            );
            if item.kind == CompiledItemKind::Acquirable && item.acquire.is_some() {
                need.sources = need.sources | SourceSet::CHILD;
            }
            needs.require(&need)?;
        }
        for need in plan.gather_tool_needs.iter() {
            let Some(stats) = snapshot.stats() else {
                continue;
            };
            let mut ready = false;
            let mut first_tool = None;
            let mut banked_tool = None;
            for &index in need.methods.iter() {
                let Some(method) = need.catalog.methods().get(index) else {
                    continue;
                };
                match crate::gatherer::supply::method_ready(snapshot, method, true) {
                    Ok(true) => {
                        ready = true;
                        break;
                    }
                    Err("gather usable tool missing") => {
                        for candidate in
                            crate::gatherer::supply::method_tool_candidates(method, stats.value)
                        {
                            let Some(item) =
                                need.tools.iter().find(|item| item.id == candidate.item)
                            else {
                                continue;
                            };
                            first_tool.get_or_insert(item);
                            if stock.bank_has(candidate.item, 1) == Truth::True {
                                banked_tool = Some(item);
                                break;
                            }
                        }
                    }
                    Ok(false) | Err(_) => {}
                }
                if banked_tool.is_some() {
                    break;
                }
            }
            if ready {
                continue;
            }
            // The banked candidate is the withdrawal; otherwise the first
            // stat-legal tool carries the need to `require`, whose origin
            // rule decides: an `Unknown` bank or a `Hint` lacking every tool
            // row costs the one verifying scan (design-bank-snapshot §2.4);
            // a `Session` bank without one leaves the optional need alone.
            if let Some(item) = banked_tool.or(first_tool) {
                needs.require(&bank_need(
                    item.id,
                    &item.name,
                    1,
                    NeedKind::Optional,
                    Presence::Carried,
                ))?;
            }
        }

        let recipe_need = needs
            .child_detail()
            .map(|need| {
                let recipe = plan
                    .items
                    .iter()
                    .find(|item| item.id == need.id && item.acquire.is_some())
                    .and_then(|item| item.acquire.as_ref())
                    .ok_or_else(|| {
                        ActionError::Unavailable(Arc::from("compiled acquisition recipe missing"))
                    })?;
                Ok::<_, ActionError>(RecipeNeed {
                    need: need.clone(),
                    recipe: Arc::clone(recipe),
                })
            })
            .transpose()?;
        let active_need = recipe_need.as_ref().filter(|_| needs.blocked().is_none());
        let active_recipe = active_need
            .map(|need| {
                plan.recipes.get(need.recipe.as_ref()).ok_or_else(|| {
                    ActionError::Unavailable(Arc::from("compiled acquisition recipe missing"))
                })
            })
            .transpose()?;
        let safe_slots = inventory
            .iter()
            .filter(|row| {
                row.count > 0
                    && !plan.base_spillover_keep.contains(&row.def.id)
                    && !survival.as_ref().is_some_and(|need| {
                        need.gear.iter().any(|item| item.id == row.def.id)
                            || need
                                .food
                                .as_ref()
                                .is_some_and(|(item, _)| item.id == row.def.id)
                    })
            })
            .count() as i32;

        // Optional floats can be deferred, so account for the active recipe
        // before admitting them to the withdrawal plan. The fit is exact once
        // the bank is known; an `Unknown` bank admits the need so the scan
        // learns it (design-bank-snapshot §4 F2).
        if plan.coin_float <= 0 {
            self.set_coin_drawn(true);
        } else if let Some(coin) = plan.coin.as_ref() {
            if !self.coin_drawn {
                if ops::count_id(inventory, coin.item.id) >= coin.qty {
                    self.set_coin_drawn(true);
                } else {
                    let fits = stock.banked(coin.item.id).is_none_or(|banked| {
                        let incoming = Stock::incoming_slots(
                            ops::count_id(inventory, coin.item.id),
                            coin.qty,
                            coin.stackable,
                            banked,
                        );
                        let already_planned = planned_required_slots(
                            plan,
                            &needs,
                            inventory,
                            active_need,
                            active_recipe,
                        );
                        incoming
                            <= capacity
                                .saturating_add(safe_slots)
                                .saturating_sub(already_planned)
                    });
                    if fits {
                        needs.require(&bank_need(
                            coin.item.id,
                            &coin.item.name,
                            coin.qty,
                            NeedKind::Optional,
                            Presence::Held,
                        ))?;
                    } else {
                        needs.note_unmet(UnmetNeed {
                            item: Arc::clone(&coin.item.name),
                            need: coin.qty,
                            have: ops::count_id(inventory, coin.item.id),
                            cause: UnmetCause::NoRoom,
                        });
                    }
                }
            }
        }

        if let Some(carry) = active_loadout.and_then(|name| plan.loadout_carry.get(name)) {
            for row in carry.iter() {
                let bit = 1u64 << row.latch_index;
                if self.carry_drawn & bit != 0 {
                    continue;
                }
                if row.qty <= 0 || ops::count_id(inventory, row.item.id) >= row.qty {
                    self.set_carry_latch(row.latch_index);
                } else {
                    let fits = stock.banked(row.item.id).is_none_or(|banked| {
                        let incoming = Stock::incoming_slots(
                            ops::count_id(inventory, row.item.id),
                            row.qty,
                            row.stackable,
                            banked,
                        );
                        let already_planned = planned_required_slots(
                            plan,
                            &needs,
                            inventory,
                            active_need,
                            active_recipe,
                        );
                        incoming
                            <= capacity
                                .saturating_add(safe_slots)
                                .saturating_sub(already_planned)
                    });
                    if fits {
                        needs.require(&bank_need(
                            row.item.id,
                            &row.item.name,
                            row.qty,
                            NeedKind::Optional,
                            Presence::Held,
                        ))?;
                    } else {
                        needs.note_unmet(UnmetNeed {
                            item: Arc::clone(&row.item.name),
                            need: row.qty,
                            have: ops::count_id(inventory, row.item.id),
                            cause: UnmetCause::NoRoom,
                        });
                    }
                }
            }
        }

        // Gear is now worn and required quest items have reserved their slots.
        // Food takes only the space left; an optional shortfall never grants or
        // refuses movement. Route admission owns that decision.
        if let Some(target) = &survival {
            if let Some((item, count)) = &target.food {
                let reserved =
                    planned_required_slots(plan, &needs, inventory, active_need, active_recipe);
                let held = stock.held(item.id).unwrap_or(0);
                let room = capacity
                    .saturating_sub(reserved)
                    .saturating_add(held)
                    .max(held);
                let fits = (*count).min(room);
                if fits > 0 || stock.banked(item.id).is_none() {
                    needs.require(&bank_need(
                        item.id,
                        &item.name,
                        if stock.banked(item.id).is_none() {
                            *count
                        } else {
                            fits
                        },
                        NeedKind::Optional,
                        Presence::Held,
                    ))?;
                }
                if fits < *count {
                    needs.note_unmet(UnmetNeed {
                        item: Arc::clone(&item.name),
                        need: *count,
                        have: stock.total(item.id).unwrap_or(held).min(fits),
                        cause: UnmetCause::NoRoom,
                    });
                }
                if target.unknown.is_some()
                    || target.bound > crate::combat::risk::consts::DEATH_BOUND
                        && stock.total(item.id).is_some_and(|have| have >= *count)
                        && fits >= *count
                {
                    needs.note_unmet(UnmetNeed {
                        item: Arc::from(target.unknown.map_or_else(
                            || format!("{} (survival bound above 1%)", item.name),
                            |why| format!("{} (survival assessment unavailable: {why})", item.name),
                        )),
                        need: *count,
                        have: held,
                        cause: UnmetCause::NoSupply,
                    });
                }
            }
        }

        if let Some(recipe) = active_recipe {
            for input in recipe.peak_items.iter() {
                needs.require(&bank_need(
                    input.item.id,
                    &input.item.name,
                    input.qty,
                    NeedKind::Optional,
                    Presence::Held,
                ))?;
            }
        }
        for item in plan
            .items
            .iter()
            .filter(|item| item.kind == CompiledItemKind::Acquirable && item.acquire.is_none())
            .filter(|item| item_due(item, current))
        {
            let target = i32::try_from(item.qty).map_err(|_| {
                ActionError::Unavailable(Arc::from("compiled item quantity overflow"))
            })?;
            if let Some(banked) = stock.banked(item.id) {
                let pack = ops::count_id(inventory, item.id);
                let incoming = Stock::incoming_slots(pack, target, item.stackable, banked);
                let immediate =
                    { planned_required_slots(plan, &needs, inventory, active_need, active_recipe) };
                if incoming
                    > capacity
                        .saturating_add(safe_slots)
                        .saturating_sub(immediate)
                {
                    // An optional later-stage hint must not park a runnable Path.
                    // Its authored steps still acquire it when it is needed.
                    needs.note_unmet(UnmetNeed {
                        item: Arc::clone(&item.name),
                        need: target,
                        have: pack,
                        cause: UnmetCause::NoRoom,
                    });
                    continue;
                }
            }
            needs.require(&bank_need(
                item.id,
                &item.name,
                target,
                NeedKind::Optional,
                Presence::Carried,
            ))?;
        }
        if let Some(scan) = needs.scan() {
            // One trip that observes the whole bank (design-bank-snapshot
            // §2.4): the `Unknown` bank learns, a `Hint` shortage is verified.
            // The next poll plans from `Session` rows at the open bank.
            let item = Arc::clone(&scan.item);
            let (need, pack, bank) = (scan.need, scan.pack, scan.bank);
            self.start_bank(
                cx,
                plan,
                BankAction::Scan,
                ProvisionPhase::Scanning,
                Some(item),
                need,
                pack,
                bank,
            );
            return Poll::Pending;
        }
        if let Some(missing) = needs.blocked() {
            if let Some(ended) = self.end_session(cx) {
                return ended;
            }
            // A `Session` shortage of a required item is final (design-bank-
            // snapshot §2.4, D4): in place, before any spillover or
            // withdrawal trip — the same observation already rules the
            // target out, however much of it the bank could still cover.
            let item = Arc::from(format!("{} x{}", missing.item, missing.need));
            self.set_status(
                ProvisionPhase::Blocked,
                Some(Arc::clone(&missing.item)),
                missing.need,
                missing.pack,
                missing.bank,
                true,
            );
            return Poll::Ready(Ok(ProvisionEvent::Blocked { item }));
        }

        let mut required_slots =
            planned_required_slots(plan, &needs, inventory, active_need, active_recipe);
        if required_slots.saturating_sub(capacity) > safe_slots {
            // Keep an impossible speculative peak out of the fallback, but
            // preserve immediate withdrawals and the active need's final slot.
            let immediate = planned_slots(plan, &needs, inventory, None, None, false, false);
            let active_need_slots = active_need.map_or(immediate, |need| {
                planned_slots(plan, &needs, inventory, Some(need), None, false, true)
            });
            required_slots = immediate.max(active_need_slots);
        }
        let deficit = required_slots.saturating_sub(capacity);
        if deficit > 0 {
            if safe_slots < deficit {
                if let Some(ended) = self.end_session(cx) {
                    return ended;
                }
                let item = Arc::from(format!(
                    "inventory capacity needs {deficit} additional safe slots"
                ));
                self.set_status(
                    ProvisionPhase::Blocked,
                    Some(Arc::clone(&item)),
                    deficit,
                    safe_slots,
                    None,
                    bank_known(cx),
                );
                return Poll::Ready(Ok(ProvisionEvent::Blocked { item }));
            }
            let slots = u8::try_from(deficit).map_err(|_| {
                ActionError::Unavailable(Arc::from("inventory capacity deficit overflow"))
            })?;
            let status = needs
                .first_withdrawal()
                .or_else(|| active_need.map(|need| &need.need));
            let keep = if let Some(survival) = &survival {
                let mut keep = plan.base_spillover_keep.to_vec();
                for item in &survival.gear {
                    if !keep.contains(&item.id) {
                        keep.push(item.id);
                    }
                }
                if let Some((item, _)) = &survival.food {
                    if !keep.contains(&item.id) {
                        keep.push(item.id);
                    }
                }
                Arc::from(keep)
            } else {
                Arc::clone(&plan.base_spillover_keep)
            };
            self.start_bank(
                cx,
                plan,
                BankAction::DepositCapacity { keep, slots },
                ProvisionPhase::Spillover,
                status.map(|need| Arc::clone(&need.item)),
                status.map_or(deficit, |need| need.need),
                status.map_or(ops::occupied(inventory), |need| need.pack),
                status.and_then(|need| need.bank),
            );
            return Poll::Pending;
        }

        if needs.withdrawal_count() > 0 {
            let first = needs
                .first_withdrawal()
                .expect("withdrawal has a detail")
                .clone();
            let withdrawals = needs.take_withdrawals();
            self.start_bank(
                cx,
                plan,
                BankAction::WithdrawTo { withdrawals },
                ProvisionPhase::Withdrawing,
                Some(Arc::clone(&first.item)),
                first.need,
                first.pack,
                first.bank,
            );
            return Poll::Pending;
        }
        if let (Some(recipe_need), Some(recipe)) = (recipe_need, active_recipe) {
            if let Some(ended) = self.end_session(cx) {
                return ended;
            }
            // A provisioning need has no step settle to restart against, so
            // this run makes one pass.
            let acquire = AcquirePlan {
                recipe: Arc::clone(&recipe_need.recipe),
                steps: Arc::clone(&recipe.steps),
                goal: None,
                max_restarts: 0,
            };
            match acquire.begin(cx) {
                Ok(run) => {
                    self.acquire_run = Some(run);
                    self.set_status(
                        ProvisionPhase::Acquiring,
                        Some(recipe_need.need.item),
                        recipe_need.need.need,
                        recipe_need.need.pack,
                        recipe_need.need.bank,
                        true,
                    );
                    return Poll::Pending;
                }
                Err(error) => return Poll::Ready(Err(error)),
            }
        }

        // Equipment was handled before quest items and food, by LoadoutRun.
        if let Some(ended) = self.end_session(cx) {
            return ended;
        }
        if survival.is_some()
            || plan.survival.is_none()
            || self.survival_stage != current.unwrap_or(usize::MAX)
        {
            self.log_unmet(plan, needs.unmet());
        }
        if survival.is_some() {
            self.survival_stage = current.unwrap_or(usize::MAX);
        }
        let shortfall = self.unmet_logged.first().cloned();
        self.set_status(
            ProvisionPhase::Ready,
            shortfall.as_ref().map(|need| Arc::clone(&need.item)),
            shortfall.as_ref().map_or(0, |need| need.need),
            shortfall.as_ref().map_or(0, |need| need.have),
            shortfall.as_ref().and_then(|need| {
                survival
                    .as_ref()
                    .and_then(|target| target.food.as_ref())
                    .filter(|(item, _)| item.name == need.item)
                    .and_then(|(item, _)| stock.banked(item.id))
            }),
            bank_known(cx),
        );
        Poll::Ready(Ok(ProvisionEvent::Ready))
    }

    fn poll_bank(
        &mut self,
        cx: &mut StepContext<'_, '_>,
    ) -> Poll<Result<ProvisionEvent, ActionError>> {
        let result = self
            .bank_run
            .as_mut()
            .expect("bank poll has a machine")
            .poll(cx.tick, OpenGate::Open);
        match result {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => {
                self.bank_run = None;
                Poll::Ready(Err(error))
            }
            Poll::Ready(Ok(AcquireOutcome::Done(receipt))) => {
                self.bank_run = None;
                let receipt = receipt.last_bank.unwrap_or(BankReceipt {
                    complete: receipt.complete,
                    deposited: receipt.deposited,
                });
                Poll::Ready(Ok(ProvisionEvent::BankReceipt(receipt)))
            }
            Poll::Ready(Ok(AcquireOutcome::ChildDue { .. })) => {
                self.bank_run = None;
                Poll::Ready(Err(ActionError::Unavailable(Arc::from(
                    "authored bank stage requested a child",
                ))))
            }
        }
    }

    fn poll_acquire(
        &mut self,
        cx: &mut StepContext<'_, '_>,
    ) -> Poll<Result<ProvisionEvent, ActionError>> {
        let result = self
            .acquire_run
            .as_mut()
            .expect("acquire poll has a run")
            .poll(cx);
        // Keep the finished run until the caller drains its final trace events.
        // The next poll drops it before starting any other preparation work.
        self.acquire_finished = result.is_ready();
        match result {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Ready(Ok(_)) => Poll::Ready(Ok(ProvisionEvent::Acquired)),
        }
    }

    pub(super) fn start_predicate_scan(
        &mut self,
        cx: &mut StepContext<'_, '_>,
        plan: &CompiledProvisioning,
    ) {
        if self.bank_run.is_some() || self.acquire_run.is_some() {
            return;
        }
        self.start_bank(
            cx,
            plan,
            BankAction::Scan,
            ProvisionPhase::Scanning,
            None,
            0,
            0,
            None,
        );
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "One bank request and its producer-side status receipt."
    )]
    fn start_bank(
        &mut self,
        cx: &mut StepContext<'_, '_>,
        plan: &CompiledProvisioning,
        action: BankAction,
        phase: ProvisionPhase,
        item: Option<Arc<str>>,
        need: i32,
        pack: i32,
        bank: Option<i32>,
    ) {
        self.in_session = true;
        self.opening = !bank_open(&cx.tick.cx);
        let request = AcquireRequest::bank_actions(
            plan.bank,
            plan.bank_required,
            Arc::from([action]),
            false,
            cx.required_after,
        );
        self.bank_run = Some(
            AcquireRun::new(
                request,
                crate::acquire::AcquirePlan::authored(),
                cx.banks,
                &cx.tick.cx,
            )
            .with_quester_walk_retries(),
        );
        self.attempts = self.attempts.saturating_add(1);
        self.bank_starts = self.bank_starts.wrapping_add(1);
        self.set_status(phase, item, need, pack, bank, bank_known(cx));
    }

    /// Ends the bank session: closes the bank this provisioner's trips opened
    /// and a frame still shows open. `None` means nothing is left to close.
    /// Otherwise the result is the poll result to return while the close runs.
    fn end_session(
        &mut self,
        cx: &mut StepContext<'_, '_>,
    ) -> Option<Poll<Result<ProvisionEvent, ActionError>>> {
        match self.closer.drive(cx.tick, &mut self.lease) {
            Closed::Done => {
                self.in_session = false;
                None
            }
            Closed::Pending => {
                if self.closer.active() {
                    self.set_status(ProvisionPhase::Closing, None, 0, 0, None, bank_known(cx));
                }
                Some(Poll::Pending)
            }
            Closed::Failed(error) => {
                self.in_session = false;
                Some(Poll::Ready(Err(error)))
            }
        }
    }

    /// Records the lease when a frame shows an open bank that this account's
    /// own bank kernel opened. Without that evidence, a bank is the player's:
    /// the lease is neither created nor renewed. A trip that began with the
    /// bank shut creates the lease. Inside the session, a bank that the kernel
    /// reopened is the same lease under its new generation. Any other
    /// generation stays unadopted, and the next frame check voids the lease
    /// (`lease_view`), so a close in progress or due never reaches it.
    fn observe_bank(&mut self, tick: &NativeTick<'_>) {
        let Some(view) = tick.cx.snapshot().bank_session().map(|seen| seen.value) else {
            return;
        };
        if !view.open || !tick.cx.bank_opened_by_machine(view.generation) {
            return;
        }
        let session = tick.cx.evidence().run.session;
        if self.opening {
            self.lease = Some(BankLease::new(session, view.generation));
            self.opening = false;
        } else if self.in_session {
            if let Some(lease) = self.lease.as_mut().filter(|lease| lease.session == session) {
                lease.generation = view.generation;
            }
        }
    }

    /// One `info` line for each optional need that provisioning leaves unmet
    /// while it returns `Ready`. A repeat of the same shortfall logs nothing.
    fn log_unmet(&mut self, plan: &CompiledProvisioning, unmet: &[UnmetNeed]) {
        if self.unmet_logged.as_slice() == unmet {
            return;
        }
        for need in unmet {
            api::host_log!(
                api::hostlog::Category::BankOp,
                api::hostlog::Level::Info,
                "quester {}: provisioning ready with an optional need unmet: {} needs {}, holds {}, short {} ({})",
                plan.path.0,
                need.item,
                need.need,
                need.have,
                need.need.saturating_sub(need.have),
                need.cause.describe()
            );
        }
        self.unmet_logged = unmet.to_vec();
    }

    fn set_coin_drawn(&mut self, drawn: bool) {
        if self.coin_drawn != drawn {
            self.coin_drawn = drawn;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    fn set_carry_drawn(&mut self, drawn: u64) {
        if self.carry_drawn != drawn {
            self.carry_drawn = drawn;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    fn set_carry_latch(&mut self, index: u8) {
        self.set_carry_drawn(self.carry_drawn | (1u64 << index));
    }

    fn set_status(
        &mut self,
        phase: ProvisionPhase,
        item: Option<Arc<str>>,
        need: i32,
        pack: i32,
        bank: Option<i32>,
        bank_known: bool,
    ) {
        let next = Status {
            phase,
            item,
            need,
            pack,
            bank,
            bank_known,
            attempts: self.attempts,
        };
        if self.status != next {
            self.status = next;
            self.revision = self.revision.wrapping_add(1);
        }
    }
}

struct RecipeNeed {
    need: NeedObservation,
    recipe: Arc<str>,
}

/// Translate a Path target into the shared planner's per-need contract.
fn bank_need(
    item: i32,
    name: &Arc<str>,
    target: i32,
    kind: NeedKind,
    presence: Presence,
) -> crate::acquire::Need {
    static NO_ALTERNATIVES: std::sync::LazyLock<Arc<[i32]>> =
        std::sync::LazyLock::new(|| Arc::from([]));
    crate::acquire::Need {
        item,
        name: Arc::clone(name),
        target,
        minimum: target,
        kind,
        presence,
        sources: SourceSet::PACK | SourceSet::WORN | SourceSet::BANK,
        alternatives: Arc::clone(&NO_ALTERNATIVES),
    }
}

/// Index of the quest's current stage in compiled sequence order.
/// Unknown (or unlisted) stages resolve to `None`, which keeps gated
/// items out: an early acquisition must never start on a guess.
fn stage_index(plan: &CompiledProvisioning, current_stage: Option<&FactKey>) -> Option<usize> {
    let stage = current_stage?;
    plan.stages.iter().position(|key| key == stage)
}

fn item_due(item: &super::compile::CompiledQuestItem, current: Option<usize>) -> bool {
    item.from_stage_index
        .is_none_or(|gate| current.is_some_and(|stage| stage >= gate))
}

fn has_withdrawal(needs: &Planner<'_>, id: i32) -> bool {
    needs.withdrawals().any(|withdrawal| withdrawal.id == id)
}

fn recipe_peak_target(recipe: &CompiledAcquireRecipe, id: i32) -> i32 {
    recipe
        .peak_items
        .iter()
        .find(|item| item.item.id == id)
        .map_or(0, |item| item.qty)
}

fn item_stackable(
    plan: &CompiledProvisioning,
    recipe: Option<&CompiledAcquireRecipe>,
    inventory: &[ItemView],
    id: i32,
) -> bool {
    inventory
        .iter()
        .find(|row| row.count > 0 && row.def.id == id)
        .map(|row| row.def.stackable)
        .or_else(|| {
            plan.items
                .iter()
                .find(|item| item.id == id)
                .map(|item| item.stackable)
        })
        .or_else(|| {
            plan.coin
                .as_ref()
                .filter(|item| item.item.id == id)
                .map(|item| item.stackable)
        })
        .or_else(|| {
            plan.loadout_carry
                .values()
                .flat_map(|rows| rows.iter())
                .find(|item| item.item.id == id)
                .map(|item| item.stackable)
        })
        .or_else(|| {
            recipe.and_then(|recipe| {
                recipe
                    .peak_items
                    .iter()
                    .find(|item| item.item.id == id)
                    .map(|item| item.stackable)
            })
        })
        .unwrap_or(false)
}

fn has_other_recipe_state_need(
    plan: &CompiledProvisioning,
    active_need: Option<&RecipeNeed>,
    id: i32,
) -> bool {
    active_need.is_some_and(|need| need.need.id == id)
        || plan.items.iter().any(|item| item.id == id)
        || plan.coin.as_ref().is_some_and(|coin| coin.item.id == id)
        || plan.tools.iter().any(|tool| tool.id == id)
        || plan
            .loadout_carry
            .values()
            .flat_map(|rows| rows.iter())
            .any(|row| row.item.id == id)
}

fn planned_slots(
    plan: &CompiledProvisioning,
    needs: &Planner<'_>,
    inventory: &[ItemView],
    active_need: Option<&RecipeNeed>,
    recipe: Option<&CompiledAcquireRecipe>,
    include_recipe_inputs: bool,
    include_final_outputs: bool,
) -> i32 {
    let mut slots = ops::occupied(inventory);
    if let Some(recipe) = recipe {
        if include_recipe_inputs {
            for id in recipe.consumed_ids.iter().copied() {
                if recipe_peak_target(recipe, id) > 0
                    || has_other_recipe_state_need(plan, active_need, id)
                {
                    continue;
                }
                let absent = Stock::slots_for(
                    ops::count_id(inventory, id),
                    item_stackable(plan, Some(recipe), inventory, id),
                );
                slots = slots.saturating_sub(absent).max(0);
            }
        }
        if include_final_outputs {
            for id in recipe.consumed_ids.iter().copied() {
                if has_other_recipe_state_need(plan, active_need, id) {
                    continue;
                }
                let absent = Stock::slots_for(
                    ops::count_id(inventory, id),
                    item_stackable(plan, Some(recipe), inventory, id),
                );
                slots = slots.saturating_sub(absent).max(0);
            }
        }
    }
    for withdrawal in needs.withdrawals() {
        if !include_recipe_inputs
            && include_final_outputs
            && recipe.is_some_and(|recipe| recipe.consumed_ids.contains(&withdrawal.id))
            && !has_other_recipe_state_need(plan, active_need, withdrawal.id)
        {
            continue;
        }
        let mut target = withdrawal.target;
        if include_recipe_inputs {
            target =
                target.max(recipe.map_or(0, |recipe| recipe_peak_target(recipe, withdrawal.id)));
        }
        if include_final_outputs {
            target = target.max(
                active_need
                    .filter(|need| need.need.id == withdrawal.id)
                    .map_or(0, |need| need.need.need),
            );
        }
        let pack = ops::count_id(inventory, withdrawal.id);
        let stackable = item_stackable(plan, recipe, inventory, withdrawal.id);
        slots = slots.saturating_add(
            Stock::slots_for(target, stackable).saturating_sub(Stock::slots_for(pack, stackable)),
        );
    }
    if include_recipe_inputs {
        if let Some(recipe) = recipe {
            for input in recipe.peak_items.iter() {
                if has_withdrawal(needs, input.item.id) {
                    continue;
                }
                let pack = ops::count_id(inventory, input.item.id);
                slots = slots.saturating_add(
                    Stock::slots_for(input.qty, input.stackable)
                        .saturating_sub(Stock::slots_for(pack, input.stackable)),
                );
            }
        }
    }
    if include_final_outputs {
        if let Some(recipe) = recipe {
            for input in recipe.peak_items.iter() {
                if has_withdrawal(needs, input.item.id)
                    || (recipe.consumed_ids.contains(&input.item.id)
                        && !has_other_recipe_state_need(plan, active_need, input.item.id))
                    || active_need.is_some_and(|need| need.need.id == input.item.id)
                {
                    continue;
                }
                let pack = ops::count_id(inventory, input.item.id);
                slots = slots.saturating_add(
                    Stock::slots_for(input.qty, input.stackable)
                        .saturating_sub(Stock::slots_for(pack, input.stackable)),
                );
            }
        }
    }
    if include_final_outputs {
        if let Some(need) = active_need {
            if !has_withdrawal(needs, need.need.id) {
                let stackable = item_stackable(plan, recipe, inventory, need.need.id);
                let recipe_target =
                    recipe.map_or(0, |recipe| recipe_peak_target(recipe, need.need.id));
                let target = recipe_target.max(need.need.need);
                let pack = ops::count_id(inventory, need.need.id);
                slots = slots.saturating_add(
                    Stock::slots_for(target, stackable)
                        .saturating_sub(Stock::slots_for(pack, stackable)),
                );
            }
        }
    }
    slots
}

fn planned_required_slots(
    plan: &CompiledProvisioning,
    needs: &Planner<'_>,
    inventory: &[ItemView],
    active_need: Option<&RecipeNeed>,
    recipe: Option<&CompiledAcquireRecipe>,
) -> i32 {
    if let (Some(need), Some(recipe)) = (active_need, recipe) {
        planned_slots(
            plan,
            needs,
            inventory,
            Some(need),
            Some(recipe),
            true,
            false,
        )
        .max(planned_slots(
            plan,
            needs,
            inventory,
            Some(need),
            Some(recipe),
            false,
            true,
        ))
    } else {
        planned_slots(plan, needs, inventory, None, None, false, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::ledger;
    use crate::native::{HostEffect, NativeTick};
    use crate::native_bank::BankPickRequest;
    use crate::quester::compile::{self, CompiledPath, CompiledQuestItem};
    use crate::quester::families::tests::{def, local_player, with_tick, with_tick_bank};
    use api::bank_memory::{BankMemory, Origin};
    use api::named_banks::{NamedBank, NamedBankFacts};
    use api::quest_facts::QuestCatalog;
    use api::selected::{ClientRevision, FactKey};
    use api::snapshot::{
        GameSnapshot, ItemActionFamily, ItemContainer, ItemView, LocLayer, LocView, StatView,
        WorldStateView, WorldTile,
    };
    use std::collections::HashMap;

    fn quest_catalog() -> QuestCatalog {
        let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
        QuestCatalog::from_identity(data.quest_identity()).unwrap()
    }

    fn item_view(id: i32, name: &str, count: i32, container: ItemContainer) -> ItemView {
        ItemView {
            def: def(id, name),
            container,
            action_family: ItemActionFamily::Held,
            slot: 0,
            count,
            actions: Vec::new(),
            component_id: 0,
        }
    }

    fn walk_receipt(
        end: crate::native::WalkEnd,
        detail: Option<Arc<str>>,
    ) -> crate::native::WalkReceipt {
        crate::native::WalkReceipt {
            goal: None,
            request_id: 7,
            evidence: api::quest_progress::EvidenceStamp {
                run: api::selected::RunKey {
                    slot: 1,
                    run: 2,
                    session: 3,
                },
                tick: 4,
                sequence: 5,
            },
            end,
            blocked: None,
            detail,
            refusal: None,
            assessment: None,
            escape: None,
            prereq_allow: None,
        }
    }

    #[test]
    fn provision_walk_requires_arrival_and_retains_refusal_detail() {
        let result = walk_receipt(
            crate::native::WalkEnd::RouteEnded,
            Some(Arc::from("route stopped short")),
        )
        .into_arrival()
        .map(|_| ());
        assert!(matches!(
            result,
            Err(ActionError::Blocked(detail)) if detail.as_ref() == "route stopped short"
        ));
    }

    #[test]
    fn provision_walk_keeps_needs_evidence_typed() {
        let gates: Arc<[api::selected::QuestGate]> =
            Arc::from([api::selected::QuestGate::Complete(FactKey::new(
                "provision-gate",
            ))]);
        let error = walk_receipt(
            crate::native::WalkEnd::NeedsEvidence(Arc::clone(&gates)),
            None,
        )
        .into_arrival()
        .map(|_| ())
        .unwrap_err();
        assert_eq!(
            format!("{error:?}"),
            format!("NeedsEvidence({gates:?})"),
            "quester needs the original typed evidence gates, not a debug-string Blocked"
        );
    }

    #[test]
    fn provision_walk_preserves_manual_cancellation() {
        assert_eq!(
            walk_receipt(crate::native::WalkEnd::UserInput, None)
                .into_arrival()
                .map(|_| ()),
            Err(ActionError::UserInput)
        );
    }

    fn compiled_item(
        id: i32,
        name: &str,
        qty: u32,
        kind: CompiledItemKind,
        acquire: Option<&str>,
    ) -> CompiledQuestItem {
        CompiledQuestItem {
            id,
            name: Arc::from(name),
            qty,
            kind,
            acquire: acquire.map(Arc::from),
            stackable: false,
            from_stage: None,
            from_stage_index: None,
        }
    }

    fn provisioning(
        items: Vec<CompiledQuestItem>,
        bank: Option<NamedBank>,
    ) -> CompiledProvisioning {
        let keep: Vec<_> = items.iter().map(|item| item.id).collect();
        CompiledProvisioning {
            path: FactKey::new("provision-test"),
            owns_inventory: false,
            bank,
            bank_required: false,
            items: Arc::from(items),
            stages: Arc::from(Vec::new()),
            tools: Arc::from(Vec::new()),
            gather_tool_needs: Arc::from(Vec::new()),
            keep_ids: Arc::from(Vec::new()),
            coin_float: 0,
            coin: None,
            loadout_carry: HashMap::new(),
            base_spillover_keep: Arc::from(keep),
            recipes: HashMap::new(),
            survival: None,
        }
    }

    #[allow(clippy::too_many_arguments)] // Explicit fixture inputs mirror the production poll context.
    fn poll_once(
        provisioner: &mut Provisioner,
        snapshot: &GameSnapshot,
        ledger: &mut Option<Box<ledger::Ledger>>,
        tick: u64,
        plan: &CompiledProvisioning,
        active_loadout: Option<&str>,
        memo: &BankMemory,
        banks: &Arc<api::named_banks::NamedBankFacts>,
        quests: &QuestCatalog,
    ) -> Poll<Result<ProvisionEvent, ActionError>> {
        with_tick_bank(snapshot, Some(memo), ledger, tick, |native| {
            let required_after = native.cx.evidence();
            let mut cx = StepContext {
                tick: native,
                quests,
                progress: &[],
                required_after,
                banks,
                choices: &crate::quester::choices::QuestChoices::default(),
            };
            provisioner.poll(&mut cx, plan, active_loadout, None)
        })
    }

    #[allow(clippy::too_many_arguments)] // Explicit fixture inputs mirror the production poll context.
    fn poll_once_at(
        provisioner: &mut Provisioner,
        snapshot: &GameSnapshot,
        ledger: &mut Option<Box<ledger::Ledger>>,
        tick: u64,
        plan: &CompiledProvisioning,
        active_loadout: Option<&str>,
        stage: Option<&FactKey>,
        memo: &BankMemory,
        banks: &Arc<api::named_banks::NamedBankFacts>,
        quests: &QuestCatalog,
    ) -> Poll<Result<ProvisionEvent, ActionError>> {
        with_tick_bank(snapshot, Some(memo), ledger, tick, |native| {
            let required_after = native.cx.evidence();
            let mut cx = StepContext {
                tick: native,
                quests,
                progress: &[],
                required_after,
                banks,
                choices: &crate::quester::choices::QuestChoices::default(),
            };
            provisioner.poll(&mut cx, plan, active_loadout, stage)
        })
    }

    fn snapshot_view(snapshot: &GameSnapshot, tick: u64) -> api::snapshot::SnapshotView<'_> {
        api::snapshot::SnapshotView::new(
            Some(snapshot),
            api::quest_progress::EvidenceStamp {
                run: api::selected::RunKey {
                    slot: 1,
                    run: 1,
                    session: 1,
                },
                tick,
                sequence: tick,
            },
        )
    }

    fn ready_snapshot(items: Vec<ItemView>) -> GameSnapshot {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(items, 28);
        snapshot
    }

    fn survival_fixture(crossing: bool) -> (Arc<CompiledPath>, QuestCatalog, GameSnapshot) {
        survival_fixture_with_gear(crossing, false)
    }

    fn survival_fixture_with_gear(
        crossing: bool,
        gear: bool,
    ) -> (Arc<CompiledPath>, QuestCatalog, GameSnapshot) {
        let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let quests = quest_catalog();
        let (lease, zone) = crate::combat::risk::provisioning_cross_fixture();
        let mut document = compile::decode_cook().unwrap();
        let header = document.quest.as_mut().unwrap();
        header.items.clear();
        header.tools.clear();
        header.coin_float = 0;
        header.loadouts.clear();
        header.owns_inventory = false;
        if gear {
            header.loadouts.insert(
                "survival".into(),
                super::super::path::QuestLoadoutDocument {
                    worn: [
                        ("hat", "bronze_full_helm"),
                        ("torso", "bronze_platebody"),
                        ("legs", "bronze_platelegs"),
                        ("lefthand", "bronze_kiteshield"),
                        ("righthand", "bronze_sword"),
                    ]
                    .into_iter()
                    .map(|(slot, name)| (slot.into(), name.into()))
                    .collect(),
                    carry: Vec::new(),
                },
            );
        }
        if crossing {
            document.roles[0].sequences[0].cross = vec![super::super::path::RequiredZoneDocument {
                id: zone,
                name: "Ice warrior crossing".into(),
                reason: "Exercise food-aware preparation before crossing.".into(),
            }];
        }
        let mut path = compile::compile_uncached_for_test_with_zones(
            &document,
            &selected,
            &quests,
            lease.zones().unwrap(),
        )
        .unwrap();
        Arc::get_mut(&mut path).unwrap().provisioning.bank = None;
        let (mut snapshot, _, _) = openable_bank_fixture();
        snapshot.seed_tile(tile(3210, 3210));
        snapshot.seed_equipment(Vec::new());
        snapshot.seed_hitmarks(api::snapshot::HitmarksView {
            marks: [api::snapshot::HitmarkView {
                value: 0,
                kind: 0,
                cycle: 0,
            }; 4],
            loop_cycle: 0,
        });
        snapshot.seed_stats(
            [
                (3, "hitpoints", 20, 40),
                (1, "defence", 1, 1),
                (5, "prayer", 1, 1),
            ]
            .into_iter()
            .map(|(index, name, effective, base)| StatView {
                index,
                name: name.into(),
                effective,
                base,
                xp: 0,
                used: true,
            })
            .collect(),
        );
        snapshot.seed_world(WorldStateView {
            members: true,
            ..Default::default()
        });
        snapshot.seed_varps(vec![api::snapshot::VarpView {
            index: 173,
            value: 0,
        }]);
        (path, quests, snapshot)
    }

    fn survival_input(
        snapshot: &GameSnapshot,
        survival: &super::super::survival::CompiledSurvival,
    ) -> crate::combat::risk::RiskInput {
        let frame = crate::combat::frame::Frame::borrow(snapshot_view(snapshot, 1)).unwrap();
        crate::combat::risk::RiskInput::capture(
            &frame,
            survival.tables(),
            &crate::combat::threats::ThreatSet::default(),
            api::condition::PoisonState::Clear,
            false,
        )
    }

    #[test]
    fn declared_crossing_withdraws_the_food_bound_needs_before_ready() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (path, quests, mut snapshot) = survival_fixture(true);
        let survival = path.provisioning.survival.as_ref().unwrap();
        let lobster = survival
            .tables()
            .selected()
            .item_by_alias("lobster")
            .unwrap();
        let name = lobster.name.as_deref().unwrap();
        let id = lobster.id;
        snapshot.seed_bank_observation(1, 0, Some(vec![bank_row(id, name, 28)]), Vec::new());
        let mut memory = BankMemory::default();
        memory.track(&snapshot, 0);
        let input = survival_input(&snapshot, survival);
        let expected = survival
            .plan(
                0,
                Stock {
                    bank: Some(&memory),
                    ..snapshot_view(&snapshot, 1).stock()
                },
                input,
            )
            .unwrap()
            .food
            .unwrap();
        assert_eq!(expected.0.id, id);
        assert!(expected.1 > 0);
        let (_, _, banks) = openable_bank_fixture();
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let mut withdrawn = 0;
        let mut ready = false;
        for tick in 1..=80 {
            memory.track(&snapshot, tick);
            let result = poll_once_at(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                tick,
                &path.provisioning,
                None,
                Some(&path.provisioning.stages[0]),
                &memory,
                &banks,
                &quests,
            );
            if let Some(effect) = ledger
                .as_ref()
                .and_then(|ledger| ledger.outbox.first())
                .map(|action| &action.effect)
            {
                match effect {
                    HostEffect::Interaction(crate::interact::InteractReq::Withdraw {
                        name: item,
                        action,
                    }) => {
                        assert_eq!(item, name);
                        withdrawn += action
                            .strip_prefix("Withdraw-")
                            .unwrap()
                            .parse::<i32>()
                            .unwrap();
                    }
                    HostEffect::Interaction(crate::interact::InteractReq::WithdrawX {
                        name: item,
                        count,
                        ..
                    }) => {
                        assert_eq!(item, name);
                        withdrawn += count;
                    }
                    HostEffect::Interaction(crate::interact::InteractReq::SetNoteMode {
                        ..
                    }) => {}
                    _ => panic!("survival preparation must only withdraw food"),
                }
                complete_open_stand(&mut ledger, tick);
                if withdrawn > 0 {
                    snapshot.seed_inventory(
                        (0..withdrawn)
                            .map(|slot| {
                                let mut row = item_view(id, name, 1, ItemContainer::Inventory);
                                row.slot = slot;
                                row
                            })
                            .collect(),
                        28,
                    );
                    snapshot.seed_bank_observation(
                        1,
                        tick,
                        Some(vec![bank_row(id, name, 28 - withdrawn)]),
                        Vec::new(),
                    );
                }
            }
            match result {
                Poll::Ready(Ok(ProvisionEvent::Ready)) => {
                    ready = true;
                    break;
                }
                Poll::Pending | Poll::Ready(Ok(ProvisionEvent::BankReceipt(_))) => {}
                other => panic!("unexpected survival preparation result: {other:?}"),
            }
        }
        assert!(
            ready,
            "the stage must become ready after the observed withdrawal"
        );
        assert_eq!(withdrawn, expected.1);
        let observed = survival_input(&snapshot, survival);
        let target = survival
            .plan(
                0,
                Stock {
                    bank: Some(&memory),
                    ..snapshot_view(&snapshot, 80).stock()
                },
                observed,
            )
            .unwrap();
        assert!(target
            .food
            .is_some_and(|(item, qty)| item.id == id && qty <= withdrawn));
        assert!(target.bound <= crate::combat::risk::consts::DEATH_BOUND);
    }

    #[test]
    fn survival_gear_is_worn_before_quest_items_reserve_food_room() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (mut path, quests, mut snapshot) = survival_fixture_with_gear(true, true);
        let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let equipment = [
            (0, "bronze_full_helm"),
            (4, "bronze_platebody"),
            (7, "bronze_platelegs"),
            (5, "bronze_kiteshield"),
            (3, "bronze_sword"),
        ]
        .map(|(slot, name)| {
            let item = selected.resolve_item_name(name).unwrap();
            (slot, item.id, item.name.as_deref().unwrap())
        });
        let food = selected.item_by_alias("lobster").unwrap();
        let food_name = food.name.as_deref().unwrap();
        let plan = &mut Arc::get_mut(&mut path).unwrap().provisioning;
        plan.items = Arc::from([compiled_item(
            42,
            "Quest token",
            25,
            CompiledItemKind::MustHave,
            None,
        )]);
        plan.base_spillover_keep = Arc::from([42]);
        let mut pack: Vec<_> = (0..25)
            .map(|slot| {
                let mut row = item_view(42, "Quest token", 1, ItemContainer::Inventory);
                row.slot = slot;
                row
            })
            .collect();
        snapshot.seed_inventory(pack.clone(), 28);
        let mut rows: Vec<_> = equipment
            .iter()
            .map(|&(_, id, name)| bank_row(id, name, 1))
            .collect();
        rows.push(bank_row(food.id, food_name, 28));
        snapshot.seed_bank_observation(1, 0, Some(rows.clone()), Vec::new());
        let mut memory = BankMemory::default();
        memory.track(&snapshot, 0);
        let survival = plan.survival.as_ref().unwrap();
        let target = survival
            .plan(
                0,
                Stock {
                    bank: Some(&memory),
                    ..snapshot_view(&snapshot, 1).stock()
                },
                survival_input(&snapshot, survival),
            )
            .unwrap();
        assert_eq!(target.gear.len(), equipment.len());
        assert!(target.food.as_ref().unwrap().1 > 3);
        assert!(
            equipment.len() > 28 - pack.len(),
            "the whole kit cannot fit in the pack"
        );
        let (_, bank, banks) = openable_bank_fixture();
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let mut worn = Vec::new();
        let mut food_drawn = 0;
        let mut ready = false;
        for tick in 1..=200 {
            memory.track(&snapshot, tick);
            let result = poll_once_at(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                tick,
                plan,
                None,
                Some(&plan.stages[0]),
                &memory,
                &banks,
                &quests,
            );
            let effect = ledger
                .as_ref()
                .and_then(|ledger| ledger.outbox.first())
                .map(|action| &action.effect);
            if let Some(effect) = effect {
                match effect {
                    HostEffect::BankPick(_) => {
                        complete_bank_pick(&mut ledger, bank, bank.tile, tick);
                    }
                    HostEffect::Interaction(crate::interact::InteractReq::OpenStand { .. }) => {
                        complete_open_stand(&mut ledger, tick);
                        snapshot.seed_bank_observation(1, tick, Some(rows.clone()), Vec::new());
                    }
                    HostEffect::Interaction(crate::interact::InteractReq::Withdraw {
                        name,
                        action,
                    }) => {
                        let count = action.strip_prefix("Withdraw-").unwrap().parse().unwrap();
                        let item = rows
                            .iter_mut()
                            .find(|row| row.def.name.as_deref() == Some(name.as_str()))
                            .unwrap();
                        if item.def.id == food.id {
                            assert_eq!(worn.len(), equipment.len(), "food follows worn gear");
                            food_drawn += count;
                        }
                        for _ in 0..count {
                            pack.push(item_view(item.def.id, name, 1, ItemContainer::Inventory));
                        }
                        item.count -= count;
                        complete_open_stand(&mut ledger, tick);
                    }
                    HostEffect::Interaction(crate::interact::InteractReq::WithdrawX {
                        name,
                        count,
                        ..
                    }) => {
                        let item = rows
                            .iter_mut()
                            .find(|row| row.def.name.as_deref() == Some(name.as_str()))
                            .unwrap();
                        if item.def.id == food.id {
                            assert_eq!(worn.len(), equipment.len(), "food follows worn gear");
                            food_drawn += count;
                        }
                        for _ in 0..*count {
                            pack.push(item_view(item.def.id, name, 1, ItemContainer::Inventory));
                        }
                        item.count -= count;
                        complete_open_stand(&mut ledger, tick);
                    }
                    HostEffect::Interaction(crate::interact::InteractReq::Wear { name }) => {
                        let &(slot, id, _) = equipment
                            .iter()
                            .find(|&&(_, _, known)| known == name.as_str())
                            .unwrap();
                        assert!(pack.iter().any(|row| row.def.id == id));
                        pack.retain(|row| row.def.id != id);
                        let mut row = item_view(id, name, 1, ItemContainer::Equipment);
                        row.slot = slot;
                        worn.push(row);
                        complete_open_stand(&mut ledger, tick);
                        snapshot.seed_bank_observation(-1, tick, None, Vec::new());
                    }
                    HostEffect::Interaction(
                        crate::interact::InteractReq::SetNoteMode { .. }
                        | crate::interact::InteractReq::Close,
                    ) => {
                        complete_open_stand(&mut ledger, tick);
                    }
                    _ => panic!("unexpected survival gear effect"),
                }
                snapshot.seed_inventory(pack.clone(), 28);
                snapshot.seed_equipment(worn.clone());
                if snapshot_view(&snapshot, tick)
                    .bank_session()
                    .is_some_and(|bank| bank.value.open)
                {
                    snapshot.seed_bank_observation(1, tick, Some(rows.clone()), Vec::new());
                }
            }
            match result {
                Poll::Ready(Ok(ProvisionEvent::Ready)) => {
                    ready = true;
                    break;
                }
                Poll::Pending | Poll::Ready(Ok(ProvisionEvent::BankReceipt(_))) => {}
                other => panic!("gear preparation must continue: {other:?}"),
            }
        }
        assert!(ready);
        assert_eq!(worn.len(), equipment.len());
        assert_eq!(food_drawn, 3);
        assert_eq!(pack.len(), 28);
        assert_eq!(provisioner.status().pack, 3);
        assert!(provisioner.status().need > 3);
    }

    #[test]
    fn declared_crossing_missing_food_records_shortfall_and_continues_to_admission() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (path, quests, snapshot) = survival_fixture(true);
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let memory = known_empty(0);
        let banks = Arc::new(NamedBankFacts::empty());
        let result = poll_once_at(
            &mut provisioner,
            &snapshot,
            &mut ledger,
            1,
            &path.provisioning,
            None,
            Some(&path.provisioning.stages[0]),
            &memory,
            &banks,
            &quests,
        );
        let status = provisioner.status();
        assert!(matches!(result, Poll::Ready(Ok(ProvisionEvent::Ready))));
        assert_eq!(status.phase, ProvisionPhase::Ready);
        assert!(status.item.is_some_and(|name| !name.is_empty()));
        assert!(status.need > 0);
        assert_eq!(status.pack, 0);
        assert_eq!(status.bank, Some(0));
        assert!(ledger
            .as_ref()
            .is_none_or(|ledger| ledger.outbox.is_empty()));
    }

    #[test]
    fn guide_without_risky_fights_or_crossings_adds_no_survival_withdrawal() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (path, quests, snapshot) = survival_fixture(false);
        assert!(path.provisioning.survival.is_none());
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let memory = BankMemory::default();
        let banks = Arc::new(NamedBankFacts::empty());
        assert!(matches!(
            poll_once_at(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                1,
                &path.provisioning,
                None,
                Some(&path.provisioning.stages[0]),
                &memory,
                &banks,
                &quests,
            ),
            Poll::Ready(Ok(ProvisionEvent::Ready))
        ));
        assert!(ledger.is_none());
    }

    #[test]
    fn recipe_capacity_uses_simultaneous_inputs_and_stackability() {
        let mut flour = compiled_item(
            100,
            "Flour",
            1,
            CompiledItemKind::Acquirable,
            Some("acquire:flour"),
        );
        flour.stackable = true;
        let plan = provisioning(vec![flour], None);
        let recipe = CompiledAcquireRecipe {
            steps: Arc::from(Vec::new()),
            peak_items: Arc::from([
                crate::quester::compile::CompiledRecipeItem {
                    item: crate::native_bank::BankItem {
                        id: 101,
                        name: Arc::from("Pot"),
                    },
                    qty: 1,
                    stackable: false,
                },
                crate::quester::compile::CompiledRecipeItem {
                    item: crate::native_bank::BankItem {
                        id: 102,
                        name: Arc::from("Grain"),
                    },
                    qty: 20,
                    stackable: true,
                },
            ]),
            consumed_ids: Arc::from([]),
        };
        let need = RecipeNeed {
            need: NeedObservation {
                id: 100,
                item: Arc::from("Flour"),
                need: 1,
                pack: 0,
                bank: Some(0),
                index: 0,
            },
            recipe: Arc::from("acquire:flour"),
        };
        let inventory = [
            item_view(201, "Egg", 1, ItemContainer::Inventory),
            item_view(202, "Milk", 1, ItemContainer::Inventory),
        ];
        let snapshot = ready_snapshot(Vec::new());
        let view = snapshot_view(&snapshot, 1);
        let stock = view.stock();
        let needs = Planner::new(&stock, WorldFacts::default());
        let inputs = planned_slots(
            &plan,
            &needs,
            &inventory,
            Some(&need),
            Some(&recipe),
            true,
            false,
        );
        let output = planned_slots(
            &plan,
            &needs,
            &inventory,
            Some(&need),
            Some(&recipe),
            false,
            true,
        );
        assert_eq!(
            inputs, 4,
            "pot and stackable grain add two rows to egg and milk"
        );
        assert_eq!(
            output, 5,
            "without explicit absence facts, retain both recipe inputs alongside flour"
        );
        assert_eq!(inputs.max(output), 5);

        let milk_plan = provisioning(
            vec![compiled_item(
                301,
                "Milk",
                1,
                CompiledItemKind::Acquirable,
                Some("acquire:milk"),
            )],
            None,
        );
        let bucket_input = crate::quester::compile::CompiledRecipeItem {
            item: crate::native_bank::BankItem {
                id: 302,
                name: Arc::from("Empty bucket"),
            },
            qty: 1,
            stackable: false,
        };
        let milk_recipe = CompiledAcquireRecipe {
            steps: Arc::from(Vec::new()),
            peak_items: Arc::from([bucket_input.clone()]),
            consumed_ids: Arc::from([]),
        };
        let milk_need = RecipeNeed {
            need: NeedObservation {
                id: 301,
                item: Arc::from("Milk"),
                need: 1,
                pack: 0,
                bank: Some(0),
                index: 0,
            },
            recipe: Arc::from("acquire:milk"),
        };
        let bucket = [item_view(302, "Empty bucket", 1, ItemContainer::Inventory)];
        assert_eq!(
            planned_slots(
                &milk_plan,
                &needs,
                &bucket,
                Some(&milk_need),
                Some(&milk_recipe),
                true,
                false,
            ),
            1
        );
        assert_eq!(
            planned_slots(
                &milk_plan,
                &needs,
                &bucket,
                Some(&milk_need),
                Some(&milk_recipe),
                false,
                true,
            ),
            2,
            "an omitted source from settle is retained beside the final output"
        );
        let bucket_proven_absent = CompiledAcquireRecipe {
            steps: Arc::from(Vec::new()),
            peak_items: Arc::from([bucket_input]),
            consumed_ids: Arc::from([302]),
        };
        assert_eq!(
            planned_slots(
                &milk_plan,
                &needs,
                &bucket,
                Some(&milk_need),
                Some(&bucket_proven_absent),
                false,
                true,
            ),
            1,
            "an explicit absence predicate proves the bucket row is released"
        );

        let mut partial_grain = item_view(303, "Grain", 5, ItemContainer::Inventory);
        partial_grain.def.stackable = true;
        let partial_grain = [partial_grain];
        let partial_recipe = CompiledAcquireRecipe {
            steps: Arc::from(Vec::new()),
            peak_items: Arc::from([crate::quester::compile::CompiledRecipeItem {
                item: crate::native_bank::BankItem {
                    id: 303,
                    name: Arc::from("Grain"),
                },
                qty: 1,
                stackable: true,
            }]),
            consumed_ids: Arc::from([]),
        };
        assert_eq!(
            planned_slots(
                &milk_plan,
                &needs,
                &partial_grain,
                Some(&milk_need),
                Some(&partial_recipe),
                false,
                true,
            ),
            2,
            "a partial source stack retains one row beside a new unstackable output"
        );

        let authored = provisioning(
            vec![compiled_item(
                401,
                "Wool",
                20,
                CompiledItemKind::Acquirable,
                None,
            )],
            None,
        );
        assert_eq!(
            planned_slots(&authored, &needs, &[], None, None, false, false),
            0,
            "authored acquisition goals do not arrive during preparation"
        );
    }

    fn known_empty(_id: i32) -> BankMemory {
        BankMemory::seeded(&[], Origin::Session)
    }

    fn tile(x: i32, z: i32) -> WorldTile {
        WorldTile { x, z, level: 0 }
    }

    fn bank_snapshot() -> GameSnapshot {
        let mut snapshot = ready_snapshot(Vec::new());
        snapshot.seed_local_player(local_player(tile(3100, 3240)));
        snapshot
    }

    fn poll_bank_pick(
        run: &mut AcquireRun,
        snapshot: &GameSnapshot,
        ledger: &mut Option<Box<ledger::Ledger>>,
    ) -> Result<BankPickRequest, ActionError> {
        let memo = BankMemory::default();
        for tick in 2..=5 {
            let result = with_tick_bank(snapshot, Some(&memo), ledger, tick, |native| {
                run.poll(native, OpenGate::Open)
            });
            if let Some(request) = ledger.as_ref().and_then(|ledger| {
                ledger
                    .outbox
                    .iter()
                    .find_map(|action| match &action.effect {
                        HostEffect::BankPick(request) => Some(request.clone()),
                        _ => None,
                    })
            }) {
                return Ok(request);
            }
            match result {
                Poll::Pending => {}
                Poll::Ready(Err(error)) => return Err(error),
                Poll::Ready(Ok(_)) => {
                    return Err(ActionError::Unavailable(Arc::from(
                        "bank run completed without selection",
                    )))
                }
            }
        }
        Err(ActionError::Unavailable(Arc::from(
            "bank selection did not emit a request",
        )))
    }

    #[test]
    fn authored_draynor_varrock_and_unmatched_tiles_select_without_explicit_bank() {
        if !api::test_support::game_data_available() {
            return;
        }
        let draynor = tile(3092, 3242);
        let varrock_west = tile(3185, 3438);
        let banks = Arc::new(NamedBankFacts::from_banks(vec![
            NamedBank::new("Draynor Village", draynor),
            NamedBank::new("Varrock West", varrock_west),
        ]));
        for authored in [draynor, varrock_west, tile(3200, 3200)] {
            let snapshot = bank_snapshot();
            let mut ledger = None;
            let memo = BankMemory::default();
            let mut run = with_tick_bank(&snapshot, Some(&memo), &mut ledger, 1, |native| {
                AcquireRun::new(
                    AcquireRequest::bank_actions(
                        Some(NamedBank::new("Path bank", authored)),
                        false,
                        Arc::from([BankAction::Scan]),
                        false,
                        native.cx.evidence(),
                    ),
                    crate::acquire::AcquirePlan::authored(),
                    &banks,
                    &native.cx,
                )
            });
            let request = poll_bank_pick(&mut run, &snapshot, &mut ledger).unwrap();
            assert!(
                request.explicit_bank.is_none(),
                "authored tile {authored:?} must not constrain nearest selection"
            );
        }
    }

    #[test]
    fn required_draynor_bank_is_explicit_and_unmatched_required_bank_refuses() {
        if !api::test_support::game_data_available() {
            return;
        }
        let draynor = tile(3092, 3242);
        let banks = Arc::new(NamedBankFacts::from_banks(vec![NamedBank::new(
            "Draynor Village",
            draynor,
        )]));
        let snapshot = bank_snapshot();
        let mut ledger = None;
        let memo = BankMemory::default();
        let mut run = with_tick_bank(&snapshot, Some(&memo), &mut ledger, 1, |native| {
            AcquireRun::new(
                AcquireRequest::bank_actions(
                    Some(NamedBank::new("Path bank", draynor)),
                    true,
                    Arc::from([BankAction::Scan]),
                    false,
                    native.cx.evidence(),
                ),
                crate::acquire::AcquirePlan::authored(),
                &banks,
                &native.cx,
            )
        });
        let request = poll_bank_pick(&mut run, &snapshot, &mut ledger).unwrap();
        assert_eq!(request.explicit_bank, Some(0));
        let mut open_snapshot = bank_snapshot();
        open_snapshot.seed_bank_observation(1, 1, Some(Vec::new()), Vec::new());
        let mut ledger = None;
        let mut run = with_tick_bank(&open_snapshot, Some(&memo), &mut ledger, 1, |native| {
            AcquireRun::new(
                AcquireRequest::bank_actions(
                    Some(NamedBank::new("Path bank", draynor)),
                    true,
                    Arc::from([BankAction::Scan]),
                    false,
                    native.cx.evidence(),
                ),
                crate::acquire::AcquirePlan::authored(),
                &banks,
                &native.cx,
            )
        });
        let request = poll_bank_pick(&mut run, &open_snapshot, &mut ledger).unwrap();
        assert_eq!(request.explicit_bank, Some(0));

        let unmatched = tile(3200, 3200);
        let mut ledger = None;
        let mut run = with_tick_bank(&snapshot, Some(&memo), &mut ledger, 1, |native| {
            AcquireRun::new(
                AcquireRequest::bank_actions(
                    Some(NamedBank::new("Path bank", unmatched)),
                    true,
                    Arc::from([BankAction::Scan]),
                    false,
                    native.cx.evidence(),
                ),
                crate::acquire::AcquirePlan::authored(),
                &banks,
                &native.cx,
            )
        });
        assert!(matches!(
            poll_bank_pick(&mut run, &snapshot, &mut ledger),
            Err(ActionError::Unavailable(reason))
                if reason.as_ref() == "required bank is not in the bank catalog"
        ));
        assert!(ledger
            .as_ref()
            .is_none_or(|ledger| ledger.outbox.is_empty()));
    }

    #[test]
    fn pack_sufficiency_does_not_open_an_unread_bank() {
        if !api::test_support::game_data_available() {
            return;
        }
        let plan = provisioning(
            vec![compiled_item(
                42,
                "Quest token",
                2,
                CompiledItemKind::MustHave,
                None,
            )],
            None,
        );
        let snapshot = ready_snapshot(vec![item_view(
            42,
            "Quest token",
            2,
            ItemContainer::Inventory,
        )]);
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let memo = BankMemory::default();
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        let quests = quest_catalog();

        assert!(matches!(
            poll_once(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                1,
                &plan,
                None,
                &memo,
                &banks,
                &quests,
            ),
            Poll::Ready(Ok(ProvisionEvent::Ready))
        ));
        assert_eq!(memo.count(42), None);
        assert!(ledger.is_none());
    }

    #[test]
    fn missing_preserved_tool_is_not_an_implicit_must_have_requirement() {
        if !api::test_support::game_data_available() {
            return;
        }
        let mut plan = provisioning(Vec::new(), None);
        plan.tools = Arc::from([crate::native_bank::BankItem {
            id: 42,
            name: Arc::from("Pot"),
        }]);
        let snapshot = ready_snapshot(Vec::new());
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let memo = BankMemory::default();
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        let quests = quest_catalog();
        assert!(matches!(
            poll_once(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                1,
                &plan,
                None,
                &memo,
                &banks,
                &quests,
            ),
            Poll::Ready(Ok(ProvisionEvent::Ready))
        ));
        assert!(ledger.is_none());
    }

    /// Cook's Path plus one level-1 woodcutting gather step, compiled: one
    /// gather-tool need whose tools include the bronze axe. The snapshot
    /// carries no axe at woodcutting 1 on a members world.
    fn woodcutting_tool_fixture() -> (Arc<CompiledPath>, QuestCatalog, GameSnapshot, i32) {
        let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let quests = QuestCatalog::from_identity(selected.quest_identity()).unwrap();
        let data = Arc::clone(&selected);
        let _gather_catalog = api::selected::FamilyPreparation::run(move |worker| {
            api::gather_methods::prepare(&data, worker)
        })
        .unwrap()
        .join()
        .unwrap()
        .unwrap();
        let mut document = compile::decode_cook().unwrap();
        let mut step = document.roles[0].sequences[0].steps[0].clone();
        step.id = FactKey::new("gather-normal-logs");
        step.kind = "gather".into();
        step.version = 1;
        step.args = serde_json::json!({
            "skill": "woodcutting",
            "resource": "normal",
            "until": { "obj": { "id": 1511 }, "qty": 3 }
        });
        step.advances = Some(false);
        step.skip_if = crate::quester::path::PredicateDocument::Any(Vec::new());
        step.settle = crate::quester::path::PredicateDocument::Any(Vec::new());
        document.roles[0].sequences[0].steps.push(step);
        let path = compile::compile_uncached_for_test(&document, &selected, &quests).unwrap();
        let axe_id = selected.item_by_alias("bronze_axe").unwrap().id;
        assert_eq!(path.provisioning.gather_tool_needs.len(), 1);
        assert!(path.provisioning.tools.iter().any(|tool| tool.id == axe_id));

        let mut snapshot = ready_snapshot(Vec::new());
        snapshot.seed_equipment(Vec::new());
        snapshot.seed_stats(vec![StatView {
            index: 8,
            name: "woodcutting".into(),
            effective: 1,
            base: 1,
            xp: 0,
            used: true,
        }]);
        snapshot.seed_world(WorldStateView {
            members: true,
            ..Default::default()
        });
        (path, quests, snapshot, axe_id)
    }

    #[test]
    fn banked_gather_tool_uses_the_shared_bank_withdrawal_run() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (path, quests, mut snapshot, axe_id) = woodcutting_tool_fixture();
        let memo = BankMemory::seeded(&[(axe_id, 1)], Origin::Session);
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let banks = pickable(&mut snapshot);
        assert!(poll_once(
            &mut provisioner,
            &snapshot,
            &mut ledger,
            1,
            &path.provisioning,
            None,
            &memo,
            &banks,
            &quests,
        )
        .is_pending());
        assert_eq!(provisioner.status().phase, ProvisionPhase::Withdrawing);
        let tool = path
            .provisioning
            .tools
            .iter()
            .find(|tool| tool.id == axe_id)
            .expect("bronze axe gather tool");
        let status = provisioner.status();
        assert_eq!(status.item, Some(tool.name.as_ref()));
        assert_eq!(status.need, 1);
        assert_eq!(status.bank, Some(1));
    }

    /// design-bank-snapshot §2.4: a `Hint` lacking every usable tool row is
    /// a Hint shortage like any other — the optional tool need costs the
    /// one verifying scan rather than being dropped before `require`. The
    /// scan is labelled with the strongest usable candidate, as an
    /// `Unknown` bank's is.
    #[test]
    fn hint_lacking_every_gather_tool_costs_the_verifying_scan() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (path, quests, mut snapshot, _) = woodcutting_tool_fixture();
        let memo = BankMemory::seeded(&[], Origin::Hint);
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let banks = pickable(&mut snapshot);
        assert!(poll_once(
            &mut provisioner,
            &snapshot,
            &mut ledger,
            1,
            &path.provisioning,
            None,
            &memo,
            &banks,
            &quests,
        )
        .is_pending());
        let status = provisioner.status();
        assert_eq!(status.phase, ProvisionPhase::Scanning);
        assert!(
            status.item.is_some_and(|item| {
                path.provisioning
                    .tools
                    .iter()
                    .any(|tool| tool.name.as_ref() == item)
            }),
            "the scan is for a hinted-absent axe: {:?}",
            status.item
        );
        assert_eq!(status.need, 1);
        assert_eq!(status.bank, Some(0));
    }

    #[test]
    fn acquirable_without_recipe_defers_to_authored_steps_after_bank_observation() {
        if !api::test_support::game_data_available() {
            return;
        }
        let plan = provisioning(
            vec![compiled_item(
                42,
                "Wool",
                20,
                CompiledItemKind::Acquirable,
                None,
            )],
            None,
        );
        let snapshot = ready_snapshot(Vec::new());
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let memo = known_empty(42);
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        let quests = quest_catalog();
        assert!(matches!(
            poll_once(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                1,
                &plan,
                None,
                &memo,
                &banks,
                &quests,
            ),
            Poll::Ready(Ok(ProvisionEvent::Ready))
        ));
        assert!(ledger.is_none());
    }

    #[test]
    fn unread_bank_is_scanned_before_a_must_have_can_be_blocked() {
        if !api::test_support::game_data_available() {
            return;
        }
        let plan = provisioning(
            vec![compiled_item(
                42,
                "Quest token",
                1,
                CompiledItemKind::MustHave,
                None,
            )],
            None,
        );
        let mut snapshot = ready_snapshot(Vec::new());
        snapshot.seed_local_player(local_player(WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        }));
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let memo = BankMemory::default();
        let banks = Arc::new(api::named_banks::NamedBankFacts::from_banks(vec![
            NamedBank::new(
                "Test bank",
                WorldTile {
                    x: 3210,
                    z: 3210,
                    level: 0,
                },
            ),
        ]));
        let quests = quest_catalog();

        assert!(poll_once(
            &mut provisioner,
            &snapshot,
            &mut ledger,
            1,
            &plan,
            None,
            &memo,
            &banks,
            &quests,
        )
        .is_pending());
        assert!(matches!(
            poll_once(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                2,
                &plan,
                None,
                &memo,
                &banks,
                &quests,
            ),
            Poll::Pending
        ));
        assert_eq!(memo.count(42), None);
        assert_eq!(provisioner.status().phase, ProvisionPhase::Scanning);
    }

    #[test]
    fn observed_empty_bank_blocks_a_must_have_shortfall() {
        if !api::test_support::game_data_available() {
            return;
        }
        let plan = provisioning(
            vec![compiled_item(
                42,
                "Quest token",
                2,
                CompiledItemKind::MustHave,
                None,
            )],
            None,
        );
        let snapshot = ready_snapshot(Vec::new());
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let memo = known_empty(42);
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        let quests = quest_catalog();

        assert!(matches!(
            poll_once(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                1,
                &plan,
                None,
                &memo,
                &banks,
                &quests,
            ),
            Poll::Ready(Ok(ProvisionEvent::Blocked { item }))
                if item.as_ref() == "Quest token x2"
        ));
        assert_eq!(provisioner.status().phase, ProvisionPhase::Blocked);
    }

    #[test]
    fn acquirable_shortfall_runs_its_recipe_without_invalidating_bank_memo() {
        if !api::test_support::game_data_available() {
            return;
        }
        let mut plan = provisioning(
            vec![compiled_item(
                42,
                "Quest token",
                1,
                CompiledItemKind::Acquirable,
                Some("acquire:token"),
            )],
            None,
        );
        plan.recipes.insert(
            Arc::from("acquire:token"),
            CompiledAcquireRecipe {
                steps: Arc::from(Vec::new()),
                peak_items: Arc::from([]),
                consumed_ids: Arc::from([]),
            },
        );
        let snapshot = ready_snapshot(Vec::new());
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let memo = known_empty(42);
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        let quests = quest_catalog();

        // The recipe is polled on the tick planning starts it (TICK-FIX #7,
        // E-Q7): an empty recipe is acquired on that same poll.
        assert!(matches!(
            poll_once(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                1,
                &plan,
                None,
                &memo,
                &banks,
                &quests,
            ),
            Poll::Ready(Ok(ProvisionEvent::Acquired))
        ));
        assert_eq!(provisioner.status().phase, ProvisionPhase::Acquiring);
    }

    #[test]
    fn duplicate_item_without_recipe_does_not_hide_a_later_acquisition_recipe() {
        if !api::test_support::game_data_available() {
            return;
        }
        let mut plan = provisioning(
            vec![
                compiled_item(42, "Quest token", 1, CompiledItemKind::Acquirable, None),
                compiled_item(
                    42,
                    "Quest token",
                    1,
                    CompiledItemKind::Acquirable,
                    Some("acquire:token"),
                ),
            ],
            None,
        );
        plan.recipes.insert(
            Arc::from("acquire:token"),
            CompiledAcquireRecipe {
                steps: Arc::from([]),
                peak_items: Arc::from([]),
                consumed_ids: Arc::from([]),
            },
        );
        let snapshot = ready_snapshot(Vec::new());
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        assert!(matches!(
            poll_once(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                1,
                &plan,
                None,
                &known_empty(42),
                &Arc::new(NamedBankFacts::empty()),
                &quest_catalog(),
            ),
            Poll::Ready(Ok(ProvisionEvent::Acquired))
        ));
        assert_eq!(provisioner.status().phase, ProvisionPhase::Acquiring);
    }

    #[test]
    fn float_latches_survive_consumption_and_reset_clears_them() {
        if !api::test_support::game_data_available() {
            return;
        }
        let coin = crate::native_bank::BankItem {
            id: 10,
            name: Arc::from("Coins"),
        };
        let food = crate::native_bank::BankItem {
            id: 42,
            name: Arc::from("Food"),
        };
        let mut plan = provisioning(Vec::new(), None);
        plan.coin_float = 100;
        plan.coin = Some(super::super::compile::CompiledCarry {
            item: coin,
            qty: 100,
            stackable: true,
            latch_index: u8::MAX,
        });
        plan.base_spillover_keep = Arc::from(vec![10, 42]);
        plan.loadout_carry.insert(
            Arc::from("provision-test/food"),
            Arc::from(vec![super::super::compile::CompiledCarry {
                item: food,
                qty: 3,
                stackable: false,
                latch_index: 0,
            }]),
        );
        let first = ready_snapshot(vec![
            item_view(10, "Coins", 100, ItemContainer::Inventory),
            item_view(42, "Food", 3, ItemContainer::Inventory),
        ]);
        let empty = ready_snapshot(Vec::new());
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let memo = known_empty(10);
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        let quests = quest_catalog();

        assert!(matches!(
            poll_once(
                &mut provisioner,
                &first,
                &mut ledger,
                1,
                &plan,
                Some("provision-test/food"),
                &memo,
                &banks,
                &quests,
            ),
            Poll::Ready(Ok(ProvisionEvent::Ready))
        ));
        assert!(provisioner.coin_drawn());
        assert_eq!(provisioner.carry_drawn(), 1);
        assert!(matches!(
            poll_once(
                &mut provisioner,
                &empty,
                &mut ledger,
                2,
                &plan,
                Some("provision-test/food"),
                &memo,
                &banks,
                &quests,
            ),
            Poll::Ready(Ok(ProvisionEvent::Ready))
        ));

        with_tick(&empty, &mut ledger, 3, |native: &mut NativeTick<'_>| {
            provisioner.reset(native.actions);
        });
        assert!(!provisioner.coin_drawn());
        assert_eq!(provisioner.carry_drawn(), 0);
    }

    #[test]
    fn r1_missing_coin_and_carry_floats_scan_unknown_bank_but_held_floats_do_not() {
        if !api::test_support::game_data_available() {
            return;
        }
        for (coin_qty, carry_qty) in [(100, 0), (0, 3), (100, 3)] {
            let mut plan = provisioning(Vec::new(), None);
            plan.coin_float = coin_qty;
            plan.coin = Some(super::super::compile::CompiledCarry {
                item: crate::native_bank::BankItem {
                    id: 10,
                    name: Arc::from("Coins"),
                },
                qty: coin_qty,
                stackable: true,
                latch_index: u8::MAX,
            });
            plan.loadout_carry.insert(
                Arc::from("food"),
                Arc::from([super::super::compile::CompiledCarry {
                    item: crate::native_bank::BankItem {
                        id: 42,
                        name: Arc::from("Food"),
                    },
                    qty: carry_qty,
                    stackable: false,
                    latch_index: 0,
                }]),
            );
            for held in [false, true] {
                let mut snapshot = ready_snapshot(if held {
                    vec![
                        item_view(10, "Coins", coin_qty, ItemContainer::Inventory),
                        item_view(42, "Food", carry_qty, ItemContainer::Inventory),
                    ]
                } else {
                    Vec::new()
                });
                let mut provisioner = Provisioner::new();
                let mut ledger = None;
                let memo = BankMemory::default();
                let banks = pickable(&mut snapshot);
                let quests = quest_catalog();
                let result = poll_once(
                    &mut provisioner,
                    &snapshot,
                    &mut ledger,
                    1,
                    &plan,
                    Some("food"),
                    &memo,
                    &banks,
                    &quests,
                );
                if held {
                    assert!(matches!(result, Poll::Ready(Ok(ProvisionEvent::Ready))));
                    assert!(ledger.is_none());
                } else {
                    assert!(result.is_pending(), "missing floats must observe the bank");
                    assert_eq!(provisioner.status().phase, ProvisionPhase::Scanning);
                }
            }
        }
    }

    #[test]
    fn owns_inventory_skips_prepare() {
        if !api::test_support::game_data_available() {
            return;
        }
        let mut plan = provisioning(Vec::new(), None);
        plan.owns_inventory = true;
        let snapshot = ready_snapshot(vec![item_view(
            9,
            "Unrelated item",
            4,
            ItemContainer::Inventory,
        )]);
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let memo = BankMemory::default();
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        let quests = quest_catalog();

        assert!(matches!(
            poll_once(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                1,
                &plan,
                None,
                &memo,
                &banks,
                &quests,
            ),
            Poll::Ready(Ok(ProvisionEvent::Ready))
        ));
        assert!(ledger.is_none());
    }

    #[test]
    fn staged_item_waits_for_its_stage_and_skips_the_unknown_bank_scan() {
        if !api::test_support::game_data_available() {
            return;
        }
        let mut item = compiled_item(
            42,
            "Quest token",
            1,
            CompiledItemKind::Acquirable,
            Some("acquire:token"),
        );
        item.from_stage = Some(FactKey::new("quest:1"));
        item.from_stage_index = Some(1);
        let mut plan = provisioning(vec![item], None);
        plan.stages = Arc::from(vec![FactKey::new("quest:0"), FactKey::new("quest:1")]);
        plan.recipes.insert(
            Arc::from("acquire:token"),
            CompiledAcquireRecipe {
                steps: Arc::from(Vec::new()),
                peak_items: Arc::from([]),
                consumed_ids: Arc::from([]),
            },
        );
        let mut snapshot = ready_snapshot(Vec::new());
        let banks = pickable(&mut snapshot);
        let quests = quest_catalog();
        let early = FactKey::new("quest:0");
        let gated = FactKey::new("quest:1");

        // Unknown and earlier stages: the item is not due, so the
        // Unknown bank is never scanned for it.
        for stage in [None, Some(&early)] {
            let mut provisioner = Provisioner::new();
            let mut ledger = None;
            let memo = BankMemory::default();
            assert!(
                matches!(
                    poll_once_at(
                        &mut provisioner,
                        &snapshot,
                        &mut ledger,
                        1,
                        &plan,
                        None,
                        stage,
                        &memo,
                        &banks,
                        &quests,
                    ),
                    Poll::Ready(Ok(ProvisionEvent::Ready))
                ),
                "gated item must produce no need before its stage"
            );
            assert!(ledger.is_none());
        }

        // At the gated stage the missing item scans the Unknown bank.
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let memo = BankMemory::default();
        assert!(poll_once_at(
            &mut provisioner,
            &snapshot,
            &mut ledger,
            1,
            &plan,
            None,
            Some(&gated),
            &memo,
            &banks,
            &quests,
        )
        .is_pending());
        assert_eq!(provisioner.status().phase, ProvisionPhase::Scanning);

        // With the bank observed empty the gated item runs its recipe.
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let memo = known_empty(42);
        // The empty recipe starts and finishes on the same poll (TICK-FIX #7).
        assert!(matches!(
            poll_once_at(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                1,
                &plan,
                None,
                Some(&gated),
                &memo,
                &banks,
                &quests,
            ),
            Poll::Ready(Ok(ProvisionEvent::Acquired))
        ));
        assert_eq!(provisioner.status().phase, ProvisionPhase::Acquiring);
    }

    /// A catalog bank and a player position, so a trip started by planning
    /// keeps its host pick pending: the trip is polled on its start tick
    /// (TICK-FIX #4/#7), and Select needs both to queue the pick.
    fn pickable(snapshot: &mut GameSnapshot) -> Arc<NamedBankFacts> {
        snapshot.seed_local_player(local_player(tile(3200, 3200)));
        Arc::new(NamedBankFacts::from_banks(vec![NamedBank::new(
            "Test bank",
            tile(3210, 3210),
        )]))
    }

    /// A fixture bank the provisioner can open offline: the player stands on
    /// the booth tile and the booth loc is posted, so a trip needs no walk.
    fn openable_bank_fixture() -> (GameSnapshot, NamedBank, Arc<NamedBankFacts>) {
        let stand = tile(3210, 3210);
        let bank = NamedBank::new("Test bank", stand);
        let banks = Arc::new(NamedBankFacts::from_banks(vec![bank]));
        let mut snapshot = ready_snapshot(Vec::new());
        snapshot.seed_local_player(local_player(stand));
        snapshot.seed_locs(vec![LocView {
            id: 2213,
            name: Some("Bank booth".into()),
            actions: vec![Some("Use-quickly".into())],
            tile: stand,
            distance: 0,
            typecode: 0,
            info: 0,
            description: None,
            layer: LocLayer::GroundDecoration,
            shape: 0,
            angle: 0,
            width: 1,
            length: 1,
            footprint_width: 1,
            footprint_length: 1,
            block_walk: false,
            block_range: false,
            active: true,
            animation: -1,
            map_function: -1,
            map_scene: -1,
            force_approach: 0,
        }]);
        (snapshot, bank, banks)
    }

    fn bank_row(id: i32, name: &str, count: i32) -> ItemView {
        ItemView {
            def: def(id, name),
            container: ItemContainer::Bank,
            action_family: ItemActionFamily::Component,
            slot: 0,
            count,
            actions: vec![Some("Withdraw-1".into())],
            component_id: 7,
        }
    }

    fn complete_bank_pick(
        ledger: &mut Option<Box<ledger::Ledger>>,
        bank: NamedBank,
        stand: WorldTile,
        tick: u64,
    ) {
        let ledger = ledger.as_mut().expect("bank trip queued a BankPick");
        let action = ledger.outbox.remove(0);
        let authority = action.authority();
        ledger.complete_bank_pick(
            &authority,
            crate::bank::BankPickReceipt {
                request_id: authority.request_id().get(),
                evidence: api::quest_progress::EvidenceStamp {
                    run: authority.run(),
                    tick,
                    sequence: tick,
                },
                outcome: crate::bank::BankPickOutcome::Selected(crate::bank::SelectedBank {
                    bank_index: 0,
                    access_tile: stand,
                    kind: crate::bank::PickKind::Reachable,
                    access: Some(Arc::new(crate::bank::BankStandAccess {
                        bank,
                        stand_tile: stand,
                        kind: crate::bank::AccessKind::Booth,
                        stand_op: 1,
                        name: None,
                        choose: None,
                    })),
                }),
            },
        );
    }

    fn complete_open_stand(ledger: &mut Option<Box<ledger::Ledger>>, tick: u64) {
        let ledger = ledger.as_mut().expect("bank trip queued an OpenStand");
        let action = ledger.outbox.remove(0);
        let authority = action.authority();
        ledger.complete_interaction(
            &authority,
            crate::native::InteractionReceipt {
                request_id: authority.request_id().get(),
                evidence: api::quest_progress::EvidenceStamp {
                    run: authority.run(),
                    tick,
                    sequence: tick,
                },
                accepted: true,
                chat_since: 0,
            },
        );
    }

    /// Poll a provisioner bank trip to its receipt, completing the host side
    /// of the pick and the open the way `BankSkipFixture::drive` does and
    /// seeding `stock` as the opened bank table.
    #[allow(clippy::too_many_arguments)] // Explicit fixture inputs mirror the production poll context.
    fn drive_trip_to_receipt(
        provisioner: &mut Provisioner,
        snapshot: &mut GameSnapshot,
        memory: &mut BankMemory,
        ledger: &mut Option<Box<ledger::Ledger>>,
        tick: &mut u64,
        plan: &CompiledProvisioning,
        banks: &Arc<NamedBankFacts>,
        quests: &QuestCatalog,
        bank: NamedBank,
        stand: WorldTile,
        stock: Option<Vec<ItemView>>,
    ) -> BankReceipt {
        for _ in 0..10 {
            *tick += 1;
            memory.track(snapshot, *tick);
            let result = poll_once(
                provisioner,
                snapshot,
                ledger,
                *tick,
                plan,
                None,
                memory,
                banks,
                quests,
            );
            let picked = ledger.as_ref().is_some_and(|ledger| {
                ledger
                    .outbox
                    .first()
                    .is_some_and(|action| matches!(&action.effect, HostEffect::BankPick(_)))
            });
            let opening = ledger.as_ref().is_some_and(|ledger| {
                ledger.outbox.first().is_some_and(|action| {
                    matches!(
                        &action.effect,
                        HostEffect::Interaction(crate::interact::InteractReq::OpenStand { .. })
                    )
                })
            });
            if picked {
                complete_bank_pick(ledger, bank, stand, *tick);
            } else if opening {
                complete_open_stand(ledger, *tick);
                snapshot.seed_bank_observation(1, *tick, stock.clone(), Vec::new());
            }
            match result {
                Poll::Ready(Ok(ProvisionEvent::BankReceipt(receipt))) => return receipt,
                Poll::Ready(ready) => panic!("bank trip ended without a receipt: {ready:?}"),
                Poll::Pending => {}
            }
        }
        panic!(
            "bank trip did not finish: phase {:?}",
            provisioner.status().phase
        );
    }

    #[test]
    fn authored_provisioning_returns_the_first_open_error_without_retrying() {
        if !api::test_support::game_data_available() {
            return;
        }
        for has_booth in [false, true] {
            let (mut snapshot, bank, banks) = openable_bank_fixture();
            if !has_booth {
                snapshot.seed_locs(Vec::new());
            }
            let plan = provisioning(
                vec![compiled_item(
                    42,
                    "Quest token",
                    1,
                    CompiledItemKind::MustHave,
                    None,
                )],
                Some(bank),
            );
            let mut provisioner = Provisioner::new();
            let mut ledger = None;
            let memory = BankMemory::default();
            let quests = quest_catalog();
            assert!(poll_once(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                1,
                &plan,
                None,
                &memory,
                &banks,
                &quests,
            )
            .is_pending());
            complete_bank_pick(&mut ledger, bank, bank.tile, 1);
            assert!(poll_once(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                2,
                &plan,
                None,
                &memory,
                &banks,
                &quests,
            )
            .is_pending());
            if has_booth {
                assert!(matches!(
                    &ledger.as_ref().unwrap().outbox[0].effect,
                    HostEffect::Interaction(crate::interact::InteractReq::OpenStand { .. })
                ));
                complete_open_stand(&mut ledger, 2);
            } else {
                assert!(ledger.as_ref().unwrap().outbox.is_empty());
            }
            let expected = if has_booth {
                "bank item table unavailable"
            } else {
                "bank did not open"
            };
            let result = poll_once(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                10,
                &plan,
                None,
                &memory,
                &banks,
                &quests,
            );
            assert!(
                matches!(&result, Poll::Ready(Err(ActionError::Failed(message))) if message.as_ref() == expected),
                "expected first open failure {expected:?}, got {result:?}"
            );
            assert!(ledger.as_ref().unwrap().outbox.is_empty());
            assert_eq!(provisioner.bank_starts(), 1);
        }
    }

    #[test]
    fn scan_spill_and_withdraw_preserve_the_provisioners_open_bank_lifetime() {
        if !api::test_support::game_data_available() {
            return;
        }
        for required in [false, true] {
            let (mut snapshot, bank, banks) = openable_bank_fixture();
            let stand = bank.tile;
            let mut plan = provisioning(
                vec![compiled_item(
                    42,
                    "Quest token",
                    1,
                    CompiledItemKind::MustHave,
                    None,
                )],
                Some(bank),
            );
            plan.bank_required = required;
            let mut pack: Vec<_> = [317, 321, 315]
                .into_iter()
                .enumerate()
                .map(|(slot, id)| {
                    let mut row = item_view(id, "Spill item", 1, ItemContainer::Inventory);
                    row.slot = slot as i32;
                    row
                })
                .collect();
            let side_rows = |pack: &[ItemView]| {
                pack.iter()
                    .map(|row| ItemView {
                        container: ItemContainer::BankSide,
                        action_family: ItemActionFamily::Component,
                        component_id: 8,
                        actions: vec![Some("Deposit-1".into())],
                        ..row.clone()
                    })
                    .collect::<Vec<_>>()
            };
            snapshot.seed_inventory(pack.clone(), 3);
            let mut bank_rows = vec![bank_row(42, "Quest token", 1)];
            let mut memory = BankMemory::seeded(&[], Origin::Hint);
            let mut ledger = None;
            let mut provisioner = Provisioner::new();
            let quests = quest_catalog();
            let mut phases = Vec::new();
            let mut receipts = Vec::new();
            let mut picks = 0;
            let mut deposits = 0;
            let mut withdrawals = 0;
            let mut closes = 0;
            let mut ready = false;

            for tick in 1..=64 {
                memory.track(&snapshot, tick);
                let result = poll_once(
                    &mut provisioner,
                    &snapshot,
                    &mut ledger,
                    tick,
                    &plan,
                    None,
                    &memory,
                    &banks,
                    &quests,
                );
                let phase = provisioner.status().phase;
                if phases.last() != Some(&phase) {
                    phases.push(phase);
                }
                match ledger
                    .as_ref()
                    .and_then(|ledger| ledger.outbox.first())
                    .map(|action| &action.effect)
                {
                    Some(HostEffect::BankPick(request)) => {
                        assert_eq!(request.explicit_bank, required.then_some(0));
                        picks += 1;
                        complete_bank_pick(&mut ledger, bank, stand, tick);
                    }
                    Some(HostEffect::Interaction(crate::interact::InteractReq::OpenStand {
                        ..
                    })) => {
                        complete_open_stand(&mut ledger, tick);
                        snapshot.seed_bank_observation(
                            1,
                            tick,
                            Some(bank_rows.clone()),
                            side_rows(&pack),
                        );
                    }
                    Some(HostEffect::Interaction(crate::interact::InteractReq::InvButton {
                        id,
                        ..
                    })) => {
                        assert_eq!(phase, ProvisionPhase::Spillover);
                        let id = *id;
                        let index = pack
                            .iter()
                            .position(|row| row.def.id == id)
                            .expect("deposit names a held row");
                        let row = pack.remove(index);
                        assert_eq!(row.count, 1);
                        deposits += 1;
                        bank_rows.push(bank_row(id, "Spill item", row.count));
                        complete_open_stand(&mut ledger, tick);
                        snapshot.seed_inventory(pack.clone(), 3);
                        snapshot.seed_bank_observation(
                            1,
                            tick,
                            Some(bank_rows.clone()),
                            side_rows(&pack),
                        );
                    }
                    Some(HostEffect::Interaction(crate::interact::InteractReq::SetNoteMode {
                        on: false,
                    })) => {
                        assert_eq!(phase, ProvisionPhase::Withdrawing);
                        complete_open_stand(&mut ledger, tick);
                    }
                    Some(HostEffect::Interaction(crate::interact::InteractReq::WithdrawX {
                        bank_item_id,
                        count,
                        ..
                    })) => {
                        assert_eq!(phase, ProvisionPhase::Withdrawing);
                        assert_eq!((*bank_item_id, *count), (42, 1));
                        withdrawals += 1;
                        let mut row = item_view(42, "Quest token", 1, ItemContainer::Inventory);
                        row.slot = (0..3)
                            .find(|slot| pack.iter().all(|row| row.slot != *slot))
                            .expect("spill freed a slot");
                        pack.push(row);
                        bank_rows.retain(|row| row.def.id != 42);
                        complete_open_stand(&mut ledger, tick);
                        snapshot.seed_inventory(pack.clone(), 3);
                        snapshot.seed_bank_observation(
                            1,
                            tick,
                            Some(bank_rows.clone()),
                            side_rows(&pack),
                        );
                    }
                    Some(HostEffect::Interaction(crate::interact::InteractReq::Close)) => {
                        assert_eq!(phase, ProvisionPhase::Closing);
                        closes += 1;
                        complete_open_stand(&mut ledger, tick);
                        snapshot.seed_bank_observation(-1, tick, None, Vec::new());
                    }
                    Some(_) => panic!("provisioning must not walk or issue another effect"),
                    None => {}
                }
                match result {
                    Poll::Ready(Ok(ProvisionEvent::BankReceipt(receipt))) => {
                        assert!(receipt.complete);
                        assert!(snapshot_view(&snapshot, tick)
                            .bank_session()
                            .is_some_and(|session| session.value.open));
                        receipts.push(receipt);
                    }
                    Poll::Ready(Ok(ProvisionEvent::Ready)) => {
                        ready = true;
                        break;
                    }
                    Poll::Ready(other) => {
                        panic!("provisioning did not complete its bank stages: {other:?}")
                    }
                    Poll::Pending => {}
                }
            }

            assert!(ready, "all three trips must finish");
            assert_eq!(
                phases,
                [
                    ProvisionPhase::Scanning,
                    ProvisionPhase::Spillover,
                    ProvisionPhase::Withdrawing,
                    ProvisionPhase::Closing,
                    ProvisionPhase::Ready
                ]
            );
            assert_eq!(receipts.len(), 3);
            assert_eq!(
                receipts
                    .iter()
                    .map(|receipt| receipt.deposited)
                    .collect::<Vec<_>>(),
                [0, 1, 0]
            );
            assert_eq!((deposits, withdrawals), (1, 1));
            assert_eq!(
                picks,
                if required { 3 } else { 1 },
                "a required bank re-selects; an ordinary trip reuses the open bank"
            );
            assert_eq!(provisioner.bank_starts(), 3);
            assert_eq!(provisioner.status().attempts, 3);
            assert_eq!(closes, 1, "one close, after the last trip");
            assert!(snapshot_view(&snapshot, 64)
                .bank_session()
                .is_some_and(|session| !session.value.open));
            assert!(ledger
                .as_ref()
                .is_none_or(|ledger| ledger.outbox.is_empty()));
        }
    }

    #[test]
    fn hint_shortage_costs_one_scan_then_withdraws_at_the_same_open_bank() {
        if !api::test_support::game_data_available() {
            return;
        }
        let plan = provisioning(
            vec![compiled_item(
                42,
                "Quest token",
                1,
                CompiledItemKind::MustHave,
                None,
            )],
            None,
        );
        let (mut snapshot, bank, banks) = openable_bank_fixture();
        let stand = tile(3210, 3210);
        let quests = quest_catalog();
        let mut memory = BankMemory::seeded(&[], Origin::Hint);
        let mut provisioner = Provisioner::new();
        let mut ledger = None;

        // A Hint-predicted shortage is advisory: one verifying scan trip,
        // never a block and never acquisition before the bank is seen.
        assert!(poll_once(
            &mut provisioner,
            &snapshot,
            &mut ledger,
            1,
            &plan,
            None,
            &memory,
            &banks,
            &quests,
        )
        .is_pending());
        assert_eq!(provisioner.status().phase, ProvisionPhase::Scanning);

        // The scan trip begins with bank selection: the run starts its
        // `Select` on the next poll and queues the pick on the one after.
        let mut tick = 1;
        for _ in 0..3 {
            tick += 1;
            memory.track(&snapshot, tick);
            assert!(poll_once(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                tick,
                &plan,
                None,
                &memory,
                &banks,
                &quests,
            )
            .is_pending());
            let picking = ledger.as_ref().is_some_and(|ledger| {
                matches!(
                    ledger.outbox.first().map(|action| &action.effect),
                    Some(HostEffect::BankPick(_))
                )
            });
            if picking {
                break;
            }
        }
        assert!(
            ledger.as_ref().is_some_and(|ledger| {
                matches!(
                    ledger.outbox.first().map(|action| &action.effect),
                    Some(HostEffect::BankPick(_))
                )
            }),
            "a Scan trip begins with bank selection"
        );
        complete_bank_pick(&mut ledger, bank, stand, tick);
        assert_eq!(provisioner.status().phase, ProvisionPhase::Scanning);

        let receipt = drive_trip_to_receipt(
            &mut provisioner,
            &mut snapshot,
            &mut memory,
            &mut ledger,
            &mut tick,
            &plan,
            &banks,
            &quests,
            bank,
            stand,
            Some(vec![bank_row(42, "Quest token", 1)]),
        );
        assert!(receipt.complete);
        assert_eq!(memory.origin(), Origin::Session);
        assert_eq!(memory.count(42), Some(1));

        // At the now-open Session bank the need plans a withdrawal that
        // reuses the open table: no second select, walk or open.
        tick += 1;
        memory.track(&snapshot, tick);
        assert!(poll_once(
            &mut provisioner,
            &snapshot,
            &mut ledger,
            tick,
            &plan,
            None,
            &memory,
            &banks,
            &quests,
        )
        .is_pending());
        assert_eq!(provisioner.status().phase, ProvisionPhase::Withdrawing);
        assert!(
            provisioner.bank_run.is_some(),
            "the withdrawal runs as a bank trip at the open bank"
        );
        // Three more polls reach the machine's first click at the open
        // table; a run that did not reuse the bank would have queued its
        // `BankPick` by then (the scan trip above needed two polls to).
        let mut first_click = None;
        for _ in 0..3 {
            tick += 1;
            memory.track(&snapshot, tick);
            let _ = poll_once(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                tick,
                &plan,
                None,
                &memory,
                &banks,
                &quests,
            );
            if let Some(ledger) = ledger.as_ref() {
                for action in &ledger.outbox {
                    assert!(
                        !matches!(
                            &action.effect,
                            HostEffect::BankPick(_)
                                | HostEffect::Walk(_)
                                | HostEffect::Interaction(
                                    crate::interact::InteractReq::OpenStand { .. }
                                )
                        ),
                        "the withdrawal must not start a second bank trip"
                    );
                    if let HostEffect::Interaction(request) = &action.effect {
                        first_click.get_or_insert_with(|| format!("{request:?}"));
                    }
                }
            }
        }
        let first_click = first_click.expect("the withdraw acts at the open bank");
        assert!(
            first_click.contains("SetNoteMode") || first_click.contains("WithdrawX"),
            "the first effect is a bank click, not a trip: {first_click}"
        );
    }

    #[test]
    fn stale_hint_stock_costs_one_withdraw_trip_then_session_zero_then_acquisition() {
        if !api::test_support::game_data_available() {
            return;
        }
        let mut plan = provisioning(
            vec![compiled_item(
                42,
                "Quest token",
                1,
                CompiledItemKind::Acquirable,
                Some("acquire:token"),
            )],
            None,
        );
        plan.recipes.insert(
            Arc::from("acquire:token"),
            CompiledAcquireRecipe {
                steps: Arc::from(Vec::new()),
                peak_items: Arc::from([]),
                consumed_ids: Arc::from([]),
            },
        );
        let (mut snapshot, bank, banks) = openable_bank_fixture();
        let stand = tile(3210, 3210);
        let quests = quest_catalog();
        let mut memory = BankMemory::seeded(&[(42, 10)], Origin::Hint);
        let mut provisioner = Provisioner::new();
        let mut ledger = None;

        // The hint claims stock, so the need plans a withdrawal trip rather
        // than a scan.
        assert!(poll_once(
            &mut provisioner,
            &snapshot,
            &mut ledger,
            1,
            &plan,
            None,
            &memory,
            &banks,
            &quests,
        )
        .is_pending());
        assert_eq!(provisioner.status().phase, ProvisionPhase::Withdrawing);

        let mut tick = 1;
        for _ in 0..3 {
            tick += 1;
            memory.track(&snapshot, tick);
            assert!(poll_once(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                tick,
                &plan,
                None,
                &memory,
                &banks,
                &quests,
            )
            .is_pending());
            let picking = ledger.as_ref().is_some_and(|ledger| {
                matches!(
                    ledger.outbox.first().map(|action| &action.effect),
                    Some(HostEffect::BankPick(_))
                )
            });
            if picking {
                break;
            }
        }
        assert!(
            ledger.as_ref().is_some_and(|ledger| {
                matches!(
                    ledger.outbox.first().map(|action| &action.effect),
                    Some(HostEffect::BankPick(_))
                )
            }),
            "a WithdrawTo trip begins with bank selection"
        );
        complete_bank_pick(&mut ledger, bank, stand, tick);
        assert_eq!(provisioner.status().phase, ProvisionPhase::Withdrawing);

        // The bank opens empty: the withdrawal cannot cover the need, and
        // the memory is now a Session zero.
        let receipt = drive_trip_to_receipt(
            &mut provisioner,
            &mut snapshot,
            &mut memory,
            &mut ledger,
            &mut tick,
            &plan,
            &banks,
            &quests,
            bank,
            stand,
            Some(Vec::new()),
        );
        assert!(!receipt.complete);
        assert_eq!(memory.origin(), Origin::Session);
        assert_eq!(memory.count(42), Some(0));

        // A Session-known shortage of an Acquirable runs its recipe: no
        // block in place and no second bank trip. The bank session ends
        // first, so the recipe's steps never act under the bank interface.
        tick += 1;
        memory.track(&snapshot, tick);
        assert!(poll_once(
            &mut provisioner,
            &snapshot,
            &mut ledger,
            tick,
            &plan,
            None,
            &memory,
            &banks,
            &quests,
        )
        .is_pending());
        assert_eq!(provisioner.status().phase, ProvisionPhase::Closing);
        assert!(matches!(
            ledger
                .as_ref()
                .and_then(|ledger| ledger.outbox.first())
                .map(|action| &action.effect),
            Some(HostEffect::Interaction(crate::interact::InteractReq::Close))
        ));
        complete_open_stand(&mut ledger, tick);
        snapshot.seed_bank_observation(-1, tick, None, Vec::new());
        tick += 1;
        memory.track(&snapshot, tick);
        // The empty recipe starts and finishes on that poll (TICK-FIX #7).
        assert!(matches!(
            poll_once(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                tick,
                &plan,
                None,
                &memory,
                &banks,
                &quests,
            ),
            Poll::Ready(Ok(ProvisionEvent::Acquired))
        ));
        assert_eq!(provisioner.status().phase, ProvisionPhase::Acquiring);
        assert!(
            provisioner.bank_run.is_none(),
            "the recipe must not start a second bank trip"
        );
        assert!(
            !ledger.as_ref().is_some_and(|ledger| ledger
                .outbox
                .iter()
                .any(|action| { matches!(&action.effect, HostEffect::BankPick(_)) })),
            "the recipe must not queue bank selection"
        );
    }

    #[test]
    fn session_known_shortage_of_a_must_have_blocks_in_place() {
        if !api::test_support::game_data_available() {
            return;
        }
        let plan = provisioning(
            vec![compiled_item(
                42,
                "Quest token",
                2,
                CompiledItemKind::MustHave,
                None,
            )],
            None,
        );
        let snapshot = ready_snapshot(Vec::new());
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let memory = BankMemory::seeded(&[], Origin::Session);
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        let quests = quest_catalog();

        assert!(matches!(
            poll_once(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                1,
                &plan,
                None,
                &memory,
                &banks,
                &quests,
            ),
            Poll::Ready(Ok(ProvisionEvent::Blocked { item }))
                if item.as_ref() == "Quest token x2"
        ));
        assert_eq!(provisioner.status().phase, ProvisionPhase::Blocked);
        assert!(ledger.is_none(), "a Session block queues no bank trip");
    }

    /// design-bank-snapshot §2.4 (D4): a `Session` bank that covers only
    /// part of a MustHave (target 2, carried 0, banked 1) still proves the
    /// target impossible — the block is in place, not after a withdrawal
    /// trip for the part it does hold.
    #[test]
    fn partially_banked_session_must_have_blocks_in_place_without_a_withdrawal() {
        if !api::test_support::game_data_available() {
            return;
        }
        let plan = provisioning(
            vec![compiled_item(
                42,
                "Quest token",
                2,
                CompiledItemKind::MustHave,
                None,
            )],
            None,
        );
        let snapshot = ready_snapshot(Vec::new());
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let memory = BankMemory::seeded(&[(42, 1)], Origin::Session);
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        let quests = quest_catalog();

        assert!(matches!(
            poll_once(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                1,
                &plan,
                None,
                &memory,
                &banks,
                &quests,
            ),
            Poll::Ready(Ok(ProvisionEvent::Blocked { item }))
                if item.as_ref() == "Quest token x2"
        ));
        let status = provisioner.status();
        assert_eq!(status.phase, ProvisionPhase::Blocked);
        assert_eq!(status.bank, Some(1), "the block reports the partial stock");
        assert!(
            ledger.is_none(),
            "no withdrawal trip starts for the part the bank holds"
        );
    }

    #[test]
    fn worn_must_have_is_carried_and_not_withdrawn() {
        if !api::test_support::game_data_available() {
            return;
        }
        let plan = provisioning(
            vec![compiled_item(
                42,
                "Quest token",
                1,
                CompiledItemKind::MustHave,
                None,
            )],
            None,
        );
        let mut snapshot = ready_snapshot(Vec::new());
        snapshot.seed_equipment(vec![item_view(
            42,
            "Quest token",
            1,
            ItemContainer::Equipment,
        )]);
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let memory = BankMemory::seeded(&[(42, 1)], Origin::Session);
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        let quests = quest_catalog();

        assert!(matches!(
            poll_once(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                1,
                &plan,
                None,
                &memory,
                &banks,
                &quests,
            ),
            Poll::Ready(Ok(ProvisionEvent::Ready))
        ));
        assert_eq!(provisioner.status().phase, ProvisionPhase::Ready);
        assert!(ledger.is_none(), "a worn MustHave queues no withdrawal");
    }

    /// The host side of one provisioning bank session on the fixture bank.
    /// It completes each effect the provisioner sends, moves a withdrawn
    /// token from the bank rows to the pack, and takes the bank interface
    /// away when the provisioner closes it.
    struct SessionRun {
        /// The effects the provisioner sent, in order.
        effects: Vec<&'static str>,
        /// Whether the bank was open on the tick provisioning returned `Ready`.
        open_at_ready: Option<bool>,
    }

    fn run_session(
        plan: &CompiledProvisioning,
        mut memory: BankMemory,
        mut bank_rows: Vec<ItemView>,
        open_at_start: bool,
    ) -> SessionRun {
        enum Seen {
            Pick,
            Open,
            Note,
            Withdraw,
            Close,
        }
        let (mut snapshot, bank, banks) = openable_bank_fixture();
        if open_at_start {
            snapshot.seed_bank_observation(1, 0, Some(bank_rows.clone()), Vec::new());
        }
        let stand = bank.tile;
        let quests = quest_catalog();
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let mut pack: Vec<ItemView> = Vec::new();
        let mut run = SessionRun {
            effects: Vec::new(),
            open_at_ready: None,
        };
        for tick in 1..=80 {
            memory.track(&snapshot, tick);
            let result = poll_once(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                tick,
                plan,
                None,
                &memory,
                &banks,
                &quests,
            );
            let seen = ledger
                .as_ref()
                .and_then(|ledger| ledger.outbox.first())
                .map(|action| match &action.effect {
                    HostEffect::BankPick(_) => Seen::Pick,
                    HostEffect::Interaction(crate::interact::InteractReq::OpenStand { .. }) => {
                        Seen::Open
                    }
                    HostEffect::Interaction(crate::interact::InteractReq::SetNoteMode {
                        ..
                    }) => Seen::Note,
                    HostEffect::Interaction(
                        crate::interact::InteractReq::Withdraw { .. }
                        | crate::interact::InteractReq::WithdrawX { .. },
                    ) => Seen::Withdraw,
                    HostEffect::Interaction(crate::interact::InteractReq::Close) => Seen::Close,
                    HostEffect::Interaction(request) => {
                        panic!("unexpected interaction {request:?}")
                    }
                    _ => panic!("unexpected non-interaction effect"),
                });
            match seen {
                Some(Seen::Pick) => {
                    run.effects.push("pick");
                    complete_bank_pick(&mut ledger, bank, stand, tick);
                }
                Some(Seen::Open) => {
                    run.effects.push("open");
                    complete_open_stand(&mut ledger, tick);
                    snapshot.seed_bank_observation(1, tick, Some(bank_rows.clone()), Vec::new());
                }
                Some(Seen::Note) => {
                    run.effects.push("note");
                    complete_open_stand(&mut ledger, tick);
                }
                Some(Seen::Withdraw) => {
                    run.effects.push("withdraw");
                    pack.push(item_view(42, "Quest token", 1, ItemContainer::Inventory));
                    bank_rows.retain(|row| row.def.id != 42);
                    complete_open_stand(&mut ledger, tick);
                    snapshot.seed_inventory(pack.clone(), 28);
                    snapshot.seed_bank_observation(1, tick, Some(bank_rows.clone()), Vec::new());
                }
                Some(Seen::Close) => {
                    run.effects.push("close");
                    complete_open_stand(&mut ledger, tick);
                    snapshot.seed_bank_observation(-1, tick, None, Vec::new());
                }
                None => {}
            }
            match result {
                Poll::Pending | Poll::Ready(Ok(ProvisionEvent::BankReceipt(_))) => {}
                Poll::Ready(Ok(ProvisionEvent::Ready)) => {
                    run.open_at_ready = Some(
                        snapshot_view(&snapshot, tick)
                            .bank_session()
                            .is_some_and(|session| session.value.open),
                    );
                    break;
                }
                Poll::Ready(other) => panic!("provisioning ended without Ready: {other:?}"),
            }
        }
        run
    }

    fn coin_float_plan(qty: i32) -> CompiledProvisioning {
        let mut plan = provisioning(Vec::new(), None);
        plan.coin_float = qty;
        plan.coin = Some(super::super::compile::CompiledCarry {
            item: crate::native_bank::BankItem {
                id: 10,
                name: Arc::from("Coins"),
            },
            qty,
            stackable: true,
            latch_index: u8::MAX,
        });
        plan
    }

    #[test]
    fn scan_then_ready_closes_the_bank_before_the_next_step_acts() {
        if !api::test_support::game_data_available() {
            return;
        }
        // The empty-bank Sheep Shearer case: the scan finds no coins, no
        // withdrawal follows, and the Path's next step is not a bank step.
        let run = run_session(
            &coin_float_plan(20),
            BankMemory::default(),
            Vec::new(),
            false,
        );
        assert_eq!(
            run.open_at_ready,
            Some(false),
            "provisioning must not hand control to a step under the bank interface: {:?}",
            run.effects
        );
        assert_eq!(run.effects.iter().filter(|e| **e == "close").count(), 1);
        assert_eq!(run.effects.last(), Some(&"close"));
    }

    #[test]
    fn scan_then_withdraw_closes_once_after_the_withdrawal() {
        if !api::test_support::game_data_available() {
            return;
        }
        let plan = provisioning(
            vec![compiled_item(
                42,
                "Quest token",
                1,
                CompiledItemKind::MustHave,
                None,
            )],
            None,
        );
        let run = run_session(
            &plan,
            BankMemory::seeded(&[], Origin::Hint),
            vec![bank_row(42, "Quest token", 1)],
            false,
        );
        let position = |name: &str| run.effects.iter().position(|effect| *effect == name);
        let open = position("open").expect("the scan opens the bank");
        let withdraw = position("withdraw").expect("the bank holds the token");
        let close = position("close").expect("the session ends with a close");
        assert!(
            !run.effects[open..withdraw].contains(&"close"),
            "no close between the scan and the withdrawal: {:?}",
            run.effects
        );
        assert!(withdraw < close, "the close follows the withdrawal");
        assert_eq!(run.effects.iter().filter(|e| **e == "close").count(), 1);
        assert_eq!(run.effects.last(), Some(&"close"));
        assert_eq!(run.open_at_ready, Some(false));
    }

    #[test]
    fn a_bank_open_before_the_first_trip_is_left_open() {
        if !api::test_support::game_data_available() {
            return;
        }
        // The player (or an earlier step) opened this bank: the provisioner
        // withdraws at it and never closes it, since the session is not its own.
        let plan = provisioning(
            vec![compiled_item(
                42,
                "Quest token",
                1,
                CompiledItemKind::MustHave,
                None,
            )],
            None,
        );
        let run = run_session(
            &plan,
            BankMemory::default(),
            vec![bank_row(42, "Quest token", 1)],
            true,
        );
        assert!(
            run.effects.contains(&"withdraw"),
            "the provisioner withdraws at the open bank: {:?}",
            run.effects
        );
        assert_eq!(run.open_at_ready, Some(true));
        assert!(
            !run.effects.contains(&"close"),
            "a bank the provisioner did not open is not closed by it: {:?}",
            run.effects
        );
    }

    #[test]
    fn ready_with_an_unmet_optional_need_logs_the_need_and_the_shortfall_once() {
        if !api::test_support::game_data_available() {
            return;
        }
        const SLOT: &str = "status-provision-unmet-supply";
        crate::slot::compiled::tests::install_status_line_sink();
        api::hostlog::bind_slot(SLOT);
        let plan = coin_float_plan(20);
        let snapshot = ready_snapshot(Vec::new());
        let memo = known_empty(10);
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        let quests = quest_catalog();
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        for tick in 1..=3 {
            assert!(matches!(
                poll_once(
                    &mut provisioner,
                    &snapshot,
                    &mut ledger,
                    tick,
                    &plan,
                    None,
                    &memo,
                    &banks,
                    &quests,
                ),
                Poll::Ready(Ok(ProvisionEvent::Ready))
            ));
        }
        let lines: Vec<_> = crate::slot::compiled::tests::status_lines_for(SLOT)
            .into_iter()
            .filter(|line| line.message.contains("optional need unmet"))
            .collect();
        assert_eq!(lines.len(), 1, "one line, not one per poll: {lines:?}");
        assert_eq!(lines[0].level, api::hostlog::Level::Info);
        assert!(
            lines[0]
                .message
                .ends_with("Coins needs 20, holds 0, short 20 (nothing more to draw it from)"),
            "{}",
            lines[0].message
        );
    }

    #[test]
    fn a_float_left_out_for_pack_room_logs_the_shortfall() {
        if !api::test_support::game_data_available() {
            return;
        }
        const SLOT: &str = "status-provision-unmet-room";
        crate::slot::compiled::tests::install_status_line_sink();
        api::hostlog::bind_slot(SLOT);
        let mut plan = coin_float_plan(20);
        plan.base_spillover_keep = Arc::from([42]);
        let mut snapshot = ready_snapshot(Vec::new());
        snapshot.seed_inventory(vec![item_view(42, "Kept", 1, ItemContainer::Inventory)], 1);
        let memo = BankMemory::seeded(&[(10, 5)], Origin::Session);
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        let quests = quest_catalog();
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        assert!(matches!(
            poll_once(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                1,
                &plan,
                None,
                &memo,
                &banks,
                &quests,
            ),
            Poll::Ready(Ok(ProvisionEvent::Ready))
        ));
        let lines: Vec<_> = crate::slot::compiled::tests::status_lines_for(SLOT)
            .into_iter()
            .filter(|line| line.message.contains("optional need unmet"))
            .collect();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0]
                .message
                .ends_with("Coins needs 20, holds 0, short 20 (no free pack slot for it)"),
            "{}",
            lines[0].message
        );
    }

    fn scan_plan(bank: NamedBank) -> CompiledProvisioning {
        provisioning(
            vec![compiled_item(
                42,
                "Quest token",
                1,
                CompiledItemKind::MustHave,
                None,
            )],
            Some(bank),
        )
    }

    /// A plan that needs no bank trip, so polling it only ends a session.
    fn owned_plan() -> CompiledProvisioning {
        let mut plan = provisioning(Vec::new(), None);
        plan.owns_inventory = true;
        plan
    }

    fn closes_sent(ledger: &Option<Box<ledger::Ledger>>) -> usize {
        ledger.as_ref().map_or(0, |ledger| {
            ledger
                .outbox
                .iter()
                .filter(|action| {
                    matches!(
                        &action.effect,
                        HostEffect::Interaction(crate::interact::InteractReq::Close)
                    )
                })
                .count()
        })
    }

    /// A scan trip to the point where a frame shows the bank open and the
    /// kernel waits on a table that never arrives. Returns the tick reached.
    #[allow(clippy::too_many_arguments)] // Explicit fixture inputs mirror the production poll context.
    fn open_the_bank_without_a_table(
        provisioner: &mut Provisioner,
        snapshot: &mut GameSnapshot,
        ledger: &mut Option<Box<ledger::Ledger>>,
        plan: &CompiledProvisioning,
        banks: &Arc<NamedBankFacts>,
        bank: NamedBank,
    ) -> u64 {
        let memory = BankMemory::default();
        let quests = quest_catalog();
        assert!(poll_once(
            provisioner,
            snapshot,
            ledger,
            1,
            plan,
            None,
            &memory,
            banks,
            &quests
        )
        .is_pending());
        complete_bank_pick(ledger, bank, bank.tile, 1);
        assert!(poll_once(
            provisioner,
            snapshot,
            ledger,
            2,
            plan,
            None,
            &memory,
            banks,
            &quests
        )
        .is_pending());
        complete_open_stand(ledger, 2);
        snapshot.seed_bank_observation(1, 2, None, Vec::new());
        assert!(poll_once(
            provisioner,
            snapshot,
            ledger,
            3,
            plan,
            None,
            &memory,
            banks,
            &quests
        )
        .is_pending());
        3
    }

    #[test]
    fn a_terminal_error_with_the_bank_open_ends_the_session_and_keeps_the_lease() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (mut snapshot, bank, banks) = openable_bank_fixture();
        let plan = scan_plan(bank);
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let reached = open_the_bank_without_a_table(
            &mut provisioner,
            &mut snapshot,
            &mut ledger,
            &plan,
            &banks,
            bank,
        );
        assert!(provisioner.session_open());
        assert_eq!(
            provisioner.bank_lease(),
            Some(BankLease::new(1, snapshot.bank_session_generation())),
            "a frame showed the bank open after a trip that began with it shut"
        );
        let result = poll_once(
            &mut provisioner,
            &snapshot,
            &mut ledger,
            reached + 10,
            &plan,
            None,
            &BankMemory::default(),
            &banks,
            &quest_catalog(),
        );
        assert!(matches!(result, Poll::Ready(Err(_))), "{result:?}");
        assert!(
            !provisioner.session_open(),
            "the error ended the session without a close"
        );
        assert_eq!(
            closes_sent(&ledger),
            0,
            "the provisioner sends no close here"
        );
        assert!(provisioner.take_unowned_lease().is_some());
        assert_eq!(provisioner.take_unowned_lease(), None, "handed over once");
    }

    #[test]
    fn cancel_keeps_a_lease_a_frame_showed_and_drops_an_unobserved_open() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (mut snapshot, bank, banks) = openable_bank_fixture();
        let plan = scan_plan(bank);
        let memory = BankMemory::default();
        let quests = quest_catalog();

        // Cancelled before any frame showed the bank open: nothing is owned.
        let mut early = Provisioner::new();
        let mut ledger = None;
        assert!(poll_once(
            &mut early,
            &snapshot,
            &mut ledger,
            1,
            &plan,
            None,
            &memory,
            &banks,
            &quests
        )
        .is_pending());
        early.cancel();
        snapshot.seed_bank_observation(1, 2, Some(Vec::new()), Vec::new());
        assert!(poll_once(
            &mut early,
            &snapshot,
            &mut ledger,
            2,
            &owned_plan(),
            None,
            &memory,
            &banks,
            &quests
        )
        .is_ready());
        assert_eq!(early.bank_lease(), None);

        // Cancelled with the bank observed open: the bank is still ours.
        let (mut snapshot, bank, banks) = openable_bank_fixture();
        let mut late = Provisioner::new();
        let mut ledger = None;
        open_the_bank_without_a_table(&mut late, &mut snapshot, &mut ledger, &plan, &banks, bank);
        late.cancel();
        assert!(!late.session_open());
        assert!(late.take_unowned_lease().is_some());
    }

    #[test]
    fn a_trip_cancelled_before_the_bank_opened_does_not_own_a_bank_the_player_opens() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (mut snapshot, bank, banks) = openable_bank_fixture();
        let plan = scan_plan(bank);
        let memory = BankMemory::default();
        let quests = quest_catalog();
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        assert!(poll_once(
            &mut provisioner,
            &snapshot,
            &mut ledger,
            1,
            &plan,
            None,
            &memory,
            &banks,
            &quests
        )
        .is_pending());
        provisioner.cancel();
        ledger.as_mut().unwrap().outbox.clear();
        snapshot.seed_bank_observation(1, 2, Some(Vec::new()), Vec::new());
        let result = poll_once(
            &mut provisioner,
            &snapshot,
            &mut ledger,
            2,
            &owned_plan(),
            None,
            &memory,
            &banks,
            &quests,
        );
        assert!(matches!(result, Poll::Ready(Ok(ProvisionEvent::Ready))));
        assert_eq!(closes_sent(&ledger), 0, "the player's bank stays open");
        assert_eq!(provisioner.bank_lease(), None);
    }

    #[test]
    fn a_replaced_bank_session_is_not_closed_by_a_stale_lease() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (mut snapshot, bank, banks) = openable_bank_fixture();
        let plan = scan_plan(bank);
        let quests = quest_catalog();
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let mut memory = BankMemory::default();
        let mut tick = 0;
        drive_trip_to_receipt(
            &mut provisioner,
            &mut snapshot,
            &mut memory,
            &mut ledger,
            &mut tick,
            &plan,
            &banks,
            &quests,
            bank,
            bank.tile,
            Some(Vec::new()),
        );
        assert!(provisioner.bank_lease().is_some());
        // The runner abandons the session, then the bank is shut and opened
        // again by someone else: a different bank session generation.
        provisioner.cancel();
        snapshot.seed_bank_observation(-1, tick + 1, None, Vec::new());
        snapshot.seed_bank_observation(1, tick + 2, Some(Vec::new()), Vec::new());
        ledger.as_mut().unwrap().outbox.clear();
        let result = poll_once(
            &mut provisioner,
            &snapshot,
            &mut ledger,
            tick + 3,
            &owned_plan(),
            None,
            &memory,
            &banks,
            &quests,
        );
        assert!(matches!(result, Poll::Ready(Ok(ProvisionEvent::Ready))));
        assert_eq!(closes_sent(&ledger), 0);
        assert_eq!(provisioner.bank_lease(), None, "the stale lease is dropped");
    }

    #[test]
    fn a_lease_from_another_login_session_is_dropped_without_a_close() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (mut snapshot, bank, banks) = openable_bank_fixture();
        let plan = scan_plan(bank);
        let quests = quest_catalog();
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let mut memory = BankMemory::default();
        let mut tick = 0;
        drive_trip_to_receipt(
            &mut provisioner,
            &mut snapshot,
            &mut memory,
            &mut ledger,
            &mut tick,
            &plan,
            &banks,
            &quests,
            bank,
            bank.tile,
            Some(Vec::new()),
        );
        // The test frames carry login session 1. The lease was made in session 2.
        provisioner.cancel();
        provisioner.lease = Some(BankLease::new(2, snapshot.bank_session_generation()));
        ledger.as_mut().unwrap().outbox.clear();
        let result = poll_once(
            &mut provisioner,
            &snapshot,
            &mut ledger,
            tick + 1,
            &owned_plan(),
            None,
            &memory,
            &banks,
            &quests,
        );
        assert!(matches!(result, Poll::Ready(Ok(ProvisionEvent::Ready))));
        assert_eq!(closes_sent(&ledger), 0);
        assert_eq!(provisioner.bank_lease(), None);
    }

    /// The state of a session that left its bank open between two trips: the
    /// first trip's lease names the bank session `generation`.
    fn session_with_a_leased_bank() -> (
        Provisioner,
        GameSnapshot,
        Option<Box<ledger::Ledger>>,
        BankMemory,
        u64,
        Arc<NamedBankFacts>,
    ) {
        let (mut snapshot, bank, banks) = openable_bank_fixture();
        let plan = scan_plan(bank);
        let quests = quest_catalog();
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let mut memory = BankMemory::default();
        let mut tick = 0;
        drive_trip_to_receipt(
            &mut provisioner,
            &mut snapshot,
            &mut memory,
            &mut ledger,
            &mut tick,
            &plan,
            &banks,
            &quests,
            bank,
            bank.tile,
            Some(Vec::new()),
        );
        assert!(provisioner.session_open());
        assert_eq!(
            provisioner.bank_lease(),
            Some(BankLease::new(1, snapshot.bank_session_generation())),
            "the bank the kernel opened is leased"
        );
        (provisioner, snapshot, ledger, memory, tick, banks)
    }

    #[test]
    fn a_bank_the_player_reopens_between_trips_is_not_closed() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (mut provisioner, mut snapshot, mut ledger, memory, tick, banks) =
            session_with_a_leased_bank();
        let quests = quest_catalog();
        // The player shuts the bank and opens it again: a new bank session.
        snapshot.seed_bank_observation(-1, tick + 1, None, Vec::new());
        snapshot.seed_bank_observation(1, tick + 2, Some(Vec::new()), Vec::new());
        ledger.as_mut().unwrap().outbox.clear();
        let result = poll_once(
            &mut provisioner,
            &snapshot,
            &mut ledger,
            tick + 3,
            &owned_plan(),
            None,
            &memory,
            &banks,
            &quests,
        );
        assert!(matches!(result, Poll::Ready(Ok(ProvisionEvent::Ready))));
        assert_eq!(closes_sent(&ledger), 0, "the player's bank stays open");
        assert_eq!(provisioner.bank_lease(), None);
        assert!(!provisioner.session_open());
    }

    #[test]
    fn a_bank_the_player_reopens_during_a_close_is_not_closed() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (mut provisioner, mut snapshot, mut ledger, memory, tick, banks) =
            session_with_a_leased_bank();
        let quests = quest_catalog();
        let leased = snapshot.bank_session_generation();
        let result = poll_once(
            &mut provisioner,
            &snapshot,
            &mut ledger,
            tick + 1,
            &owned_plan(),
            None,
            &memory,
            &banks,
            &quests,
        );
        assert!(result.is_pending());
        assert_eq!(closes_sent(&ledger), 1, "the close of the leased bank");
        assert!(provisioner.closer.active());

        // Before the close lands, the player shuts the bank and opens it again.
        snapshot.seed_bank_observation(-1, tick + 2, None, Vec::new());
        snapshot.seed_bank_observation(1, tick + 3, Some(Vec::new()), Vec::new());
        assert_eq!(snapshot.bank_session_generation(), leased + 2);
        ledger.as_mut().unwrap().outbox.clear();
        for step in 4..8 {
            let result = poll_once(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                tick + step,
                &owned_plan(),
                None,
                &memory,
                &banks,
                &quests,
            );
            if matches!(result, Poll::Ready(_)) {
                assert!(matches!(result, Poll::Ready(Ok(ProvisionEvent::Ready))));
                break;
            }
        }
        assert_eq!(
            closes_sent(&ledger),
            0,
            "no close is sent for the player's bank session"
        );
        assert_eq!(provisioner.bank_lease(), None);
        assert!(!provisioner.session_open());
    }

    /// A withdraw trip whose bank session closes after the click. In the first
    /// case the trip's own kernel opens it again with its stand click. In the
    /// second, the player opens a new session before the kernel can click.
    #[test]
    fn a_reopen_inside_a_trip_keeps_the_lease_only_when_the_kernel_made_it() {
        if !api::test_support::game_data_available() {
            return;
        }
        for player in [false, true] {
            let (mut snapshot, bank, banks) = openable_bank_fixture();
            // A required bank is picked again for each trip, so the trip knows its stand.
            let mut plan = scan_plan(bank);
            plan.bank_required = true;
            let quests = quest_catalog();
            let mut bank_rows = vec![bank_row(42, "Quest token", 1)];
            let mut pack: Vec<ItemView> = Vec::new();
            let mut memory = BankMemory::seeded(&[], Origin::Hint);
            let mut ledger = None;
            let mut provisioner = Provisioner::new();
            let mut reopened = false;
            let mut closes = 0;
            let mut closed_generation = None;
            let mut ready = false;
            let mut generations = Vec::new();

            for tick in 1..=80 {
                memory.track(&snapshot, tick);
                let result = poll_once(
                    &mut provisioner,
                    &snapshot,
                    &mut ledger,
                    tick,
                    &plan,
                    None,
                    &memory,
                    &banks,
                    &quests,
                );
                match ledger
                    .as_ref()
                    .and_then(|ledger| ledger.outbox.first())
                    .map(|action| &action.effect)
                {
                    Some(HostEffect::BankPick(_)) => {
                        complete_bank_pick(&mut ledger, bank, bank.tile, tick);
                    }
                    Some(HostEffect::Interaction(crate::interact::InteractReq::OpenStand {
                        ..
                    })) => {
                        complete_open_stand(&mut ledger, tick);
                        snapshot.seed_bank_observation(
                            1,
                            tick,
                            Some(bank_rows.clone()),
                            Vec::new(),
                        );
                        generations.push(snapshot.bank_session_generation());
                    }
                    Some(HostEffect::Interaction(crate::interact::InteractReq::SetNoteMode {
                        ..
                    })) => complete_open_stand(&mut ledger, tick),
                    Some(HostEffect::Interaction(crate::interact::InteractReq::WithdrawX {
                        ..
                    })) if !reopened => {
                        reopened = true;
                        complete_open_stand(&mut ledger, tick);
                        // The session ends before the click's effect shows.
                        snapshot.seed_bank_observation(-1, tick, None, Vec::new());
                        if player {
                            snapshot.seed_bank_observation(
                                1,
                                tick,
                                Some(bank_rows.clone()),
                                Vec::new(),
                            );
                            generations.push(snapshot.bank_session_generation());
                        }
                    }
                    Some(HostEffect::Interaction(crate::interact::InteractReq::WithdrawX {
                        ..
                    })) => {
                        let mut row = item_view(42, "Quest token", 1, ItemContainer::Inventory);
                        row.slot = 0;
                        pack.push(row);
                        bank_rows.retain(|row| row.def.id != 42);
                        complete_open_stand(&mut ledger, tick);
                        snapshot.seed_inventory(pack.clone(), 28);
                        snapshot.seed_bank_observation(
                            1,
                            tick,
                            Some(bank_rows.clone()),
                            Vec::new(),
                        );
                    }
                    Some(HostEffect::Interaction(crate::interact::InteractReq::Close)) => {
                        closes += 1;
                        closed_generation = Some(snapshot.bank_session_generation());
                        complete_open_stand(&mut ledger, tick);
                        snapshot.seed_bank_observation(-1, tick, None, Vec::new());
                    }
                    Some(_) => panic!("provisioning must not walk or issue another effect"),
                    None => {}
                }
                match result {
                    Poll::Ready(Ok(ProvisionEvent::Ready)) => {
                        ready = true;
                        break;
                    }
                    Poll::Ready(Ok(ProvisionEvent::BankReceipt(_))) | Poll::Pending => {}
                    Poll::Ready(other) => panic!("provisioning did not finish: {other:?}"),
                }
            }

            assert!(ready, "player={player}: provisioning finished");
            assert!(reopened, "player={player}: the session closed mid-trip");
            assert_eq!(generations.len(), 2, "player={player}: {generations:?}");
            assert_ne!(generations[0], generations[1]);
            if player {
                assert_eq!(closes, 0, "the player's session is never closed");
            } else {
                assert_eq!(closes, 1, "the kernel's own reopen is closed once");
                assert_eq!(
                    closed_generation,
                    Some(generations[1]),
                    "the close names the reopened session"
                );
            }
        }
    }

    /// What happens before a bank opens on a teller trip.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum TellerStart {
        /// No banker is in the scene. The kernel waits and sends nothing.
        NoBanker,
        /// No banker at first, so the kernel waits. Then a banker appears, but
        /// the tick's one interaction event is already spent: the kernel's talk
        /// is refused. (A trip polls its kernel even on such a tick, while it
        /// begins one only on a tick with the event unspent.)
        BudgetDenied,
        /// The kernel talks to the banker, and that request opens the bank.
        KernelOpens,
        /// The `BudgetDenied` timeline with the event unspent: the kernel's
        /// talk goes out on the tick that `BudgetDenied` refuses, and that
        /// request opens the bank.
        KernelOpensAfterWait,
    }

    fn teller_npc(stand: WorldTile) -> api::snapshot::NpcView {
        api::snapshot::NpcView {
            index: 7,
            r#type: None,
            name: Some("Banker".into()),
            actions: vec![Some("Talk-to".into()), Some("Bank".into())],
            tile: stand,
            distance: 1,
            animation: -1,
            animation_frame: 0,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            spot_animation_stamp: -1,
            health: 0,
            total_health: 0,
            face_entity: -1,
            target: None,
            moving: false,
            running: false,
            in_combat: false,
            level: 0,
            size: 1,
            network: stand,
            x: stand.x,
            z: stand.z,
            yaw: 0,
        }
    }

    /// Drive one teller trip to its receipt, then end the session. Returns the
    /// ledger's bank mark after the trip, the lease after the trip, and the
    /// number of closes that ending the session sent.
    fn teller_trip(
        start: TellerStart,
    ) -> (Option<ledger::BankSessionMark>, Option<BankLease>, usize) {
        let (mut snapshot, bank, banks) = openable_bank_fixture();
        if start == TellerStart::KernelOpens {
            snapshot.seed_npcs(vec![teller_npc(bank.tile)]);
        }
        let plan = scan_plan(bank);
        let quests = quest_catalog();
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let mut memory = BankMemory::default();
        let mut picked = false;
        let mut kernel_waited = false;
        let mut deny_next = false;
        // The tick on which the kernel's talk to a late banker is due.
        let mut talk_due = None;
        let mut receipt = false;
        let mut tick = 0;
        while tick < 20 && !receipt {
            tick += 1;
            memory.track(&snapshot, tick);
            let deny = std::mem::take(&mut deny_next);
            let result = with_tick_bank(&snapshot, Some(&memory), &mut ledger, tick, |native| {
                if deny {
                    assert!(native.cx.budget.event(false), "the tick's one event");
                }
                let required_after = native.cx.evidence();
                let mut cx = StepContext {
                    tick: native,
                    quests: &quests,
                    progress: &[],
                    required_after,
                    banks: &banks,
                    choices: &crate::quester::choices::QuestChoices::default(),
                };
                provisioner.poll(&mut cx, &plan, None, None)
            });
            match ledger
                .as_ref()
                .and_then(|ledger| ledger.outbox.first())
                .map(|action| &action.effect)
            {
                Some(HostEffect::BankPick(_)) => {
                    let ledger = ledger.as_mut().unwrap();
                    let action = ledger.outbox.remove(0);
                    let authority = action.authority();
                    ledger.complete_bank_pick(
                        &authority,
                        crate::bank::BankPickReceipt {
                            request_id: authority.request_id().get(),
                            evidence: api::quest_progress::EvidenceStamp {
                                run: authority.run(),
                                tick,
                                sequence: tick,
                            },
                            outcome: crate::bank::BankPickOutcome::Selected(
                                crate::bank::SelectedBank {
                                    bank_index: 0,
                                    access_tile: bank.tile,
                                    kind: crate::bank::PickKind::Reachable,
                                    access: Some(Arc::new(crate::bank::BankStandAccess {
                                        bank,
                                        stand_tile: bank.tile,
                                        kind: crate::bank::AccessKind::Teller,
                                        stand_op: 2,
                                        name: Some(Arc::from("Banker")),
                                        choose: None,
                                    })),
                                },
                            ),
                        },
                    );
                    picked = true;
                }
                Some(HostEffect::Interaction(crate::interact::InteractReq::Npc {
                    name, ..
                })) => {
                    assert!(
                        matches!(
                            start,
                            TellerStart::KernelOpens | TellerStart::KernelOpensAfterWait
                        ),
                        "{start:?}: no talk goes out"
                    );
                    assert_eq!(name, "Banker");
                    if start == TellerStart::KernelOpensAfterWait {
                        assert_eq!(
                            talk_due,
                            Some(tick),
                            "the talk goes out as the banker shows"
                        );
                    }
                    complete_open_stand(&mut ledger, tick);
                    snapshot.seed_bank_observation(1, tick, Some(Vec::new()), Vec::new());
                }
                Some(_) => panic!("{start:?}: the teller trip issued an unexpected effect"),
                None if deny => {
                    // The kernel's talk to the banker was refused on the tick
                    // it was due. Nothing went out. The player now opens a bank.
                    assert_eq!(talk_due, Some(tick));
                    kernel_waited = true;
                    assert_eq!(
                        ledger.as_ref().unwrap().bank_session,
                        None,
                        "{start:?}: nothing is marked before any bank opens"
                    );
                    snapshot.seed_bank_observation(1, tick, Some(Vec::new()), Vec::new());
                }
                None if picked && !kernel_waited && start == TellerStart::NoBanker => {
                    // The kernel waits for a banker without a request. The
                    // player now opens a bank.
                    kernel_waited = true;
                    assert_eq!(
                        ledger.as_ref().unwrap().bank_session,
                        None,
                        "{start:?}: nothing is marked before any bank opens"
                    );
                    snapshot.seed_bank_observation(1, tick, Some(Vec::new()), Vec::new());
                }
                None if picked
                    && !kernel_waited
                    && talk_due.is_none()
                    && matches!(
                        start,
                        TellerStart::BudgetDenied | TellerStart::KernelOpensAfterWait
                    ) =>
                {
                    // The kernel waited for a banker. One appears now. For
                    // `BudgetDenied`, the next tick's event is spent before
                    // the kernel polls.
                    snapshot.seed_npcs(vec![teller_npc(bank.tile)]);
                    deny_next = start == TellerStart::BudgetDenied;
                    talk_due = Some(tick + 1);
                }
                None => {}
            }
            match result {
                Poll::Ready(Ok(ProvisionEvent::BankReceipt(_))) => receipt = true,
                Poll::Pending => {}
                other => panic!("{start:?}: teller trip did not reach its receipt: {other:?}"),
            }
        }
        assert!(receipt, "{start:?}: teller trip reached its receipt");
        assert_eq!(
            kernel_waited,
            matches!(start, TellerStart::NoBanker | TellerStart::BudgetDenied),
            "{start:?}: the player open happened only after the kernel waited"
        );
        let mark = ledger.as_ref().unwrap().bank_session;
        let lease = provisioner.bank_lease();
        ledger.as_mut().unwrap().outbox.clear();
        let ended = poll_once(
            &mut provisioner,
            &snapshot,
            &mut ledger,
            tick + 1,
            &owned_plan(),
            None,
            &memory,
            &banks,
            &quests,
        );
        let closes = closes_sent(&ledger);
        // A close keeps the session in Closing. Without one, it ends now.
        if closes == 0 {
            assert!(
                matches!(ended, Poll::Ready(Ok(ProvisionEvent::Ready))),
                "{start:?}: {ended:?}"
            );
        } else {
            assert!(ended.is_pending(), "{start:?}: {ended:?}");
        }
        (mark, lease, closes)
    }

    /// The player's bank after a teller wait: no ownership mark, no lease and
    /// no close.
    fn assert_the_players_bank(start: TellerStart) {
        let (mark, lease, closes) = teller_trip(start);
        assert_eq!(
            (
                mark.is_some_and(|mark| mark.opened_by_machine),
                lease,
                closes
            ),
            (false, None, 0),
            "{start:?}: (marked as the kernel's, lease, closes); mark {mark:?}"
        );
    }

    #[test]
    fn a_bank_the_player_opens_while_the_teller_kernel_finds_no_banker_is_not_its_own() {
        if !api::test_support::game_data_available() {
            return;
        }
        assert_the_players_bank(TellerStart::NoBanker);
    }

    #[test]
    fn a_bank_the_player_opens_after_a_denied_teller_talk_is_not_its_own() {
        if !api::test_support::game_data_available() {
            return;
        }
        assert_the_players_bank(TellerStart::BudgetDenied);
    }

    /// The kernel's own teller open is marked, leased and closed once: at
    /// once, and after a wait for a banker.
    #[test]
    fn a_bank_the_teller_kernel_opens_is_its_own_and_closed() {
        if !api::test_support::game_data_available() {
            return;
        }
        for start in [TellerStart::KernelOpens, TellerStart::KernelOpensAfterWait] {
            let (mark, lease, closes) = teller_trip(start);
            let mark = mark.expect("the kernel's open is marked");
            assert!(mark.opened_by_machine, "{start:?}");
            assert_eq!(lease, Some(BankLease::new(1, mark.generation)), "{start:?}");
            assert_eq!(
                closes, 1,
                "{start:?}: the kernel's teller bank is closed once"
            );
        }
    }
}
