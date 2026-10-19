use crate::bank::ops::{
    self, CloseBaseline, CloseScan, DepositKind, DepositRequest, DepositScan, DepositSpec,
    LoadClick, NoteIntent, Progress, WithdrawGoal, DEPOSIT_VIEW_MS, MAX_DEPOSITS, SERVER_TICK_MS,
    TRANSFER_OBSERVED_TICK_LIMIT,
};
use crate::bank::request::{
    BankSeatArgs, BankSeatOperation, BANK_NEAREST_HOOK_DEPOSIT, BANK_NEAREST_HOOK_LOG,
    DEPOSIT_MATCH_HOOK_LOG, DEPOSIT_MATCH_HOOK_MATCH, HOOK_LOG,
};
use crate::native::{defer_budget, ActionContext, ActionError, NativeMachine};
use crate::shim::InteractReq;
use api::snapshot::{ItemView, SnapshotView};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::Poll;
use std::time::Duration;

const WAIT_DEFAULT_MS: f64 = 5_000.0;
const VIEW_NOT_READY: &str = "deposit view not ready — waiting for the side backpack";
const DEPOSIT_VIEW_ERROR: &str = "bank deposit view did not post";
const CALLBACK_ERROR: &str = "bank callback failed";

/// One synchronous callback call posted to the isolate mailbox.
#[derive(Clone, Debug)]
pub struct TransferCall {
    pub hook: &'static str,
    pub args: Vec<Value>,
}

/// The isolate's synchronous callback result. Promise mirrors arrive as an
/// opaque object value, exactly as they do for the compatibility adapter.
#[derive(Clone, Debug)]
pub enum TransferAnswer {
    Value(Value),
    Failed,
}

/// The parent consumes `pending` once, then clears it and writes `answer`.
#[derive(Default)]
pub struct TransferHooks {
    pub pending: Option<TransferCall>,
    pub answer: Option<TransferAnswer>,
}

/// Arguments and slot-local callback and note-intent state for one transfer.
pub struct TransferArgs {
    pub request: BankSeatArgs,
    pub hooks: Arc<Mutex<TransferHooks>>,
    pub note_intent: NoteIntent,
    /// `match` for Bank.depositAllMatching, or `deposit` for a bank trip.
    pub matcher_hook: &'static str,
    pub matcher_with_id: bool,
    pub common_junk: bool,
    pub common_casket_id: Option<i32>,
}

/// The JavaScript-visible result for a native bank transfer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferValue {
    Bool(bool),
    Void,
    NoteMode { generation: u64, intent: NoteIntent },
}

/// Native owner for the bank transfer operations that use shared bank kernels.
pub struct Transfer {
    args: TransferArgs,
    phase: Phase,
    callback: Option<CallbackContinuation>,
    login_session: u64,
}

#[derive(Clone, Copy)]
enum CallbackContinuation {
    WaitReadyLog,
    DepositLog,
    DepositMatch,
}

enum Phase {
    Start,
    Emit {
        request: InteractReq,
        after: SendAfter,
    },
    AwaitDepositLabel {
        request_id: u64,
    },
    AwaitWithdrawLabel {
        request_id: u64,
        generation: u64,
    },
    DeltaNext {
        goal: WithdrawGoal,
        sent_tick: u64,
    },
    DeltaAwait {
        request_id: u64,
        goal: WithdrawGoal,
        baseline: i32,
        sent_tick: u64,
        sent_at: Duration,
    },
    LoadAllAwait {
        request_id: u64,
        item_id: i32,
        used_before: i32,
        generation: u64,
        started_tick: u64,
        started_at: Duration,
    },
    LoadFillNext {
        goal: WithdrawGoal,
        sent_tick: u64,
    },
    LoadFillAwait {
        request_id: u64,
        goal: WithdrawGoal,
        sent_tick: u64,
    },
    Deposit(DepositLoop),
    CloseAwait {
        request_id: u64,
        baseline: CloseBaseline,
        started_tick: u64,
        started_at: Duration,
        timeout: Option<Duration>,
    },
    NoteModeAwait {
        request_id: u64,
        generation: u64,
        intent: NoteIntent,
        from_tick: u64,
    },
    WaitReady {
        deadline: Duration,
        log_present: bool,
        log_called: bool,
    },
    WaitSnapshotAfter {
        baseline: f64,
        deadline: Duration,
    },
}

enum SendAfter {
    DepositLabel,
    WithdrawLabel {
        generation: u64,
    },
    Delta {
        goal: WithdrawGoal,
        baseline: i32,
    },
    LoadAll {
        item_id: i32,
        used_before: i32,
        generation: u64,
    },
    LoadFill {
        goal: WithdrawGoal,
    },
    Deposit {
        loop_state: DepositLoop,
        id: i32,
        before: i32,
        generation: u64,
    },
    Close {
        baseline: CloseBaseline,
        timeout: Option<Duration>,
    },
    NoteMode {
        generation: u64,
        intent: NoteIntent,
    },
}

enum DepositMatcher {
    All,
    Keep { names: Vec<String>, ids: Vec<i32> },
    Callback { enabled: bool },
}

#[derive(Clone, Copy)]
enum DepositPhase {
    Scan,
    Match {
        index: usize,
        asked: Option<(i32, i32)>,
        predicate_hit: Option<bool>,
    },
    Await {
        request_id: u64,
        id: i32,
        before: i32,
        generation: u64,
        started_tick: u64,
        started_at: Duration,
    },
}

struct DepositLoop {
    matcher: DepositMatcher,
    hook_name: &'static str,
    with_id: bool,
    common_junk: bool,
    common_casket_id: Option<i32>,
    log_present: bool,
    log_called: bool,
    generation: Option<u64>,
    round: u8,
    view_started_at: Option<Duration>,
    phase: DepositPhase,
}

enum DepositStep {
    Continue,
    Wait,
    Done,
    Fail,
    Call {
        hook: &'static str,
        args: Vec<Value>,
        continuation: CallbackContinuation,
    },
    Send {
        request: InteractReq,
        id: i32,
        before: i32,
        generation: u64,
    },
}

impl DepositLoop {
    fn new(
        matcher: DepositMatcher,
        hook_name: &'static str,
        with_id: bool,
        common_junk: bool,
        common_casket_id: Option<i32>,
        log_present: bool,
    ) -> Self {
        Self {
            matcher,
            hook_name,
            with_id,
            common_junk,
            common_casket_id,
            log_present,
            log_called: false,
            generation: None,
            round: 0,
            view_started_at: None,
            phase: DepositPhase::Scan,
        }
    }

    fn same_session(&mut self, snapshot: SnapshotView<'_>, login_matches: bool) -> bool {
        let Some(session) = snapshot.bank_session() else {
            return false;
        };
        let generation = *self.generation.get_or_insert(session.value.generation);
        login_matches && session.value.open && session.value.generation == generation
    }

    fn view_expired(&self, now: Duration) -> bool {
        self.view_started_at.is_some_and(|started| {
            now.saturating_sub(started) >= Duration::from_millis(DEPOSIT_VIEW_MS)
        })
    }

    fn wait_view(&mut self, now: Duration) -> DepositStep {
        if self.view_started_at.is_none() {
            self.view_started_at = Some(now);
            if self.log_present && !self.log_called {
                self.log_called = true;
                return DepositStep::Call {
                    hook: "log",
                    args: vec![json!(VIEW_NOT_READY)],
                    continuation: CallbackContinuation::DepositLog,
                };
            }
        }
        DepositStep::Wait
    }

    fn step(&mut self, cx: &mut ActionContext<'_>, login_matches: bool) -> DepositStep {
        if self.round >= MAX_DEPOSITS {
            return DepositStep::Done;
        }
        let now = cx.active_now();
        let tick = cx.evidence().tick;
        let snapshot = cx.snapshot();
        if !self.same_session(snapshot, login_matches) {
            return DepositStep::Done;
        }
        match self.phase {
            DepositPhase::Scan => self.scan(snapshot, now),
            DepositPhase::Match {
                index,
                asked,
                predicate_hit,
            } => self.match_row(snapshot, index, asked, predicate_hit),
            DepositPhase::Await {
                request_id,
                id,
                before,
                generation,
                started_tick,
                started_at,
            } => {
                let Some(receipt) = cx.interaction_receipt(request_id) else {
                    return DepositStep::Wait;
                };
                if !receipt.accepted {
                    return DepositStep::Done;
                }
                let Some(session) = snapshot.bank_session() else {
                    return DepositStep::Done;
                };
                if !session.value.open || session.value.generation != generation {
                    return DepositStep::Done;
                }
                if snapshot
                    .inventory()
                    .is_some_and(|pack| ops::count_id(pack.value, id) < before)
                {
                    self.round = self.round.saturating_add(1);
                    self.phase = DepositPhase::Scan;
                    return DepositStep::Continue;
                }
                if transfer_expired(started_tick, tick, started_at, now) {
                    DepositStep::Done
                } else {
                    DepositStep::Wait
                }
            }
        }
    }

