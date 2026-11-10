//! Slot-owned bank operations and the BankNearest callback continuation.
//!
//! Only the running primitive owns a native action. The trip releases that
//! action before afterDeposit, so a callback may start its own bank operation.
use super::*;
use crate::bank::ops::{self, NoteIntent, TRANSFER_OBSERVED_TICK_LIMIT};
use crate::bank::request::{
    BankPackRow, BankSeatArgs, BankSeatOperation, BANK_NEAREST_HOOK_AFTER_DEPOSIT,
    BANK_NEAREST_HOOK_DEPOSIT, BANK_NEAREST_HOOK_LOG, DEPOSIT_MATCH_HOOK_MATCH, HOOK_LOG,
};
use crate::bank::seat_open::{Opening, OpeningArgs, OpeningValue};
use crate::bank::seat_sort::{Sort, SortArgs};
use crate::bank::seat_transfer::{
    Transfer, TransferAnswer, TransferArgs, TransferCall, TransferHooks, TransferValue,
};
use crate::local_state::SlotState;
use crate::native::request::NativeKind;
use crate::native::snapshot_source::BankSelectionInput;
use crate::native::{ActionContext, ActionError, ActionHandle, NativeActions, NativeMachine};
use crate::native_bank::{bank_value, PickKind, SelectedBank};
use crate::quester::prereq::reach::PrereqOpening;
use crate::quester::prereq::walk::PrereqWalk;
use api::named_banks::{BankPreferences, NamedBankFacts};
use api::quest_progress::EvidenceStamp;
use api::snapshot::{ItemView, SnapshotView, WorldTile};
use serde_json::{json, Value};
use std::sync::Mutex;
use std::task::Poll;
use std::time::Duration;

/// Which bank rows one poll visits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BankPoll {
    /// Every row: a game tick or an evidence wake.
    All,
    /// Only the rows whose answer the seat holds: a seat answer wake.
    Answered,
}

#[derive(Default)]
pub(super) struct BankSeat {
    rows: Vec<BankRow>,
    values: Vec<(RequestId, Vec<u8>)>,
    /// The native ledger is shared by the slot. This id supplies its seat lineage.
    pub(super) action_owner: Option<RequestId>,
    note: Option<(u64, NoteIntent)>,
    selection: BankSelectionState,
}

#[derive(Default)]
struct BankSelectionState {
    generation: u64,
    current_request_id: Option<RequestId>,
    published: BankSelectionInput,
    published_runtime_generation: Option<u64>,
}

impl BankSelectionState {
    fn advance(&mut self) -> Option<u64> {
        let generation = self.generation.checked_add(1)?;
        self.generation = generation;
        Some(generation)
    }

    fn admit(&mut self, id: RequestId) -> Option<u64> {
        let generation = self.advance()?;
        self.current_request_id = Some(id);
        Some(generation)
    }

    fn invalidate(&mut self) {
        let _ = self.advance();
        self.current_request_id = None;
        self.published = BankSelectionInput::default();
        self.published_runtime_generation = None;
    }

    fn for_runtime(&self, runtime_generation: u64) -> BankSelectionInput {
        if self.published_runtime_generation == Some(runtime_generation) {
            self.published
        } else {
            BankSelectionInput::default()
        }
    }

    fn publish(
        &mut self,
        id: RequestId,
        generation: u64,
        runtime_generation: u64,
        current_runtime_generation: u64,
        selected: SelectedBank,
    ) {
        if self.current_request_id != Some(id)
            || self.generation != generation
            || runtime_generation != current_runtime_generation
        {
            return;
        }
        self.published = BankSelectionInput {
            request_id: id.get(),
            generation,
            bank_index: if selected.kind == PickKind::NoCandidate {
                -1
            } else {
                i32::from(selected.bank_index)
            },
            kind: selected.kind as u8,
        };
        self.published_runtime_generation = Some(runtime_generation);
    }
}

struct BankRow {
    id: RequestId,
    runtime_generation: u64,
    selection_generation: Option<u64>,
    selected_bank: Option<SelectedBank>,
    family: &'static str,
    job: Job,
    trip: Option<Trip>,
    /// Native transaction behind purgePackAtBank or withdrawCoins.
    task: Option<Task>,
    hooks: Arc<Mutex<TransferHooks>>,
    prompt_posted: bool,
}

/// A multi-step native bank transaction run by this row's jobs.
enum Task {
    Purge(Purge),
    Coins(Coins),
}

