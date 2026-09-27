use super::*;

/// Host-owned second phase of a bank Withdraw-X operation. The first phase
/// sent the X menu action; this record authorizes one count response only
/// while the same bank session remains current and before its deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingWithdrawXPhase {
    Dialog,
    Settlement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingWithdrawResult {
    WithdrawX,
    WithdrawLoad,
}

#[derive(Debug, Clone, Copy)]
pub struct PendingFillBaseline {
    pub bank_item_id: i32,
    pub before_used: usize,
    pub before_count: i32,
    pub before_stock: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct PendingWithdrawX {
    pub item_id: i32,
    pub count: i32,
    pub before: i32,
    pub target: i32,
    pub bank_generation: u64,
    pub phase: PendingWithdrawXPhase,
    pub deadline: Option<Instant>,
    pub remaining: Duration,
    pub result: PendingWithdrawResult,
    pub fill: Option<PendingFillBaseline>,
}

impl PendingWithdrawX {
    pub fn waiting_dialog(
        item_id: i32,
        count: i32,
        before: i32,
        target: i32,
        bank_generation: u64,
    ) -> Self {
        let remaining = Duration::from_millis(3000);
        Self {
            item_id,
            count,
            before,
            target,
            bank_generation,
            phase: PendingWithdrawXPhase::Dialog,
            deadline: Some(Instant::now() + remaining),
            remaining,
            result: PendingWithdrawResult::WithdrawX,
            fill: None,
        }
    }

    pub fn waiting_load_dialog(
        bank_item_id: i32,
        count: i32,
        before_used: usize,
        before_count: i32,
        before_stock: i32,
        bank_generation: u64,
    ) -> Self {
        let mut pending = Self::waiting_dialog(bank_item_id, count, 0, 0, bank_generation);
        pending.result = PendingWithdrawResult::WithdrawLoad;
        pending.fill = Some(PendingFillBaseline {
            bank_item_id,
            before_used,
            before_count,
            before_stock,
        });
        pending
    }

    pub fn waiting_settlement(mut self) -> Self {
        self.phase = PendingWithdrawXPhase::Settlement;
        self.remaining = Duration::from_millis(4000);
        self.deadline = Some(Instant::now() + self.remaining);
        self
    }

    pub fn expired(self) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    pub(super) fn freeze(&mut self) {
        if let Some(deadline) = self.deadline.take() {
            self.remaining = deadline.saturating_duration_since(Instant::now());
        }
    }