    fn scan(&mut self, snapshot: SnapshotView<'_>, now: Duration) -> DepositStep {
        let side = snapshot.bank_side().map(|observed| observed.value);
        let pack = snapshot.inventory().map(|observed| observed.value);
        let expired = self.view_expired(now);

        if let DepositMatcher::Callback { enabled } = &self.matcher {
            let Some(pack) = pack else {
                return if expired {
                    DepositStep::Fail
                } else {
                    self.wait_view(now)
                };
            };
            if !pack.iter().any(|row| row.count > 0) {
                return DepositStep::Done;
            }
            let usable = side.is_some_and(|side| {
                pack.iter()
                    .filter(|row| row.count > 0)
                    .all(|row| ops::pack_has_current_row(side, row.def.id, Some(row.slot)))
            });
            if !usable {
                return if expired {
                    DepositStep::Fail
                } else {
                    self.wait_view(now)
                };
            }
            self.phase = DepositPhase::Match {
                index: 0,
                asked: None,
                predicate_hit: None,
            };
            if !*enabled {
                // With no predicate, BankNearest can still apply its common-loot rule.
                return self.match_row(snapshot, 0, None, None);
            }
            return DepositStep::Continue;
        }

        let keep = match (&mut self.matcher, side) {
            (DepositMatcher::All, _) => &[][..],
            (DepositMatcher::Keep { names, ids }, Some(side)) => {
                ids.clear();
                for id in names.iter().flat_map(|name| ops::ids_named(side, name)) {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
                ids.as_slice()
            }
            (DepositMatcher::Keep { ids, .. }, None) => ids.as_slice(),
            (DepositMatcher::Callback { .. }, _) => unreachable!(),
        };
        let spec = DepositSpec {
            kind: DepositKind::UntilEmpty,
            keep,
            only: None,
        };
        match ops::deposit_next(&spec, side, pack, expired, true) {
            DepositScan::Click(click) => self.prepare_click(snapshot, click),
            DepositScan::WaitView => {
                if expired {
                    DepositStep::Fail
                } else {
                    self.wait_view(now)
                }
            }
            DepositScan::ViewExpired => DepositStep::Fail,
            DepositScan::Done | DepositScan::SessionGone => DepositStep::Done,
            DepositScan::MissingRequired => unreachable!("until-empty has no capacity operation"),
        }
    }

    fn match_row(
        &mut self,
        snapshot: SnapshotView<'_>,
        index: usize,
        asked: Option<(i32, i32)>,
        predicate_hit: Option<bool>,
    ) -> DepositStep {
        let (Some(side), Some(pack)) = (
            snapshot.bank_side().map(|observed| observed.value),
            snapshot.inventory().map(|observed| observed.value),
        ) else {
            self.phase = DepositPhase::Scan;
            return DepositStep::Continue;
        };
        let Some(row) = side.get(index) else {
            if asked.is_some() {
                self.phase = DepositPhase::Scan;
                return DepositStep::Continue;
            }
            return DepositStep::Done;
        };
        let id = row.def.id;
        let slot = row.slot;
        let current = row.count > 0 && ops::pack_has_current_row(pack, id, Some(slot));
        if asked.is_none() {
            if !current {
                self.phase = DepositPhase::Match {
                    index: index + 1,
                    asked: None,
                    predicate_hit: None,
                };
                return DepositStep::Continue;
            }
            let enabled = matches!(&self.matcher, DepositMatcher::Callback { enabled: true });
            if enabled {
                let name = row.def.name.as_deref().unwrap_or_default();
                let mut args = Vec::with_capacity(if self.with_id { 2 } else { 1 });
                args.push(json!(name));
                if self.with_id {
                    args.push(json!(id));
                }
                self.phase = DepositPhase::Match {
                    index,
                    asked: Some((id, slot)),
                    predicate_hit: None,
                };
                return DepositStep::Call {
                    hook: self.hook_name,
                    args,
                    continuation: CallbackContinuation::DepositMatch,
                };
            }
            let name = row.def.name.as_deref().unwrap_or_default();
            if !self.common_junk
                || !api::content::matches_common_bank_loot(self.common_casket_id, name, id)
            {
                self.phase = DepositPhase::Match {
                    index: index + 1,
                    asked: None,
                    predicate_hit: None,
                };
                return DepositStep::Continue;
            }
            return self.match_click(snapshot, row, id, slot);
        }

        let name = row.def.name.as_deref().unwrap_or_default();
        if !current {
            self.phase = DepositPhase::Scan;
            return DepositStep::Continue;
        }
        let Some(predicate_hit) = predicate_hit else {
            return DepositStep::Wait;
        };
        let hit = predicate_hit
            || (self.common_junk
                && api::content::matches_common_bank_loot(self.common_casket_id, name, id));
        if !hit {
            self.phase = DepositPhase::Match {
                index: index + 1,
                asked: None,
                predicate_hit: None,
            };
            return DepositStep::Continue;
        }
        if asked != Some((id, slot)) {
            self.round = self.round.saturating_add(1);
            self.phase = DepositPhase::Scan;
            return DepositStep::Continue;
        }
        self.match_click(snapshot, row, id, slot)
    }

    fn match_click(
        &mut self,
        snapshot: SnapshotView<'_>,
        row: &ItemView,
        id: i32,
        slot: i32,
    ) -> DepositStep {
        let Some(click) = ops::deposit_click(row, DepositRequest::Sweep) else {
            return DepositStep::Done;
        };
        if (click.id, click.slot) != (id, slot) {
            self.phase = DepositPhase::Scan;
            return DepositStep::Continue;
        }
        self.prepare_click(snapshot, click)
    }

    fn prepare_click(
        &mut self,
        snapshot: SnapshotView<'_>,
        click: ops::DepositClick,
    ) -> DepositStep {
        let Some(pack) = snapshot.inventory().map(|observed| observed.value) else {
            self.phase = DepositPhase::Scan;
            return DepositStep::Continue;
        };
        let before = ops::count_id(pack, click.id);
        if before <= 0 {
            self.phase = DepositPhase::Scan;
            return DepositStep::Continue;
        }
        let Some(generation) = self.generation else {
            self.phase = DepositPhase::Scan;
            return DepositStep::Continue;
        };
        self.view_started_at = None;
        DepositStep::Send {
            request: ops::deposit_req(click, generation),
            id: click.id,
            before,
            generation,
        }
    }
}

impl NativeMachine for Transfer {
    type Args = TransferArgs;
    type Output = TransferValue;

    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        Ok(Self {
            args,
            phase: Phase::Start,
            callback: None,
            login_session: cx.run().session,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        if cx.run().session != self.login_session && !matches!(&self.phase, Phase::Deposit(_)) {
            // A kept request resumes, but its old interface operation does not.
            // Deposit loops take their normal closed-bank exit below.
            return Poll::Ready(Ok(TransferValue::Bool(false)));
        }
        if let Some(continuation) = self.callback {
            let answer = lock_hooks(&self.args.hooks).answer.take();
            let Some(answer) = answer else {
                return Poll::Pending;
            };
            self.callback = None;
            match answer {
                TransferAnswer::Failed => {
                    return Poll::Ready(Err(unavailable(CALLBACK_ERROR)));
                }
                TransferAnswer::Value(value) => {
                    if let Err(error) = self.apply_callback(value, continuation) {
                        return Poll::Ready(Err(error));
                    }
                }
            }
        }

        loop {
            let phase = std::mem::replace(&mut self.phase, Phase::Start);
            match phase {
                Phase::Start => match self.prepare_start(cx) {
                    Ok(Some(value)) => return Poll::Ready(Ok(value)),
                    Ok(None) => continue,
                    Err(error) => return Poll::Ready(Err(error)),
                },
                Phase::Emit { request, after } => {
                    if cx.interaction_event_spent() {
                        self.phase = Phase::Emit { request, after };
                        return Poll::Pending;
                    }
                    let after = match after {
                        SendAfter::Delta { goal, baseline } => {
                            let snapshot = cx.snapshot();
                            let baseline = snapshot.inventory().map_or(baseline, |observed| {
                                ops::count_id(observed.value, goal.lands_as_id)
                            });
                            SendAfter::Delta { goal, baseline }
                        }
                        after => after,
                    };

                    let request_id = match defer_budget(cx.emit(request.clone())) {
                        Poll::Pending => {
                            self.phase = Phase::Emit { request, after };
                            return Poll::Pending;
                        }
                        Poll::Ready(Err(error)) => {
                            return Poll::Ready(Err(error));
                        }
                        Poll::Ready(Ok(request_id)) => request_id,
                    };
                    self.phase = after.into_phase(request_id, cx.evidence().tick, cx.active_now());
                    return Poll::Pending;
                }
                Phase::AwaitDepositLabel { request_id } => {
                    if cx.interaction_receipt(request_id).is_some() {
                        return Poll::Ready(Ok(TransferValue::Bool(true)));
                    }
                    self.phase = Phase::AwaitDepositLabel { request_id };
                    return Poll::Pending;
                }
                Phase::AwaitWithdrawLabel {
                    request_id,
                    generation,
                } => {
                    let Some(receipt) = cx.interaction_receipt(request_id) else {
                        self.phase = Phase::AwaitWithdrawLabel {
                            request_id,
                            generation,
                        };
                        return Poll::Pending;
                    };
                    let same_session = self.same_bank_session(cx, generation);
                    return Poll::Ready(Ok(TransferValue::Bool(receipt.accepted && same_session)));
                }
                Phase::DeltaNext { goal, sent_tick } => {
                    match self.current_withdraw_progress(cx, &goal) {
                        Progress::Complete | Progress::PackFull => {
                            return Poll::Ready(Ok(TransferValue::Bool(true)));
                        }
                        Progress::SessionGone | Progress::OverTarget => {
                            return Poll::Ready(Ok(TransferValue::Bool(false)));
                        }
                        Progress::Incomplete => {}
                    }
                    match self.prepare_delta_next(cx, goal.clone(), sent_tick) {
                        Poll::Pending => {
                            self.phase = Phase::DeltaNext { goal, sent_tick };
                            return Poll::Pending;
                        }
                        Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                        Poll::Ready(Ok(true)) => continue,
                        Poll::Ready(Ok(false)) => {
                            return Poll::Ready(Ok(TransferValue::Bool(false)));
                        }
                    }
                }

                Phase::DeltaAwait {
                    request_id,
                    goal,
                    baseline,
                    sent_tick,
                    sent_at,
                } => {
                    let Some(receipt) = cx.interaction_receipt(request_id) else {
                        self.phase = Phase::DeltaAwait {
                            request_id,
                            goal,
                            baseline,
                            sent_tick,
                            sent_at,
                        };
                        return Poll::Pending;
                    };
                    if !receipt.accepted {
                        return Poll::Ready(Ok(TransferValue::Bool(false)));
                    }
                    let snapshot = cx.snapshot();
                    let same_session = self.same_bank_session(cx, goal.generation);
                    let pack = snapshot
                        .inventory()
                        .map_or(&[][..], |observed| observed.value);
                    let held = ops::count_id(pack, goal.lands_as_id);
                    let full = ops::pack_full(
                        pack,
                        snapshot
                            .inventory_capacity()
                            .map_or(0, |capacity| i32::from(capacity.value)),
                    );
                    match ops::withdraw_progress(&goal, held, full, same_session) {
                        Progress::Complete | Progress::PackFull => {
                            return Poll::Ready(Ok(TransferValue::Bool(true)));
                        }
                        Progress::SessionGone | Progress::OverTarget => {
                            return Poll::Ready(Ok(TransferValue::Bool(false)));
                        }
                        Progress::Incomplete => {}
                    }
                    if held <= baseline {
                        if transfer_expired(sent_tick, cx.evidence().tick, sent_at, cx.active_now())
                        {
                            return Poll::Ready(Ok(TransferValue::Bool(false)));
                        }
                        self.phase = Phase::DeltaAwait {
                            request_id,
                            goal,
                            baseline,
                            sent_tick,
                            sent_at,
                        };
                        return Poll::Pending;
                    }
                    self.phase = Phase::DeltaNext { goal, sent_tick };
                    continue;
                }
                Phase::LoadAllAwait {
                    request_id,
                    item_id,
                    used_before,
                    generation,
                    started_tick,
                    started_at,
                } => {
                    let Some(receipt) = cx.interaction_receipt(request_id) else {
                        self.phase = Phase::LoadAllAwait {
                            request_id,
                            item_id,
                            used_before,
                            generation,
                            started_tick,
                            started_at,
                        };
                        return Poll::Pending;
                    };
                    if !receipt.accepted {
                        return Poll::Ready(Ok(TransferValue::Bool(false)));
                    }
                    let snapshot = cx.snapshot();
                    let same_session = self.same_bank_session(cx, generation);
                    let progress = match (snapshot.inventory(), snapshot.bank_rows()) {
                        (Some(inv), Some(bank)) if same_session => ops::load_all_progress(
                            used_before,
                            ops::occupied(inv.value),
                            ops::pack_full(
                                inv.value,
                                snapshot
                                    .inventory_capacity()
                                    .map_or(0, |capacity| i32::from(capacity.value)),
                            ),
                            ops::count_id(bank.value, item_id),
                            true,
                        ),
                        _ if !same_session => Progress::SessionGone,
                        _ => Progress::Incomplete,
                    };
                    match progress {
                        Progress::Complete => {
                            return Poll::Ready(Ok(TransferValue::Bool(true)));
                        }
                        Progress::SessionGone | Progress::PackFull | Progress::OverTarget => {
                            return Poll::Ready(Ok(TransferValue::Bool(false)));
                        }
                        Progress::Incomplete => {
                            if transfer_expired(
                                started_tick,
                                cx.evidence().tick,
                                started_at,
                                cx.active_now(),
                            ) {
                                return Poll::Ready(Ok(TransferValue::Bool(false)));
                            }
                            self.phase = Phase::LoadAllAwait {
                                request_id,
                                item_id,
                                used_before,
                                generation,
                                started_tick,
                                started_at,
                            };
                            return Poll::Pending;
                        }
                    }
                }
                Phase::LoadFillNext { goal, sent_tick } => {
                    match self.current_withdraw_progress(cx, &goal) {
                        Progress::Complete | Progress::PackFull => {
                            return Poll::Ready(Ok(TransferValue::Bool(true)));
                        }
                        Progress::SessionGone | Progress::OverTarget => {
                            return Poll::Ready(Ok(TransferValue::Bool(false)));
                        }
                        Progress::Incomplete => {}
                    }
                    match self.prepare_load_fill_next(cx, goal.clone(), sent_tick) {
                        Poll::Pending => {
                            self.phase = Phase::LoadFillNext { goal, sent_tick };
                            return Poll::Pending;
                        }
                        Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                        Poll::Ready(Ok(true)) => continue,
                        Poll::Ready(Ok(false)) => {
                            return Poll::Ready(Ok(TransferValue::Bool(false)));
                        }
                    }
                }
                Phase::LoadFillAwait {
                    request_id,
                    goal,
                    sent_tick,
                } => {
                    let Some(receipt) = cx.interaction_receipt(request_id) else {
                        self.phase = Phase::LoadFillAwait {
                            request_id,
                            goal,
                            sent_tick,
                        };
                        return Poll::Pending;
                    };
                    if !receipt.accepted {
                        return Poll::Ready(Ok(TransferValue::Bool(false)));
                    }
                    let snapshot = cx.snapshot();
                    let same_session = self.same_bank_session(cx, goal.generation);
                    let pack = snapshot
                        .inventory()
                        .map_or(&[][..], |observed| observed.value);
                    let full = ops::pack_full(
                        pack,
                        snapshot
                            .inventory_capacity()
                            .map_or(0, |capacity| i32::from(capacity.value)),
                    );
                    match ops::withdraw_progress(
                        &goal,
                        ops::count_id(pack, goal.lands_as_id),
                        full,
                        same_session,
                    ) {
                        Progress::Complete | Progress::PackFull => {
                            return Poll::Ready(Ok(TransferValue::Bool(true)));
                        }
                        Progress::SessionGone | Progress::OverTarget => {
                            return Poll::Ready(Ok(TransferValue::Bool(false)));
                        }
                        Progress::Incomplete => {}
                    }
                    if snapshot
                        .count_dialog_open()
                        .is_some_and(|dialog| dialog.value)
                    {
                        return Poll::Ready(Ok(TransferValue::Bool(false)));
                    }
                    self.phase = Phase::LoadFillNext { goal, sent_tick };
                    continue;
                }
                Phase::Deposit(mut deposit) => {
                    let login_matches = cx.run().session == self.login_session;
                    match deposit.step(cx, login_matches) {
                        DepositStep::Continue => {
                            self.phase = Phase::Deposit(deposit);
                        }
                        DepositStep::Wait => {
                            self.phase = Phase::Deposit(deposit);
                            return Poll::Pending;
                        }
                        DepositStep::Done => {
                            return Poll::Ready(Ok(TransferValue::Void));
                        }
                        DepositStep::Fail => {
                            return Poll::Ready(Err(unavailable(DEPOSIT_VIEW_ERROR)));
                        }
                        DepositStep::Call {
                            hook,
                            args,
                            continuation,
                        } => {
                            self.phase = Phase::Deposit(deposit);
                            if let Err(error) = self.post_callback(hook, args, continuation) {
                                return Poll::Ready(Err(error));
                            }
                            return Poll::Pending;
                        }
                        DepositStep::Send {
                            request,
                            id,
                            before,
                            generation,
                        } => {
                            self.phase = Phase::Emit {
                                request,
                                after: SendAfter::Deposit {
                                    loop_state: deposit,
                                    id,
                                    before,
                                    generation,
                                },
                            };
                        }
                    }
                }
                Phase::CloseAwait {
                    request_id,
                    baseline,
                    started_tick,
                    started_at,
                    timeout,
                } => {
                    let Some(receipt) = cx.interaction_receipt(request_id) else {
                        self.phase = Phase::CloseAwait {
                            request_id,
                            baseline,
                            started_tick,
                            started_at,
                            timeout,
                        };
                        return Poll::Pending;
                    };
                    if !receipt.accepted {
                        return Poll::Ready(Ok(TransferValue::Bool(false)));
                    }
                    let snapshot = cx.snapshot();
                    let session = snapshot.bank_session();
                    let open = session.is_some_and(|session| session.value.open);
                    let side = session.map_or(-1, |session| session.value.side);
                    let generation = session.map_or(0, |session| session.value.generation);
                    let timeout_reached = match timeout {
                        Some(timeout) => cx.active_now().saturating_sub(started_at) >= timeout,
                        None => {
                            open && cx.evidence().tick.saturating_sub(started_tick)
                                >= TRANSFER_OBSERVED_TICK_LIMIT
                        }
                    };
                    match ops::close_progress(
                        open,
                        side,
                        generation,
                        baseline,
                        cx.run().session == self.login_session,
                        timeout_reached,
                    ) {
                        CloseScan::Complete | CloseScan::AlreadyShut => {
                            return Poll::Ready(Ok(TransferValue::Bool(true)));
                        }
                        CloseScan::SessionReplaced | CloseScan::TimedOut => {
                            return Poll::Ready(Ok(TransferValue::Bool(false)));
                        }
                        CloseScan::SideHeld | CloseScan::Waiting => {
                            self.phase = Phase::CloseAwait {
                                request_id,
                                baseline,
                                started_tick,
                                started_at,
                                timeout,
                            };
                            return Poll::Pending;
                        }
                    }
                }
                Phase::NoteModeAwait {
                    request_id,
                    generation,
                    intent,
                    from_tick,
                } => {
                    let Some(receipt) = cx.interaction_receipt(request_id) else {
                        self.phase = Phase::NoteModeAwait {
                            request_id,
                            generation,
                            intent,
                            from_tick,
                        };
                        return Poll::Pending;
                    };
                    if !receipt.accepted {
                        return Poll::Ready(Ok(TransferValue::Void));
                    }
                    if cx.evidence().tick > from_tick {
                        return Poll::Ready(Ok(TransferValue::NoteMode { generation, intent }));
                    }
                    self.phase = Phase::NoteModeAwait {
                        request_id,
                        generation,
                        intent,
                        from_tick,
                    };
                    return Poll::Pending;
                }
                Phase::WaitReady {
                    deadline,
                    log_present,
                    log_called,
                } => {
                    let snapshot = cx.snapshot();
                    if bank_ready(snapshot) {
                        return Poll::Ready(Ok(TransferValue::Bool(true)));
                    }
                    if !bank_open(snapshot) {
                        return Poll::Ready(Ok(TransferValue::Bool(false)));
                    }
                    if cx.active_now() < deadline {
                        self.phase = Phase::WaitReady {
                            deadline,
                            log_present,
                            log_called,
                        };
                        return Poll::Pending;
                    }
                    if log_present && !log_called {
                        self.phase = Phase::WaitReady {
                            deadline,
                            log_present,
                            log_called: true,
                        };
                        if let Err(error) = self.post_callback(
                            "log",
                            vec![json!("bank: opened but the item list never arrived")],
                            CallbackContinuation::WaitReadyLog,
                        ) {
                            return Poll::Ready(Err(error));
                        }
                        return Poll::Pending;
                    }
                    return Poll::Ready(Ok(TransferValue::Bool(false)));
                }
                Phase::WaitSnapshotAfter { baseline, deadline } => {
                    let snapshot = cx.snapshot();
                    if !bank_open(snapshot) {
                        return Poll::Ready(Ok(TransferValue::Bool(false)));
                    }
                    if snapshot.bank().is_some()
                        && snapshot
                            .bank_snapshot_generation()
                            .is_some_and(|generation| (generation as f64) > baseline)
                    {
                        return Poll::Ready(Ok(TransferValue::Bool(true)));
                    }
                    if cx.active_now() >= deadline {
                        return Poll::Ready(Ok(TransferValue::Bool(false)));
                    }
                    self.phase = Phase::WaitSnapshotAfter { baseline, deadline };
                    return Poll::Pending;
                }
            }
        }
    }
}

impl Transfer {
    fn prepare_start(
        &mut self,
        cx: &mut ActionContext<'_>,
    ) -> Result<Option<TransferValue>, ActionError> {
        let snapshot = cx.snapshot();
        let generation = snapshot
            .bank_session()
            .map_or(0, |session| session.value.generation);
        let operation = self.args.request.operation;
        match operation {
            BankSeatOperation::DepositLabel => {
                if !bank_ready(snapshot) {
                    return Ok(Some(TransferValue::Bool(false)));
                }
                let request = &self.args.request;
                let (Some(name), Some(operation), Some(side)) = (
                    request.item_name.as_deref(),
                    request.menu_op.as_deref(),
                    snapshot.bank_side().map(|side| side.value),
                ) else {
                    return Ok(Some(TransferValue::Bool(false)));
                };
                let click = side
                    .iter()
                    .find(|row| {
                        row.def
                            .name
                            .as_deref()
                            .is_some_and(|got| ops::same_name(got, name))
                    })
                    .and_then(|row| ops::deposit_click(row, DepositRequest::Label(operation)));
                let Some(click) = click else {
                    return Ok(Some(TransferValue::Bool(false)));
                };
                self.phase = Phase::Emit {
                    request: ops::deposit_req(click, generation),
                    after: SendAfter::DepositLabel,
                };
                Ok(None)
            }
            BankSeatOperation::WithdrawLabel => {
                if !bank_ready(snapshot) {
                    return Ok(Some(TransferValue::Bool(false)));
                }
                let request = &self.args.request;
                let rows = snapshot
                    .bank_rows()
                    .map_or(&[][..], |observed| observed.value);
                let (name, action) = if let Some(id) = request.item_id {
                    let Some(row) = rows.iter().find(|row| row.def.id == id) else {
                        return Ok(Some(TransferValue::Bool(false)));
                    };
                    let Some(name) = row.def.name.as_deref().filter(|name| !name.is_empty()) else {
                        return Ok(Some(TransferValue::Bool(false)));
                    };
                    let Some(op) = request.menu_op.as_deref() else {
                        return Ok(Some(TransferValue::Bool(false)));
                    };
                    let action = if matches!(op.to_lowercase().as_str(), "withdraw-all" | "all") {
                        "Withdraw All".to_owned()
                    } else {
                        op.replace('-', " ")
                    };
                    (name.to_owned(), action)
                } else {
                    let Some(name) = request.item_name.as_deref() else {
                        return Ok(Some(TransferValue::Bool(false)));
                    };
                    let action = match request.menu_op.as_deref() {
                        Some(label) if label.to_lowercase() == "all" => "Withdraw All".to_owned(),
                        Some(label) => label.to_owned(),
                        None => withdraw_amount_action(rows, name, request.quantity),
                    };
                    (name.to_owned(), action)
                };
                self.phase = Phase::Emit {
                    request: InteractReq::Withdraw { name, action },
                    after: SendAfter::WithdrawLabel { generation },
                };
                Ok(None)
            }
            BankSeatOperation::WithdrawDelta => {
                let request = &self.args.request;
                if !bank_ready(snapshot)
                    || snapshot
                        .count_dialog_open()
                        .is_some_and(|dialog| dialog.value)
                {
                    return Ok(Some(TransferValue::Bool(false)));
                }
                let Some(quantity) = request
                    .quantity
                    .and_then(|quantity| i32::try_from(quantity).ok())
                else {
                    return Ok(Some(TransferValue::Bool(false)));
                };
                if quantity <= 0 {
                    return Ok(Some(TransferValue::Bool(true)));
                }
                let rows = snapshot
                    .bank_rows()
                    .map_or(&[][..], |observed| observed.value);
                let Some(row) = find_bank_row(rows, request.item_id, request.item_name.as_deref())
                else {
                    return Ok(Some(TransferValue::Bool(false)));
                };
                if request.item_id.is_some() && !row_name(row).is_some_and(|name| !name.is_empty())
                {
                    return Ok(Some(TransferValue::Bool(false)));
                }
                let available = row.count.max(0);
                if available == 0 {
                    return Ok(Some(TransferValue::Bool(false)));
                }
                let lands_as_id = request
                    .lands_as_id
                    .unwrap_or_else(|| landing_id(row, self.args.note_intent));
                let pack = snapshot
                    .inventory()
                    .map_or(&[][..], |observed| observed.value);
                let goal = WithdrawGoal::available_limited(
                    row.def.id,
                    Arc::from(row_name(row).unwrap_or_default()),
                    lands_as_id,
                    quantity,
                    available,
                    ops::count_id(pack, lands_as_id),
                    generation,
                );
                let Some(click) =
                    ops::withdraw_click(row, ops::withdraw_remaining(&goal, goal.baseline), &goal)
                else {
                    return Ok(Some(TransferValue::Bool(false)));
                };
                self.phase = Phase::Emit {
                    request: click,
                    after: SendAfter::Delta {
                        baseline: goal.baseline,
                        goal,
                    },
                };
                Ok(None)
            }
            BankSeatOperation::WithdrawLoad => {
                if !bank_ready(snapshot) {
                    return Ok(Some(TransferValue::Bool(false)));
                }
                let request = &self.args.request;
                let inv = snapshot
                    .inventory()
                    .map_or(&[][..], |observed| observed.value);
                let size = snapshot
                    .inventory_capacity()
                    .map_or(0, |capacity| i32::from(capacity.value));
                if size <= 0 {
                    return Ok(Some(TransferValue::Bool(false)));
                }
                let used = ops::occupied(inv);
                if used >= size {
                    return Ok(Some(TransferValue::Bool(true)));
                }
                let Some(name) = request.item_name.as_deref() else {
                    return Ok(Some(TransferValue::Bool(false)));
                };
                let rows = snapshot
                    .bank_rows()
                    .map_or(&[][..], |observed| observed.value);
                let Some(row) = rows.iter().find(|row| {
                    row.count > 0
                        && row
                            .def
                            .name
                            .as_deref()
                            .is_some_and(|got| ops::same_name(got, name))
                }) else {
                    return Ok(Some(TransferValue::Bool(false)));
                };
                let free = size - used;
                let lands_as_id = landing_id(row, self.args.note_intent);
                let goal = WithdrawGoal::available_limited(
                    row.def.id,
                    Arc::from(row.def.name.as_deref().unwrap_or_default()),
                    lands_as_id,
                    free,
                    row.count,
                    ops::count_id(inv, lands_as_id),
                    generation,
                );
                match ops::load_click(row, free, &goal) {
                    Some(LoadClick::All(click)) => {
                        self.phase = Phase::Emit {
                            request: click,
                            after: SendAfter::LoadAll {
                                item_id: row.def.id,
                                used_before: used,
                                generation,
                            },
                        };
                    }
                    Some(LoadClick::Fill(click))
                        if !snapshot
                            .count_dialog_open()
                            .is_some_and(|dialog| dialog.value) =>
                    {
                        self.phase = Phase::Emit {
                            request: click,
                            after: SendAfter::LoadFill { goal },
                        };
                    }
                    _ => return Ok(Some(TransferValue::Bool(false))),
                }
                Ok(None)
            }
            BankSeatOperation::DepositAll
            | BankSeatOperation::DepositKeep
            | BankSeatOperation::DepositMatch
            | BankSeatOperation::BankNearest => {
                let (matcher, hook_name, with_id, common_junk, log_present) = match operation {
                    BankSeatOperation::DepositAll => (
                        DepositMatcher::All,
                        self.args.matcher_hook,
                        false,
                        false,
                        false,
                    ),
                    BankSeatOperation::DepositKeep => (
                        DepositMatcher::Keep {
                            names: std::mem::take(&mut self.args.request.keep_names),
                            ids: Vec::new(),
                        },
                        self.args.matcher_hook,
                        false,
                        false,
                        false,
                    ),
                    BankSeatOperation::DepositMatch => {
                        if self.args.request.hooks_present & DEPOSIT_MATCH_HOOK_MATCH == 0 {
                            return Err(unavailable("requires a function"));
                        }
                        (
                            DepositMatcher::Callback { enabled: true },
                            self.args.matcher_hook,
                            self.args.matcher_with_id,
                            self.args.common_junk,
                            self.args.request.hooks_present & DEPOSIT_MATCH_HOOK_LOG != 0,
                        )
                    }
                    BankSeatOperation::BankNearest => (
                        DepositMatcher::Callback {
                            enabled: self.args.request.hooks_present & BANK_NEAREST_HOOK_DEPOSIT
                                != 0,
                        },
                        self.args.matcher_hook,
                        self.args.matcher_with_id,
                        self.args.common_junk,
                        self.args.request.hooks_present & BANK_NEAREST_HOOK_LOG != 0,
                    ),
                    _ => unreachable!(),
                };
                self.phase = Phase::Deposit(DepositLoop::new(
                    matcher,
                    hook_name,
                    with_id,
                    common_junk,
                    self.args.common_casket_id,
                    log_present,
                ));
                Ok(None)
            }
            BankSeatOperation::SetNoteMode => {
                let Some(session) = snapshot.bank_session().filter(|session| session.value.open)
                else {
                    return Ok(Some(TransferValue::Void));
                };
                let generation = session.value.generation;
                let intent = if self.args.request.note_mode {
                    NoteIntent::Noted
                } else {
                    NoteIntent::Item
                };
                self.phase = Phase::Emit {
                    request: ops::note_req(intent),
                    after: SendAfter::NoteMode { generation, intent },
                };
                Ok(None)
            }
            BankSeatOperation::Close => {
                let Some(session) = snapshot.bank_session().filter(|session| session.value.open)
                else {
                    return Ok(Some(TransferValue::Bool(true)));
                };
                self.phase = Phase::Emit {
                    request: InteractReq::Close,
                    after: SendAfter::Close {
                        baseline: CloseBaseline {
                            generation: session.value.generation,
                            side: session.value.side,
                        },
                        timeout: self.args.request.timeout_ms.map(close_timeout),
                    },
                };
                Ok(None)
            }
            BankSeatOperation::WaitReady => {
                let snapshot = cx.snapshot();
                if !bank_open(snapshot) {
                    return Ok(Some(TransferValue::Bool(false)));
                }
                if bank_ready(snapshot) {
                    return Ok(Some(TransferValue::Bool(true)));
                }
                self.phase = Phase::WaitReady {
                    deadline: cx
                        .active_now()
                        .saturating_add(wait_timeout(self.args.request.timeout_ms)),
                    log_present: self.args.request.hooks_present & HOOK_LOG != 0,
                    log_called: false,
                };
                Ok(None)
            }
            BankSeatOperation::WaitSnapshotAfter => {
                let Some(baseline) = self
                    .args
                    .request
                    .baseline_generation
                    .filter(|baseline| baseline.is_finite() && *baseline >= 0.0)
                else {
                    return Ok(Some(TransferValue::Bool(false)));
                };
                let snapshot = cx.snapshot();
                if !bank_open(snapshot) {
                    return Ok(Some(TransferValue::Bool(false)));
                }
                if snapshot.bank().is_some()
                    && snapshot
                        .bank_snapshot_generation()
                        .is_some_and(|generation| (generation as f64) > baseline)
                {
                    return Ok(Some(TransferValue::Bool(true)));
                }
                self.phase = Phase::WaitSnapshotAfter {
                    baseline,
                    deadline: cx
                        .active_now()
                        .saturating_add(wait_timeout(self.args.request.timeout_ms)),
                };
                Ok(None)
            }
            _ => Err(unavailable("unsupported bank transfer operation")),
        }
    }