/// `purgePackAtBank` (`Banking.ts:62-111`): open, deposit every unkept row,
/// close, then verify the remaining pack.
struct Purge {
    keep: Vec<String>,
    log: bool,
    stage: PurgeStage,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PurgeStage {
    Opening,
    Depositing,
    Closing,
}

/// `withdrawCoins` (`Banking.ts:354-390`): the held target the fresh pack
/// count must reach after the withdrawal.
struct Coins {
    coin: String,
    target: i64,
}

/// `Bank.backpackReady` (`Bank.ts:124-146`) counted in observed ticks.
struct BackpackWait {
    expected: Option<Vec<BankPackRow>>,
    log: bool,
    matching: bool,
    started_tick: Option<u64>,
}

struct Trip {
    return_to: Option<WorldTile>,
    deposit_all: bool,
    common_junk: bool,
    hooks_present: u32,
}

enum Job {
    Start(BankSeatArgs),
    StartOpening(OpeningArgs),
    Opening {
        handle: ActionHandle<Opening>,
        args: OpeningArgs,
    },
    PrereqOpening(PrereqOpening),
    Transfer(ActionHandle<Transfer>),
    ResumeOpening {
        kind: NativeKind,
        handle: ActionHandle<Opening>,
        args: OpeningArgs,
    },
    ResumePrereqOpening {
        kind: NativeKind,
        action: PrereqOpening,
    },
    ResumeTransfer {
        kind: NativeKind,
        handle: ActionHandle<Transfer>,
    },
    /// Observation-only waits do not contend with a running transfer.
    Wait(Transfer),
    AfterDeposit {
        asked: bool,
    },
    Settle {
        due: u64,
    },
    Return {
        deadline: Option<Duration>,
    },
    ReturnWalk {
        walk: PrereqWalk,
        deadline: Duration,
    },
    /// `bankPace`: the evidence accepted at the first poll; settles on the
    /// next observed game tick of that run. A kept reconnect holds the pause
    /// and re-accepts it on the new session's first observed tick.
    Pace {
        accepted: Option<EvidenceStamp>,
    },
    /// `Bank.backpackReady`: observation-only, no action is owned.
    Backpack(BackpackWait),
    /// `sortBank`.
    Sort(ActionHandle<Sort>),
    /// A sort held across a kept reconnect; it resumes at its cursor and
    /// reports its retained progress through the closed-bank path.
    ResumeSort {
        kind: NativeKind,
        handle: ActionHandle<Sort>,
    },
    /// Post one synchronous `log` notification, then continue with `then`.
    Log {
        message: String,
        asked: bool,
        then: Box<Job>,
    },
    /// A settled result that waits only for a preceding log.
    Settled(Value),
    Finished,
}

fn is_opening(operation: BankSeatOperation) -> bool {
    matches!(
        operation,
        BankSeatOperation::OpenBooth
            | BankSeatOperation::OpenNearest
            | BankSeatOperation::OpenNearestWorld
            | BankSeatOperation::OpenObject
            | BankSeatOperation::OpenNpc
            | BankSeatOperation::OpenAccess
            | BankSeatOperation::Open
            | BankSeatOperation::Select
    )
}

fn family(operation: BankSeatOperation) -> &'static str {
    match operation {
        BankSeatOperation::OpenBooth
        | BankSeatOperation::OpenNearest
        | BankSeatOperation::OpenNearestWorld => "bank_open",
        BankSeatOperation::OpenObject => "bank_access",
        BankSeatOperation::OpenNpc => "bank_npc_access",
        BankSeatOperation::OpenAccess => "bank_open_access",
        BankSeatOperation::Open => "banking_open",
        BankSeatOperation::Select => "bank_select",
        BankSeatOperation::DepositAll
        | BankSeatOperation::DepositKeep
        | BankSeatOperation::DepositMatch => "bank_deposit",
        BankSeatOperation::BankNearest => "bank_nearest",
        BankSeatOperation::WithdrawLoad => "bank_withdraw_load",
        BankSeatOperation::SetNoteMode => "bank_note_mode",
        BankSeatOperation::Close => "bank_close",
        BankSeatOperation::WaitReady | BankSeatOperation::WaitSnapshotAfter => "bank_wait",
        BankSeatOperation::PurgePack
        | BankSeatOperation::WithdrawCoins
        | BankSeatOperation::Pace
        | BankSeatOperation::BackpackReady
        | BankSeatOperation::Sort => "bank_task",
        _ => "bank_op",
    }
}

impl BankSeat {
    pub(super) fn result_values<'a>(
        &'a self,
        output: &mut [crate::native::seat_rows::SeatResultValue<'a>],
    ) -> usize {
        for (out, (id, value)) in output.iter_mut().zip(&self.values) {
            *out = crate::native::seat_rows::SeatResultValue { id: *id, value };
        }
        self.values.len()
    }

    fn start(
        &mut self,
        id: RequestId,
        mut request: BankSeatArgs,
        runtime_generation: u64,
    ) -> Result<(), ActionError> {
        if self.rows.iter().any(|row| row.id == id) {
            return Ok(());
        }
        let selection_generation = if request.operation == BankSeatOperation::Select {
            Some(self.selection.admit(id).ok_or(ActionError::Cancelled)?)
        } else {
            None
        };
        let family = family(request.operation);
        let trip = (request.operation == BankSeatOperation::BankNearest).then(|| {
            let trip = Trip {
                return_to: request.return_to.take(),
                deposit_all: request.deposit_all,
                common_junk: request.common_junk,
                hooks_present: request.hooks_present,
            };
            request.operation = BankSeatOperation::Open;
            request.deposit_all = false;
            request.common_junk = true;
            request.hooks_present = if trip.hooks_present & BANK_NEAREST_HOOK_LOG != 0 {
                HOOK_LOG
            } else {
                0
            };
            trip
        });
        self.rows.push(BankRow {
            id,
            runtime_generation,
            selection_generation,
            selected_bank: None,
            family,
            job: Job::Start(request),
            trip,
            task: None,
            hooks: Arc::new(Mutex::new(TransferHooks::default())),
            prompt_posted: false,
        });
        Ok(())
    }

    fn clear_work(&mut self) {
        self.rows.clear();
        self.values.clear();
        self.action_owner = None;
        self.note = None;
        self.selection.invalidate();
    }
}

impl BankRow {
    fn transfer_args(
        &self,
        request: BankSeatArgs,
        note_intent: NoteIntent,
        common_casket_id: Option<i32>,
    ) -> TransferArgs {
        TransferArgs {
            request,
            hooks: Arc::clone(&self.hooks),
            note_intent,
            matcher_hook: if self.trip.is_some() {
                "deposit"
            } else {
                "match"
            },
            matcher_with_id: self.trip.is_none(),
            common_junk: self.trip.as_ref().is_some_and(|trip| trip.common_junk),
            common_casket_id,
        }
    }

    fn after_open(&mut self, ready: bool) -> Option<Value> {
        if let Some(Task::Purge(purge)) = &mut self.task {
            if purge.stage == PurgeStage::Opening {
                if !ready {
                    let log = purge.log;
                    self.job = log_then(
                        log,
                        "start purge: could not open bank — leaving pack as-is".into(),
                        Job::Settled(json!(false)),
                    );
                    return None;
                }
                purge.stage = PurgeStage::Depositing;
                let mut request = BankSeatArgs::new(BankSeatOperation::DepositKeep);
                request.keep_names = purge.keep.clone();
                self.job = Job::Start(request);
                return None;
            }
        }
        let Some(trip) = self.trip.as_ref().filter(|_| ready) else {
            return Some(json!(ready));
        };
        let operation = if trip.hooks_present & BANK_NEAREST_HOOK_DEPOSIT != 0 {
            Some(BankSeatOperation::DepositMatch)
        } else if trip.deposit_all {
            Some(BankSeatOperation::DepositAll)
        } else {
            None
        };
        self.job = match operation {
            Some(operation) => {
                let mut request = BankSeatArgs::new(operation);
                request.hooks_present = if operation == BankSeatOperation::DepositMatch {
                    DEPOSIT_MATCH_HOOK_MATCH
                } else {
                    0
                };
                Job::Start(request)
            }
            None => Job::AfterDeposit { asked: false },
        };
        None
    }

