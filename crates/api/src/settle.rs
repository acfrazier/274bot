//! Settle evidence: deltas folded from before/after family reads plus a
//! tick/ms budget. The host checks `Settle::done` after an interact to
//! decide whether the action landed.

/// Evidence an interaction settled, within a tick/ms budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settle {
    pub arrived: bool,
    pub item_delta: i32,
    pub xp_gained: i32,
    pub modal_opened: Option<i32>,
    pub modal_closed: Option<i32>,
    /// Ticks elapsed since the send.
    pub ticks: u32,
    /// Milliseconds elapsed since the send.
    pub ms: u64,
    pub budget_ticks: u32,
    pub budget_ms: u64,
}

impl Default for Settle {
    fn default() -> Self {
        Settle {
            arrived: false,
            item_delta: 0,
            xp_gained: 0,
            modal_opened: None,
            modal_closed: None,
            ticks: 0,
            ms: 0,
            budget_ticks: 10,
            budget_ms: 2_000,
        }
    }
}

impl Settle {
    /// True when some evidence arm is armed and the budget held.
    pub fn done(&self) -> bool {
        (self.arrived
            || self.item_delta != 0
            || self.xp_gained > 0
            || self.modal_opened.is_some()
            || self.modal_closed.is_some())
            && self.ticks <= self.budget_ticks
            && self.ms <= self.budget_ms
    }
}

/// Inv count delta across every slot (`after - before`).
pub fn item_delta(before: &[i32], after: &[i32]) -> i32 {
    before
        .iter()
        .zip(after)
        .map(|(b, a)| (a - b) as i64)
        .sum::<i64>() as i32
}

/// Total XP gained across skills (only positive per-skill gains count).
pub fn xp_gained(before: &[i32], after: &[i32]) -> i32 {
    before
        .iter()
        .zip(after)
        .map(|(b, a)| if a > b { (a - b) as i64 } else { 0 })
        .sum::<i64>() as i32
}

/// Modal transitions: `(opened, closed)` from before/after modal ids. The
/// closed arm carries the id that was open.
pub fn modal_delta(before: Option<i32>, after: Option<i32>) -> (Option<i32>, Option<i32>) {
    match (before, after) {
        (None, Some(id)) => (Some(id), None),
        (Some(id), None) => (None, Some(id)),
        _ => (None, None),
    }
}