    fn prepare_delta_next(
        &mut self,
        cx: &mut ActionContext<'_>,
        goal: WithdrawGoal,
        sent_tick: u64,
    ) -> Poll<Result<bool, ActionError>> {
        let snapshot = cx.snapshot();
        if !self.same_bank_session(cx, goal.generation) {
            return Poll::Ready(Ok(false));
        }
        if cx.evidence().tick == sent_tick {
            return Poll::Pending;
        }
        if snapshot
            .count_dialog_open()
            .is_some_and(|dialog| dialog.value)
        {
            return Poll::Ready(Ok(false));
        }
        let rows = snapshot
            .bank_rows()
            .map_or(&[][..], |observed| observed.value);
        let held = snapshot
            .inventory()
            .map_or(&[][..], |observed| observed.value);
        let Some(row) = rows
            .iter()
            .find(|row| row.def.id == goal.bank_item_id && row.count > 0)
        else {
            return Poll::Ready(Ok(false));
        };
        let held_count = ops::count_id(held, goal.lands_as_id);
        let Some(request) =
            ops::withdraw_click(row, ops::withdraw_remaining(&goal, held_count), &goal)
        else {
            return Poll::Ready(Ok(false));
        };
        self.phase = Phase::Emit {
            request,
            after: SendAfter::Delta {
                goal,
                baseline: held_count,
            },
        };
        Poll::Ready(Ok(true))
    }