    /// The first poll of a bank task: decide from the posted facts, then hand
    /// the shared open/transfer jobs their typed requests.
    fn start_task(&mut self, request: BankSeatArgs, cx: &ActionContext<'_>) -> Job {
        let log = request.hooks_present & HOOK_LOG != 0;
        match request.operation {
            BankSeatOperation::PurgePack => {
                let snapshot = cx.snapshot();
                let junk = purge_junk(snapshot, &request.keep_names);
                if junk == 0 {
                    return Job::Settled(json!(true));
                }
                let keep = if request.keep_names.is_empty() {
                    "none".to_owned()
                } else {
                    request.keep_names.join(", ")
                };
                let message = format!("start purge: banking {junk} stack(s) (keep: {keep})");
                let mut open = BankSeatArgs::new(BankSeatOperation::Open);
                open.stand = request.stand;
                open.destination = request.destination;
                open.object_access = request.object_access;
                open.prefer_nearby = request.prefer_nearby;
                open.obstacles = request.obstacles;
                open.hooks_present = if log { HOOK_LOG } else { 0 };
                self.task = Some(Task::Purge(Purge {
                    keep: request.keep_names,
                    log,
                    stage: PurgeStage::Opening,
                }));
                log_then(log, message, Job::Start(open))
            }
            BankSeatOperation::WithdrawCoins => {
                let snapshot = cx.snapshot();
                let coin = request.item_name.unwrap_or_else(|| "Coins".into());
                let need = request.quantity.unwrap_or(0);
                let held = i64::from(count_named(
                    snapshot.inventory().map_or(&[][..], |pack| pack.value),
                    &coin,
                ));
                let take = need.saturating_sub(held).max(0);
                if take <= 0 {
                    return Job::Settled(json!(true));
                }
                let open = snapshot
                    .bank_session()
                    .is_some_and(|session| session.value.open);
                if !open {
                    return Job::Settled(json!(false));
                }
                let banked = i64::from(
                    snapshot
                        .bank()
                        .map_or(0, |rows| count_named(rows.value, &coin)),
                );
                if banked <= 0 {
                    return log_then(
                        log,
                        format!("acquire: no coins in bank (need {need}gp)"),
                        Job::Settled(json!(false)),
                    );
                }
                let amount = take.min(banked).min(i64::from(i32::MAX));
                let mut withdraw = BankSeatArgs::new(BankSeatOperation::WithdrawDelta);
                withdraw.item_name = Some(coin.clone());
                withdraw.quantity = Some(amount);
                self.task = Some(Task::Coins(Coins {
                    target: need.min(held.saturating_add(amount)),
                    coin,
                }));
                log_then(
                    log,
                    format!("acquire: withdraw {amount}gp"),
                    Job::Start(withdraw),
                )
            }
            BankSeatOperation::Pace => Job::Pace { accepted: None },
            BankSeatOperation::BackpackReady => Job::Backpack(BackpackWait {
                expected: request.expected_pack,
                log,
                matching: false,
                started_tick: None,
            }),
            _ => Job::Start(request),
        }
    }

