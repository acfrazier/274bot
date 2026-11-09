//! Native owner for `sortBank` (`src/bot/api/bank/bankSort.ts:55-126`).
//!
//! The planner is [`crate::bank::sort`]; this machine owns the arrange-mode
//! varp, the drag packets, the observed-tick waits and the result. Content:
//! `interface_bank/scripts/bank.rs2:6-15` applies one `INV_BUTTOND` per
//! packet, reading `%bankinsert` (varp 304) for swap or insert, and
//! `[if_button,bank_main:com_99|com_100]` sets that varp.
//!
//! Pacing is one drag per observed game tick: the slot's single interaction
//! event per tick (`native/actions.rs:46-49`), which is also a human rate.
//! The upstream batch of four and its wall-clock waits are not timing
//! authority here.

use crate::bank::ops::TRANSFER_OBSERVED_TICK_LIMIT;
use crate::bank::request::BankCategoryOverride;
use crate::bank::seat_transfer::{TransferAnswer, TransferCall, TransferHooks};
use crate::bank::sort::{BankMove, BankSortMode, BankSortScratch, CATEGORY_ORDER};
use crate::interact::InteractReq;
use crate::native::{defer_budget, ActionContext, ActionError, NativeMachine};
use api::snapshot::{ItemView, SnapshotView};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::Poll;

/// `%bankinsert`: 1 inserts, 0 swaps (`bank.rs2:6-11,14-15`).
pub const BANK_INSERT_VARP: i32 = 304;
/// `bank_main:com_99` (`%bankinsert = 0`, swap) as the 289 client numbers it.
pub const ARRANGE_SWAP_COM: i32 = 8130;
/// `bank_main:com_100` (`%bankinsert = 1`, insert).
pub const ARRANGE_INSERT_COM: i32 = 8131;
/// Observed rounds in a row whose drag left the bank order unchanged before
/// the sort reports `no progress`. Content applies an accepted drag within
/// its tick (`bank.rs2:6-11`), so an unchanged order one observed tick later
/// is a refusal; one repeat confirms it.
pub const SORT_STALL_LIMIT: u8 = 2;

/// Arguments and callback mailbox for one sort.
pub struct SortArgs {
    pub overrides: Vec<BankCategoryOverride>,
    pub hooks: Arc<Mutex<TransferHooks>>,
    pub log: bool,
}

/// The native `sortBank` machine; its output is the `BankSortResult` value.
pub struct Sort {
    args: SortArgs,
    scratch: Option<BankSortScratch>,
    phase: Phase,
    login_session: u64,
    generation: u64,
    restore_to: i32,
    sent: u32,
    mode: Option<BankSortMode>,
    unmatched: Vec<i32>,
    swap_only: bool,
    stalled: u8,
    round: u32,
    max_rounds: u32,
    sorted: bool,
    reason: &'static str,
    awaiting_log: bool,
}

#[derive(Clone, Copy)]
enum Phase {
    Start,
    Plan,
    Arrange {
        want: i32,
        restore: bool,
    },
    ArrangeAwait {
        want: i32,
        restore: bool,
        request_id: u64,
        started_tick: u64,
    },
    Drag {
        mv: BankMove,
        to_go: usize,
    },
    DragAwait {
        request_id: u64,
        before: u64,
        sent_tick: u64,
        to_go: usize,
    },
    Restore,
    Done,
}

impl Sort {
    fn result(&self) -> Value {
        sort_result(
            self.sorted,
            self.sent,
            self.mode,
            &self.unmatched,
            self.reason,
        )
    }

    /// The closed-bank round with the progress this sort already made.
    pub(crate) fn closed_result(&self) -> Value {
        sort_result(false, self.sent, self.mode, &self.unmatched, "bank closed")
    }

    /// The closed-bank round of a sort whose machine is already gone.
    pub(crate) fn closed_empty() -> Value {
        sort_result(false, 0, None, &[], "bank closed")
    }

    fn idle(&mut self, reason: &'static str) -> Value {
        self.reason = reason;
        self.result()
    }