    fn prepare_load_fill_next(
        &mut self,
        cx: &mut ActionContext<'_>,
        goal: WithdrawGoal,
        sent_tick: u64,
    ) -> Poll<Result<bool, ActionError>> {
        let snapshot = cx.snapshot();
        if !self.same_bank_session(cx, goal.generation) {
            return Poll::Ready(Ok(false));
        }
        if cx.evidence().tick == sent_tick {
            return Poll::Pending;
        }
        if snapshot
            .count_dialog_open()
            .is_some_and(|dialog| dialog.value)
        {
            return Poll::Ready(Ok(false));
        }
        let rows = snapshot
            .bank_rows()
            .map_or(&[][..], |observed| observed.value);
        let pack = snapshot
            .inventory()
            .map_or(&[][..], |observed| observed.value);
        let Some(row) = rows
            .iter()
            .find(|row| row.def.id == goal.bank_item_id && row.count > 0)
        else {
            return Poll::Ready(Ok(false));
        };
        let free = snapshot
            .inventory_capacity()
            .map_or(0, |capacity| i32::from(capacity.value))
            .saturating_sub(ops::occupied(pack));
        let Some(request) = ops::withdraw_click(
            row,
            ops::withdraw_remaining(&goal, ops::count_id(pack, goal.lands_as_id)),
            &goal,
        ) else {
            return Poll::Ready(Ok(false));
        };
        if free <= 0 {
            return Poll::Ready(Ok(false));
        }
        self.phase = Phase::Emit {
            request,
            after: SendAfter::LoadFill { goal },
        };
        Poll::Ready(Ok(true))
    }