    /// A finished transfer inside a task: continue the transaction, or
    /// `None` when this row has no task.
    fn task_transfer_done(
        &mut self,
        result: Result<TransferValue, ActionError>,
        cx: &ActionContext<'_>,
    ) -> Option<Result<Job, ActionError>> {
        match self.task.as_mut()? {
            Task::Purge(purge) => match purge.stage {
                PurgeStage::Opening => Some(Err(ActionError::Stale)),
                PurgeStage::Depositing => {
                    if let Err(error) = result {
                        return Some(Err(error));
                    }
                    // Upstream ignores the close result (`Banking.ts:101`).
                    purge.stage = PurgeStage::Closing;
                    Some(Ok(Job::Start(BankSeatArgs::new(BankSeatOperation::Close))))
                }
                PurgeStage::Closing => {
                    let left = purge_junk(cx.snapshot(), &purge.keep);
                    let (message, clean) = if left > 0 {
                        (
                            format!(
                                "start purge: still holding {left} junk stack(s) after deposit"
                            ),
                            false,
                        )
                    } else {
                        ("start purge: pack cleared".to_owned(), true)
                    };
                    Some(Ok(log_then(purge.log, message, Job::Settled(json!(clean)))))
                }
            },
            Task::Coins(coins) => match result {
                Err(error) => Some(Err(error)),
                Ok(TransferValue::Bool(true)) => {
                    let held = cx
                        .snapshot()
                        .inventory()
                        .map_or(0, |pack| count_named(pack.value, &coins.coin));
                    Some(Ok(Job::Settled(json!(i64::from(held) >= coins.target))))
                }
                Ok(_) => Some(Ok(Job::Settled(json!(false)))),
            },
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "The bank seat borrows its separate owners and selected facts without another context allocation."
    )]
    fn poll(
        &mut self,
        actions: &mut NativeActions,
        cx: &mut ActionContext<'_>,
        slot_state: Option<&SlotState>,
        facts: &Arc<NamedBankFacts>,
        preferences: BankPreferences,
        note: &mut Option<(u64, NoteIntent)>,
        common_casket_id: Option<i32>,
    ) -> Poll<Result<Value, ActionError>> {
        // A same-frame continuation can change policy but never replenishes
        // the slot's action or transition budget.
        loop {
            let job = std::mem::replace(&mut self.job, Job::Finished);
            match job {
                Job::Start(request) => {
                    if matches!(
                        request.operation,
                        BankSeatOperation::PurgePack
                            | BankSeatOperation::WithdrawCoins
                            | BankSeatOperation::Pace
                            | BankSeatOperation::BackpackReady
                    ) {
                        self.job = self.start_task(request, cx);
                        continue;
                    }
                    if request.operation == BankSeatOperation::Sort {
                        if !cx.may_begin_this_tick() {
                            self.job = Job::Start(request);
                            return Poll::Pending;
                        }
                        let args = SortArgs {
                            log: request.hooks_present & HOOK_LOG != 0,
                            overrides: request.category_overrides,
                            hooks: Arc::clone(&self.hooks),
                        };
                        match actions.begin::<Sort>(args, cx) {
                            Ok(handle) => self.job = Job::Sort(handle),
                            Err(error) => return Poll::Ready(Err(error)),
                        }
                        return Poll::Pending;
                    }
                    let waiting = matches!(
                        request.operation,
                        BankSeatOperation::WaitReady | BankSeatOperation::WaitSnapshotAfter
                    );
                    if !waiting && !cx.may_begin_this_tick() {
                        self.job = Job::Start(request);
                        return Poll::Pending;
                    }
                    if is_opening(request.operation) {
                        self.job = Job::StartOpening(OpeningArgs {
                            request,
                            facts: Arc::clone(facts),
                            preferences,
                            hooks: Arc::clone(&self.hooks),
                            selection_refused: false,
                        });
                        continue;
                    } else {
                        let generation = cx
                            .snapshot()
                            .bank_session()
                            .map_or(0, |session| session.value.generation);
                        let intent = note.map_or(NoteIntent::Item, |(stored, intent)| {
                            ops::note_intent(stored, intent, generation)
                        });
                        let waiting = matches!(
                            request.operation,
                            BankSeatOperation::WaitReady | BankSeatOperation::WaitSnapshotAfter
                        );
                        let args = self.transfer_args(request, intent, common_casket_id);
                        if waiting {
                            match Transfer::begin(args, cx) {
                                Ok(wait) => self.job = Job::Wait(wait),
                                Err(error) => return Poll::Ready(Err(error)),
                            }
                        } else {
                            match actions.begin::<Transfer>(args, cx) {
                                Ok(handle) => self.job = Job::Transfer(handle),
                                Err(error) => return Poll::Ready(Err(error)),
                            }
                        }
                    }
                    // The begin may emit. Never complete its owner before that
                    // emission reaches the host's dispatch lane.
                    return Poll::Pending;
                }
                Job::StartOpening(args) => {
                    if !cx.may_begin_this_tick() {
                        self.job = Job::StartOpening(args);
                        return Poll::Pending;
                    }
                    match actions.begin::<Opening>(args.clone(), cx) {
                        Ok(handle) => self.job = Job::Opening { handle, args },
                        Err(ActionError::BudgetExhausted | ActionError::Held) => {
                            return Poll::Ready(Err(ActionError::BudgetExhausted));
                        }
                        Err(error) => return Poll::Ready(Err(error)),
                    }
                    return Poll::Pending;
                }
                Job::ResumeOpening {
                    kind,
                    mut handle,
                    args,
                } => match actions.resume_kept(kind, &mut handle, cx) {
                    Ok(()) => self.job = Job::Opening { handle, args },
                    Err(ActionError::Held | ActionError::Busy | ActionError::BudgetExhausted) => {
                        self.job = Job::ResumeOpening { kind, handle, args };
                        return Poll::Pending;
                    }
                    Err(error) => return Poll::Ready(Err(error)),
                },
                Job::ResumePrereqOpening { kind, mut action } => {
                    match action.resume_after_session(actions, kind, cx) {
                        Ok(()) => self.job = Job::PrereqOpening(action),
                        Err(
                            ActionError::Held | ActionError::Busy | ActionError::BudgetExhausted,
                        ) => {
                            self.job = Job::ResumePrereqOpening { kind, action };
                            return Poll::Pending;
                        }
                        Err(error) => return Poll::Ready(Err(error)),
                    }
                }
                Job::ResumeTransfer { kind, mut handle } => {
                    match actions.resume_kept(kind, &mut handle, cx) {
                        Ok(()) => self.job = Job::Transfer(handle),
                        Err(
                            ActionError::Held | ActionError::Busy | ActionError::BudgetExhausted,
                        ) => {
                            self.job = Job::ResumeTransfer { kind, handle };
                            return Poll::Pending;
                        }
                        Err(error) => return Poll::Ready(Err(error)),
                    }
                }
                Job::Opening { handle, args } => {
                    let allow = handle
                        .inspect_in(cx, |opening, cx| opening.prereq_allow(cx))
                        .flatten();
                    match actions.poll(&handle, cx) {
                        Poll::Pending => {
                            self.job = Job::Opening { handle, args };
                            return Poll::Pending;
                        }
                        Poll::Ready(Ok(OpeningValue::Bool(ready))) => {
                            if let Some(value) = self.after_open(ready) {
                                return Poll::Ready(Ok(value));
                            }
                        }
                        Poll::Ready(Ok(OpeningValue::Selected { bank, selected })) => {
                            self.selected_bank = Some(selected);
                            return Poll::Ready(Ok(bank
                                .as_ref()
                                .map(bank_value)
                                .unwrap_or(Value::Null)));
                        }
                        Poll::Ready(Err(ActionError::NeedsStep(gates))) => {
                            let Some(allow) = allow else {
                                return Poll::Ready(Err(ActionError::NeedsStep(gates)));
                            };
                            match PrereqOpening::resolve_after(
                                args,
                                ActionError::NeedsStep(gates),
                                allow,
                                cx,
                            ) {
                                Ok(action) => {
                                    self.job = Job::PrereqOpening(action);
                                    continue;
                                }
                                Err((
                                    args,
                                    ActionError::NeedsEvidence(_) | ActionError::Prerequisite(_),
                                )) => {
                                    self.job = Job::StartOpening(args.after_prerequisite_refusal());
                                    continue;
                                }
                                Err((_, error)) => return Poll::Ready(Err(error)),
                            }
                        }
                        Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                    }
                }
                Job::PrereqOpening(mut action) => match action.poll(actions, cx, slot_state) {
                    Poll::Pending => {
                        self.job = Job::PrereqOpening(action);
                        return Poll::Pending;
                    }
                    Poll::Ready(Ok(OpeningValue::Bool(ready))) => {
                        if let Some(value) = self.after_open(ready) {
                            return Poll::Ready(Ok(value));
                        }
                    }
                    Poll::Ready(Ok(OpeningValue::Selected { bank, selected })) => {
                        self.selected_bank = Some(selected);
                        return Poll::Ready(Ok(bank
                            .as_ref()
                            .map(bank_value)
                            .unwrap_or(Value::Null)));
                    }
                    Poll::Ready(Err(
                        ActionError::NeedsEvidence(_) | ActionError::Prerequisite(_),
                    )) => {
                        self.job =
                            Job::StartOpening(action.into_args().after_prerequisite_refusal());
                        continue;
                    }
                    Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                },
                Job::Transfer(handle) => match actions.poll(&handle, cx) {
                    Poll::Pending => {
                        self.job = Job::Transfer(handle);
                        return Poll::Pending;
                    }
                    Poll::Ready(result) => {
                        if self.task.is_some() {
                            match self.task_transfer_done(result, cx) {
                                Some(Ok(job)) => self.job = job,
                                Some(Err(error)) => return Poll::Ready(Err(error)),
                                None => return Poll::Ready(Err(ActionError::Stale)),
                            }
                        } else if let Some(result) = self.transfer_done(result, note) {
                            return Poll::Ready(result);
                        }
                    }
                },
                Job::Wait(mut wait) => match wait.poll(cx) {
                    Poll::Pending => {
                        self.job = Job::Wait(wait);
                        return Poll::Pending;
                    }
                    Poll::Ready(result) => {
                        if let Some(result) = self.transfer_done(result, note) {
                            return Poll::Ready(result);
                        }
                    }
                },
                Job::AfterDeposit { asked } => {
                    if self.trip.as_ref().is_some_and(|trip| {
                        trip.hooks_present & BANK_NEAREST_HOOK_AFTER_DEPOSIT != 0
                    }) {
                        let mut hooks =
                            self.hooks.lock().unwrap_or_else(|error| error.into_inner());
                        if !asked {
                            hooks.pending = Some(TransferCall {
                                hook: "afterDeposit",
                                args: Vec::new(),
                            });
                            self.job = Job::AfterDeposit { asked: true };
                            return Poll::Pending;
                        }
                        match hooks.answer.take() {
                            None => {
                                self.job = Job::AfterDeposit { asked: true };
                                return Poll::Pending;
                            }
                            Some(TransferAnswer::Failed) => {
                                return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                    "bank callback failed",
                                ))));
                            }
                            Some(TransferAnswer::Value(_)) => {}
                        }
                    }
                    self.job = Job::Settle {
                        due: cx.evidence().tick.saturating_add(1),
                    };
                }
                Job::Settle { due } => {
                    if cx.evidence().tick < due {
                        self.job = Job::Settle { due };
                        return Poll::Pending;
                    }
                    self.job = Job::Return { deadline: None };
                }
                Job::Return { deadline } => {
                    let Some(target) = self.trip.as_ref().and_then(|trip| trip.return_to) else {
                        return Poll::Ready(Ok(json!(true)));
                    };
                    if deadline.is_some_and(|deadline| cx.active_now() >= deadline) {
                        return Poll::Ready(Ok(json!(true)));
                    }
                    if cx
                        .snapshot()
                        .here()
                        .is_some_and(|here| cx.snapshot().walk_arrived(here.value, target, 6))
                    {
                        return Poll::Ready(Ok(json!(true)));
                    }
                    let request = crate::native::WalkRequest {
                        target,
                        candidate_goals: None,
                        loc_id: None,
                        radius: 6,
                        arrival: nav_planner::contract::ArrivalKind::Reach,
                        options: crate::native::WalkOptions {
                            allow_teleports: crate::native::WalkBit::Forbid,
                            allow_wilderness: crate::native::WalkBit::Allow,
                            ..Default::default()
                        },
                        required_after: cx.evidence(),
                        evidence: None,
                        cross: Box::new([]),
                        avoid: Box::new([]),
                        protect: false,
                        food_guard: false,
                        cure_guard: false,
                        allow: Default::default(),
                    };
                    match PrereqWalk::begin(request, actions, cx) {
                        Ok(walk) => {
                            self.job = Job::ReturnWalk {
                                walk,
                                deadline: deadline.unwrap_or_else(|| {
                                    cx.active_now().saturating_add(Duration::from_secs(120))
                                }),
                            };
                            return Poll::Pending;
                        }
                        Err(ActionError::BudgetExhausted | ActionError::Held) => {
                            self.job = Job::Return { deadline };
                            return Poll::Pending;
                        }
                        // Frozen BankNearest does not use the walk's result.
                        Err(_) => return Poll::Ready(Ok(json!(true))),
                    }
                }
                Job::ReturnWalk { mut walk, deadline } => {
                    if cx.active_now() >= deadline {
                        return Poll::Ready(Ok(json!(true)));
                    }
                    match walk.poll(actions, cx, slot_state) {
                        Poll::Ready(Ok(receipt))
                            if receipt.end == crate::native::WalkEnd::Cancelled =>
                        {
                            // Pause ended the host follow, not the bank trip.
                            // Revalidate its return target without extending
                            // the original active-time bound.
                            self.job = Job::Return {
                                deadline: Some(deadline),
                            };
                        }
                        Poll::Ready(_) => return Poll::Ready(Ok(json!(true))),
                        Poll::Pending => self.job = Job::ReturnWalk { walk, deadline },
                    }
                    return Poll::Pending;
                }
                Job::Pace { accepted } => {
                    let now = cx.evidence();
                    let Some(accepted) = accepted else {
                        self.job = Job::Pace {
                            accepted: Some(now),
                        };
                        return Poll::Pending;
                    };
                    if now.run.slot != accepted.run.slot || now.run.run != accepted.run.run {
                        // Another run never inherits this pause.
                        return Poll::Ready(Err(ActionError::Cancelled));
                    }
                    if now.run.session != accepted.run.session {
                        // A kept reconnect holds the request: the pause
                        // restarts on the new session's own observed ticks.
                        self.job = Job::Pace {
                            accepted: Some(now),
                        };
                        return Poll::Pending;
                    }
                    if now.tick > accepted.tick {
                        return Poll::Ready(Ok(Value::Null));
                    }
                    self.job = Job::Pace {
                        accepted: Some(accepted),
                    };
                    return Poll::Pending;
                }
                Job::Backpack(mut wait) => {
                    let snapshot = cx.snapshot();
                    let tick = cx.evidence().tick;
                    let open = snapshot
                        .bank_session()
                        .is_some_and(|session| session.value.open);
                    let started = *wait.started_tick.get_or_insert(tick);
                    let expired = tick.saturating_sub(started) >= TRANSFER_OBSERVED_TICK_LIMIT;
                    let side_ready = open && snapshot.bank_side_ready();
                    if !wait.matching {
                        if side_ready {
                            if wait.expected.is_none() {
                                return Poll::Ready(Ok(json!(true)));
                            }
                            // The expectation gets its own observed-tick budget.
                            wait.matching = true;
                            wait.started_tick = Some(tick);
                            self.job = Job::Backpack(wait);
                            continue;
                        }
                        if !open || expired {
                            self.job = log_then(
                                wait.log,
                                "bank: side backpack modal did not become ready".into(),
                                Job::Settled(json!(false)),
                            );
                            continue;
                        }
                        self.job = Job::Backpack(wait);
                        return Poll::Pending;
                    }
                    let matched = side_ready
                        && wait.expected.as_deref().is_some_and(|expected| {
                            snapshot
                                .bank_side()
                                .is_some_and(|side| same_backpack(side.value, expected))
                        });
                    if matched {
                        return Poll::Ready(Ok(json!(true)));
                    }
                    if !open || expired {
                        self.job = log_then(
                            wait.log,
                            "bank: side backpack did not match the pre-open backpack".into(),
                            Job::Settled(json!(false)),
                        );
                        continue;
                    }
                    self.job = Job::Backpack(wait);
                    return Poll::Pending;
                }
                Job::Sort(handle) => match actions.poll(&handle, cx) {
                    Poll::Pending => {
                        self.job = Job::Sort(handle);
                        return Poll::Pending;
                    }
                    Poll::Ready(Ok(value)) => return Poll::Ready(Ok(value)),
                    // A replaced session or a cancelled owner ends the sort
                    // as the upstream closed-bank round does, with the
                    // progress the machine retained.
                    Poll::Ready(Err(ActionError::Stale | ActionError::Cancelled)) => {
                        return Poll::Ready(Ok(handle
                            .inspect(Sort::closed_result)
                            .unwrap_or_else(Sort::closed_empty)));
                    }
                    Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                },
                Job::ResumeSort { kind, mut handle } => {
                    match actions.resume_kept(kind, &mut handle, cx) {
                        Ok(()) => self.job = Job::Sort(handle),
                        Err(
                            ActionError::Held | ActionError::Busy | ActionError::BudgetExhausted,
                        ) => {
                            self.job = Job::ResumeSort { kind, handle };
                            return Poll::Pending;
                        }
                        Err(_) => {
                            return Poll::Ready(Ok(handle
                                .inspect(Sort::closed_result)
                                .unwrap_or_else(Sort::closed_empty)));
                        }
                    }
                }
                Job::Log {
                    message,
                    asked,
                    then,
                } => {
                    let mut hooks = self.hooks.lock().unwrap_or_else(|error| error.into_inner());
                    if !asked {
                        if hooks.pending.is_some() || hooks.answer.is_some() {
                            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                "bank callback mailbox is occupied",
                            ))));
                        }
                        hooks.pending = Some(TransferCall {
                            hook: "log",
                            args: vec![json!(message)],
                        });
                        drop(hooks);
                        self.job = Job::Log {
                            message: String::new(),
                            asked: true,
                            then,
                        };
                        return Poll::Pending;
                    }
                    match hooks.answer.take() {
                        None => {
                            drop(hooks);
                            self.job = Job::Log {
                                message,
                                asked: true,
                                then,
                            };
                            return Poll::Pending;
                        }
                        Some(TransferAnswer::Failed) => {
                            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                                "bank callback failed",
                            ))));
                        }
                        Some(TransferAnswer::Value(_)) => {
                            drop(hooks);
                            self.job = *then;
                        }
                    }
                }
                Job::Settled(value) => return Poll::Ready(Ok(value)),
                Job::Finished => return Poll::Ready(Err(ActionError::Stale)),
            }
        }
    }

    fn transfer_done(
        &mut self,
        result: Result<TransferValue, ActionError>,
        note: &mut Option<(u64, NoteIntent)>,
    ) -> Option<Result<Value, ActionError>> {
        let value = match result {
            Err(error) => return Some(Err(error)),
            Ok(TransferValue::Bool(value)) => json!(value),
            Ok(TransferValue::Void) => Value::Null,
            Ok(TransferValue::NoteMode { generation, intent }) => {
                *note = Some((generation, intent));
                Value::Null
            }
        };
        if self.trip.is_some() {
            self.job = Job::AfterDeposit { asked: false };
            None
        } else {
            Some(Ok(value))
        }
    }
}

