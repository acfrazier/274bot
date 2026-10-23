//! The advisory-row overlay with resumable copying, matching and insertion.

use super::cursor::{growing_vec_bytes, reserve_growth, vec_bytes};
use super::MissingRequirementsView;
use crate::router::MissingReqView;

#[derive(Debug, Default, Clone, Copy)]
enum Phase {
    #[default]
    Start,
    Copy,
    Carry,
    Wear,
    Shift,
    Done,
    Failed,
    Taken,
}

#[derive(Debug, Default, Clone, Copy)]
enum Resume {
    #[default]
    Carry,
    Wear,
}

/// Incremental `Hint` overlay for [`super::planning_rows`].
///
/// Use only for `Origin::Hint`; other origins keep their existing rows. Keep
/// the original sorted rows and diagnosis unchanged between calls. Each unit
/// copies or shifts one row, reads one diagnosis, or tests one original row
/// against one wearable alternative. Binary search is bounded by the row
/// count, and an insertion's row movement is resumed one swap at a time.
#[derive(Debug, Default)]
pub struct PlanningRowsCursor {
    phase: Phase,
    resume: Resume,
    planned: Vec<(i32, i32)>,
    request: usize,
    alternative: usize,
    row: usize,
    insert_at: usize,
    shift_from: usize,
    peak_bytes: usize,
    bank_rows_limit: Option<usize>,
    item_rows_limit: Option<usize>,
    planned_rows_limit: Option<usize>,
}

impl PlanningRowsCursor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a cursor bounded by captured bank rows and unique plan items.
    /// Construction performs no allocation.
    pub fn with_limits(bank_rows: usize, item_rows: usize) -> Option<Self> {
        bank_rows.checked_add(item_rows)?;
        Some(Self {
            bank_rows_limit: Some(bank_rows),
            item_rows_limit: Some(item_rows),
            ..Self::default()
        })
    }

    /// Checked pre-submit heap peak, excluding input and inline state.
    ///
    /// Each diagnosis inserts at most one row. The bound covers the exact
    /// initial reservation, later geometric growth, and one old allocation
    /// during a growth copy. The transferred rows keep the same allocation.
    pub fn heap_bound(rows: usize, missing_reqs: usize) -> Option<usize> {
        Self::rows_heap_bound(rows, missing_reqs)?.checked_mul(2)
    }

    /// Capacity bound for the transferred overlay, without a growth predecessor.
    pub fn rows_heap_bound(rows: usize, missing_reqs: usize) -> Option<usize> {
        growing_vec_bytes::<(i32, i32)>(rows.checked_add(missing_reqs)?)
    }

    /// Owned heap bytes only; inline cursor state belongs to its containing job.
    pub fn retained_bytes(&self) -> usize {
        vec_bytes(&self.planned)
    }

    /// Conservative heap peak, including the old buffer during vector growth.
    pub fn peak_retained_bytes(&self) -> usize {
        self.peak_bytes
    }

    /// Advance one unit and return whether the overlay is complete.
    pub fn advance<M: MissingRequirementsView + ?Sized>(
        &mut self,
        rows: &[(i32, i32)],
        missing: &M,
    ) -> bool {
        let before = self.retained_bytes();
        match self.phase {
            Phase::Start => {
                let planned_rows_limit = self
                    .item_rows_limit
                    .and_then(|item_rows| rows.len().checked_add(item_rows));
                let within_limits = self.bank_rows_limit.is_none_or(|limit| rows.len() <= limit)
                    && (self.item_rows_limit.is_none() || planned_rows_limit.is_some());
                self.planned_rows_limit = planned_rows_limit;
                self.phase = if within_limits && self.planned.try_reserve_exact(rows.len()).is_ok()
                {
                    Phase::Copy
                } else {
                    Phase::Failed
                };
            }
            Phase::Copy => {
                if let Some(&row) = rows.get(self.row) {
                    if reserve_growth(&mut self.planned, 1, self.planned_rows_limit) {
                        self.planned.push(row);
                        self.row += 1;
                    } else {
                        self.phase = Phase::Failed;
                    }
                } else {
                    self.row = 0;
                    self.phase = Phase::Carry;
                }
            }
            Phase::Carry => {
                if self.request >= missing.len() {
                    self.request = 0;
                    self.phase = Phase::Wear;
                } else {
                    match missing.requirement_at(self.request) {
                        Some(MissingReqView::Carry { id, count }) => {
                            self.request += 1;
                            if count > 0 {
                                self.raise(id, count, false, Resume::Carry);
                            }
                        }
                        Some(MissingReqView::WearAny { .. }) => self.request += 1,
                        None => self.phase = Phase::Failed,
                    }
                }
            }
            Phase::Wear => self.check_wear(rows, missing),
            Phase::Shift => {
                if self.shift_from > self.insert_at {
                    self.planned.swap(self.shift_from, self.shift_from - 1);
                    self.shift_from -= 1;
                }
                if self.shift_from == self.insert_at {
                    self.phase = match self.resume {
                        Resume::Carry => Phase::Carry,
                        Resume::Wear => Phase::Wear,
                    };
                }
            }
            Phase::Done | Phase::Failed | Phase::Taken => {}
        }
        let after = self.retained_bytes();
        let overlap = if after > before { before } else { 0 };
        self.peak_bytes = self.peak_bytes.max(after.saturating_add(overlap));
        matches!(self.phase, Phase::Done | Phase::Failed | Phase::Taken)
    }

    /// Transfer the completed owned rows exactly once.
    pub fn take_rows(&mut self) -> Option<Vec<(i32, i32)>> {
        if !matches!(self.phase, Phase::Done) {
            return None;
        }
        self.phase = Phase::Taken;
        Some(std::mem::take(&mut self.planned))
    }

    fn check_wear<M: MissingRequirementsView + ?Sized>(
        &mut self,
        rows: &[(i32, i32)],
        missing: &M,
    ) {
        match missing.requirement_at(self.request) {
            None if self.request >= missing.len() => self.phase = Phase::Done,
            None => self.phase = Phase::Failed,
            Some(MissingReqView::Carry { .. }) => self.next_request(),
            Some(MissingReqView::WearAny { ids }) => {
                let Some(&first) = ids.first() else {
                    self.next_request();
                    return;
                };
                if let Some(&id) = ids.get(self.alternative) {
                    if let Some(&(row, count)) = rows.get(self.row) {
                        self.row += 1;
                        if row == id && count > 0 {
                            self.next_request();
                        }
                    } else {
                        self.alternative += 1;
                        self.row = 0;
                    }
                } else {
                    // Only the original hint settles availability. Earlier
                    // Carry overlays and WearAny additions do not count.
                    self.next_request();
                    self.raise(first, 1, true, Resume::Wear);
                }
            }
        }
    }

    fn next_request(&mut self) {
        self.request += 1;
        self.alternative = 0;
        self.row = 0;
    }

    fn raise(&mut self, id: i32, count: i32, add: bool, resume: Resume) {
        match self.planned.binary_search_by_key(&id, |&(row, _)| row) {
            Ok(at) if add => {
                self.planned[at].1 = self.planned[at].1.saturating_add(count);
            }
            Ok(at) => self.planned[at].1 = self.planned[at].1.max(count),
            Err(at) => {
                if !reserve_growth(&mut self.planned, 1, self.planned_rows_limit) {
                    self.phase = Phase::Failed;
                    return;
                }
                self.planned.push((id, count));
                self.insert_at = at;
                self.shift_from = self.planned.len() - 1;
                self.resume = resume;
                self.phase = Phase::Shift;
            }
        }
    }
}