    fn same_bank_session(&self, cx: &ActionContext<'_>, generation: u64) -> bool {
        cx.run().session == self.login_session
            && cx
                .snapshot()
                .bank_session()
                .is_some_and(|session| session.value.open && session.value.generation == generation)
    }
    fn current_withdraw_progress(&self, cx: &ActionContext<'_>, goal: &WithdrawGoal) -> Progress {
        let snapshot = cx.snapshot();
        let pack = snapshot
            .inventory()
            .map_or(&[][..], |observed| observed.value);
        ops::withdraw_progress(
            goal,
            ops::count_id(pack, goal.lands_as_id),
            ops::pack_full(
                pack,
                snapshot
                    .inventory_capacity()
                    .map_or(0, |capacity| i32::from(capacity.value)),
            ),
            self.same_bank_session(cx, goal.generation),
        )
    }

    fn post_callback(
        &mut self,
        hook: &'static str,
        args: Vec<Value>,
        continuation: CallbackContinuation,
    ) -> Result<(), ActionError> {
        let mut hooks = lock_hooks(&self.args.hooks);
        if hooks.pending.is_some() || hooks.answer.is_some() {
            return Err(unavailable("bank callback mailbox is occupied"));
        }
        hooks.pending = Some(TransferCall { hook, args });
        self.callback = Some(continuation);
        Ok(())
    }