/// Post `message` to the task's `log` callback before `then`, when one was
/// supplied.
fn log_then(log: bool, message: String, then: Job) -> Job {
    if log {
        Job::Log {
            message,
            asked: false,
            then: Box::new(then),
        }
    } else {
        then
    }
}

/// Posted pack stacks `depositAllExcept(keep)` would bank: a named row whose
/// name is not a keep name, case-insensitively (`bankRules.ts:107-110`).
fn purge_junk(snapshot: SnapshotView<'_>, keep: &[String]) -> usize {
    snapshot.inventory().map_or(0, |pack| {
        pack.value
            .iter()
            .filter(|row| row.count > 0)
            .filter(|row| {
                row.def.name.as_deref().is_some_and(|name| {
                    !name.is_empty() && !keep.iter().any(|kept| ops::same_name(kept, name))
                })
            })
            .count()
    })
}

/// Held count of every row named `name`, case-insensitively.
fn count_named(rows: &[ItemView], name: &str) -> i32 {
    rows.iter()
        .filter(|row| {
            row.def
                .name
                .as_deref()
                .is_some_and(|actual| ops::same_name(actual, name))
        })
        .fold(0i32, |sum, row| sum.saturating_add(row.count))
}

/// Upstream `sameBackpack` (`Bank.ts:25-37`): equal length, and every posted
/// side row has an expected row with the same slot, id and count.
fn same_backpack(side: &[ItemView], expected: &[BankPackRow]) -> bool {
    side.len() == expected.len()
        && side.iter().all(|row| {
            expected.iter().any(|want| {
                want.slot == row.slot && want.id == row.def.id && want.count == row.count
            })
        })
}