    fn post_log(&mut self, message: String) -> Result<(), ActionError> {
        if !self.args.log {
            return Ok(());
        }
        let mut hooks = lock_hooks(&self.args.hooks);
        if hooks.pending.is_some() || hooks.answer.is_some() {
            return Err(unavailable("bank callback mailbox is occupied"));
        }
        hooks.pending = Some(TransferCall {
            hook: "log",
            args: vec![json!(message)],
        });
        self.awaiting_log = true;
        Ok(())
    }

    fn same_session(&self, cx: &ActionContext<'_>) -> bool {
        cx.run().session == self.login_session
            && cx.snapshot().bank_session().is_some_and(|session| {
                session.value.open && session.value.generation == self.generation
            })
    }

    /// Plan one round from the posted rows. `Ok(None)` ends the loop with
    /// [`Self::reason`] set.
    fn plan(
        &mut self,
        cx: &ActionContext<'_>,
    ) -> Result<Option<(BankMove, usize, BankSortMode)>, ActionError> {
        let snapshot = cx.snapshot();
        let Some(rows) = snapshot.bank().map(|observed| observed.value) else {
            self.reason = "snapshot not ready";
            return Ok(None);
        };
        // Scratch is sized by the posted bank and reused for every re-plan.
        if self
            .scratch
            .as_ref()
            .is_none_or(|scratch| scratch.capacity() < rows.len())
        {
            self.scratch = Some(
                BankSortScratch::new(rows.len())
                    .map_err(|error| unavailable_owned(error.to_string()))?,
            );
        }
        let Some(scratch) = self.scratch.as_mut() else {
            return Err(unavailable("bank sort scratch is unavailable"));
        };
        let overrides = |id| {
            self.args
                .overrides
                .iter()
                .find(|row| row.id == id)
                .and_then(|row| CATEGORY_ORDER.get(usize::from(row.category)).copied())
        };
        let force = self.swap_only.then_some(BankSortMode::Swap);
        let summary = scratch
            .plan(rows, overrides, force)
            .map_err(|error| unavailable_owned(error.to_string()))?;
        self.unmatched.clear();
        self.unmatched
            .extend(scratch.unmatched_ids(rows, overrides));
        if summary.move_count == 0 {
            self.reason = "sorted";
            self.sorted = true;
            return Ok(None);
        }
        let mut first = [BankMove::default(); 1];
        if scratch.first_moves(&mut first) == 0 {
            self.reason = "sorted";
            self.sorted = true;
            return Ok(None);
        }
        Ok(Some((first[0], summary.move_count, summary.mode)))
    }
}

