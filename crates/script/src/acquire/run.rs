//! One acquisition run over the existing native bank, walk and wear machines.

use super::plan::{self, Counts};
use super::*;
use crate::bank::{self, BankStandAccess, PickKind};
use crate::gatherer::oneop::{OneOp, OneOpArgs};
use crate::native::walk::Walk;
use crate::native::{ActionContext, ActionHandle, NativeActions, NativeTick, WalkRequest};
use crate::native_bank::{BankMachine, BankRequest};
use api::named_banks::NamedBankFacts;
use api::selected::RunKey;
use api::snapshot::ItemView;
use api::stock::Stock;
use std::task::Poll;

const MAX_OPEN_ATTEMPTS: u8 = 3;
const MAX_CLOSE_ATTEMPTS: u8 = 3;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Select,
    Walk,
    Open,
    Deposit,
    Replan,
    Actions,
    Close,
    Wear,
    Child,
    Resume,
    Done,
}

#[derive(Default)]
enum Active {
    #[default]
    None,
    Select(ActionHandle<bank::Select>),
    Walk(ActionHandle<Walk>),
    Open(ActionHandle<bank::Open>),
    Deposit(ActionHandle<bank::Deposit>),
    Machine(ActionHandle<BankMachine>),
    Close(ActionHandle<bank::Close>),
    Wear(ActionHandle<OneOp>),
}

pub struct AcquireRun {
    request: AcquireRequest,
    banks: Arc<NamedBankFacts>,
    run: RunKey,
    phase: Phase,
    active: Active,
    bank: Option<NamedBank>,
    explicit: Option<Arc<str>>,
    required_bank_missing: bool,
    access: Option<Arc<BankStandAccess>>,
    target: Option<WorldTile>,
    withdrawals: Option<Arc<[Withdrawal]>>,
    wears: Option<Arc<[i32]>>,
    child: Option<u8>,
    index: usize,
    wear_index: usize,
    open_attempts: u8,
    close_attempts: u8,
    deposit_done: bool,
    cancelled: bool,
    complete: bool,
    withdrew_any: bool,
    withdrawal_before: i64,
    deposited: u32,
    kept_unbankable: bool,
    last_bank: Option<BankReceipt>,
    block: Option<Box<AcquireBlock>>,
}

impl AcquireRun {
    pub fn new(
        request: AcquireRequest,
        plan: AcquirePlan,
        banks: &Arc<NamedBankFacts>,
        cx: &ActionContext<'_>,
    ) -> Self {
        let explicit = request.bank.explicit.clone().or_else(|| {
            request
                .bank
                .required
                .then(|| {
                    request.bank.bank.and_then(|wanted| {
                        banks
                            .banks()
                            .iter()
                            .find(|bank| bank.tile == wanted.tile)
                            .map(|bank| Arc::from(bank.name))
                    })
                })
                .flatten()
        });
        let required_bank_missing = request.bank.required && explicit.is_none();
        let block = if let BankStage::Authored(actions) = &request.stage {
            plan::session_shortage(actions, request.partial_ok, &cx.snapshot().stock()).map(
                |message| {
                    Box::new(AcquireBlock {
                        item: Arc::from(""),
                        need: 0,
                        reason: BlockReason::AuthoredShortage,
                        looked: Looked::default(),
                        message,
                    })
                },
            )
        } else {
            None
        };
        let mut run = Self {
            request,
            banks: Arc::clone(banks),
            run: cx.run(),
            phase: Phase::Done,
            active: Active::None,
            bank: None,
            explicit,
            required_bank_missing,
            access: None,
            target: None,
            withdrawals: None,
            wears: None,
            child: None,
            index: 0,
            wear_index: 0,
            open_attempts: 0,
            close_attempts: 0,
            deposit_done: false,
            cancelled: false,
            complete: true,
            withdrew_any: false,
            withdrawal_before: 0,
            deposited: 0,
            kept_unbankable: false,
            last_bank: None,
            block,
        };
        let bank_work = run.install(plan);
        run.start_stage(bank_work, cx);
        run
    }