impl Job {
    /// A resumable bank request across a kept reconnect (contract v5 R5-7,
    /// R5-8.5): request state is held, session-bound primitives are re-keyed
    /// or revalidated on the new session.
    fn held_across_kept_reconnect(self, kind: NativeKind) -> Job {
        match self {
            Job::Opening { handle, args } | Job::ResumeOpening { handle, args, .. } => {
                Job::ResumeOpening { kind, handle, args }
            }
            Job::PrereqOpening(action) | Job::ResumePrereqOpening { action, .. } => {
                Job::ResumePrereqOpening { kind, action }
            }
            Job::Transfer(handle) | Job::ResumeTransfer { handle, .. } => {
                Job::ResumeTransfer { kind, handle }
            }
            Job::ReturnWalk { walk, deadline } => {
                // The old logical walk belongs to the dropped session. Retain
                // the trip's return cursor and deadline, then revalidate a
                // fresh follow.
                drop(walk);
                Job::Return {
                    deadline: Some(deadline),
                }
            }
            // A sort keeps its progress: the resumed machine sees the dropped
            // bank session and settles as a closed-bank round with the moves
            // it already sent.
            Job::Sort(handle) | Job::ResumeSort { handle, .. } => Job::ResumeSort { kind, handle },
            job => job,
        }
    }
}