    fn apply_callback(
        &mut self,
        value: Value,
        continuation: CallbackContinuation,
    ) -> Result<(), ActionError> {
        match continuation {
            CallbackContinuation::WaitReadyLog | CallbackContinuation::DepositLog => Ok(()),
            CallbackContinuation::DepositMatch => {
                let Phase::Deposit(deposit) = &mut self.phase else {
                    return Err(unavailable(CALLBACK_ERROR));
                };
                let DepositPhase::Match {
                    index,
                    asked: Some(asked),
                    predicate_hit: None,
                } = deposit.phase
                else {
                    return Err(unavailable(CALLBACK_ERROR));
                };
                deposit.phase = DepositPhase::Match {
                    index,
                    asked: Some(asked),
                    predicate_hit: Some(js_truthy(&value)),
                };
                Ok(())
            }
        }
    }
}

impl SendAfter {
    fn into_phase(self, request_id: u64, tick: u64, now: Duration) -> Phase {
        match self {
            Self::DepositLabel => Phase::AwaitDepositLabel { request_id },
            Self::WithdrawLabel { generation } => Phase::AwaitWithdrawLabel {
                request_id,
                generation,
            },
            Self::Delta { goal, baseline } => Phase::DeltaAwait {
                request_id,
                goal,
                baseline,
                sent_tick: tick,
                sent_at: now,
            },
            Self::LoadAll {
                item_id,
                used_before,
                generation,
            } => Phase::LoadAllAwait {
                request_id,
                item_id,
                used_before,
                generation,
                started_tick: tick,
                started_at: now,
            },
            Self::LoadFill { goal } => Phase::LoadFillAwait {
                request_id,
                goal,
                sent_tick: tick,
            },
            Self::Deposit {
                mut loop_state,
                id,
                before,
                generation,
            } => {
                loop_state.phase = DepositPhase::Await {
                    request_id,
                    id,
                    before,
                    generation,
                    started_tick: tick,
                    started_at: now,
                };
                Phase::Deposit(loop_state)
            }
            Self::Close { baseline, timeout } => Phase::CloseAwait {
                request_id,
                baseline,
                started_tick: tick,
                started_at: now,
                timeout,
            },
            Self::NoteMode { generation, intent } => Phase::NoteModeAwait {
                request_id,
                generation,
                intent,
                from_tick: tick,
            },
        }
    }
}

fn lock_hooks(hooks: &Mutex<TransferHooks>) -> MutexGuard<'_, TransferHooks> {
    hooks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn unavailable(message: &'static str) -> ActionError {
    ActionError::Unavailable(Arc::from(message))
}

fn bank_open(snapshot: SnapshotView<'_>) -> bool {
    snapshot
        .bank_session()
        .is_some_and(|session| session.value.open)
}

fn bank_ready(snapshot: SnapshotView<'_>) -> bool {
    let Some(session) = snapshot.bank_session() else {
        return false;
    };
    session.value.open
        && (snapshot.bank().is_some()
            || snapshot
                .bank_rows()
                .is_some_and(|rows| !rows.value.is_empty()))
}

fn row_name(row: &ItemView) -> Option<&str> {
    row.def.name.as_deref()
}

fn find_bank_row<'a>(
    rows: &'a [ItemView],
    id: Option<i32>,
    name: Option<&str>,
) -> Option<&'a ItemView> {
    match (id, name) {
        (Some(id), None) => rows.iter().find(|row| row.def.id == id),
        (None, Some(name)) => rows.iter().find(|row| {
            row.def
                .name
                .as_deref()
                .is_some_and(|actual| ops::same_name(actual, name))
        }),
        _ => None,
    }
}

fn landing_id(row: &ItemView, intent: NoteIntent) -> i32 {
    match intent {
        NoteIntent::Noted if !row.def.noted && row.def.certificate_link >= 0 => {
            row.def.certificate_link
        }
        _ => row.def.id,
    }
}

fn withdraw_amount_action(rows: &[ItemView], name: &str, quantity: Option<i64>) -> String {
    let amount = quantity.map_or(f64::NAN, |quantity| quantity as f64);
    let count = rows
        .iter()
        .find(|row| {
            row.def
                .name
                .as_deref()
                .is_some_and(|actual| ops::same_name(actual, name))
        })
        .map_or(0.0, |row| f64::from(row.count));
    if amount >= 10.0 && amount >= count {
        "Withdraw All".to_owned()
    } else if amount >= 10.0 {
        "Withdraw 10".to_owned()
    } else {
        "Withdraw 1".to_owned()
    }
}

fn wait_timeout(timeout_ms: Option<f64>) -> Duration {
    let millis = timeout_ms
        .filter(|millis| *millis > 0.0)
        .unwrap_or(WAIT_DEFAULT_MS);
    if !millis.is_finite() || millis >= u64::MAX as f64 {
        Duration::MAX
    } else {
        Duration::from_secs_f64(millis / 1_000.0)
    }
}

fn close_timeout(timeout_ms: f64) -> Duration {
    Duration::from_millis(if timeout_ms > 0.0 {
        timeout_ms as u64
    } else {
        0
    })
}

fn transfer_expired(
    started_tick: u64,
    current_tick: u64,
    started_at: Duration,
    active_now: Duration,
) -> bool {
    ops::observed_wait_expired_with_time_bound(
        started_tick,
        current_tick,
        active_now.saturating_sub(started_at)
            >= Duration::from_millis(TRANSFER_OBSERVED_TICK_LIMIT.saturating_mul(SERVER_TICK_MS)),
        TRANSFER_OBSERVED_TICK_LIMIT,
    )
}