    pub fn poll(
        &mut self,
        tick: &mut NativeTick<'_>,
        gate: OpenGate,
    ) -> Poll<Result<AcquireOutcome, ActionError>> {
        if tick.cx.run() != self.run {
            self.cancel(tick.actions);
            return Poll::Ready(Err(ActionError::Stale));
        }
        if self.cancelled {
            return Poll::Ready(Err(ActionError::Cancelled));
        }
        if self.required_bank_missing {
            return Poll::Ready(Err(ActionError::Unavailable(Arc::from(
                "required bank is not in the bank catalog",
            ))));
        }
        if let Some(block) = &self.block {
            return Poll::Ready(Err(block.action_error()));
        }
        loop {
            if !matches!(self.active, Active::None) {
                if gate == OpenGate::Hold && matches!(self.active, Active::Open(_)) {
                    return Poll::Pending;
                }
                match self.poll_active(tick) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                    Poll::Ready(Ok(())) => {}
                }
            }
            match self.phase {
                Phase::Select => {
                    if !tick.cx.may_continue_this_tick() {
                        return Poll::Pending;
                    }
                    let Some(from) = tick.cx.snapshot().here() else {
                        return Poll::Ready(Err(ActionError::Unavailable(Arc::from(
                            "player position unavailable for bank selection",
                        ))));
                    };
                    let handle = tick.actions.begin::<bank::Select>(
                        bank::SelectArgs {
                            facts: Arc::clone(&self.banks),
                            from: from.value,
                            preferences: self.request.bank.preferences,
                            options: self.request.walk,
                            explicit: self.explicit.clone(),
                            allow_air_fallback: false,
                        },
                        &mut tick.cx,
                    )?;
                    self.active = Active::Select(handle);
                }
                Phase::Walk => {
                    let Some(target) = self.target else {
                        self.phase = Phase::Open;
                        continue;
                    };
                    if tick
                        .cx
                        .snapshot()
                        .here()
                        .is_some_and(|here| here.value == target)
                    {
                        self.phase = Phase::Open;
                        continue;
                    }
                    if !tick.cx.may_continue_this_tick() {
                        return Poll::Pending;
                    }
                    let handle = tick.actions.begin::<Walk>(
                        WalkRequest {
                            target,
                            candidate_goals: None,
                            loc_id: None,
                            radius: 0,
                            arrival: nav_planner::contract::ArrivalKind::Reach,
                            options: self.request.walk,
                            required_after: self.request.required_after,
                            evidence: None,
                            cross: Box::default(),
                            protect: false,
                            food_guard: false,
                            allow: Default::default(),
                        },
                        &mut tick.cx,
                    )?;
                    self.active = Active::Walk(handle);
                }
                Phase::Open => {
                    if gate == OpenGate::Hold || !tick.cx.may_continue_this_tick() {
                        return Poll::Pending;
                    }
                    if self.open_attempts == MAX_OPEN_ATTEMPTS {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "bank-unavailable: bank did not open after 3 attempts",
                        ))));
                    }
                    let Some(access) = &self.access else {
                        return Poll::Ready(Err(ActionError::Unavailable(Arc::from(
                            "selected bank has no access stand",
                        ))));
                    };
                    let handle = tick.actions.begin::<bank::Open>(
                        bank::OpenArgs {
                            access: Arc::clone(access),
                        },
                        &mut tick.cx,
                    )?;
                    self.open_attempts += 1;
                    self.active = Active::Open(handle);
                }
                Phase::Deposit => {
                    if self.deposit_done || self.request.deposit.is_none() {
                        self.phase = self.after_deposit();
                        continue;
                    }
                    if !tick.cx.may_continue_this_tick() {
                        return Poll::Pending;
                    }
                    let selected = api::game_data::for_revision(tick.cx.pin().revision)
                        .map_err(|error| ActionError::Unavailable(Arc::from(error)))?;
                    match self.request.deposit.as_ref().expect("checked deposit") {
                        DepositSpec::Ids { products, keep } => {
                            if let Some(rows) = tick.cx.snapshot().inventory() {
                                self.kept_unbankable |= rows.value.iter().any(|row| {
                                    row.count > 0
                                        && keep.contains(&row.def.id)
                                        && super::unbankable_index(&selected, row).is_some()
                                });
                            }
                            let handle = tick.actions.begin::<bank::Deposit>(
                                bank::DepositArgs {
                                    products: Arc::clone(products),
                                    keep: Arc::clone(keep),
                                    access: self.access.clone(),
                                },
                                &mut tick.cx,
                            )?;
                            self.active = Active::Deposit(handle);
                        }
                        DepositSpec::Capacity { keep, slots } => {
                            let handle = tick.actions.begin::<BankMachine>(
                                BankRequest {
                                    bank: self.bank,
                                    action: BankAction::DepositCapacity {
                                        keep: Arc::clone(keep),
                                        slots: *slots,
                                    },
                                    partial_ok: false,
                                    access: self.access.clone(),
                                },
                                &mut tick.cx,
                            )?;
                            self.active = Active::Machine(handle);
                        }
                    }
                }
                Phase::Replan => {
                    match self.replan(tick, true)? {
                        AcquireVerdict::Satisfied => {
                            self.clear_work();
                        }
                        AcquireVerdict::Plan(plan) => {
                            self.install(plan);
                        }
                        AcquireVerdict::Blocked(block) => return self.refuse(block),
                    }
                    self.phase = Phase::Actions;
                }
                Phase::Actions => {
                    let Some(action) = self.action() else {
                        self.phase = Phase::Close;
                        continue;
                    };
                    if !tick.cx.may_continue_this_tick() {
                        return Poll::Pending;
                    }
                    let before = withdrawn_count(&action, &tick.cx.snapshot().stock());
                    let handle = tick.actions.begin::<BankMachine>(
                        BankRequest {
                            bank: self.bank,
                            action,
                            partial_ok: self.request.partial_ok,
                            access: self.access.clone(),
                        },
                        &mut tick.cx,
                    )?;
                    self.withdrawal_before = before;
                    self.active = Active::Machine(handle);
                }
                Phase::Close => {
                    if self.request.close == CloseBank::LeaveOpen || !bank_open(&tick.cx) {
                        self.phase = Phase::Wear;
                        continue;
                    }
                    if !tick.cx.may_continue_this_tick() {
                        return Poll::Pending;
                    }
                    if self.close_attempts == MAX_CLOSE_ATTEMPTS {
                        return Poll::Ready(Err(ActionError::Failed(Arc::from(
                            "bank-close-failed: bank did not close after 3 attempts",
                        ))));
                    }
                    let handle = tick.actions.begin::<bank::Close>((), &mut tick.cx)?;
                    self.close_attempts += 1;
                    self.active = Active::Close(handle);
                }
                Phase::Wear => {
                    let Some(id) = self
                        .wears
                        .as_deref()
                        .and_then(|ids| ids.get(self.wear_index))
                        .copied()
                    else {
                        self.phase = Phase::Child;
                        continue;
                    };
                    if tick
                        .cx
                        .snapshot()
                        .stock()
                        .worn(id)
                        .is_some_and(|count| count > 0)
                    {
                        self.wear_index += 1;
                        continue;
                    }
                    if !tick.cx.may_continue_this_tick() {
                        return Poll::Pending;
                    }
                    let snapshot = tick.cx.snapshot();
                    let Some(rows) = snapshot.inventory() else {
                        return Poll::Pending;
                    };
                    let row = rows
                        .value
                        .iter()
                        .find(|row| row.def.id == id && row.count > 0)
                        .ok_or_else(|| {
                            ActionError::Unavailable(Arc::from(
                                "the selected item left the inventory",
                            ))
                        })?;
                    let name = row.def.name.as_deref().ok_or_else(|| {
                        ActionError::Unavailable(Arc::from(
                            "the selected item has no observed name",
                        ))
                    })?;
                    let action = row
                        .actions
                        .iter()
                        .flatten()
                        .find(|action| {
                            action.eq_ignore_ascii_case("Wield")
                                || action.eq_ignore_ascii_case("Wear")
                        })
                        .ok_or_else(|| {
                            ActionError::Unavailable(Arc::from(
                                "the selected item has no observed equip action",
                            ))
                        })?;
                    let handle = tick.actions.begin::<OneOp>(
                        OneOpArgs::wear(Arc::from(name), id, action),
                        &mut tick.cx,
                    )?;
                    self.active = Active::Wear(handle);
                }
                Phase::Child => {
                    if let Some(need) = self.child {
                        return Poll::Ready(Ok(AcquireOutcome::ChildDue { need }));
                    }
                    self.phase = Phase::Done;
                }
                Phase::Resume => {
                    let live = bank_open(&tick.cx);
                    match self.replan(tick, live)? {
                        AcquireVerdict::Satisfied => {
                            self.clear_work();
                            self.phase = Phase::Close;
                        }
                        AcquireVerdict::Plan(plan) => {
                            let bank_work = self.install(plan);
                            self.start_stage(bank_work, &tick.cx);
                        }
                        AcquireVerdict::Blocked(block) => return self.refuse(block),
                    }
                }
                Phase::Done => {
                    return Poll::Ready(Ok(AcquireOutcome::Done(AcquireReceipt {
                        complete: self.complete,
                        withdrew_any: self.withdrew_any,
                        deposited: self.deposited,
                        kept_unbankable: self.kept_unbankable,
                        bank_open: bank_open(&tick.cx),
                        bought: 0,
                        last_bank: self.last_bank.take(),
                    })));
                }
            }
            if !tick.cx.may_continue_this_tick() && !matches!(self.active, Active::None) {
                return Poll::Pending;
            }
        }
    }

    pub fn resume_after_child(&mut self) {
        if !self.cancelled {
            self.phase = Phase::Resume;
        }
    }

    pub fn block(&self) -> Option<&AcquireBlock> {
        self.block.as_deref()
    }

    pub fn cancel(&mut self, actions: &mut NativeActions) {
        match std::mem::take(&mut self.active) {
            Active::None => {}
            Active::Select(handle) => actions.cancel(handle),
            Active::Walk(handle) => actions.cancel(handle),
            Active::Open(handle) => actions.cancel(handle),
            Active::Deposit(handle) => actions.cancel(handle),
            Active::Machine(handle) => actions.cancel(handle),
            Active::Close(handle) => actions.cancel(handle),
            Active::Wear(handle) => actions.cancel(handle),
        }
        self.phase = Phase::Done;
        self.cancelled = true;
    }

    fn install(&mut self, mut plan: AcquirePlan) -> bool {
        self.withdrawals = (plan.withdrawal_count > 0).then(|| {
            plan.withdrawals[..plan.withdrawal_count]
                .iter_mut()
                .filter_map(Option::take)
                .collect::<Vec<_>>()
                .into()
        });
        self.wears = (plan.wear_count > 0).then(|| {
            plan.wears[..plan.wear_count]
                .iter()
                .flatten()
                .copied()
                .collect::<Vec<_>>()
                .into()
        });
        self.child = plan.child;
        self.index = 0;
        self.wear_index = 0;
        plan.authored
            || plan.scan
            || self.withdrawals.is_some()
            || (self.request.deposit.is_some() && !self.deposit_done)
    }

    fn clear_work(&mut self) {
        self.withdrawals = None;
        self.wears = None;
        self.child = None;
        self.index = 0;
        self.wear_index = 0;
    }

    fn start_stage(&mut self, bank_work: bool, cx: &ActionContext<'_>) {
        if !bank_work {
            self.phase = Phase::Close;
        } else if bank_open(cx) && !self.request.bank.required {
            self.bank = self.request.bank.bank;
            self.phase = Phase::Deposit;
        } else {
            self.phase = Phase::Select;
        }
    }

    fn after_deposit(&self) -> Phase {
        if matches!(self.request.stage, BankStage::Authored(_)) {
            Phase::Actions
        } else {
            Phase::Replan
        }
    }

    fn action(&self) -> Option<BankAction> {
        match &self.request.stage {
            BankStage::Authored(actions) => actions.get(self.index).cloned(),
            BankStage::Planned if self.index == 0 => {
                self.withdrawals
                    .as_ref()
                    .map(|withdrawals| BankAction::WithdrawTo {
                        withdrawals: Arc::clone(withdrawals),
                    })
            }
            BankStage::Planned => None,
        }
    }

    fn replan(&self, tick: &NativeTick<'_>, live: bool) -> Result<AcquireVerdict, ActionError> {
        let snapshot = tick.cx.snapshot();
        let data = api::game_data::for_revision(tick.cx.pin().revision)
            .map_err(|error| ActionError::Unavailable(Arc::from(error)))?;
        let world = WorldFacts {
            data: Some(&data),
            members: snapshot.world().is_some_and(|world| world.value.members),
        };
        let stock = snapshot.stock();
        if live {
            let rows = snapshot.bank().ok_or_else(|| {
                ActionError::Unavailable(Arc::from("bank item table unavailable"))
            })?;
            plan::plan(
                &self.request,
                &LiveCounts {
                    stock,
                    rows: rows.value,
                },
                world,
            )
        } else {
            plan::plan(&self.request, &stock, world)
        }
    }

    fn refuse(&mut self, block: AcquireBlock) -> Poll<Result<AcquireOutcome, ActionError>> {
        let error = block.action_error();
        self.block = Some(Box::new(block));
        Poll::Ready(Err(error))
    }

    fn poll_active(&mut self, tick: &mut NativeTick<'_>) -> Poll<Result<(), ActionError>> {
        match std::mem::take(&mut self.active) {
            Active::None => {}
            Active::Select(handle) => match tick.actions.poll(&handle, &mut tick.cx) {
                Poll::Pending => self.active = Active::Select(handle),
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(selected)) => {
                    if selected.kind == PickKind::NoCandidate {
                        return Poll::Ready(Err(ActionError::Unavailable(Arc::from(
                            "no eligible bank",
                        ))));
                    }
                    let access = selected.access.ok_or_else(|| {
                        ActionError::Unavailable(Arc::from("selected bank has no access stand"))
                    })?;
                    self.bank = Some(
                        *self
                            .banks
                            .banks()
                            .get(usize::from(selected.bank_index))
                            .ok_or(ActionError::Stale)?,
                    );
                    self.target = Some(selected.access_tile);
                    self.access = Some(access);
                    self.phase = Phase::Walk;
                }
            },
            Active::Walk(handle) => match tick.actions.poll(&handle, &mut tick.cx) {
                Poll::Pending => self.active = Active::Walk(handle),
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(receipt)) => {
                    receipt.into_arrival()?;
                    self.phase = Phase::Open;
                }
            },
            Active::Open(handle) => match tick.actions.poll(&handle, &mut tick.cx) {
                Poll::Pending => self.active = Active::Open(handle),
                Poll::Ready(Ok(())) => self.phase = Phase::Deposit,
                Poll::Ready(Err(ActionError::Failed(message)))
                    if matches!(self.request.stage, BankStage::Planned)
                        && matches!(
                            message.as_ref(),
                            "bank did not open" | "bank item table unavailable"
                        ) =>
                {
                    self.phase = Phase::Open
                }
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            },
            Active::Deposit(handle) => match tick.actions.poll(&handle, &mut tick.cx) {
                Poll::Pending => self.active = Active::Deposit(handle),
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(deposited)) => {
                    self.deposited = self.deposited.saturating_add(deposited);
                    self.deposit_done = true;
                    self.phase = self.after_deposit();
                }
            },
            Active::Machine(handle) => match tick.actions.poll(&handle, &mut tick.cx) {
                Poll::Pending => self.active = Active::Machine(handle),
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(receipt)) => {
                    self.complete &= receipt.complete;
                    self.deposited = self.deposited.saturating_add(receipt.deposited);
                    if self.phase == Phase::Deposit {
                        self.deposit_done = true;
                        self.phase = self.after_deposit();
                    } else {
                        if let Some(action) = self.action() {
                            self.withdrew_any |=
                                withdrawn_count(&action, &tick.cx.snapshot().stock())
                                    > self.withdrawal_before;
                            if !matches!(action, BankAction::Close) {
                                self.last_bank = Some(receipt);
                            }
                        }
                        self.index += 1;
                    }
                }
            },
            Active::Close(handle) => match tick.actions.poll(&handle, &mut tick.cx) {
                Poll::Pending => self.active = Active::Close(handle),
                Poll::Ready(Ok(())) => self.phase = Phase::Wear,
                Poll::Ready(Err(ActionError::Failed(message)))
                    if message.as_ref() == "bank did not close" =>
                {
                    self.phase = Phase::Close;
                }
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            },
            Active::Wear(handle) => match tick.actions.poll(&handle, &mut tick.cx) {
                Poll::Pending => self.active = Active::Wear(handle),
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(false)) => {
                    return Poll::Ready(Err(ActionError::Failed(Arc::from(
                        "the selected item was not equipped",
                    ))))
                }
                Poll::Ready(Ok(true)) => self.wear_index += 1,
            },
        }
        if matches!(self.active, Active::None) {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }
}

