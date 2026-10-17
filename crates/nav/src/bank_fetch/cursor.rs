//! Resumable BankBudget selection, step construction and state deltas.

use std::mem::size_of;

use api::snapshot::WorldTile;

use super::{bank_access_tiles, BankStep, MissingRequirementsView};
use crate::collision::WorldCollision;
use crate::pack::BankStand;
use crate::router::MissingReqView;
use crate::world_state::WorldState;

#[derive(Debug, Default, Clone, Copy)]
enum Phase {
    #[default]
    Start,
    Bank,
    Carry,
    Shift,
    WornCheck,
    WearChoice,
    CarryTargets,
    WearTargets,
    Access,
    WithdrawSteps,
    Close,
    WearSteps,
    Done,
    Failed,
    Taken,
}

#[derive(Debug, Clone, Copy)]
enum Index {
    Bank,
    Carry,
    Selected,
}

type AccessKey = (i32, i32, i32, i32, i32, usize);

pub(super) fn vec_bytes<T>(values: &Vec<T>) -> usize {
    values.capacity().saturating_mul(size_of::<T>())
}

/// `Vec::push` grows geometrically. Twice the maximum row count also covers
/// a non-power-of-two `reserve_exact` starting capacity. The initial growth
/// minimum is eight byte elements or at most four larger elements.
pub(super) fn growing_vec_bytes<T>(maximum_rows: usize) -> Option<usize> {
    let item_bytes = size_of::<T>();
    if maximum_rows == 0 || item_bytes == 0 {
        return Some(0);
    }
    let minimum_rows = if item_bytes == 1 { 8 } else { 4 };
    maximum_rows
        .checked_mul(2)?
        .max(minimum_rows)
        .checked_mul(item_bytes)
}

pub(super) fn reserve_growth<T>(
    values: &mut Vec<T>,
    additional: usize,
    limit: Option<usize>,
) -> bool {
    let Some(required_rows) = values.len().checked_add(additional) else {
        return false;
    };
    if limit.is_some_and(|limit| required_rows > limit) {
        return false;
    }
    values.try_reserve(additional).is_ok()
}

/// Incremental selection for [`super::plan_bank_fetch`].
///
/// Keep the input slices and state unchanged between calls. Each call reads
/// one input row or alternative, constructs one fixed-size step group, or
/// checks one stand's four in-line access neighbours. Sorted lookup indices
/// shift at most one row per unit. It never clones the world state; every
/// owned allocation is a vector with an observable capacity.
#[derive(Debug, Default)]
pub struct BankFetchCursor {
    phase: Phase,
    index: usize,
    request: usize,
    alternative: usize,
    bank_counts: Vec<(i32, i32)>,
    carry_index: Vec<(i32, usize)>,
    carry: Vec<(i32, i32)>,
    wear: Vec<i32>,
    selected: Vec<i32>,
    carried_choice: Option<i32>,
    available_choice: Option<i32>,
    withdrawals: Vec<(i32, i32)>,
    nearest: Option<(AccessKey, WorldTile)>,
    rejected_access: Option<AccessKey>,
    steps: Vec<BankStep>,
    shift: Option<(Index, usize, usize, Phase)>,
    peak_bytes: usize,
    bank_rows_limit: Option<usize>,
    item_rows_limit: Option<usize>,
    step_rows_limit: Option<usize>,
}

