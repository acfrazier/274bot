//! Inventory-first preparation and selective capacity deposits for one Path.
//!
//! Needs are planned from the frame's [`Stock`]: the pack, the worn table and
//! the account's bank memory (design-bank-snapshot §4 N2). An `Unknown` bank
//! or a `Hint`-predicted shortage costs one scan trip that observes the whole
//! bank; only a `Session`-known shortage blocks in place (§2.4).
use super::bank_run::BankRun;
use super::compile::{
    CompiledAcquireRecipe, CompiledItemKind, CompiledProvisioning, StepContext, StepPlan, StepRun,
};
use super::families::AcquirePlan;
use crate::bank::ops::{self, MAX_MEMO};
use crate::native::{ActionError, NativeActions};
use crate::native_bank::{BankAction, BankReceipt, Withdrawal};
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

pub struct Provisioner {
    path: Option<api::selected::FactKey>,
    bank_run: Option<BankRun>,
    acquire_run: Option<Box<dyn StepRun>>,
    acquire_finished: bool,
    coin_drawn: bool,
    carry_drawn: u64,
    attempts: u32,
    status: Status,
    revision: u64,
}

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
            attempts: 0,
            status: Status::default(),
            revision: 0,
        }
    }

    pub fn poll(
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

        if self.bank_run.is_some() {
            return self.poll_bank(cx);
        }
        if self.acquire_run.is_some() {
            return self.poll_acquire(cx);
        }
        if plan.owns_inventory {
            self.set_status(ProvisionPhase::Ready, None, 0, 0, None, bank_known(cx));
            return Poll::Ready(Ok(ProvisionEvent::Ready));
        }

        self.poll_prepare(cx, plan, active_loadout, current_stage)
    }

    pub fn cancel(&mut self) {
        self.bank_run = None;
        self.acquire_run = None;
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
        self.acquire_finished = false;
        self.path = None;
        self.set_coin_drawn(false);
        self.set_carry_drawn(0);
        self.attempts = 0;
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
        self.bank_run.as_ref().map(|_| self.status.phase)
    }

    pub(super) fn take_trace_event(&mut self) -> Option<super::compile::StepTraceEvent> {
        self.acquire_run.as_mut()?.take_trace_event()
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
        self.bank_run.is_some() || (self.acquire_run.is_some() && !self.acquire_finished)
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
        let mut needs = Needs::new();
        let current = stage_index(plan, current_stage);

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
            let bank_item = crate::native_bank::BankItem {
                id: item.id,
                name: Arc::clone(&item.name),
            };
            let target = i32::try_from(item.qty).map_err(|_| {
                ActionError::Unavailable(Arc::from("compiled item quantity overflow"))
            })?;
            let kind = match item.kind {
                CompiledItemKind::MustHave => MissingKind::Required,
                CompiledItemKind::Acquirable => item
                    .acquire
                    .clone()
                    .map(MissingKind::Acquire)
                    .unwrap_or(MissingKind::Optional),
            };
            needs.require(&bank_item, target, kind, Presence::Carried, &stock)?;
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
                needs.require(item, 1, MissingKind::Optional, Presence::Carried, &stock)?;
            }
        }

        let active_recipe =
            if let Some(need) = needs.acquire.as_ref().filter(|_| needs.blocked.is_none()) {
                Some(plan.recipes.get(need.recipe.as_ref()).ok_or_else(|| {
                    ActionError::Unavailable(Arc::from("compiled acquisition recipe missing"))
                })?)
            } else {
                None
            };
        let safe_slots = safe_deposit_rows(inventory, &plan.base_spillover_keep) as i32;

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
                        let active_need =
                            needs.acquire.as_ref().filter(|_| needs.blocked.is_none());
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
                        needs.require(
                            &coin.item,
                            coin.qty,
                            MissingKind::Optional,
                            Presence::Held,
                            &stock,
                        )?;
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
                        let active_need =
                            needs.acquire.as_ref().filter(|_| needs.blocked.is_none());
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
                        needs.require(
                            &row.item,
                            row.qty,
                            MissingKind::Optional,
                            Presence::Held,
                            &stock,
                        )?;
                    }
                }
            }
        }

        if let Some(recipe) = active_recipe {
            for input in recipe.peak_items.iter() {
                needs.require(
                    &input.item,
                    input.qty,
                    MissingKind::Optional,
                    Presence::Held,
                    &stock,
                )?;
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
                let immediate = {
                    let active_need = needs.acquire.as_ref().filter(|_| needs.blocked.is_none());
                    planned_required_slots(plan, &needs, inventory, active_need, active_recipe)
                };
                if incoming
                    > capacity
                        .saturating_add(safe_slots)
                        .saturating_sub(immediate)
                {
                    // An optional later-stage hint must not park a runnable Path.
                    // Its authored steps still acquire it when it is needed.
                    continue;
                }
            }
            needs.require(
                &crate::native_bank::BankItem {
                    id: item.id,
                    name: Arc::clone(&item.name),
                },
                target,
                MissingKind::Optional,
                Presence::Carried,
                &stock,
            )?;
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
        if let Some(missing) = needs.blocked {
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
        let active_need = needs.acquire.as_ref();

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
                .first_withdrawal
                .as_ref()
                .or_else(|| active_need.map(|need| &need.need));
            self.start_bank(
                cx,
                plan,
                BankAction::DepositCapacity {
                    keep: Arc::clone(&plan.base_spillover_keep),
                    slots,
                },
                ProvisionPhase::Spillover,
                status.map(|need| Arc::clone(&need.item)),
                status.map_or(deficit, |need| need.need),
                status.map_or(ops::occupied(inventory), |need| need.pack),
                status.and_then(|need| need.bank),
            );
            return Poll::Pending;
        }

        if needs.withdrawal_count > 0 {
            let first = needs
                .first_withdrawal
                .as_ref()
                .expect("withdrawal has a detail");
            let mut withdrawals = Vec::with_capacity(needs.withdrawal_count);
            for entry in needs.withdrawals.iter_mut().take(needs.withdrawal_count) {
                if let Some(withdrawal) = entry.take() {
                    withdrawals.push(withdrawal);
                }
            }
            self.start_bank(
                cx,
                plan,
                BankAction::WithdrawTo {
                    withdrawals: Arc::from(withdrawals),
                },
                ProvisionPhase::Withdrawing,
                Some(Arc::clone(&first.item)),
                first.need,
                first.pack,
                first.bank,
            );
            return Poll::Pending;
        }
        if let (Some(recipe_need), Some(recipe)) = (needs.acquire, active_recipe) {
            let acquire = AcquirePlan {
                recipe: Arc::clone(&recipe_need.recipe),
                steps: Arc::clone(&recipe.steps),
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

        self.set_status(ProvisionPhase::Ready, None, 0, 0, None, bank_known(cx));
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
            .poll(cx);
        match result {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => {
                self.bank_run = None;
                Poll::Ready(Err(error))
            }
            Poll::Ready(Ok(receipt)) => {
                self.bank_run = None;
                Poll::Ready(Ok(ProvisionEvent::BankReceipt(receipt)))
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

    #[allow(clippy::too_many_arguments)] // One bank request and its producer-side status receipt.
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
        self.bank_run = Some(BankRun::new(
            plan.bank,
            plan.bank_required,
            Arc::from([action]),
            false,
            cx,
        ));
        self.attempts = self.attempts.saturating_add(1);
        self.set_status(phase, item, need, pack, bank, bank_known(cx));
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

struct Need {
    id: i32,
    item: Arc<str>,
    need: i32,
    pack: i32,
    bank: Option<i32>,
}
struct RecipeNeed {
    need: Need,
    recipe: Arc<str>,
}

enum MissingKind {
    Required,
    Acquire(Arc<str>),
    Optional,
}

/// What counts as already present for one need (design-bank-snapshot §2.3
/// rule 2): kit and quest items you may be wearing, or pack-only consumables.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Presence {
    Carried,
    Held,
}

struct Needs {
    withdrawals: [Option<Withdrawal>; MAX_MEMO],
    withdrawal_count: usize,
    first_withdrawal: Option<Need>,
    /// A need the `Unknown` bank cannot answer: one scan learns it.
    unknown: Option<Need>,
    /// A need the `Hint` predicts short: one verifying scan (§2.4), after
    /// which the plan is rebuilt from `Session` rows at the open bank.
    hint_short: Option<Need>,
    /// A `Session`-known shortage of a required item: final, in place.
    blocked: Option<Need>,
    acquire: Option<RecipeNeed>,
}

impl Needs {
    fn new() -> Self {
        Self {
            withdrawals: std::array::from_fn(|_| None),
            withdrawal_count: 0,
            first_withdrawal: None,
            unknown: None,
            hint_short: None,
            blocked: None,
            acquire: None,
        }
    }

    /// The need that sends the provisioner to look at the bank, if any.
    fn scan(&self) -> Option<&Need> {
        self.unknown.as_ref().or(self.hint_short.as_ref())
    }

    fn require(
        &mut self,
        item: &crate::native_bank::BankItem,
        target: i32,
        kind: MissingKind,
        presence: Presence,
        stock: &Stock<'_>,
    ) -> Result<(), ActionError> {
        if target <= 0 {
            return Ok(());
        }
        // The pack page is posted here (`poll_prepare` waits for it); an
        // unposted worn page counts as nothing worn, which only ever
        // withdraws again, never skips.
        let held = stock.held(item.id).unwrap_or(0);
        let present = match presence {
            Presence::Carried => held.saturating_add(stock.worn(item.id).unwrap_or(0)),
            Presence::Held => held,
        };
        if present >= target {
            return Ok(());
        }
        let Some(banked) = stock.banked(item.id) else {
            if self.unknown.is_none() {
                self.unknown = Some(Need {
                    id: item.id,
                    item: Arc::clone(&item.name),
                    need: target,
                    pack: present,
                    bank: None,
                });
            }
            return Ok(());
        };
        let banked = banked.max(0);
        let short = target.saturating_sub(present);
        let take = short.min(banked);
        if take > 0 {
            self.add_withdrawal(item, held.saturating_add(take))?;
            if self.first_withdrawal.is_none() {
                self.first_withdrawal = Some(Need {
                    id: item.id,
                    item: Arc::clone(&item.name),
                    need: target,
                    pack: present,
                    bank: Some(banked),
                });
            }
        }
        if take == short {
            return Ok(());
        }
        let missing = Need {
            id: item.id,
            item: Arc::clone(&item.name),
            need: target,
            pack: present,
            bank: Some(banked),
        };
        if stock.banked_origin() == Origin::Hint {
            // Advisory: the one verifying trip, whatever the need's kind.
            if self.hint_short.is_none() {
                self.hint_short = Some(missing);
            }
            return Ok(());
        }
        match kind {
            MissingKind::Required => {
                if self.blocked.is_none() {
                    self.blocked = Some(missing);
                }
            }
            MissingKind::Acquire(recipe) => {
                if self.acquire.is_none() {
                    self.acquire = Some(RecipeNeed {
                        need: missing,
                        recipe,
                    });
                }
            }
            MissingKind::Optional => {}
        }
        Ok(())
    }

    fn add_withdrawal(
        &mut self,
        item: &crate::native_bank::BankItem,
        target: i32,
    ) -> Result<(), ActionError> {
        if let Some(existing) = self
            .withdrawals
            .iter_mut()
            .take(self.withdrawal_count)
            .flatten()
            .find(|existing| existing.id == item.id)
        {
            existing.target = existing.target.max(target);
            return Ok(());
        }
        if self.withdrawal_count == MAX_MEMO {
            return Err(ActionError::Unavailable(Arc::from(
                "provision withdrawal exceeds the bank batch",
            )));
        }
        self.withdrawals[self.withdrawal_count] = Some(Withdrawal {
            id: item.id,
            name: Arc::clone(&item.name),
            target,
        });
        self.withdrawal_count += 1;
        Ok(())
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

fn safe_deposit_rows(inventory: &[ItemView], keep: &[i32]) -> usize {
    inventory
        .iter()
        .filter(|row| row.count > 0 && !keep.contains(&row.def.id))
        .count()
}

fn has_withdrawal(needs: &Needs, id: i32) -> bool {
    needs
        .withdrawals
        .iter()
        .take(needs.withdrawal_count)
        .flatten()
        .any(|withdrawal| withdrawal.id == id)
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
    needs: &Needs,
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
    for withdrawal in needs
        .withdrawals
        .iter()
        .take(needs.withdrawal_count)
        .flatten()
    {
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
    needs: &Needs,
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

    fn ready_snapshot(items: Vec<ItemView>) -> GameSnapshot {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(items, 28);
        snapshot
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
            need: Need {
                id: 100,
                item: Arc::from("Flour"),
                need: 1,
                pack: 0,
                bank: Some(0),
            },
            recipe: Arc::from("acquire:flour"),
        };
        let inventory = [
            item_view(201, "Egg", 1, ItemContainer::Inventory),
            item_view(202, "Milk", 1, ItemContainer::Inventory),
        ];
        let needs = Needs::new();
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
            need: Need {
                id: 301,
                item: Arc::from("Milk"),
                need: 1,
                pack: 0,
                bank: Some(0),
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
        run: &mut BankRun,
        snapshot: &GameSnapshot,
        ledger: &mut Option<Box<ledger::Ledger>>,
        banks: &Arc<NamedBankFacts>,
    ) -> Result<BankPickRequest, ActionError> {
        let memo = BankMemory::default();
        let quests = quest_catalog();
        for tick in 2..=5 {
            let result = with_tick_bank(snapshot, Some(&memo), ledger, tick, |native| {
                let required_after = native.cx.evidence();
                let mut cx = StepContext {
                    tick: native,
                    quests: &quests,
                    progress: &[],
                    required_after,
                    banks,
                    choices: &crate::quester::choices::QuestChoices::default(),
                };
                run.poll(&mut cx)
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
        let draynor = tile(3092, 3242);
        let varrock_west = tile(3185, 3438);
        let banks = Arc::new(NamedBankFacts::from_banks(vec![
            NamedBank::new("Draynor Village", draynor),
            NamedBank::new("Varrock West", varrock_west),
        ]));
        for authored in [draynor, varrock_west, tile(3200, 3200)] {
            let snapshot = bank_snapshot();
            let mut ledger = None;
            let quests = quest_catalog();
            let memo = BankMemory::default();
            let mut run = with_tick_bank(&snapshot, Some(&memo), &mut ledger, 1, |native| {
                let required_after = native.cx.evidence();
                let cx = StepContext {
                    tick: native,
                    quests: &quests,
                    progress: &[],
                    required_after,
                    banks: &banks,
                    choices: &crate::quester::choices::QuestChoices::default(),
                };
                BankRun::new(
                    Some(NamedBank::new("Path bank", authored)),
                    false,
                    Arc::from([BankAction::Scan]),
                    false,
                    &cx,
                )
            });
            let request = poll_bank_pick(&mut run, &snapshot, &mut ledger, &banks).unwrap();
            assert!(
                request.explicit_bank.is_none(),
                "authored tile {authored:?} must not constrain nearest selection"
            );
        }
    }

    #[test]
    fn required_draynor_bank_is_explicit_and_unmatched_required_bank_refuses() {
        let draynor = tile(3092, 3242);
        let banks = Arc::new(NamedBankFacts::from_banks(vec![NamedBank::new(
            "Draynor Village",
            draynor,
        )]));
        let snapshot = bank_snapshot();
        let mut ledger = None;
        let quests = quest_catalog();
        let memo = BankMemory::default();
        let mut run = with_tick_bank(&snapshot, Some(&memo), &mut ledger, 1, |native| {
            let required_after = native.cx.evidence();
            let cx = StepContext {
                tick: native,
                quests: &quests,
                progress: &[],
                required_after,
                banks: &banks,
                choices: &crate::quester::choices::QuestChoices::default(),
            };
            BankRun::new(
                Some(NamedBank::new("Path bank", draynor)),
                true,
                Arc::from([BankAction::Scan]),
                false,
                &cx,
            )
        });
        let request = poll_bank_pick(&mut run, &snapshot, &mut ledger, &banks).unwrap();
        assert_eq!(request.explicit_bank, Some(0));
        let mut open_snapshot = bank_snapshot();
        open_snapshot.seed_bank_observation(1, 1, Some(Vec::new()), Vec::new());
        let mut ledger = None;
        let mut run = with_tick_bank(&open_snapshot, Some(&memo), &mut ledger, 1, |native| {
            let required_after = native.cx.evidence();
            let cx = StepContext {
                tick: native,
                quests: &quests,
                progress: &[],
                required_after,
                banks: &banks,
                choices: &crate::quester::choices::QuestChoices::default(),
            };
            BankRun::new(
                Some(NamedBank::new("Path bank", draynor)),
                true,
                Arc::from([BankAction::Scan]),
                false,
                &cx,
            )
        });
        let request = poll_bank_pick(&mut run, &open_snapshot, &mut ledger, &banks).unwrap();
        assert_eq!(request.explicit_bank, Some(0));

        let unmatched = tile(3200, 3200);
        let mut ledger = None;
        let mut run = with_tick_bank(&snapshot, Some(&memo), &mut ledger, 1, |native| {
            let required_after = native.cx.evidence();
            let cx = StepContext {
                tick: native,
                quests: &quests,
                progress: &[],
                required_after,
                banks: &banks,
                choices: &crate::quester::choices::QuestChoices::default(),
            };
            BankRun::new(
                Some(NamedBank::new("Path bank", unmatched)),
                true,
                Arc::from([BankAction::Scan]),
                false,
                &cx,
            )
        });
        assert!(matches!(
            poll_bank_pick(&mut run, &snapshot, &mut ledger, &banks),
            Err(ActionError::Unavailable(reason))
                if reason.as_ref() == "required bank is not in the bank catalog"
        ));
        assert!(ledger
            .as_ref()
            .is_none_or(|ledger| ledger.outbox.is_empty()));
    }

    #[test]
    fn pack_sufficiency_does_not_open_an_unread_bank() {
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
        let (path, quests, snapshot, axe_id) = woodcutting_tool_fixture();
        let memo = BankMemory::seeded(&[(axe_id, 1)], Origin::Session);
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let banks = Arc::new(NamedBankFacts::empty());
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
        let (path, quests, snapshot, _) = woodcutting_tool_fixture();
        let memo = BankMemory::seeded(&[], Origin::Hint);
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let banks = Arc::new(NamedBankFacts::empty());
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
        assert_eq!(provisioner.status().phase, ProvisionPhase::Acquiring);
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
            Poll::Ready(Ok(ProvisionEvent::Acquired))
        ));
    }

    #[test]
    fn float_latches_survive_consumption_and_reset_clears_them() {
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
                let snapshot = ready_snapshot(if held {
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
                let banks = Arc::new(NamedBankFacts::empty());
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
        let snapshot = ready_snapshot(Vec::new());
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
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
        assert_eq!(provisioner.status().phase, ProvisionPhase::Acquiring);
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
                selected: crate::bank::SelectedBank {
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
                },
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
                        HostEffect::Interaction(crate::shim::InteractReq::OpenStand { .. })
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
    fn hint_shortage_costs_one_scan_then_withdraws_at_the_same_open_bank() {
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
                                    crate::shim::InteractReq::OpenStand { .. }
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
        // block in place and no second bank trip.
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
}
