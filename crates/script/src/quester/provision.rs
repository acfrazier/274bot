//! Pack-first preparation, bank spillover, and completion retreat for one Path.
use super::bank_memo::{BankMemo, MAX_BANK_MEMO};
use super::compile::{CompiledItemKind, CompiledProvisioning, StepContext, StepPlan, StepRun};
use super::families::{self, AcquirePlan};
use crate::bank::{Open, OpenArgs, Select, SelectArgs};
use crate::native::walk::Walk;
use crate::native::{ActionError, ActionHandle, NativeActions, WalkOptions};
use crate::native_bank::{BankAction, BankMachine, BankReceipt, BankRequest, Withdrawal};
use api::named_banks::NamedBank;
use api::snapshot::{ItemView, WorldTile};
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvisionMode {
    Prepare,
    Retreat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvisionPhase {
    Idle,
    Scanning,
    Freshening,
    Spillover,
    Withdrawing,
    Acquiring,
    Retreating,
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
    BankMemoUnknown,
    Blocked { item: Arc<str> },
}

#[derive(Clone, Copy)]
enum BankPurpose {
    Freshen,
    Spillover,
    Scan,
    Withdraw,
    Retreat,
}

pub struct Provisioner {
    path: Option<api::selected::FactKey>,
    bank_run: Option<BankRun>,
    bank_purpose: Option<BankPurpose>,
    acquire_run: Option<Box<dyn StepRun>>,
    freshened: bool,
    freshen_attempts: u8,
    spillover_done: bool,
    coin_drawn: bool,
    carry_drawn: u64,
    retreated: bool,
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
            bank_purpose: None,
            acquire_run: None,
            freshened: false,
            freshen_attempts: 0,
            spillover_done: false,
            coin_drawn: false,
            carry_drawn: 0,
            retreated: false,
            attempts: 0,
            status: Status::default(),
            revision: 0,
        }
    }

    pub fn poll(
        &mut self,
        cx: &mut StepContext<'_, '_>,
        plan: &CompiledProvisioning,
        mode: ProvisionMode,
        active_loadout: Option<&str>,
        not_started: bool,
    ) -> Poll<Result<ProvisionEvent, ActionError>> {
        if self.path.as_ref().is_some_and(|path| path != &plan.path) {
            self.reset(cx.tick.actions);
        }
        if self.path.is_none() {
            self.path = Some(plan.path.clone());
            self.set_coin_drawn(plan.coin_float <= 0);
        }

        if self.bank_run.is_some() {
            return self.poll_bank(cx, plan);
        }
        if self.acquire_run.is_some() {
            return self.poll_acquire(cx);
        }
        if plan.owns_inventory {
            self.set_status(ProvisionPhase::Ready, None, 0, 0, None, cx.bank.known());
            return Poll::Ready(Ok(ProvisionEvent::Ready));
        }

        match mode {
            ProvisionMode::Retreat => self.poll_retreat(cx, plan),
            ProvisionMode::Prepare => self.poll_prepare(cx, plan, active_loadout, not_started),
        }
    }

    pub fn cancel(&mut self) {
        self.bank_run = None;
        self.bank_purpose = None;
        self.acquire_run = None;
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
        self.bank_purpose = None;
        self.path = None;
        self.freshened = false;
        self.freshen_attempts = 0;
        self.spillover_done = false;
        self.set_coin_drawn(false);
        self.set_carry_drawn(0);
        self.retreated = false;
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

    pub fn status_revision(&self) -> u64 {
        self.revision
    }

    pub fn coin_drawn(&self) -> bool {
        self.coin_drawn
    }

    pub fn carry_drawn(&self) -> u64 {
        self.carry_drawn
    }

    pub fn retreat_performed(&self) -> bool {
        self.retreated
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

    fn poll_retreat(
        &mut self,
        cx: &mut StepContext<'_, '_>,
        plan: &CompiledProvisioning,
    ) -> Poll<Result<ProvisionEvent, ActionError>> {
        if self.retreated || (plan.bank.is_none() && cx.banks.banks().is_empty()) {
            self.set_status(ProvisionPhase::Ready, None, 0, 0, None, cx.bank.known());
            return Poll::Ready(Ok(ProvisionEvent::Ready));
        }
        let keep = Arc::clone(&plan.keep_ids);
        self.start_bank(
            cx,
            plan,
            BankAction::DepositAll { keep },
            BankPurpose::Retreat,
            ProvisionPhase::Retreating,
            None,
            0,
            0,
            None,
        );
        Poll::Pending
    }

    fn poll_prepare(
        &mut self,
        cx: &mut StepContext<'_, '_>,
        plan: &CompiledProvisioning,
        active_loadout: Option<&str>,
        not_started: bool,
    ) -> Poll<Result<ProvisionEvent, ActionError>> {
        let Some(inventory) = cx.tick.cx.snapshot().inventory() else {
            self.set_status(ProvisionPhase::Scanning, None, 0, 0, None, cx.bank.known());
            return Poll::Pending;
        };
        let inventory = inventory.value;

        if not_started && !self.freshened {
            if !has_unkept_item(inventory, &plan.keep_ids) {
                self.freshened = true;
            } else if self.freshen_attempts < 3 {
                self.freshen_attempts += 1;
                self.start_bank(
                    cx,
                    plan,
                    BankAction::DepositAll {
                        keep: Arc::clone(&plan.keep_ids),
                    },
                    BankPurpose::Freshen,
                    ProvisionPhase::Freshening,
                    None,
                    0,
                    0,
                    None,
                );
                return Poll::Pending;
            } else {
                self.freshened = true;
            }
        }

        if !self.spillover_done {
            let keep = &plan.base_spillover_keep;
            if !has_spillover(inventory, keep) {
                self.spillover_done = true;
            } else {
                self.start_bank(
                    cx,
                    plan,
                    BankAction::DepositAll {
                        keep: Arc::clone(keep),
                    },
                    BankPurpose::Spillover,
                    ProvisionPhase::Spillover,
                    None,
                    0,
                    0,
                    None,
                );
                return Poll::Pending;
            }
        }

        let mut needs = Needs::new();
        for item in plan.items.iter() {
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
                    // Schema-2 Paths may acquire through their authored
                    // sequence (Sheep), rather than a synthetic recipe.
                    .unwrap_or(MissingKind::Optional),
            };
            needs.require(&bank_item, target, kind, inventory, cx.bank)?;
        }
        for tool in plan.tools.iter() {
            // Tools are preservation/withdrawal hints, not extra mustHave rows:
            // acquire recipes may obtain or consume them (e.g. pot and grain).
            needs.require(tool, 1, MissingKind::Optional, inventory, cx.bank)?;
        }

        if plan.coin_float <= 0 {
            self.set_coin_drawn(true);
        } else if let Some(coin) = plan.coin.as_ref() {
            if !self.coin_drawn {
                if count_item(inventory, coin.item.id) >= coin.qty {
                    self.set_coin_drawn(true);
                } else {
                    needs.require(
                        &coin.item,
                        coin.qty,
                        MissingKind::Optional,
                        inventory,
                        cx.bank,
                    )?;
                }
            }
        }

        if let Some(carry) = active_loadout.and_then(|name| plan.loadout_carry.get(name)) {
            for row in carry.iter() {
                let bit = 1u64 << row.latch_index;
                if self.carry_drawn & bit != 0 {
                    continue;
                }
                if row.qty <= 0 || count_item(inventory, row.item.id) >= row.qty {
                    self.set_carry_latch(row.latch_index);
                } else {
                    needs.require(
                        &row.item,
                        row.qty,
                        MissingKind::Optional,
                        inventory,
                        cx.bank,
                    )?;
                }
            }
        }

        if let Some(unknown) = needs.unknown {
            self.start_bank(
                cx,
                plan,
                BankAction::Scan,
                BankPurpose::Scan,
                ProvisionPhase::Scanning,
                Some(unknown.item),
                unknown.need,
                unknown.pack,
                None,
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
                BankPurpose::Withdraw,
                ProvisionPhase::Withdrawing,
                Some(Arc::clone(&first.item)),
                first.need,
                first.pack,
                first.bank,
            );
            return Poll::Pending;
        }
        if let Some(missing) = needs.blocked {
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
        if let Some(recipe) = needs.acquire {
            let Some(steps) = plan.recipes.get(recipe.recipe.as_ref()) else {
                return Poll::Ready(Err(ActionError::Unavailable(Arc::from(
                    "compiled acquisition recipe missing",
                ))));
            };
            let acquire = AcquirePlan {
                recipe: Arc::clone(&recipe.recipe),
                steps: steps.to_vec(),
            };
            match acquire.begin(cx) {
                Ok(run) => {
                    self.acquire_run = Some(run);
                    self.set_status(
                        ProvisionPhase::Acquiring,
                        Some(recipe.need.item),
                        recipe.need.need,
                        recipe.need.pack,
                        recipe.need.bank,
                        true,
                    );
                    return Poll::Pending;
                }
                Err(error) => return Poll::Ready(Err(error)),
            }
        }

        self.set_status(ProvisionPhase::Ready, None, 0, 0, None, cx.bank.known());
        Poll::Ready(Ok(ProvisionEvent::Ready))
    }

    fn poll_bank(
        &mut self,
        cx: &mut StepContext<'_, '_>,
        plan: &CompiledProvisioning,
    ) -> Poll<Result<ProvisionEvent, ActionError>> {
        let result = self
            .bank_run
            .as_mut()
            .expect("bank poll has a machine")
            .poll(cx);
        match result {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => {
                let purpose = self.bank_purpose.take();
                self.bank_run = None;
                if matches!(purpose, Some(BankPurpose::Freshen)) {
                    if self.freshen_attempts >= 3 {
                        self.freshened = true;
                    }
                    self.set_status(
                        ProvisionPhase::Freshening,
                        None,
                        0,
                        0,
                        None,
                        cx.bank.known(),
                    );
                    Poll::Pending
                } else {
                    Poll::Ready(Err(error))
                }
            }
            Poll::Ready(Ok(receipt)) => {
                let purpose = self.bank_purpose.take();
                self.bank_run = None;
                match purpose {
                    Some(BankPurpose::Freshen) => {
                        let inventory = cx.tick.cx.snapshot().inventory();
                        if inventory.is_some_and(|inventory| {
                            !has_unkept_item(inventory.value, &plan.keep_ids)
                        }) || self.freshen_attempts >= 3
                        {
                            self.freshened = true;
                        }
                    }
                    Some(BankPurpose::Spillover) => self.spillover_done = true,
                    Some(BankPurpose::Retreat) => self.retreated = true,
                    Some(BankPurpose::Scan | BankPurpose::Withdraw) | None => {}
                }
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
        match result {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => {
                self.acquire_run = None;
                Poll::Ready(Err(error))
            }
            Poll::Ready(Ok(_)) => {
                self.acquire_run = None;
                Poll::Ready(Ok(ProvisionEvent::BankMemoUnknown))
            }
        }
    }

    #[allow(clippy::too_many_arguments)] // One bank request and its producer-side status receipt.
    fn start_bank(
        &mut self,
        cx: &mut StepContext<'_, '_>,
        plan: &CompiledProvisioning,
        action: BankAction,
        purpose: BankPurpose,
        phase: ProvisionPhase,
        item: Option<Arc<str>>,
        need: i32,
        pack: i32,
        bank: Option<i32>,
    ) {
        let memo_ids = Arc::clone(&plan.memo_ids);
        self.bank_run = Some(if plan.bank_required {
            BankRun::new_with_required(plan.bank, true, action, memo_ids, cx)
        } else {
            BankRun::new(plan.bank, action, memo_ids, cx)
        });
        self.bank_purpose = Some(purpose);
        self.attempts = self.attempts.saturating_add(1);
        self.set_status(phase, item, need, pack, bank, cx.bank.known());
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

struct Needs {
    withdrawals: [Option<Withdrawal>; MAX_BANK_MEMO],
    withdrawal_count: usize,
    first_withdrawal: Option<Need>,
    unknown: Option<Need>,
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
            blocked: None,
            acquire: None,
        }
    }

    fn require(
        &mut self,
        item: &crate::native_bank::BankItem,
        target: i32,
        kind: MissingKind,
        inventory: &[ItemView],
        memo: &BankMemo,
    ) -> Result<(), ActionError> {
        if target <= 0 {
            return Ok(());
        }
        let pack = count_item(inventory, item.id);
        if pack >= target {
            return Ok(());
        }
        let need = Need {
            item: Arc::clone(&item.name),
            need: target,
            pack,
            bank: None,
        };
        if !memo.known() {
            if self.unknown.is_none() {
                self.unknown = Some(need);
            }
            return Ok(());
        }

        let banked = memo.count(item.id).unwrap_or(0).max(0);
        let short = target.saturating_sub(pack);
        let take = short.min(banked);
        if take > 0 {
            self.add_withdrawal(item, pack.saturating_add(take))?;
            if self.first_withdrawal.is_none() {
                self.first_withdrawal = Some(Need {
                    item: Arc::clone(&item.name),
                    need: target,
                    pack,
                    bank: Some(banked),
                });
            }
        }
        if take == short {
            return Ok(());
        }
        let missing = Need {
            item: Arc::clone(&item.name),
            need: target,
            pack,
            bank: Some(banked),
        };
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
        if self.withdrawal_count == MAX_BANK_MEMO {
            return Err(ActionError::Unavailable(Arc::from(
                "provision withdrawal exceeds active bank memo",
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

struct BankRun {
    bank: Option<NamedBank>,
    explicit: Option<Arc<str>>,
    required_bank_missing: bool,
    selection: Option<ActionHandle<Select>>,
    picked: bool,
    access: Option<Arc<crate::native_bank::BankStandAccess>>,
    target: Option<WorldTile>,
    walk: Option<ActionHandle<Walk>>,
    walk_started: bool,
    opening: Option<ActionHandle<Open>>,
    open_started: bool,
    machine: Option<ActionHandle<BankMachine>>,
    action: BankAction,
    memo_ids: Arc<[i32]>,
}

impl BankRun {
    fn new(
        path_bank: Option<NamedBank>,
        action: BankAction,
        memo_ids: Arc<[i32]>,
        cx: &StepContext<'_, '_>,
    ) -> Self {
        Self::new_with_required(path_bank, false, action, memo_ids, cx)
    }

    fn new_with_required(
        path_bank: Option<NamedBank>,
        bank_required: bool,
        action: BankAction,
        memo_ids: Arc<[i32]>,
        cx: &StepContext<'_, '_>,
    ) -> Self {
        let already_open = cx
            .tick
            .cx
            .snapshot()
            .bank_session()
            .is_some_and(|session| session.value.open);
        let already_open_for_optional = already_open && !bank_required;
        let explicit: Option<Arc<str>> = if bank_required {
            path_bank.and_then(|wanted| {
                cx.banks
                    .banks()
                    .iter()
                    .find(|candidate| candidate.tile == wanted.tile)
                    .map(|candidate| Arc::from(candidate.name))
            })
        } else {
            None
        };
        let required_bank_missing = bank_required && explicit.is_none();
        Self {
            bank: if already_open_for_optional {
                path_bank
            } else {
                None
            },
            explicit,
            required_bank_missing,
            selection: None,
            picked: already_open_for_optional,
            access: None,
            target: None,
            walk: None,
            walk_started: false,
            opening: None,
            open_started: false,
            machine: None,
            action,
            memo_ids,
        }
    }
}
impl BankRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<BankReceipt, ActionError>> {
        if self.required_bank_missing {
            return Poll::Ready(Err(ActionError::Unavailable(Arc::from(
                "required bank is not in the bank catalog",
            ))));
        }
        if !self.picked {
            if let Some(handle) = self.selection.as_ref() {
                match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                    Poll::Ready(Ok(selected)) => {
                        self.selection = None;
                        if selected.kind == crate::native_bank::PickKind::NoCandidate {
                            return Poll::Ready(Err(ActionError::Unavailable(Arc::from(
                                "no eligible bank",
                            ))));
                        }
                        let Some(access) = selected.access else {
                            return Poll::Ready(Err(ActionError::Unavailable(Arc::from(
                                "selected bank has no access stand",
                            ))));
                        };
                        let Some(bank) = cx
                            .banks
                            .banks()
                            .get(usize::from(selected.bank_index))
                            .copied()
                        else {
                            return Poll::Ready(Err(ActionError::Stale));
                        };
                        self.bank = Some(bank);
                        self.target = Some(selected.access_tile);
                        self.access = Some(access);
                        self.picked = true;
                    }
                }
            }
            if !self.picked {
                let Some(from) = cx.tick.cx.snapshot().here() else {
                    return Poll::Ready(Err(ActionError::Unavailable(Arc::from(
                        "player position unavailable for bank selection",
                    ))));
                };
                self.selection = Some(
                    match cx.tick.actions.begin::<Select>(
                        SelectArgs {
                            facts: Arc::clone(cx.banks),
                            from: from.value,
                            preferences: api::named_banks::BankPreferences::default(),
                            options: WalkOptions::default(),
                            explicit: self.explicit.clone(),
                        },
                        &mut cx.tick.cx,
                    ) {
                        Ok(handle) => handle,
                        Err(error) => return Poll::Ready(Err(error)),
                    },
                );
                return Poll::Pending;
            }
        }

        if let Some(handle) = self.walk.as_ref() {
            match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(receipt)) => {
                    if let Err(error) = walk_evidence(receipt) {
                        return Poll::Ready(Err(error));
                    }
                    self.walk = None;
                }
            }
        }
        if !self.walk_started {
            self.walk_started = true;
            if let Some(target) = self.target {
                let near = cx
                    .tick
                    .cx
                    .snapshot()
                    .here()
                    .is_some_and(|here| families::reach::within(here.value, target, 0));
                if !near {
                    self.walk = Some(
                        match cx.tick.actions.begin::<Walk>(
                            families::reach::walk_request(target, 0, None, cx.required_after),
                            &mut cx.tick.cx,
                        ) {
                            Ok(handle) => handle,
                            Err(error) => return Poll::Ready(Err(error)),
                        },
                    );
                    return Poll::Pending;
                }
            }
        }

        if let Some(handle) = self.opening.as_ref() {
            match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(())) => self.opening = None,
            }
        }
        if !self.open_started {
            if let Some(access) = self.access.as_ref() {
                self.opening = Some(
                    match cx.tick.actions.begin::<Open>(
                        OpenArgs {
                            access: Arc::clone(access),
                        },
                        &mut cx.tick.cx,
                    ) {
                        Ok(handle) => handle,
                        Err(error) => return Poll::Ready(Err(error)),
                    },
                );
                self.open_started = true;
                return Poll::Pending;
            }
            self.open_started = true;
        }

        if let Some(handle) = self.machine.as_ref() {
            match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(receipt)) => {
                    self.machine = None;
                    return Poll::Ready(Ok(receipt));
                }
            }
        }
        self.machine = Some(
            match cx.tick.actions.begin::<BankMachine>(
                BankRequest {
                    bank: self.bank,
                    action: self.action.clone(),
                    memo_ids: Arc::clone(&self.memo_ids),
                    partial_ok: false,
                },
                &mut cx.tick.cx,
            ) {
                Ok(handle) => handle,
                Err(error) => return Poll::Ready(Err(error)),
            },
        );
        Poll::Pending
    }

    fn cancel(&mut self, actions: &mut NativeActions) {
        if let Some(handle) = self.selection.take() {
            actions.cancel(handle);
        }
        if let Some(handle) = self.walk.take() {
            actions.cancel(handle);
        }
        if let Some(handle) = self.opening.take() {
            actions.cancel(handle);
        }
        if let Some(handle) = self.machine.take() {
            actions.cancel(handle);
        }
    }
}

fn walk_evidence(receipt: crate::native::WalkReceipt) -> Result<(), ActionError> {
    receipt.into_arrival().map(|_| ())
}

fn count_item(inventory: &[ItemView], id: i32) -> i32 {
    inventory
        .iter()
        .filter(|row| row.def.id == id)
        .map(|row| row.count.max(0))
        .sum()
}

fn has_unkept_item(inventory: &[ItemView], keep: &[i32]) -> bool {
    inventory
        .iter()
        .any(|row| row.count > 0 && !keep.contains(&row.def.id))
}

fn has_spillover(inventory: &[ItemView], keep: &[i32]) -> bool {
    inventory
        .iter()
        .any(|row| row.count > 0 && !keep.contains(&row.def.id))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::ledger;
    use crate::native::{HostEffect, NativeTick};
    use crate::native_bank::BankPickRequest;
    use crate::quester::compile::CompiledQuestItem;
    use crate::quester::families::tests::{def, local_player, with_tick};
    use api::named_banks::{NamedBank, NamedBankFacts};
    use api::quest_facts::QuestCatalog;
    use api::selected::{ClientRevision, FactKey};
    use api::snapshot::{GameSnapshot, ItemActionFamily, ItemContainer, ItemView};
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
        }
    }

    #[test]
    fn provision_walk_requires_arrival_and_retains_refusal_detail() {
        let result = walk_evidence(walk_receipt(
            crate::native::WalkEnd::RouteEnded,
            Some(Arc::from("route stopped short")),
        ));
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
        let error = walk_evidence(walk_receipt(
            crate::native::WalkEnd::NeedsEvidence(Arc::clone(&gates)),
            None,
        ))
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
            walk_evidence(walk_receipt(crate::native::WalkEnd::UserInput, None)),
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
        }
    }

    fn provisioning(
        items: Vec<CompiledQuestItem>,
        memo_ids: Vec<i32>,
        bank: Option<NamedBank>,
    ) -> CompiledProvisioning {
        let keep: Vec<_> = items.iter().map(|item| item.id).collect();
        CompiledProvisioning {
            path: FactKey::new("provision-test"),
            owns_inventory: false,
            bank,
            bank_required: false,
            items: Arc::from(items),
            tools: Arc::from(Vec::new()),
            keep_ids: Arc::from(Vec::new()),
            coin_float: 0,
            coin: None,
            loadout_carry: HashMap::new(),
            base_spillover_keep: Arc::from(keep),
            recipes: HashMap::new(),
            memo_ids: Arc::from(memo_ids),
        }
    }

    #[allow(clippy::too_many_arguments)] // Explicit fixture inputs mirror the production poll context.
    fn poll_once(
        provisioner: &mut Provisioner,
        snapshot: &GameSnapshot,
        ledger: &mut Option<Box<ledger::Ledger>>,
        tick: u64,
        plan: &CompiledProvisioning,
        mode: ProvisionMode,
        active_loadout: Option<&str>,
        not_started: bool,
        memo: &BankMemo,
        banks: &Arc<api::named_banks::NamedBankFacts>,
        quests: &QuestCatalog,
    ) -> Poll<Result<ProvisionEvent, ActionError>> {
        with_tick(snapshot, ledger, tick, |native| {
            let required_after = native.cx.evidence();
            let mut cx = StepContext {
                tick: native,
                quests,
                progress: &[],
                required_after,
                bank: memo,
                banks,
                choices: &crate::quester::choices::QuestChoices::default(),
            };
            provisioner.poll(&mut cx, plan, mode, active_loadout, not_started)
        })
    }

    fn ready_snapshot(items: Vec<ItemView>) -> GameSnapshot {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(items, 28);
        snapshot
    }

    fn known_empty(id: i32) -> BankMemo {
        let mut memo = BankMemo::default();
        memo.update(&BankReceipt {
            counts: vec![crate::native_bank::BankCount { id, count: 0 }],
            complete: true,
        });
        memo
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
        let memo = BankMemo::default();
        let quests = quest_catalog();
        for tick in 2..=5 {
            let result = with_tick(snapshot, ledger, tick, |native| {
                let required_after = native.cx.evidence();
                let mut cx = StepContext {
                    tick: native,
                    quests: &quests,
                    progress: &[],
                    required_after,
                    bank: &memo,
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
            let memo = BankMemo::default();
            let mut run = with_tick(&snapshot, &mut ledger, 1, |native| {
                let required_after = native.cx.evidence();
                let cx = StepContext {
                    tick: native,
                    quests: &quests,
                    progress: &[],
                    required_after,
                    bank: &memo,
                    banks: &banks,
                    choices: &crate::quester::choices::QuestChoices::default(),
                };
                BankRun::new(
                    Some(NamedBank::new("Path bank", authored)),
                    BankAction::Scan,
                    Arc::from([]),
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
        let memo = BankMemo::default();
        let mut run = with_tick(&snapshot, &mut ledger, 1, |native| {
            let required_after = native.cx.evidence();
            let cx = StepContext {
                tick: native,
                quests: &quests,
                progress: &[],
                required_after,
                bank: &memo,
                banks: &banks,
                choices: &crate::quester::choices::QuestChoices::default(),
            };
            BankRun::new_with_required(
                Some(NamedBank::new("Path bank", draynor)),
                true,
                BankAction::Scan,
                Arc::from([]),
                &cx,
            )
        });
        let request = poll_bank_pick(&mut run, &snapshot, &mut ledger, &banks).unwrap();
        assert_eq!(request.explicit_bank, Some(0));
        let mut open_snapshot = bank_snapshot();
        open_snapshot.seed_bank_observation(1, 1, Some(Vec::new()), Vec::new());
        let mut ledger = None;
        let mut run = with_tick(&open_snapshot, &mut ledger, 1, |native| {
            let required_after = native.cx.evidence();
            let cx = StepContext {
                tick: native,
                quests: &quests,
                progress: &[],
                required_after,
                bank: &memo,
                banks: &banks,
                choices: &crate::quester::choices::QuestChoices::default(),
            };
            BankRun::new_with_required(
                Some(NamedBank::new("Path bank", draynor)),
                true,
                BankAction::Scan,
                Arc::from([]),
                &cx,
            )
        });
        let request = poll_bank_pick(&mut run, &open_snapshot, &mut ledger, &banks).unwrap();
        assert_eq!(request.explicit_bank, Some(0));

        let unmatched = tile(3200, 3200);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |native| {
            let required_after = native.cx.evidence();
            let cx = StepContext {
                tick: native,
                quests: &quests,
                progress: &[],
                required_after,
                bank: &memo,
                banks: &banks,
                choices: &crate::quester::choices::QuestChoices::default(),
            };
            BankRun::new_with_required(
                Some(NamedBank::new("Path bank", unmatched)),
                true,
                BankAction::Scan,
                Arc::from([]),
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
            vec![42],
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
        let memo = BankMemo::default();
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        let quests = quest_catalog();

        assert!(matches!(
            poll_once(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                1,
                &plan,
                ProvisionMode::Prepare,
                None,
                false,
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
        let mut plan = provisioning(Vec::new(), vec![42], None);
        plan.tools = Arc::from([crate::native_bank::BankItem {
            id: 42,
            name: Arc::from("Pot"),
        }]);
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
                ProvisionMode::Prepare,
                None,
                false,
                &memo,
                &banks,
                &quests,
            ),
            Poll::Ready(Ok(ProvisionEvent::Ready))
        ));
        assert!(ledger.is_none());
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
            vec![42],
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
                ProvisionMode::Prepare,
                None,
                false,
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
            vec![42],
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
        let memo = BankMemo::default();
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
            ProvisionMode::Prepare,
            None,
            false,
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
                ProvisionMode::Prepare,
                None,
                false,
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
            vec![42],
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
                ProvisionMode::Prepare,
                None,
                false,
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
    fn acquirable_shortfall_runs_its_recipe_and_invalidates_bank_memo() {
        let mut plan = provisioning(
            vec![compiled_item(
                42,
                "Quest token",
                1,
                CompiledItemKind::Acquirable,
                Some("acquire:token"),
            )],
            vec![42],
            None,
        );
        plan.recipes
            .insert(Arc::from("acquire:token"), Arc::from(Vec::new()));
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
            ProvisionMode::Prepare,
            None,
            false,
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
                ProvisionMode::Prepare,
                None,
                false,
                &memo,
                &banks,
                &quests,
            ),
            Poll::Ready(Ok(ProvisionEvent::BankMemoUnknown))
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
        let mut plan = provisioning(Vec::new(), vec![10, 42], None);
        plan.coin_float = 100;
        plan.coin = Some(super::super::compile::CompiledCarry {
            item: coin,
            qty: 100,
            latch_index: u8::MAX,
        });
        plan.base_spillover_keep = Arc::from(vec![10, 42]);
        plan.loadout_carry.insert(
            Arc::from("provision-test/food"),
            Arc::from(vec![super::super::compile::CompiledCarry {
                item: food,
                qty: 3,
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
                ProvisionMode::Prepare,
                Some("provision-test/food"),
                false,
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
                ProvisionMode::Prepare,
                Some("provision-test/food"),
                false,
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
    fn retreat_deposits_extras_and_keeps_every_protected_id() {
        let path_bank = NamedBank::new(
            "Test path bank",
            WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
        );
        let mut plan = provisioning(Vec::new(), vec![7, 42], Some(path_bank));
        plan.tools = Arc::from(vec![crate::native_bank::BankItem {
            id: 7,
            name: Arc::from("Tool"),
        }]);
        plan.keep_ids = Arc::from(vec![7, 8]);
        plan.base_spillover_keep = Arc::from(vec![7, 8]);
        let junk_inventory = item_view(42, "Junk", 1, ItemContainer::Inventory);
        let tool_inventory = item_view(7, "Tool", 1, ItemContainer::Inventory);
        let protected_inventory = item_view(8, "Protected kit", 1, ItemContainer::Inventory);
        let mut snapshot = ready_snapshot(vec![
            protected_inventory.clone(),
            junk_inventory.clone(),
            tool_inventory.clone(),
        ]);
        let side_row = |id, name, slot| ItemView {
            slot,
            component_id: 2006,
            actions: vec![Some("Deposit-1".to_owned()), Some("Deposit-All".to_owned())],
            ..item_view(id, name, 1, ItemContainer::BankSide)
        };
        snapshot.seed_bank_observation(
            1,
            1,
            Some(Vec::new()),
            vec![
                side_row(42, "Junk", 0),
                side_row(7, "Tool", 1),
                side_row(8, "Protected kit", 2),
            ],
        );
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let memo = BankMemo::default();
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        let quests = quest_catalog();

        assert!(poll_once(
            &mut provisioner,
            &snapshot,
            &mut ledger,
            1,
            &plan,
            ProvisionMode::Retreat,
            None,
            false,
            &memo,
            &banks,
            &quests,
        )
        .is_pending());
        assert!(poll_once(
            &mut provisioner,
            &snapshot,
            &mut ledger,
            2,
            &plan,
            ProvisionMode::Retreat,
            None,
            false,
            &memo,
            &banks,
            &quests,
        )
        .is_pending());
        assert!(poll_once(
            &mut provisioner,
            &snapshot,
            &mut ledger,
            3,
            &plan,
            ProvisionMode::Retreat,
            None,
            false,
            &memo,
            &banks,
            &quests,
        )
        .is_pending());
        assert!(ledger.as_ref().is_some_and(|ledger| {
            ledger.outbox.iter().any(|action| {
                matches!(
                    &action.effect,
                    HostEffect::Interaction(crate::shim::InteractReq::InvButton {
                        id: 42,
                        slot: 0,
                        component: 2006,
                        operation: 2,
                        ..
                    })
                )
            })
        }));
        assert!(!ledger.as_ref().is_some_and(|ledger| {
            ledger.outbox.iter().any(|action| {
                matches!(
                    &action.effect,
                    HostEffect::Interaction(crate::shim::InteractReq::InvButton { id: 8, .. })
                )
            })
        }));

        snapshot.seed_inventory(vec![tool_inventory.clone(), protected_inventory], 28);
        snapshot.seed_bank_observation(
            1,
            2,
            Some(vec![item_view(42, "Junk", 1, ItemContainer::Bank)]),
            vec![side_row(7, "Tool", 1), side_row(8, "Protected kit", 2)],
        );
        assert!(matches!(
            poll_once(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                4,
                &plan,
                ProvisionMode::Retreat,
                None,
                false,
                &memo,
                &banks,
                &quests,
            ),
            Poll::Ready(Ok(ProvisionEvent::BankReceipt(BankReceipt {
                complete: true,
                ..
            })))
        ));
        assert!(provisioner.retreat_performed());
        assert!(matches!(
            poll_once(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                5,
                &plan,
                ProvisionMode::Retreat,
                None,
                false,
                &memo,
                &banks,
                &quests,
            ),
            Poll::Ready(Ok(ProvisionEvent::Ready))
        ));
        assert_eq!(provisioner.status().phase, ProvisionPhase::Ready);
    }

    #[test]
    fn owns_inventory_skips_prepare_and_retreat() {
        let mut plan = provisioning(Vec::new(), Vec::new(), None);
        plan.owns_inventory = true;
        let snapshot = ready_snapshot(vec![item_view(
            9,
            "Unrelated item",
            4,
            ItemContainer::Inventory,
        )]);
        let mut provisioner = Provisioner::new();
        let mut ledger = None;
        let memo = BankMemo::default();
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        let quests = quest_catalog();

        assert!(matches!(
            poll_once(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                1,
                &plan,
                ProvisionMode::Prepare,
                None,
                false,
                &memo,
                &banks,
                &quests,
            ),
            Poll::Ready(Ok(ProvisionEvent::Ready))
        ));
        assert!(matches!(
            poll_once(
                &mut provisioner,
                &snapshot,
                &mut ledger,
                2,
                &plan,
                ProvisionMode::Retreat,
                None,
                false,
                &memo,
                &banks,
                &quests,
            ),
            Poll::Ready(Ok(ProvisionEvent::Ready))
        ));
        assert!(!provisioner.retreat_performed());
        assert!(ledger.is_none());
    }
}