impl BankFetchCursor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a cursor whose owned row buffers fit the captured input and
    /// plan-item limits. Construction performs no allocation.
    pub fn with_limits(bank_rows: usize, item_rows: usize) -> Option<Self> {
        let step_rows = item_rows.checked_mul(3)?.checked_add(3)?;
        Some(Self {
            bank_rows_limit: Some(bank_rows),
            item_rows_limit: Some(item_rows),
            step_rows_limit: Some(step_rows),
            ..Self::default()
        })
    }

    /// Checked pre-submit heap peak, including old/new allocation overlap.
    ///
    /// There is at most one bank index row per input bank row. Every diagnosis
    /// adds at most one carry/index row, selected/wear row and withdrawal.
    /// A withdrawal emits at most two steps, a wear emits one, and access/close
    /// add at most three. The input, inline cursor and post-state are separate.
    pub fn heap_bound(bank_rows: usize, missing_reqs: usize) -> Option<usize> {
        if missing_reqs == 0 {
            return Some(0);
        }
        let buffers = [
            growing_vec_bytes::<(i32, i32)>(bank_rows)?,
            growing_vec_bytes::<(i32, usize)>(missing_reqs)?,
            growing_vec_bytes::<(i32, i32)>(missing_reqs)?,
            growing_vec_bytes::<i32>(missing_reqs)?,
            growing_vec_bytes::<i32>(missing_reqs)?,
            growing_vec_bytes::<(i32, i32)>(missing_reqs)?,
            BankFetchPlan::steps_heap_bound(missing_reqs)?,
        ];
        let mut retained = 0usize;
        let mut overlap = 0usize;
        for bytes in buffers {
            retained = retained.checked_add(bytes)?;
            overlap = overlap.max(bytes);
        }
        // The vectors grow sequentially, so only one predecessor allocation
        // can overlap the final retained buffers.
        retained.checked_add(overlap)
    }

    /// Owned heap bytes only. The containing job accounts for inline state.
    pub fn retained_bytes(&self) -> usize {
        self.allocation_bytes()
            .into_iter()
            .fold(0, usize::saturating_add)
    }

    /// Conservative heap peak, including old/new buffers during growth.
    pub fn peak_retained_bytes(&self) -> usize {
        self.peak_bytes
    }

    /// Advance one bounded unit. `true` means success or domain failure is
    /// settled; [`Self::take_plan`] then distinguishes them.
    /// After a plan is ready, access retries no longer read `missing` or `bank`.
    /// Callers may release those inputs and pass empty slices after
    /// [`Self::reject_access`]. Keep the world, stands and origin unchanged.
    #[allow(clippy::too_many_arguments)] // borrows the same planner inputs without copying them
    pub fn advance<M: MissingRequirementsView + ?Sized>(
        &mut self,
        missing: &M,
        state: &WorldState,
        bank: &[(i32, i32)],
        stands: &[BankStand],
        from: WorldTile,
        collision: &WorldCollision,
    ) -> bool {
        let before = self.allocation_bytes();
        match self.phase {
            Phase::Start => {
                self.phase = if missing.is_empty() {
                    Phase::Failed
                } else {
                    Phase::Bank
                };
            }
            Phase::Bank => {
                if let Some(&(id, count)) = bank.get(self.index) {
                    // The legacy planner uses the first row, including a
                    // nonpositive first row followed by a positive duplicate.
                    if let Err(at) = self.bank_counts.binary_search_by_key(&id, |row| row.0) {
                        if reserve_growth(&mut self.bank_counts, 1, self.bank_rows_limit) {
                            self.bank_counts.push((id, count.max(0)));
                            self.shift =
                                Some((Index::Bank, at, self.bank_counts.len() - 1, Phase::Bank));
                            self.phase = Phase::Shift;
                        } else {
                            self.phase = Phase::Failed;
                        }
                    }
                    self.index += 1;
                } else {
                    self.index = 0;
                    self.phase = Phase::Carry;
                }
            }
            Phase::Carry => {
                if self.index < missing.len() {
                    match missing.requirement_at(self.index) {
                        Some(MissingReqView::Carry { id, count }) if count > 0 => {
                            match self.carry_index.binary_search_by_key(&id, |row| row.0) {
                                Ok(at) => {
                                    let target = &mut self.carry[self.carry_index[at].1].1;
                                    *target = (*target).max(count);
                                }
                                Err(at) => {
                                    if reserve_growth(
                                        &mut self.carry_index,
                                        1,
                                        self.item_rows_limit,
                                    ) && reserve_growth(&mut self.carry, 1, self.item_rows_limit)
                                    {
                                        self.carry_index.push((id, self.carry.len()));
                                        self.carry.push((id, count));
                                        self.shift = Some((
                                            Index::Carry,
                                            at,
                                            self.carry_index.len() - 1,
                                            Phase::Carry,
                                        ));
                                        self.phase = Phase::Shift;
                                    } else {
                                        self.phase = Phase::Failed;
                                    }
                                }
                            }
                        }
                        Some(MissingReqView::Carry { .. } | MissingReqView::WearAny { .. }) => {}
                        None => self.phase = Phase::Failed,
                    }
                    self.index += 1;
                } else {
                    self.phase = Phase::WornCheck;
                }
            }
            Phase::Shift => self.advance_shift(),
            Phase::WornCheck => self.check_worn(missing, state),
            Phase::WearChoice => self.choose_wear(missing, state),
            Phase::CarryTargets => {
                if let Some(&(id, _)) = self.carry.get(self.index) {
                    if !self.record_target(id, state) {
                        self.phase = Phase::Failed;
                    }
                    self.index += 1;
                } else {
                    self.index = 0;
                    self.phase = Phase::WearTargets;
                }
            }
            Phase::WearTargets => {
                if let Some(&id) = self.wear.get(self.index) {
                    if self
                        .carry_index
                        .binary_search_by_key(&id, |row| row.0)
                        .is_err()
                        && !self.record_target(id, state)
                    {
                        self.phase = Phase::Failed;
                    }
                    self.index += 1;
                } else {
                    self.index = 0;
                    self.phase = if !self.withdrawals.is_empty() {
                        Phase::Access
                    } else if self.wear.is_empty() {
                        Phase::Failed
                    } else {
                        Phase::WearSteps
                    };
                }
            }
            Phase::Access => self.check_access(collision, stands, from),
            Phase::WithdrawSteps => {
                if let Some(&(id, count)) = self.withdrawals.get(self.index) {
                    let simple = matches!(count, 1 | 5 | 10) || count == self.bank_count(id);
                    let step_count = if simple { 1 } else { 2 };
                    if reserve_growth(&mut self.steps, step_count, self.step_rows_limit) {
                        if simple {
                            self.steps.push(BankStep::Withdraw { id, count });
                        } else {
                            self.steps.push(BankStep::WithdrawX { id });
                            self.steps.push(BankStep::WithdrawXAmount { id, count });
                        }
                        self.index += 1;
                    } else {
                        self.phase = Phase::Failed;
                    }
                } else {
                    self.phase = Phase::Close;
                }
            }
            Phase::Close => {
                if self.wear.is_empty() {
                    if reserve_growth(&mut self.steps, 1, self.step_rows_limit) {
                        self.steps.push(BankStep::Close);
                    } else {
                        self.phase = Phase::Failed;
                    }
                }
                self.index = 0;
                if !matches!(self.phase, Phase::Failed) {
                    self.phase = Phase::WearSteps;
                }
            }
            Phase::WearSteps => {
                if let Some(&id) = self.wear.get(self.index) {
                    if reserve_growth(&mut self.steps, 1, self.step_rows_limit) {
                        self.steps.push(BankStep::Wear { id });
                        self.index += 1;
                    } else {
                        self.phase = Phase::Failed;
                    }
                } else {
                    self.phase = Phase::Done;
                }
            }
            Phase::Done | Phase::Failed | Phase::Taken => {}
        }
        self.record_peak(before);
        matches!(self.phase, Phase::Done | Phase::Failed | Phase::Taken)
    }

    /// Access tile awaiting a strict search with the pre-withdrawal state.
    /// A wear-only plan has no access route.
    pub fn planned_access(&self) -> Option<WorldTile> {
        if !matches!(self.phase, Phase::Done) {
            return None;
        }
        self.nearest.map(|(_, tile)| tile)
    }

    /// Continue the existing air-distance order after an unreachable access.
    /// Keep the item selection and its buffers for the next candidate.
    pub fn reject_access(&mut self) -> bool {
        if !matches!(self.phase, Phase::Done) {
            return false;
        }
        let Some((key, _)) = self.nearest.take() else {
            return false;
        };
        self.rejected_access = Some(key);
        self.steps.clear();
        self.index = 0;
        self.phase = Phase::Access;
        true
    }

    /// Transfer a successful plan exactly once, releasing selection scratch.
    pub fn take_plan(&mut self) -> Option<BankFetchPlan> {
        if !matches!(self.phase, Phase::Done) {
            return None;
        }
        self.phase = Phase::Taken;
        self.bank_counts = Vec::new();
        self.carry_index = Vec::new();
        self.carry = Vec::new();
        self.selected = Vec::new();
        Some(BankFetchPlan {
            steps: std::mem::take(&mut self.steps),
            withdrawals: std::mem::take(&mut self.withdrawals),
            wear: std::mem::take(&mut self.wear),
            withdraw_at: 0,
            wear_at: 0,
        })
    }

    fn allocation_bytes(&self) -> [usize; 7] {
        [
            vec_bytes(&self.bank_counts),
            vec_bytes(&self.carry_index),
            vec_bytes(&self.carry),
            vec_bytes(&self.wear),
            vec_bytes(&self.selected),
            vec_bytes(&self.withdrawals),
            vec_bytes(&self.steps),
        ]
    }

    fn record_peak(&mut self, before: [usize; 7]) {
        let mut retained = 0usize;
        let mut overlap = 0usize;
        for (after, old) in self.allocation_bytes().into_iter().zip(before) {
            retained = retained.saturating_add(after);
            if after > old {
                overlap = overlap.max(old);
            }
        }
        // Buffers grow sequentially. At most one old allocation is still
        // alive beside the final buffers during an allocator's copy.
        self.peak_bytes = self.peak_bytes.max(retained.saturating_add(overlap));
    }

    fn advance_shift(&mut self) {
        let (index, at, mut from, resume) = self.shift.expect("an index insertion is active");
        if from > at {
            match index {
                Index::Bank => self.bank_counts.swap(from, from - 1),
                Index::Carry => self.carry_index.swap(from, from - 1),
                Index::Selected => self.selected.swap(from, from - 1),
            }
            from -= 1;
        }
        if from == at {
            self.shift = None;
            self.phase = resume;
        } else {
            self.shift = Some((index, at, from, resume));
        }
    }

    fn bank_count(&self, id: i32) -> i32 {
        self.bank_counts
            .binary_search_by_key(&id, |row| row.0)
            .map_or(0, |at| self.bank_counts[at].1)
    }

    fn carry_count(&self, id: i32) -> i32 {
        self.carry_index
            .binary_search_by_key(&id, |row| row.0)
            .map_or(0, |at| self.carry[self.carry_index[at].1].1)
    }

    fn held_count(state: &WorldState, id: i32) -> i32 {
        state.inv.get(&id).copied().unwrap_or(0).max(0)
    }

    fn check_worn<M: MissingRequirementsView + ?Sized>(&mut self, missing: &M, state: &WorldState) {
        match missing.requirement_at(self.request) {
            None => {
                self.index = 0;
                self.phase = Phase::CarryTargets;
            }
            Some(MissingReqView::Carry { .. }) => self.next_wear_request(),
            Some(MissingReqView::WearAny { ids }) => {
                if let Some(id) = ids.get(self.alternative) {
                    if state.worn.contains(id) || self.selected.binary_search(id).is_ok() {
                        self.next_wear_request();
                    } else {
                        self.alternative += 1;
                    }
                } else {
                    self.alternative = 0;
                    self.carried_choice = None;
                    self.available_choice = None;
                    self.phase = Phase::WearChoice;
                }
            }
        }
    }

    fn choose_wear<M: MissingRequirementsView + ?Sized>(
        &mut self,
        missing: &M,
        state: &WorldState,
    ) {
        let Some(MissingReqView::WearAny { ids }) = missing.requirement_at(self.request) else {
            unreachable!("the same WearAny request is retained between units");
        };
        if let Some(&id) = ids.get(self.alternative) {
            let reserved = self.carry_count(id);
            let held = Self::held_count(state, id);
            if held > reserved && self.carried_choice.is_none() {
                self.carried_choice = Some(id);
            }
            if held.saturating_add(self.bank_count(id)) > reserved
                && self.available_choice.is_none()
            {
                self.available_choice = Some(id);
            }
            self.alternative += 1;
        } else if let Some(id) = self.carried_choice.or(self.available_choice) {
            if reserve_growth(&mut self.wear, 1, self.item_rows_limit)
                && reserve_growth(&mut self.selected, 1, self.item_rows_limit)
            {
                let at = self
                    .selected
                    .binary_search(&id)
                    .expect_err("the selected alternative was not already worn");
                self.wear.push(id);
                self.selected.push(id);
                self.next_wear_request();
                self.shift = Some((
                    Index::Selected,
                    at,
                    self.selected.len() - 1,
                    Phase::WornCheck,
                ));
                self.phase = Phase::Shift;
            } else {
                self.phase = Phase::Failed;
            }
        } else {
            self.phase = Phase::Failed;
        }
    }
    fn next_wear_request(&mut self) {
        self.request += 1;
        self.alternative = 0;
    }

    fn record_target(&mut self, id: i32, state: &WorldState) -> bool {
        let Some(target) = self
            .carry_count(id)
            .checked_add(i32::from(self.selected.binary_search(&id).is_ok()))
        else {
            return false;
        };
        let held = Self::held_count(state, id);
        if held.saturating_add(self.bank_count(id)) < target {
            return false;
        }
        let count = target.saturating_sub(held);
        if count > 0 {
            if !reserve_growth(&mut self.withdrawals, 1, self.item_rows_limit) {
                return false;
            }
            self.withdrawals.push((id, count));
        }
        true
    }

    fn check_access(&mut self, collision: &WorldCollision, stands: &[BankStand], from: WorldTile) {
        if let Some(stand) = stands.get(self.index) {
            // Preserve the first equal minimum. The stand index keeps a tied
            // access on another plane eligible after the first query fails.
            for tile in bank_access_tiles(collision, stand) {
                let key = (
                    i32::from(stand.tile.level != from.level),
                    (stand.tile.x - from.x)
                        .abs()
                        .max((stand.tile.z - from.z).abs()),
                    (tile.x - from.x).abs().max((tile.z - from.z).abs()),
                    tile.x,
                    tile.z,
                    self.index,
                );
                if self.rejected_access.is_some_and(|rejected| key <= rejected) {
                    continue;
                }
                if self.nearest.as_ref().is_none_or(|(old, _)| key < *old) {
                    self.nearest = Some((key, tile));
                }
            }
            self.index += 1;
        } else if let Some((_, access)) = self.nearest {
            if reserve_growth(&mut self.steps, 2, self.step_rows_limit) {
                self.steps.push(BankStep::Walk {
                    x: access.x,
                    z: access.z,
                    level: access.level,
                });
                self.steps.push(BankStep::Open);
                self.index = 0;
                self.phase = Phase::WithdrawSteps;
            } else {
                self.phase = Phase::Failed;
            }
        } else {
            self.phase = Phase::Failed;
        }
    }
}

