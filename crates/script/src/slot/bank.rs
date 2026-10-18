//! Slot-owned bank operations and the BankNearest callback continuation.
//!
//! Only the running primitive owns a native action. The trip releases that
//! action before afterDeposit, so a callback may start its own bank operation.
use super::*;
use crate::bank::ops::{self, NoteIntent};
use crate::bank::request::{
    BankSeatArgs, BankSeatOperation, BANK_NEAREST_HOOK_AFTER_DEPOSIT, BANK_NEAREST_HOOK_DEPOSIT,
    BANK_NEAREST_HOOK_LOG, DEPOSIT_MATCH_HOOK_MATCH, HOOK_LOG,
};
use crate::bank::seat_open::{Opening, OpeningArgs, OpeningValue};
use crate::bank::seat_transfer::{
    Transfer, TransferAnswer, TransferArgs, TransferCall, TransferHooks, TransferValue,
};
use crate::isolate_fb::BankSelectionInput;
use crate::native::{ActionContext, ActionError, ActionHandle, NativeActions, NativeMachine};
use crate::native_bank::{PickKind, SelectedBank};
use api::named_banks::{BankPreferences, NamedBankFacts};
use api::snapshot::WorldTile;
use serde_json::{json, Value};
use std::sync::Mutex;
use std::task::Poll;
use std::time::Duration;

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
    hooks: Arc<Mutex<TransferHooks>>,
    prompt_posted: bool,
}

struct Trip {
    return_to: Option<WorldTile>,
    deposit_all: bool,
    common_junk: bool,
    hooks_present: u32,
}

enum Job {
    Start(BankSeatArgs),
    Opening(ActionHandle<Opening>),
    Transfer(ActionHandle<Transfer>),
    /// Observation-only waits do not contend with a running transfer.
    Wait(Transfer),
    AfterDeposit {
        asked: bool,
    },
    Settle {
        due: u64,
    },
    Return,
    ReturnWalk {
        handle: ActionHandle<crate::native::walk::Walk>,
        deadline: Duration,
    },
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
        _ => "bank_op",
    }
}