impl SlotScript {
    pub(super) fn bank_session_boundary(&mut self, reconnect: bool) {
        if let Some(bank) = &mut self.bank {
            if reconnect {
                bank.note = None;
                bank.action_owner = None;
                bank.selection.invalidate();
                for row in &mut bank.rows {
                    let kind = self.seat.as_ref().and_then(|seat| {
                        seat.live()
                            .iter()
                            .find(|entry| entry.id == row.id)
                            .map(|entry| entry.kind)
                    });
                    let Some(kind) = kind.filter(|kind| {
                        kind.policy().reconnect() == api::script_contract::ReconnectPolicy::Resume
                    }) else {
                        continue;
                    };
                    let job = std::mem::replace(&mut row.job, Job::Finished);
                    row.job = job.held_across_kept_reconnect(kind);
                }
            } else {
                bank.clear_work();
            }
        }
    }

    pub(super) fn bank_runtime_boundary(&mut self) {
        if let Some(bank) = &mut self.bank {
            bank.clear_work();
        }
    }

    pub fn bank_selection_input(&self) -> BankSelectionInput {
        self.bank
            .as_ref()
            .map_or_else(BankSelectionInput::default, |bank| {
                bank.selection.for_runtime(self.runtime_generation)
            })
    }

    pub(super) fn start_bank(&mut self, id: RequestId, request: BankSeatArgs) {
        let started = self.bank.get_or_insert_with(Default::default).start(
            id,
            request,
            self.runtime_generation,
        );
        if let Err(error) = started {
            self.pending_logs
                .push(format!("bank seat start refused: {error:?}"));
            if let Some(seat) = &mut self.seat {
                if let Err(error) = seat.complete(id, true) {
                    self.pending_logs
                        .push(format!("bank seat refusal terminal failed: {error:?}"));
                }
            }
        }
    }

    pub(super) fn stop_cancelled_bank_work(&mut self) {
        let (Some(bank), Some(seat)) = (self.bank.as_mut(), self.seat.as_ref()) else {
            return;
        };
        bank.rows.retain(|row| {
            let live = seat.lookup(row.id) == api::script_contract::AuthorityStatus::Live;
            if !live && bank.action_owner == Some(row.id) {
                bank.action_owner = None;
            }
            live
        });
        bank.values
            .retain(|(id, _)| seat.results().iter().any(|result| result.id == *id));
    }