/// Owned session steps and exact post-state deltas, without a state copy.
#[derive(Debug)]
pub struct BankFetchPlan {
    steps: Vec<BankStep>,
    withdrawals: Vec<(i32, i32)>,
    wear: Vec<i32>,
    withdraw_at: usize,
    wear_at: usize,
}

impl BankFetchPlan {
    /// Retained heap while the plan waits for its pre-bank stand route.
    /// The transferred buffers do not grow during state application.
    pub fn heap_bound(missing_reqs: usize) -> Option<usize> {
        Self::steps_heap_bound(missing_reqs)?
            .checked_add(growing_vec_bytes::<(i32, i32)>(missing_reqs)?)?
            .checked_add(growing_vec_bytes::<i32>(missing_reqs)?)
    }

    /// Step capacity shared by cursor and plan bounds without copying storage.
    fn steps_heap_bound(missing_reqs: usize) -> Option<usize> {
        if missing_reqs == 0 {
            return Some(0);
        }
        let steps = missing_reqs.checked_mul(3)?.checked_add(3)?;
        growing_vec_bytes::<BankStep>(steps)
    }

    pub fn steps(&self) -> &[BankStep] {
        &self.steps
    }

    /// Exact heap allocation transferred by `into_steps`, excluding deltas.
    pub fn steps_retained_bytes(&self) -> usize {
        vec_bytes(&self.steps)
    }