impl BankSeat {
    pub(super) fn result_values<'a>(
        &'a self,
        output: &mut [crate::isolate_fb::SeatResultValue<'a>],
    ) -> usize {
        for (out, (id, value)) in output.iter_mut().zip(&self.values) {
            *out = crate::isolate_fb::SeatResultValue { id: *id, value };
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
            Some(
                self.selection
                    .admit(id)
                    .ok_or(ActionError::Cancelled)?,
            )
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

    fn poll(
        &mut self,
        actions: &mut NativeActions,
        cx: &mut ActionContext<'_>,
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
                    let waiting = matches!(
                        request.operation,
                        BankSeatOperation::WaitReady | BankSeatOperation::WaitSnapshotAfter
                    );
                    if !waiting && !cx.may_begin_this_tick() {
                        self.job = Job::Start(request);
                        return Poll::Pending;
                    }
                    if is_opening(request.operation) {
                        match actions.begin::<Opening>(
                            OpeningArgs {
                                request,
                                facts: Arc::clone(facts),
                                preferences,
                                hooks: Arc::clone(&self.hooks),
                            },
                            cx,
                        ) {
                            Ok(handle) => self.job = Job::Opening(handle),
                            Err(ActionError::BudgetExhausted | ActionError::Held) => {
                                return Poll::Ready(Err(ActionError::BudgetExhausted));
                            }
                            Err(error) => return Poll::Ready(Err(error)),
                        }
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
                Job::Opening(handle) => match actions.poll(&handle, cx) {
                    Poll::Pending => {
                        self.job = Job::Opening(handle);
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
                    Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                },
                Job::Transfer(handle) => match actions.poll(&handle, cx) {
                    Poll::Pending => {
                        self.job = Job::Transfer(handle);
                        return Poll::Pending;
                    }
                    Poll::Ready(result) => match self.transfer_done(result, note) {
                        Some(result) => return Poll::Ready(result),
                        None => {}
                    },
                },
                Job::Wait(mut wait) => match wait.poll(cx) {
                    Poll::Pending => {
                        self.job = Job::Wait(wait);
                        return Poll::Pending;
                    }
                    Poll::Ready(result) => match self.transfer_done(result, note) {
                        Some(result) => return Poll::Ready(result),
                        None => {}
                    },
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
                    self.job = Job::Return;
                }
                Job::Return => {
                    let Some(target) = self.trip.as_ref().and_then(|trip| trip.return_to) else {
                        return Poll::Ready(Ok(json!(true)));
                    };
                    if cx
                        .snapshot()
                        .here()
                        .is_some_and(|here| cx.snapshot().walk_arrived(here.value, target, 6))
                    {
                        return Poll::Ready(Ok(json!(true)));
                    }
                    let request = crate::native::WalkRequest {
                        target,
                        loc_id: None,
                        radius: 6,
                        arrival: nav::contract::ArrivalKind::Reach,
                        options: crate::native::WalkOptions {
                            allow_teleports: crate::native::WalkBit::Forbid,
                            allow_wilderness: crate::native::WalkBit::Allow,
                            ..Default::default()
                        },
                        required_after: cx.evidence(),
                        evidence: None,
                        cross: Box::new([]),
                        protect: false,
                        food_guard: false,
                        allow: Default::default(),
                    };
                    match actions.begin::<crate::native::walk::Walk>(request, cx) {
                        Ok(handle) => {
                            self.job = Job::ReturnWalk {
                                handle,
                                deadline: cx.active_now().saturating_add(Duration::from_secs(120)),
                            };
                            return Poll::Pending;
                        }
                        Err(ActionError::BudgetExhausted | ActionError::Held) => {
                            self.job = Job::Return;
                            return Poll::Pending;
                        }
                        // Frozen BankNearest does not use the walk's result.
                        Err(_) => return Poll::Ready(Ok(json!(true))),
                    }
                }
                Job::ReturnWalk { handle, deadline } => {
                    if cx.active_now() >= deadline || actions.poll(&handle, cx).is_ready() {
                        return Poll::Ready(Ok(json!(true)));
                    }
                    self.job = Job::ReturnWalk { handle, deadline };
                    return Poll::Pending;
                }
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

// Materialization is shared with local readers and consumes the selected
// catalog entry, not a second bank catalog or a current snapshot page.
fn bank_value(bank: &api::named_banks::NamedBank) -> Value {
    crate::bank_select::bank_value(bank)
}

impl SlotScript {
    pub(super) fn bank_session_boundary(&mut self, reconnect: bool) {
        if let Some(bank) = &mut self.bank {
            if reconnect {
                bank.note = None;
                bank.action_owner = None;
                bank.selection.invalidate();
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
        self.bank.as_ref().map_or_else(BankSelectionInput::default, |bank| {
            bank.selection.for_runtime(self.runtime_generation)
        })
    }

    pub(super) fn start_bank(&mut self, id: RequestId, request: BankSeatArgs) {
        let started = self
            .bank
            .get_or_insert_with(Default::default)
            .start(id, request, self.runtime_generation);
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

    pub(super) fn tick_bank(&mut self, ctx: &mut ScriptCtx<'_>) {
        self.stop_cancelled_bank_work();
        let Some(mut bank) = self.bank.take() else {
            return;
        };
        if bank.rows.is_empty() || !self.dispatch_open() || ctx.compiled.hold {
            self.bank = Some(bank);
            return;
        }
        let identity = self
            .load_identity
            .as_ref()
            .expect("bank start belongs to Load");
        let pin = identity
            .game_data
            .as_ref()
            .and_then(|data| data.selected_pin().ok());
        let Some(pin) = pin else {
            for row in bank.rows.drain(..) {
                self.finish_bank(
                    row.id,
                    Err(ActionError::Unavailable(Arc::from(
                        "selected game data unavailable",
                    ))),
                    None,
                    &mut bank.values,
                    &mut bank.selection,
                );
            }
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
            if let Some(answer) = seat.take_answer(row.id) {
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
                let mut cx =
                    compiled::frame_context(ctx, run, &pin, &mut memory, &mut self.native_runtime);
                row.poll(
                    &mut actions,
                    &mut cx,
                    &facts,
                    preferences,
                    &mut bank.note,
                    casket,
                )
            };
            if matches!(
                row.job,
                Job::Opening(_) | Job::Transfer(_) | Job::ReturnWalk { .. }
            ) {
                bank.action_owner = Some(row.id);
            } else if bank.action_owner == Some(row.id) {
                bank.action_owner = None;
            }
            if let Poll::Ready(result) = result {
                let selection = row
                    .selected_bank
                    .take()
                    .zip(row.selection_generation)
                    .map(|(selected, generation)| {
                        (selected, generation, row.runtime_generation)
                    });
                let id = row.id;
                bank.rows.remove(index);
                if bank.action_owner == Some(id) {
                    bank.action_owner = None;
                }
                self.finish_bank(
                    id,
                    result,
                    selection,
                    &mut bank.values,
                    &mut bank.selection,
                );
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
                    &mut bank.values,
                    &mut bank.selection,
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
        values: &mut Vec<(RequestId, Vec<u8>)>,
        bank_selection: &mut BankSelectionState,
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
                        values.push((id, value));
                        if let Some((selected, generation, runtime_generation)) = selection {
                            bank_selection.publish(
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