fn js_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value
            .as_f64()
            .is_some_and(|value| value != 0.0 && !value.is_nan()),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_withdraw_uses_the_frozen_action_ladder() {
        let rows = [ItemView {
            def: api::obj_names::ItemDefView {
                id: 1,
                name: Some("Lobster".to_owned()),
                stackable: false,
                members: false,
                base_value: 0,
                noted: false,
                certificate_link: -1,
                certificate_template: -1,
            },
            container: api::snapshot::ItemContainer::Bank,
            action_family: api::snapshot::ItemActionFamily::Component,
            slot: 0,
            count: 12,
            actions: Vec::new(),
            component_id: 1,
        }];
        assert_eq!(
            withdraw_amount_action(&rows, "lobster", Some(12)),
            "Withdraw All"
        );
        assert_eq!(
            withdraw_amount_action(&rows, "Lobster", Some(11)),
            "Withdraw 10"
        );
        assert_eq!(
            withdraw_amount_action(&rows, "Lobster", Some(4)),
            "Withdraw 1"
        );
        assert_eq!(withdraw_amount_action(&rows, "Lobster", None), "Withdraw 1");
    }

    #[test]
    fn wait_and_close_timeouts_keep_their_distinct_defaults() {
        assert_eq!(wait_timeout(None), Duration::from_secs(5));
        assert_eq!(wait_timeout(Some(0.0)), Duration::from_secs(5));
        assert_eq!(wait_timeout(Some(-2.0)), Duration::from_secs(5));
        assert_eq!(
            wait_timeout(Some(1_250.5)),
            Duration::from_micros(1_250_500)
        );
        assert_eq!(close_timeout(0.0), Duration::ZERO);
        assert_eq!(close_timeout(1_250.9), Duration::from_millis(1_250));
    }

    #[test]
    fn callback_results_use_javascript_truthiness() {
        assert!(!js_truthy(&Value::Null));
        assert!(!js_truthy(&Value::Bool(false)));
        assert!(!js_truthy(&json!(0)));
        assert!(!js_truthy(&json!("")));
        assert!(js_truthy(&json!({})));
        assert!(js_truthy(&json!([])));
    }

    #[test]
    fn observed_transfer_expiry_needs_ticks_and_active_time() {
        assert!(!transfer_expired(
            10,
            18,
            Duration::ZERO,
            Duration::from_secs(4)
        ));
        assert!(!transfer_expired(
            10,
            17,
            Duration::ZERO,
            Duration::from_secs(20)
        ));
        assert!(transfer_expired(
            10,
            18,
            Duration::ZERO,
            Duration::from_millis(TRANSFER_OBSERVED_TICK_LIMIT * SERVER_TICK_MS),
        ));
    }
    use crate::native::{HostEffect, InteractionReceipt};
    use crate::quester::families::tests::with_tick;
    use api::quest_progress::EvidenceStamp;
    use api::snapshot::{GameSnapshot, ItemActionFamily, ItemContainer};

    fn item(
        id: i32,
        name: &str,
        count: i32,
        slot: i32,
        container: ItemContainer,
        actions: &[&str],
    ) -> ItemView {
        ItemView {
            def: api::obj_names::ItemDefView {
                id,
                name: Some(name.to_owned()),
                stackable: false,
                members: false,
                base_value: 0,
                noted: false,
                certificate_link: -1,
                certificate_template: -1,
            },
            container,
            action_family: if container == ItemContainer::Inventory {
                ItemActionFamily::Held
            } else {
                ItemActionFamily::Component
            },
            slot,
            count,
            actions: actions
                .iter()
                .map(|action| Some((*action).to_owned()))
                .collect(),
            component_id: 7,
        }
    }

    fn pack_item(id: i32, name: &str, slot: i32) -> ItemView {
        item(id, name, 1, slot, ItemContainer::Inventory, &[])
    }

    fn bank_item(id: i32, name: &str, count: i32, actions: &[&str]) -> ItemView {
        item(id, name, count, 0, ItemContainer::Bank, actions)
    }

    fn side_item(id: i32, name: &str, slot: i32) -> ItemView {
        item(id, name, 1, slot, ItemContainer::BankSide, &["Deposit-All"])
    }

    fn bank_snapshot(
        pack: Vec<ItemView>,
        bank: Vec<ItemView>,
        side: Vec<ItemView>,
    ) -> GameSnapshot {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(1);
        snapshot.seed_inventory(pack, 28);
        snapshot.seed_bank_observation(10, 41, Some(bank), side);
        snapshot
    }

    fn transfer_args(operation: BankSeatOperation) -> TransferArgs {
        TransferArgs {
            request: BankSeatArgs::new(operation),
            hooks: Arc::new(Mutex::new(TransferHooks::default())),
            note_intent: NoteIntent::Item,
            matcher_hook: "match",
            matcher_with_id: true,
            common_junk: false,
            common_casket_id: None,
        }
    }

    fn begin_transfer(
        snapshot: &GameSnapshot,
        ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
        args: TransferArgs,
    ) -> crate::native::ActionHandle<Transfer> {
        with_tick(snapshot, ledger, 1, |tick| {
            tick.actions.begin::<Transfer>(args, &mut tick.cx).unwrap()
        })
    }

    fn poll_transfer(
        snapshot: &GameSnapshot,
        ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
        handle: &crate::native::ActionHandle<Transfer>,
        tick_number: u64,
    ) -> Poll<Result<TransferValue, ActionError>> {
        with_tick(snapshot, ledger, tick_number, |tick| {
            tick.actions.poll(handle, &mut tick.cx)
        })
    }

    fn acknowledge(
        ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
        tick: u64,
        accepted: bool,
    ) -> HostEffect {
        let ledger = ledger
            .as_mut()
            .expect("transfer must have an action ledger");
        let action = ledger.outbox.remove(0);
        ledger.complete_interaction(
            &action.authority(),
            InteractionReceipt {
                request_id: action.request_id.get(),
                evidence: EvidenceStamp {
                    run: action.run(),
                    tick,
                    sequence: tick,
                },
                accepted,
                chat_since: 0,
            },
        );
        action.effect
    }

    fn respond(hooks: &Arc<Mutex<TransferHooks>>, answer: TransferAnswer) -> TransferCall {
        let mut hooks = lock_hooks(hooks);
        let call = hooks.pending.take().expect("transfer callback is pending");
        hooks.answer = Some(answer);
        call
    }

    fn spend_interaction(cx: &mut ActionContext<'_>) {
        cx.action_id = cx.ledger.as_ref().unwrap().owner.as_ref().unwrap().id.get();
        let request_id = cx
            .emit(InteractReq::ContinueDialog { component_id: None })
            .unwrap();
        let evidence = cx.evidence();
        let ledger = cx.ledger.as_mut().unwrap();
        let action = ledger.outbox.pop().expect("budget-filling interaction");
        assert_eq!(action.request_id.get(), request_id);
        ledger.complete_interaction(
            &action.authority(),
            InteractionReceipt {
                request_id,
                evidence,
                accepted: true,
                chat_since: 0,
            },
        );
    }

    fn filler_rows(count: usize) -> Vec<ItemView> {
        (0..count)
            .map(|slot| {
                item(
                    1_000 + slot as i32,
                    "Filler",
                    1,
                    slot as i32,
                    ItemContainer::Inventory,
                    &[],
                )
            })
            .collect()
    }

    fn pack_with_withdrawn(count: i32) -> Vec<ItemView> {
        let mut pack = filler_rows(21);
        pack.extend((0..count).map(|offset| pack_item(2, "Lobster", 21 + offset)));
        pack
    }

    #[test]
    fn matching_deposit_visits_side_order_waits_for_budget_receipt_and_drop() {
        let mut snapshot = bank_snapshot(
            vec![pack_item(1, "Anchovy", 0), pack_item(2, "Lobster", 1)],
            vec![
                bank_item(1, "Anchovy", 10, &[]),
                bank_item(2, "Lobster", 10, &[]),
            ],
            vec![side_item(2, "Lobster", 1), side_item(1, "Anchovy", 0)],
        );
        let mut args = transfer_args(BankSeatOperation::DepositMatch);
        args.request.hooks_present = DEPOSIT_MATCH_HOOK_MATCH;
        let hooks = Arc::clone(&args.hooks);
        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
            let handle = tick.actions.begin::<Transfer>(args, &mut tick.cx).unwrap();
            spend_interaction(&mut tick.cx);
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());

            let first = respond(&hooks, TransferAnswer::Value(json!(false)));
            assert_eq!(first.hook, "match");
            assert_eq!(first.args, vec![json!("Lobster"), json!(2)]);
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());

            let second = respond(&hooks, TransferAnswer::Value(json!(true)));
            assert_eq!(second.args, vec![json!("Anchovy"), json!(1)]);
            assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
            handle
        });
        assert!(ledger.as_ref().unwrap().outbox.is_empty());

        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 2).is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 2, true),
            HostEffect::Interaction(InteractReq::InvButton { id: 1, slot: 0, .. })
        ));
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 2).is_pending());
        assert!(ledger.as_ref().unwrap().outbox.is_empty());

        snapshot.seed_inventory(vec![pack_item(2, "Lobster", 1)], 28);
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 3).is_pending());
        let remaining = respond(&hooks, TransferAnswer::Value(json!(false)));
        assert_eq!(remaining.args, vec![json!("Lobster"), json!(2)]);
        assert!(matches!(
            poll_transfer(&snapshot, &mut ledger, &handle, 3),
            Poll::Ready(Ok(TransferValue::Void))
        ));
    }

    #[test]
    fn deposit_all_restarts_in_order_only_after_the_posted_item_drop() {
        let mut snapshot = bank_snapshot(
            vec![pack_item(1, "Anchovy", 0), pack_item(2, "Lobster", 1)],
            vec![
                bank_item(1, "Anchovy", 10, &[]),
                bank_item(2, "Lobster", 10, &[]),
            ],
            vec![side_item(1, "Anchovy", 0), side_item(2, "Lobster", 1)],
        );
        let mut ledger = None;
        let handle = begin_transfer(
            &snapshot,
            &mut ledger,
            transfer_args(BankSeatOperation::DepositAll),
        );
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 1).is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1, true),
            HostEffect::Interaction(InteractReq::InvButton { id: 1, slot: 0, .. })
        ));
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 1).is_pending());
        assert!(ledger.as_ref().unwrap().outbox.is_empty());

        snapshot.seed_inventory(vec![pack_item(2, "Lobster", 1)], 28);
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 2).is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 2, true),
            HostEffect::Interaction(InteractReq::InvButton { id: 2, slot: 1, .. })
        ));
        snapshot.seed_inventory(Vec::new(), 28);
        assert!(matches!(
            poll_transfer(&snapshot, &mut ledger, &handle, 3),
            Poll::Ready(Ok(TransferValue::Void))
        ));
    }

    #[test]
    fn failed_deposit_match_callback_is_unavailable_without_a_click() {
        let snapshot = bank_snapshot(
            vec![pack_item(1, "Anchovy", 0)],
            vec![bank_item(1, "Anchovy", 10, &[])],
            vec![side_item(1, "Anchovy", 0)],
        );
        let mut args = transfer_args(BankSeatOperation::DepositMatch);
        args.request.hooks_present = DEPOSIT_MATCH_HOOK_MATCH;
        let hooks = Arc::clone(&args.hooks);
        let mut ledger = None;
        let handle = begin_transfer(&snapshot, &mut ledger, args);
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 1).is_pending());
        assert_eq!(
            respond(&hooks, TransferAnswer::Failed).args,
            vec![json!("Anchovy"), json!(1)]
        );
        assert!(matches!(
            poll_transfer(&snapshot, &mut ledger, &handle, 1),
            Poll::Ready(Err(ActionError::Unavailable(_)))
        ));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    #[test]
    fn delta_withdraw_uses_the_selected_source_row_and_settles_from_inventory() {
        let mut snapshot = bank_snapshot(
            Vec::new(),
            vec![
                bank_item(1, "Lobster", 4, &["Withdraw-1"]),
                bank_item(2, "Lobster", 4, &["Withdraw-1"]),
            ],
            Vec::new(),
        );
        let mut args = transfer_args(BankSeatOperation::WithdrawDelta);
        args.request.item_id = Some(2);
        args.request.quantity = Some(1);
        let mut ledger = None;
        let handle = begin_transfer(&snapshot, &mut ledger, args);
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 1).is_pending());
        assert!(matches!(
            &ledger.as_ref().unwrap().outbox[0].effect,
            HostEffect::Interaction(InteractReq::WithdrawX {
                bank_item_id: 2,
                count: 1,
                ..
            })
        ));
        assert!(matches!(
            acknowledge(&mut ledger, 1, true),
            HostEffect::Interaction(InteractReq::WithdrawX {
                bank_item_id: 2,
                lands_as_id: 2,
                count: 1,
                ..
            })
        ));
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 1).is_pending());

        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 2).is_pending());
        assert!(ledger.as_ref().unwrap().outbox.is_empty());

        snapshot.seed_inventory(vec![pack_item(2, "Lobster", 0)], 28);
        assert!(matches!(
            poll_transfer(&snapshot, &mut ledger, &handle, 3),
            Poll::Ready(Ok(TransferValue::Bool(true)))
        ));
    }

    #[test]
    fn delta_withdraw_waits_for_each_partial_landing_before_the_next_bounded_click() {
        let mut snapshot = bank_snapshot(
            Vec::new(),
            vec![bank_item(2, "Lobster", 10, &["Withdraw-5", "Withdraw-1"])],
            Vec::new(),
        );
        let mut args = transfer_args(BankSeatOperation::WithdrawDelta);
        args.request.item_id = Some(2);
        args.request.quantity = Some(7);
        let mut ledger = None;
        let handle = begin_transfer(&snapshot, &mut ledger, args);

        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 1).is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1, true),
            HostEffect::Interaction(InteractReq::WithdrawX {
                bank_item_id: 2,
                count: 5,
                ..
            })
        ));
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 1).is_pending());
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 2).is_pending());
        assert!(ledger.as_ref().unwrap().outbox.is_empty());

        snapshot.seed_inventory(pack_with_withdrawn(5), 28);
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 3).is_pending());
        assert!(matches!(
            &ledger.as_ref().unwrap().outbox[0].effect,
            HostEffect::Interaction(InteractReq::WithdrawX {
                bank_item_id: 2,
                count: 1,
                ..
            })
        ));
        assert!(matches!(
            acknowledge(&mut ledger, 3, true),
            HostEffect::Interaction(InteractReq::WithdrawX { count: 1, .. })
        ));
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 3).is_pending());
        assert!(ledger.as_ref().unwrap().outbox.is_empty());

        snapshot.seed_inventory(pack_with_withdrawn(6), 28);
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 4).is_pending());
        assert!(matches!(
            &ledger.as_ref().unwrap().outbox[0].effect,
            HostEffect::Interaction(InteractReq::WithdrawX {
                bank_item_id: 2,
                count: 1,
                ..
            })
        ));
        assert!(matches!(
            acknowledge(&mut ledger, 4, true),
            HostEffect::Interaction(InteractReq::WithdrawX { count: 1, .. })
        ));

        snapshot.seed_inventory(pack_with_withdrawn(7), 28);
        assert!(matches!(
            poll_transfer(&snapshot, &mut ledger, &handle, 5),
            Poll::Ready(Ok(TransferValue::Bool(true)))
        ));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    #[test]
    fn rejected_or_replaced_bank_sessions_do_not_complete_a_withdrawal() {
        let bank = vec![bank_item(2, "Lobster", 4, &["Withdraw-1"])];
        let snapshot = bank_snapshot(Vec::new(), bank.clone(), Vec::new());
        let mut args = transfer_args(BankSeatOperation::WithdrawDelta);
        args.request.item_id = Some(2);
        args.request.quantity = Some(1);
        let mut ledger = None;
        let handle = begin_transfer(&snapshot, &mut ledger, args);
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 1).is_pending());
        let _ = acknowledge(&mut ledger, 1, false);
        assert!(matches!(
            poll_transfer(&snapshot, &mut ledger, &handle, 2),
            Poll::Ready(Ok(TransferValue::Bool(false)))
        ));

        let snapshot = bank_snapshot(Vec::new(), bank.clone(), Vec::new());
        let mut args = transfer_args(BankSeatOperation::WithdrawDelta);
        args.request.item_id = Some(2);
        args.request.quantity = Some(1);
        let mut ledger = None;
        let handle = begin_transfer(&snapshot, &mut ledger, args);
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 1).is_pending());
        let _ = acknowledge(&mut ledger, 1, true);
        let mut replaced = bank_snapshot(Vec::new(), bank, Vec::new());
        replaced.seed_bank_observation(11, 42, Some(Vec::new()), Vec::new());
        assert!(matches!(
            poll_transfer(&replaced, &mut ledger, &handle, 2),
            Poll::Ready(Ok(TransferValue::Bool(false)))
        ));
    }

    #[test]
    fn withdraw_load_uses_the_fixed_ladder_and_settles_each_click() {
        let mut snapshot = bank_snapshot(
            filler_rows(21),
            vec![bank_item(2, "Lobster", 10, &["Withdraw-5", "Withdraw-1"])],
            Vec::new(),
        );
        let mut args = transfer_args(BankSeatOperation::WithdrawLoad);
        args.request.item_name = Some("Lobster".to_owned());
        let mut ledger = None;
        let handle = begin_transfer(&snapshot, &mut ledger, args);
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 1).is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1, true),
            HostEffect::Interaction(InteractReq::WithdrawX {
                bank_item_id: 2,
                count: 5,
                ..
            })
        ));
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 1).is_pending());
        assert!(ledger.as_ref().unwrap().outbox.is_empty());

        snapshot.seed_inventory(pack_with_withdrawn(5), 28);
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 2).is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 2, true),
            HostEffect::Interaction(InteractReq::WithdrawX {
                bank_item_id: 2,
                count: 1,
                ..
            })
        ));

        snapshot.seed_inventory(pack_with_withdrawn(6), 28);
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 3).is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 3, true),
            HostEffect::Interaction(InteractReq::WithdrawX {
                bank_item_id: 2,
                count: 1,
                ..
            })
        ));

        snapshot.seed_inventory(pack_with_withdrawn(7), 28);
        assert!(matches!(
            poll_transfer(&snapshot, &mut ledger, &handle, 4),
            Poll::Ready(Ok(TransferValue::Bool(true)))
        ));
    }

    #[test]
    fn withdraw_load_all_waits_for_a_posted_pack_change() {
        let mut snapshot = bank_snapshot(
            vec![pack_item(9, "Rune", 0)],
            vec![bank_item(2, "Lobster", 30, &["Withdraw-All", "Withdraw-1"])],
            Vec::new(),
        );
        let mut args = transfer_args(BankSeatOperation::WithdrawLoad);
        args.request.item_name = Some("Lobster".to_owned());
        let mut ledger = None;
        let handle = begin_transfer(&snapshot, &mut ledger, args);
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 1).is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1, true),
            HostEffect::Interaction(InteractReq::InvButton { id: 2, .. })
        ));
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 1).is_pending());
        assert!(ledger.as_ref().unwrap().outbox.is_empty());

        snapshot.seed_inventory(
            vec![pack_item(9, "Rune", 0), pack_item(2, "Lobster", 1)],
            28,
        );
        assert!(matches!(
            poll_transfer(&snapshot, &mut ledger, &handle, 2),
            Poll::Ready(Ok(TransferValue::Bool(true)))
        ));
    }

    #[test]
    fn wait_ready_logs_after_its_deadline_then_returns_false() {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(1);
        snapshot.seed_inventory(Vec::new(), 28);
        snapshot.seed_bank_observation(10, 41, None, Vec::new());
        let mut args = transfer_args(BankSeatOperation::WaitReady);
        args.request.timeout_ms = Some(600.0);
        args.request.hooks_present = HOOK_LOG;
        let hooks = Arc::clone(&args.hooks);
        let mut ledger = None;
        let handle = begin_transfer(&snapshot, &mut ledger, args);
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 1).is_pending());
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 2).is_pending());
        let log = respond(&hooks, TransferAnswer::Value(Value::Null));
        assert_eq!(
            log.args,
            vec![json!("bank: opened but the item list never arrived")]
        );
        assert!(matches!(
            poll_transfer(&snapshot, &mut ledger, &handle, 2),
            Poll::Ready(Ok(TransferValue::Bool(false)))
        ));
    }

    #[test]
    fn snapshot_wait_uses_packet_generation_not_observation_sequence() {
        let mut snapshot = bank_snapshot(Vec::new(), Vec::new(), Vec::new());
        let mut args = transfer_args(BankSeatOperation::WaitSnapshotAfter);
        args.request.baseline_generation = Some(41.0);
        let mut ledger = None;
        let handle = begin_transfer(&snapshot, &mut ledger, args);
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 1).is_pending());

        snapshot.seed_bank_observation(10, 42, Some(Vec::new()), Vec::new());
        assert!(matches!(
            poll_transfer(&snapshot, &mut ledger, &handle, 2),
            Poll::Ready(Ok(TransferValue::Bool(true)))
        ));
    }

    #[test]
    fn note_mode_returns_slot_update_only_after_the_receipt_and_next_tick() {
        let snapshot = bank_snapshot(Vec::new(), Vec::new(), Vec::new());
        let mut args = transfer_args(BankSeatOperation::SetNoteMode);
        args.request.note_mode = true;
        let mut ledger = None;
        let handle = begin_transfer(&snapshot, &mut ledger, args);
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 1).is_pending());
        assert!(matches!(
            acknowledge(&mut ledger, 1, true),
            HostEffect::Interaction(InteractReq::SetNoteMode { on: true })
        ));
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 1).is_pending());
        assert!(matches!(
            poll_transfer(&snapshot, &mut ledger, &handle, 2),
            Poll::Ready(Ok(TransferValue::NoteMode {
                intent: NoteIntent::Noted,
                ..
            }))
        ));
    }

    #[test]
    fn deposit_all_without_a_side_view_fails_at_the_active_time_bound() {
        let mut snapshot = bank_snapshot(
            vec![pack_item(1, "Anchovy", 0)],
            vec![bank_item(1, "Anchovy", 10, &[])],
            Vec::new(),
        );
        snapshot.seed_side_modal(-1);
        let mut ledger = None;
        let handle = begin_transfer(
            &snapshot,
            &mut ledger,
            transfer_args(BankSeatOperation::DepositAll),
        );

        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 1).is_pending());
        assert!(poll_transfer(&snapshot, &mut ledger, &handle, 2).is_pending());
        assert!(matches!(
            poll_transfer(&snapshot, &mut ledger, &handle, 3),
            Poll::Ready(Err(ActionError::Unavailable(message)))
                if message.as_ref() == DEPOSIT_VIEW_ERROR
        ));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    #[test]
    fn labelled_withdraw_keeps_the_original_amount_to_action_characterization() {
        for (label, quantity, expected) in [
            (Some("all"), None, "Withdraw All"),
            (Some("Withdraw 5"), None, "Withdraw 5"),
            (None, Some(12), "Withdraw All"),
            (None, Some(11), "Withdraw 10"),
            (None, Some(4), "Withdraw 1"),
            (None, None, "Withdraw 1"),
        ] {
            let snapshot = bank_snapshot(
                Vec::new(),
                vec![bank_item(379, "Lobster", 12, &[])],
                Vec::new(),
            );
            let mut args = transfer_args(BankSeatOperation::WithdrawLabel);
            args.request.item_name = Some("Lobster".into());
            args.request.menu_op = label.map(str::to_owned);
            args.request.quantity = quantity;
            let mut ledger = None;
            let handle = begin_transfer(&snapshot, &mut ledger, args);
            assert!(poll_transfer(&snapshot, &mut ledger, &handle, 1).is_pending());
            assert!(matches!(
                acknowledge(&mut ledger, 1, true),
                HostEffect::Interaction(InteractReq::Withdraw { name, action })
                    if name == "Lobster" && action == expected
            ));
            assert!(matches!(
                poll_transfer(&snapshot, &mut ledger, &handle, 2),
                Poll::Ready(Ok(TransferValue::Bool(true)))
            ));
            assert!(
                ledger.as_ref().unwrap().outbox.is_empty(),
                "exactly one label click"
            );
        }
    }
}