    /// Heap bytes in steps and unapplied deltas, excluding the owned state.
    pub fn retained_bytes(&self) -> usize {
        self.steps_retained_bytes()
            .saturating_add(vec_bytes(&self.withdrawals))
            .saturating_add(vec_bytes(&self.wear))
    }

    /// Apply at most one withdrawal or wear move to the owned pre-state.
    /// Returns `true` once all deltas have been applied; later calls do nothing.
    pub fn apply_next(&mut self, state: &mut WorldState) -> bool {
        if let Some(&(id, count)) = self.withdrawals.get(self.withdraw_at) {
            let held = state.inv.entry(id).or_insert(0);
            *held = held.saturating_add(count);
            self.withdraw_at += 1;
        } else if let Some(&id) = self.wear.get(self.wear_at) {
            state.worn.insert(id);
            let exhausted = if let Some(held) = state.inv.get_mut(&id) {
                *held -= 1;
                *held <= 0
            } else {
                false
            };
            if exhausted {
                state.inv.remove(&id);
            }
            self.wear_at += 1;
        }
        self.withdraw_at == self.withdrawals.len() && self.wear_at == self.wear.len()
    }

    pub fn into_steps(self) -> Vec<BankStep> {
        self.steps
    }
}

#[cfg(test)]
#[path = "cursor_tests.rs"]
mod tests;