fn bank_open(cx: &ActionContext<'_>) -> bool {
    cx.snapshot()
        .bank_session()
        .is_some_and(|session| session.value.open)
}

fn withdrawn_count(action: &BankAction, stock: &Stock<'_>) -> i64 {
    match action {
        BankAction::Withdraw { item, .. } => i64::from(stock.held(item.id).unwrap_or(0)),
        BankAction::WithdrawAny { items, .. } => items
            .iter()
            .map(|item| i64::from(stock.held(item.id).unwrap_or(0)))
            .sum(),
        BankAction::WithdrawTo { withdrawals } => withdrawals
            .iter()
            .map(|item| i64::from(stock.held(item.id).unwrap_or(0)))
            .sum(),
        _ => 0,
    }
}

struct LiveCounts<'a> {
    stock: Stock<'a>,
    rows: &'a [ItemView],
}

impl Counts for LiveCounts<'_> {
    fn held(&self, id: i32) -> Option<i32> {
        self.stock.held(id)
    }
    fn worn(&self, id: i32) -> Option<i32> {
        self.stock.worn(id)
    }
    fn banked(&self, id: i32) -> Option<i32> {
        Some(
            self.rows
                .iter()
                .filter(|row| row.def.id == id)
                .fold(0_i32, |count, row| count.saturating_add(row.count.max(0))),
        )
    }
    fn banked_origin(&self) -> Origin {
        Origin::Session
    }
    fn bank_seen(&self) -> Option<BankSeen> {
        let observed_at_unix = self
            .stock
            .bank
            .and_then(|bank| bank.observed_at())
            .map_or_else(
                || {
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |time| time.as_secs())
                },
                |at| at.unix_secs,
            );
        Some(BankSeen {
            origin: Origin::Session,
            observed_at_unix,
            rows: u16::try_from(self.rows.iter().filter(|row| row.count > 0).count())
                .unwrap_or(u16::MAX),
            count: 0,
        })
    }
    fn free_slots(&self) -> Option<i32> {
        self.stock.free_slots()
    }
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