    pub(super) fn resume(&mut self) {
        if self.deadline.is_none() {
            self.deadline = Some(Instant::now() + self.remaining);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingBankOpKind {
    Deposit,
    Withdraw,
    /// Raw open-only `Withdraw X` (`Bank.withdraw(name, 'Withdraw X')`).
    /// The frozen caller reads the accepted click as the result
    /// (`Input.invButton` → `actions.menuAction`) and types the amount +
    /// Enter itself, so this kind never settles on an inventory delta:
    /// the sent action plus the still-current bank session acknowledge it.
    WithdrawXAction,
}

#[derive(Debug, Clone, Copy)]
pub struct PendingBankOp {
    pub kind: PendingBankOpKind,
    pub item_id: i32,
    pub before_count: i32,
    pub before_inventory_count: i32,
    pub bank_generation: u64,
    pub(super) deadline: Option<Instant>,
    pub(super) remaining: Duration,
}

impl PendingBankOp {
    pub fn new(
        kind: PendingBankOpKind,
        item_id: i32,
        before_count: i32,
        before_inventory_count: i32,
        bank_generation: u64,
    ) -> Self {
        let remaining = Duration::from_millis(match kind {
            PendingBankOpKind::Deposit => 2000,
            PendingBankOpKind::Withdraw => 4000,
            // The acknowledgment lands on the next observe pass; this bound
            // only backstops a stalled session, so it keeps the ordinary
            // withdrawal bound rather than inventing a new one.
            PendingBankOpKind::WithdrawXAction => 4000,
        });
        Self {
            kind,
            item_id,
            before_count,
            before_inventory_count,
            bank_generation,
            deadline: Some(Instant::now() + remaining),
            remaining,
        }
    }

    pub fn expired(self) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    pub(super) fn freeze(&mut self) {
        if let Some(deadline) = self.deadline.take() {
            self.remaining = deadline.saturating_duration_since(Instant::now());
        }
    }

    pub(super) fn resume(&mut self) {
        if self.deadline.is_none() {
            self.deadline = Some(Instant::now() + self.remaining);
        }
    }
}

impl SlotScript {
    /// Current host-owned Withdraw-X continuation, if one is armed.
    pub fn pending_withdraw_x(&self) -> Option<PendingWithdrawX> {
        self.pending_withdraw_x
    }

    /// Replace the one bounded Withdraw-X continuation for this slot.
    pub fn set_pending_withdraw_x(&mut self, pending: Option<PendingWithdrawX>) {
        self.pending_withdraw_x = pending;
    }

    /// Freeze the monotonic deadline without discarding the operation.
    pub fn freeze_pending_withdraw_x(&mut self) {
        if let Some(pending) = &mut self.pending_withdraw_x {
            pending.freeze();
        }
    }

    /// Resume a previously frozen monotonic deadline.
    pub fn resume_pending_withdraw_x(&mut self) {
        if let Some(pending) = &mut self.pending_withdraw_x {
            pending.resume();
        }
    }

    /// Last host-owned Withdraw-X result posted to this isolate.
    pub fn withdraw_x_result(&self) -> (u64, bool) {
        (self.withdraw_x_result_seq, self.withdraw_x_result)
    }

    /// Last host-owned withdrawLoad result posted to this isolate.
    pub fn withdraw_load_result(&self) -> (u64, bool) {
        (self.withdraw_load_result_seq, self.withdraw_load_result)
    }

    pub fn pending_bank_op(&self) -> Option<PendingBankOp> {
        self.pending_bank_op
    }

    pub fn set_pending_bank_op(&mut self, pending: Option<PendingBankOp>) {
        self.pending_bank_op = pending;
    }

    pub fn freeze_pending_bank_op(&mut self) {
        if let Some(pending) = &mut self.pending_bank_op {
            pending.freeze();
        }
    }

    pub fn resume_pending_bank_op(&mut self) {
        if let Some(pending) = &mut self.pending_bank_op {
            pending.resume();
        }
    }

    pub fn bank_op_result(&self) -> (u64, bool) {
        (self.bank_op_result_seq, self.bank_op_result)
    }

    pub fn complete_bank_op(&mut self, result: bool) {
        self.pending_bank_op = None;
        self.bank_op_result_seq = self.bank_op_result_seq.wrapping_add(1);
        self.bank_op_result = result;
    }

    /// Lifecycle stamp used to reject work that raced a stop/reconnect.
    pub fn work_epoch(&self) -> u64 {
        self.work_epoch
    }

    /// Complete the current operation and advance the posted result token.
    pub fn complete_withdraw_x(&mut self, result: bool) {
        self.pending_withdraw_x = None;
        self.withdraw_x_result_seq = self.withdraw_x_result_seq.wrapping_add(1);
        self.withdraw_x_result = result;
    }

    pub fn complete_withdraw_load(&mut self, result: bool) {
        self.pending_withdraw_x = None;
        self.withdraw_load_result_seq = self.withdraw_load_result_seq.wrapping_add(1);
        self.withdraw_load_result = result;
    }

    /// Complete the armed shared withdrawal continuation on its result channel.
    pub fn complete_current_withdrawal(&mut self, result: bool) {
        match self.pending_withdraw_x.map(|pending| pending.result) {
            Some(PendingWithdrawResult::WithdrawLoad) => self.complete_withdraw_load(result),
            Some(PendingWithdrawResult::WithdrawX) | None => self.complete_withdraw_x(result),
        }
    }
}