impl NativeMachine for Sort {
    type Args = SortArgs;
    type Output = Value;

    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        Ok(Self {
            args,
            scratch: None,
            phase: Phase::Start,
            login_session: cx.run().session,
            generation: 0,
            restore_to: 0,
            sent: 0,
            mode: None,
            unmatched: Vec::new(),
            swap_only: false,
            stalled: 0,
            round: 0,
            max_rounds: 0,
            sorted: false,
            reason: "round limit",
            awaiting_log: false,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        if self.awaiting_log {
            let Some(answer) = lock_hooks(&self.args.hooks).answer.take() else {
                return Poll::Pending;
            };
            self.awaiting_log = false;
            if matches!(answer, TransferAnswer::Failed) {
                return Poll::Ready(Err(unavailable("bank callback failed")));
            }
        }
        loop {
            // A dropped bank session (closed bank, kept reconnect) ends every
            // in-session phase with the progress already made; restoring the
            // arrange mode needs the open bank and is skipped.
            if !matches!(self.phase, Phase::Start | Phase::Restore | Phase::Done)
                && !self.same_session(cx)
            {
                return Poll::Ready(Ok(self.closed_result()));
            }
            match self.phase {
                Phase::Start => {
                    let snapshot = cx.snapshot();
                    let Some(session) =
                        snapshot.bank_session().filter(|session| session.value.open)
                    else {
                        return Poll::Ready(Ok(self.idle("bank not open")));
                    };
                    let Some(rows) = snapshot.bank().map(|observed| observed.value) else {
                        return Poll::Ready(Ok(self.idle("snapshot not ready")));
                    };
                    self.generation = session.value.generation;
                    self.restore_to = varp(snapshot, BANK_INSERT_VARP).unwrap_or(0);
                    // Every accepted drag fixes at least one row of the plan,
                    // and an arrange fallback costs one round: two rounds per
                    // posted row bound the loop by the bank's own size.
                    let rows = u32::try_from(rows.len()).unwrap_or(u32::MAX);
                    self.max_rounds = rows.saturating_mul(2).saturating_add(2);
                    self.phase = Phase::Plan;
                }
                Phase::Plan => {
                    if self.round >= self.max_rounds {
                        self.reason = "round limit";
                        self.phase = Phase::Restore;
                        continue;
                    }
                    let planned = match self.plan(cx) {
                        Ok(planned) => planned,
                        Err(error) => return Poll::Ready(Err(error)),
                    };
                    let Some((mv, to_go, mode)) = planned else {
                        self.phase = Phase::Restore;
                        continue;
                    };
                    let want = i32::from(mode == BankSortMode::Insert);
                    if varp(cx.snapshot(), BANK_INSERT_VARP) == Some(want) {
                        self.mode = Some(mode);
                        self.phase = Phase::Drag { mv, to_go };
                    } else {
                        self.phase = Phase::Arrange {
                            want,
                            restore: false,
                        };
                    }
                }
                Phase::Arrange { want, restore } => {
                    if cx.interaction_event_spent() {
                        return Poll::Pending;
                    }
                    let component_id = if want == 1 {
                        ARRANGE_INSERT_COM
                    } else {
                        ARRANGE_SWAP_COM
                    };
                    let request_id =
                        match defer_budget(cx.emit(InteractReq::IfButton { component_id })) {
                            Poll::Pending => return Poll::Pending,
                            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                            Poll::Ready(Ok(request_id)) => request_id,
                        };
                    self.phase = Phase::ArrangeAwait {
                        want,
                        restore,
                        request_id,
                        started_tick: cx.evidence().tick,
                    };
                    return Poll::Pending;
                }
                Phase::ArrangeAwait {
                    want,
                    restore,
                    request_id,
                    started_tick,
                } => {
                    let stuck = cx
                        .interaction_receipt(request_id)
                        .is_some_and(|receipt| !receipt.accepted)
                        || cx.evidence().tick.saturating_sub(started_tick)
                            >= TRANSFER_OBSERVED_TICK_LIMIT;
                    if varp(cx.snapshot(), BANK_INSERT_VARP) == Some(want) {
                        if restore {
                            self.phase = Phase::Done;
                            continue;
                        }
                        self.phase = Phase::Plan;
                        // The varp is current; re-plan against the same rows.
                        continue;
                    }
                    if !stuck {
                        return Poll::Pending;
                    }
                    if restore {
                        self.phase = Phase::Done;
                        continue;
                    }
                    if self.swap_only {
                        self.reason = "arrange mode stuck";
                        self.phase = Phase::Restore;
                        continue;
                    }
                    self.swap_only = true;
                    self.round = self.round.saturating_add(1);
                    self.phase = Phase::Plan;
                    if let Err(error) = self.post_log(
                        "bank sort: arrange mode did not stick, falling back to swaps".into(),
                    ) {
                        return Poll::Ready(Err(error));
                    }
                    if self.awaiting_log {
                        return Poll::Pending;
                    }
                }
                Phase::Drag { mv, to_go } => {
                    if cx.interaction_event_spent() {
                        return Poll::Pending;
                    }
                    let snapshot = cx.snapshot();
                    let Some(rows) = snapshot.bank().map(|observed| observed.value) else {
                        return Poll::Pending;
                    };
                    let Some(row) = row_at(rows, i32::from(mv.from)) else {
                        self.phase = Phase::Plan;
                        continue;
                    };
                    let before = signature(rows);
                    let request = InteractReq::BankDrag {
                        id: row.def.id,
                        slot: row.slot,
                        component: row.component_id,
                        target_slot: i32::from(mv.to),
                        insert: self.mode == Some(BankSortMode::Insert),
                        bank_generation: self.generation,
                    };
                    let request_id = match defer_budget(cx.emit(request)) {
                        Poll::Pending => return Poll::Pending,
                        Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                        Poll::Ready(Ok(request_id)) => request_id,
                    };
                    self.sent = self.sent.saturating_add(1);
                    self.phase = Phase::DragAwait {
                        request_id,
                        before,
                        sent_tick: cx.evidence().tick,
                        to_go,
                    };
                    return Poll::Pending;
                }
                Phase::DragAwait {
                    request_id,
                    before,
                    sent_tick,
                    to_go,
                } => {
                    let Some(receipt) = cx.interaction_receipt(request_id) else {
                        return Poll::Pending;
                    };
                    let accepted = receipt.accepted;
                    // The drag's inventory update arrives with the next
                    // observed tick after the one that carried the packet.
                    if accepted && cx.evidence().tick <= sent_tick {
                        return Poll::Pending;
                    }
                    let changed = cx
                        .snapshot()
                        .bank()
                        .is_some_and(|rows| signature(rows.value) != before);
                    self.round = self.round.saturating_add(1);
                    self.stalled = if changed {
                        0
                    } else {
                        self.stalled.saturating_add(1)
                    };
                    let mode = self.mode.map_or("swap", mode_name);
                    let message = format!(
                        "bank sort: round {}, {mode}, {} sent, {to_go} to go",
                        self.round,
                        u8::from(accepted)
                    );
                    if self.stalled >= SORT_STALL_LIMIT {
                        self.reason = "no progress";
                        self.sorted = false;
                        self.phase = Phase::Restore;
                    } else {
                        self.phase = Phase::Plan;
                    }
                    if let Err(error) = self.post_log(message) {
                        return Poll::Ready(Err(error));
                    }
                    if self.awaiting_log {
                        return Poll::Pending;
                    }
                }
                Phase::Restore => {
                    let want = i32::from(self.restore_to == 1);
                    let open = cx
                        .snapshot()
                        .bank_session()
                        .is_some_and(|session| session.value.open);
                    if !open || varp(cx.snapshot(), BANK_INSERT_VARP) == Some(want) {
                        self.phase = Phase::Done;
                        continue;
                    }
                    self.phase = Phase::Arrange {
                        want,
                        restore: true,
                    };
                }
                Phase::Done => return Poll::Ready(Ok(self.result())),
            }
        }
    }
}

/// The `BankSortResult` value as the seat terminal carries it. The bounded
/// channel codec allows at most [`crate::channel::MAX_COLLECTION`] entries per
/// array and a full 288-row bank can hold more unmatched ids, so `unmatched`
/// travels as consecutive chunks that the isolate machine joins before it settles `sortBank`.
fn sort_result(
    sorted: bool,
    moves: u32,
    mode: Option<BankSortMode>,
    unmatched: &[i32],
    reason: &'static str,
) -> Value {
    let chunks: Vec<Value> = unmatched
        .chunks(crate::channel::MAX_COLLECTION)
        .map(|chunk| json!(chunk))
        .collect();
    json!({
        "sorted": sorted,
        "moves": moves,
        "mode": mode.map(mode_name),
        "unmatched": chunks,
        "reason": reason,
    })
}

/// Upstream `BankSortMode` strings (`bankSortPlan.ts:4`).
fn mode_name(mode: BankSortMode) -> &'static str {
    match mode {
        BankSortMode::Swap => "swap",
        BankSortMode::Insert => "insert",
    }
}

fn varp(snapshot: SnapshotView<'_>, index: i32) -> Option<i32> {
    snapshot
        .varps()?
        .value
        .iter()
        .find(|row| row.index == index)
        .map(|row| row.value)
}

fn row_at(rows: &[ItemView], slot: i32) -> Option<&ItemView> {
    rows.iter().find(|row| row.slot == slot && row.count > 0)
}

/// FNV-1a over the posted `(slot, id)` order, the upstream `signature`
/// (`bankSort.ts:40-42`) without building a string.
fn signature(rows: &[ItemView]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for row in rows {
        for value in [row.slot, row.def.id] {
            for byte in value.to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    hash
}

fn lock_hooks(hooks: &Mutex<TransferHooks>) -> MutexGuard<'_, TransferHooks> {
    hooks.lock().unwrap_or_else(|error| error.into_inner())
}

fn unavailable(message: &'static str) -> ActionError {
    ActionError::Unavailable(Arc::from(message))
}

fn unavailable_owned(message: String) -> ActionError {
    ActionError::Unavailable(Arc::from(message))
}

#[cfg(test)]
#[path = "seat_sort_tests.rs"]
mod tests;