    pub(super) fn tick_bank_scoped(&mut self, ctx: &mut ScriptCtx<'_>, scope: BankPoll) {
        self.stop_cancelled_bank_work();
        let Some(mut bank) = self.bank.take() else {
            return;
        };
        if bank.rows.is_empty() || !self.dispatch_open() || ctx.compiled.hold {
            self.bank = Some(bank);
            return;
        }
        let identity = self
            .surface_start
            .as_ref()
            .expect("bank start belongs to Load");
        let revision = identity
            .game_data
            .as_ref()
            .and_then(|data| super::compiled::selected_revision(data).ok());
        let Some(revision) = revision else {
            let mut rows = std::mem::take(&mut bank.rows);
            for row in rows.drain(..) {
                self.finish_bank(
                    row.id,
                    Err(ActionError::Unavailable(Arc::from(
                        "selected game data unavailable",
                    ))),
                    None,
                    &mut bank,
                );
            }
            bank.rows = rows;
            self.bank = Some(bank);
            return;
        };
        let facts = Arc::clone(&identity.named_banks);
        let bag = identity.settings_bag.as_deref();
        let preferences = BankPreferences {
            use_mage_bank: bag
                .and_then(|bag| bag.get("useMageBank"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            use_zanaris_bank: bag
                .and_then(|bag| bag.get("useZanarisBank"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
        };
        let casket = api::content::random_event_casket_id(identity.game_data.as_deref());
        let retained = Arc::clone(
            self.retained
                .get_or_insert_with(|| Arc::new(Mutex::new(RetainedMemory::default()))),
        );
        let run = self.seat.as_ref().expect("admitted bank seat").run();
        let mut actions = NativeActions { _private: () };
        let mut index = 0;
        while index < bank.rows.len() {
            let row = &mut bank.rows[index];
            let seat = self.seat.as_mut().expect("admitted bank seat");
            let answer = seat.take_answer(row.id);
            if scope == BankPoll::Answered && answer.is_none() {
                // A seat answer wake polls only the consumer of the answer.
                index += 1;
                continue;
            }
            if let Some(answer) = answer {
                let value = crate::channel::decode(&answer.answer);
                let mut hooks = row.hooks.lock().unwrap_or_else(|error| error.into_inner());
                hooks.pending = None;
                hooks.answer = Some(match value {
                    Ok(value) => TransferAnswer::Value(value),
                    Err(_) => TransferAnswer::Failed,
                });
                row.prompt_posted = false;
            }
            let result = {
                let mut memory = retained.lock().unwrap_or_else(|error| error.into_inner());
                let mut cx = compiled::frame_context(
                    ctx,
                    run,
                    revision,
                    &mut memory,
                    &mut self.native_runtime,
                );
                row.poll(
                    &mut actions,
                    &mut cx,
                    self.local_state.as_deref(),
                    &facts,
                    preferences,
                    &mut bank.note,
                    casket,
                )
            };
            if matches!(
                row.job,
                Job::Opening { .. }
                    | Job::PrereqOpening(_)
                    | Job::Transfer(_)
                    | Job::ReturnWalk { .. }
                    | Job::Sort(_)
            ) {
                bank.action_owner = Some(row.id);
            } else if bank.action_owner == Some(row.id) {
                bank.action_owner = None;
            }
            if let Poll::Ready(result) = result {
                let selection =
                    row.selected_bank.take().zip(row.selection_generation).map(
                        |(selected, generation)| (selected, generation, row.runtime_generation),
                    );
                let id = row.id;
                let family = row.family;
                bank.rows.remove(index);
                if bank.action_owner == Some(id) {
                    bank.action_owner = None;
                }
                // The typed result the seat publishes for this request: the
                // live cell's record of what a close or a deposit answered.
                api::host_log!(
                    api::hostlog::Category::Echo,
                    api::hostlog::Level::Debug,
                    "bank-terminal request={} family={family} tick={} result={result:?}",
                    id.get(),
                    ctx.tick,
                );
                self.finish_bank(id, result, selection, &mut bank);
                continue;
            }
            let prompt_failed = if row.prompt_posted {
                false
            } else {
                let hooks = row.hooks.lock().unwrap_or_else(|error| error.into_inner());
                if let Some(call) = &hooks.pending {
                    let point = crate::native::abi::HOOK_CHARACTERIZATIONS
                        .iter()
                        .find(|point| {
                            point.families().contains(&row.family) && point.member() == call.hook
                        })
                        .and_then(crate::native::PromptPoint::from_contract_row);
                    match (point, crate::channel::encode_arguments(&call.args)) {
                        (Some(point), Ok(args)) => {
                            let posted = self
                                .seat
                                .as_mut()
                                .expect("admitted bank seat")
                                .prompt_with_details(row.id, point, &args)
                                .is_ok();
                            row.prompt_posted = posted;
                            !posted
                        }
                        _ => true,
                    }
                } else {
                    false
                }
            };
            if prompt_failed {
                let id = row.id;
                bank.rows.remove(index);
                if bank.action_owner == Some(id) {
                    bank.action_owner = None;
                }
                self.finish_bank(
                    id,
                    Err(ActionError::Failed(Arc::from(
                        "bank callback prompt could not be encoded",
                    ))),
                    None,
                    &mut bank,
                );
                continue;
            }
            index += 1;
        }
        self.bank = Some(bank);
    }

    fn finish_bank(
        &mut self,
        id: RequestId,
        result: Result<Value, ActionError>,
        selection: Option<(SelectedBank, u64, u64)>,
        bank: &mut BankSeat,
    ) {
        let (failed, value) = match result {
            Ok(value) => (false, value),
            // Existing bank pending guards and lost sessions answer false.
            Err(
                ActionError::Busy
                | ActionError::Stale
                | ActionError::Cancelled
                | ActionError::UserInput,
            ) => (false, json!(false)),
            Err(ActionError::Failed(reason) | ActionError::Unavailable(reason)) => {
                (true, json!(reason.as_ref()))
            }
            Err(error) => (true, json!(format!("{error:?}"))),
        };
        let current_runtime_generation = self.runtime_generation;
        match crate::channel::encode(&value) {
            Ok(value) => {
                match self
                    .seat
                    .as_mut()
                    .expect("admitted bank seat")
                    .complete(id, failed)
                {
                    Ok(()) => {
                        bank.values.push((id, value));
                        if let Some((selected, generation, runtime_generation)) = selection {
                            bank.selection.publish(
                                id,
                                generation,
                                runtime_generation,
                                current_runtime_generation,
                                selected,
                            );
                        }
                    }
                    Err(error) => self
                        .pending_logs
                        .push(format!("bank terminal refused: {error:?}")),
                }
            }
            Err(error) => {
                self.pending_logs
                    .push(format!("bank terminal encoding failed: {error}"));
                let _ = self
                    .seat
                    .as_mut()
                    .expect("admitted bank seat")
                    .complete(id, true);
            }
        }
    }
}

#[cfg(test)]
impl SlotScript {
    /// The bank rows no poll has started yet (still `Job::Start`).
    pub(super) fn unstarted_bank_rows_for_test(&self) -> Vec<RequestId> {
        self.bank.as_ref().map_or_else(Vec::new, |bank| {
            bank.rows
                .iter()
                .filter(|row| matches!(row.job, Job::Start(_)))
                .map(|row| row.id)
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selected(kind: PickKind, bank_index: u16) -> SelectedBank {
        SelectedBank {
            bank_index,
            access_tile: WorldTile {
                x: i32::from(bank_index),
                z: 0,
                level: 0,
            },
            kind,
            access: None,
        }
    }

    #[test]
    fn selection_publication_rejects_superseded_and_invalidated_results() {
        let first = RequestId::isolate(1).unwrap();
        let second = RequestId::isolate(2).unwrap();
        let mut state = BankSelectionState::default();
        let first_generation = state.admit(first).unwrap();
        let first_selection = selected(PickKind::Reachable, 1);
        state.publish(first, first_generation, 7, 7, first_selection);
        let published = BankSelectionInput {
            request_id: first.get(),
            generation: first_generation,
            bank_index: 1,
            kind: PickKind::Reachable as u8,
        };
        assert_eq!(state.for_runtime(7), published);

        let second_generation = state.admit(second).unwrap();
        state.publish(
            first,
            first_generation,
            7,
            7,
            selected(PickKind::NearShortcut, 2),
        );
        assert_eq!(
            state.for_runtime(7),
            published,
            "an older request cannot overwrite the latest published result"
        );
        state.publish(
            second,
            second_generation,
            7,
            8,
            selected(PickKind::AirFallback, 2),
        );
        assert_eq!(
            state.for_runtime(7),
            published,
            "a previous runtime cannot publish into its replacement"
        );
        assert_eq!(
            state.for_runtime(8),
            BankSelectionInput::default(),
            "a prior runtime's publication is not visible in its replacement"
        );

        state.invalidate();
        state.publish(
            second,
            second_generation,
            7,
            7,
            selected(PickKind::AirFallback, 2),
        );
        assert_eq!(
            state.for_runtime(7),
            BankSelectionInput::default(),
            "reset, stop, or replacement invalidates in-flight selection results"
        );
    }
}

#[cfg(test)]
#[path = "bank_task_tests.rs"]
mod task_tests;
