//! The one bank transfer kernel. Compiled cards ([`crate::native_bank`]) and
//! the compatibility families ([`crate::bank_op`]) both call it.
//!
//! The kernel is stateless and copies nothing. Callers pass the rows they
//! already hold through [`BankRow`]: native `ItemView` and isolate `ItemRow`
//! both implement it, so a scan never converts or collects a row. Machines
//! own the clocks, the emits and the loops; the kernel only decides.

#[cfg(feature = "load")]
use crate::observed::ItemRow;
use crate::shim::InteractReq;
use api::snapshot::ItemView;
use std::sync::Arc;
use std::time::Duration;

/// One transfer's settle bound, and the close bound when the caller omits one.
pub const TRANSFER_BOUND: Duration = Duration::from_secs(4);
/// Frozen wait for the side backpack to post before an until-empty deposit
/// treats it as empty.
pub const DEPOSIT_VIEW_MS: u64 = 1_200;
/// Frozen `for (let guard = 0; guard < 32; guard++)` deposit rounds.
pub const MAX_DEPOSITS: u8 = 32;
/// Cap on caller id lists (keep, only, memo, withdrawals).
pub const MAX_MEMO: usize = 64;

// --- borrowed row ----------------------------------------------------------

/// Facts a transfer scan needs from one row. No policy.
pub trait BankRow {
    fn id(&self) -> i32;
    fn count(&self) -> i32;
    fn name(&self) -> Option<&str>;
    /// `None` when the representation omitted the slot (isolate `ItemRow`).
    fn slot(&self) -> Option<i32>;
    /// `None` when the representation omitted the component.
    fn component_id(&self) -> Option<i32>;
    /// Length of the 1-based op table, holes included.
    fn op_len(&self) -> i32;
    /// The non-empty label at 1-based op slot `one_based`. `None` for a hole
    /// and for any slot outside `1..=op_len()` (op 0 is never valid).
    fn op_label(&self, one_based: i32) -> Option<&str>;
}

/// The 0-based index of 1-based op slot `one_based`, if it is one.
fn op_index(one_based: i32) -> Option<usize> {
    usize::try_from(one_based).ok()?.checked_sub(1)
}

impl BankRow for ItemView {
    fn id(&self) -> i32 {
        self.def.id
    }

    fn count(&self) -> i32 {
        self.count
    }

    fn name(&self) -> Option<&str> {
        self.def.name.as_deref()
    }

    fn slot(&self) -> Option<i32> {
        Some(self.slot)
    }

    fn component_id(&self) -> Option<i32> {
        Some(self.component_id)
    }

    fn op_len(&self) -> i32 {
        i32::try_from(self.actions.len()).unwrap_or(i32::MAX)
    }

    fn op_label(&self, one_based: i32) -> Option<&str> {
        self.actions
            .get(op_index(one_based)?)?
            .as_deref()
            .filter(|label| !label.is_empty())
    }
}

/// The host posts a native action hole as an empty label in its slot, so
/// the 1-based positions match the native row.
#[cfg(feature = "load")]
impl BankRow for ItemRow {
    fn id(&self) -> i32 {
        self.id
    }

    fn count(&self) -> i32 {
        self.count
    }

    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    fn slot(&self) -> Option<i32> {
        self.slot
    }

    fn component_id(&self) -> Option<i32> {
        self.component_id
    }

    fn op_len(&self) -> i32 {
        i32::try_from(self.ops.len()).unwrap_or(i32::MAX)
    }

    fn op_label(&self, one_based: i32) -> Option<&str> {
        self.ops
            .get(op_index(one_based)?)
            .map(|label| &**label)
            .filter(|label| !label.is_empty())
    }
}

// --- identity --------------------------------------------------------------

/// JS `a.toLowerCase() === b.toLowerCase()` without allocating for ASCII.
pub fn same_name(a: &str, b: &str) -> bool {
    if a.is_ascii() && b.is_ascii() {
        a.eq_ignore_ascii_case(b)
    } else {
        a.to_lowercase() == b.to_lowercase()
    }
}

/// Every positive-count row id whose display name matches `name`, in row
/// order. A noted and an unnoted variant of one name are two ids.
pub fn ids_named<'a, R: BankRow>(rows: &'a [R], name: &'a str) -> impl Iterator<Item = i32> + 'a {
    rows.iter()
        .filter(move |row| {
            row.count() > 0 && row.name().is_some_and(|actual| same_name(actual, name))
        })
        .map(BankRow::id)
}

/// Total count of `id` across `rows`.
pub fn count_id<R: BankRow>(rows: &[R], id: i32) -> i32 {
    rows.iter()
        .filter(|row| row.id() == id)
        .map(BankRow::count)
        .sum()
}

/// Whether every slot of a posted pack of `inv_size` slots is taken.
pub fn pack_full<R: BankRow>(inv: &[R], inv_size: i32) -> bool {
    inv_size > 0 && inv.iter().filter(|row| row.count() > 0).count() >= inv_size as usize
}

/// Label bytes with whitespace, `-` and `_` gone, ASCII-folded.
pub fn normalized_action_bytes(value: &str) -> impl Iterator<Item = u8> + '_ {
    value
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace() && !matches!(*byte, b'-' | b'_'))
        .map(|byte| byte.to_ascii_lowercase())
}

/// `Withdraw-1`, `withdraw 1` and `Withdraw_1` are one label.
pub fn normalized_action_eq(actual: &str, expected: &str) -> bool {
    normalized_action_bytes(actual).eq(normalized_action_bytes(expected))
}

/// The 1-based op slot whose normalized label equals `wanted`.
pub fn action_op<R: BankRow>(row: &R, wanted: &str) -> Option<i32> {
    (1..=row.op_len()).find(|&op| {
        row.op_label(op)
            .is_some_and(|label| normalized_action_eq(label, wanted))
    })
}

/// The sweep op: the first label containing `all`, else the last non-empty
/// label (frozen `depositAllMatching` / `bestOpIndex`).
pub fn sweep_op<R: BankRow>(row: &R) -> Option<i32> {
    (1..=row.op_len())
        .find(|&op| row.op_label(op).is_some_and(contains_all))
        .or_else(|| {
            (1..=row.op_len())
                .rev()
                .find(|&op| row.op_label(op).is_some())
        })
}

fn contains_all(label: &str) -> bool {
    let mut window = [0u8; 3];
    normalized_action_bytes(label).any(|byte| {
        window = [window[1], window[2], byte];
        &window == b"all"
    })
}

// --- note mode -------------------------------------------------------------

/// The withdraw mode a caller asked for in one open bank session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NoteIntent {
    /// The game resets to Item whenever the bank opens.
    #[default]
    Item,
    Noted,
}

/// The intent for the open session `open_generation`. An intent stored for
/// another session is Item: opening the bank reset the toggle.
pub fn note_intent(stored_generation: u64, stored: NoteIntent, open_generation: u64) -> NoteIntent {
    if stored_generation == open_generation {
        stored
    } else {
        NoteIntent::Item
    }
}

/// The host maps this onto the live Note/Item pair (`set_note_mode`).
pub fn note_req(want: NoteIntent) -> InteractReq {
    InteractReq::SetNoteMode {
        on: matches!(want, NoteIntent::Noted),
    }
}

// --- withdraw click vs completion -------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletePolicy {
    /// Native `Withdraw` / `WithdrawAny` / BankBudget: `target` is the exact
    /// final inventory count; complete at `held >= target`.
    ExactAtLeast,
    /// Native `WithdrawTo`: complete at `held == target`; more is OverTarget.
    ExactEqual,
    /// Frozen `withdrawX` / `withdrawXById`: `target` was clipped once to the
    /// stock at begin; a full pack after progress also completes.
    AvailableLimited,
}

/// One withdraw request, built at begin and never updated per click.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WithdrawGoal {
    pub bank_item_id: i32,
    pub name: Arc<str>,
    /// The caller argument: a native final count or a `withdrawX` add-count.
    pub requested: i32,
    /// The observed landing id. Explicit; never rewritten to the source id.
    pub lands_as_id: i32,
    /// Held count of `lands_as_id` at begin.
    pub baseline: i32,
    /// The bank row's stock at begin.
    pub available: i32,
    /// The final held count this request must reach.
    pub target: i32,
    pub generation: u64,
    pub policy: CompletePolicy,
}

impl WithdrawGoal {
    /// Native `Withdraw` / `WithdrawAny` / BankBudget: `final_count` is the
    /// exact inventory count, whatever the stock.
    pub fn exact_at_least(
        bank_item_id: i32,
        name: Arc<str>,
        lands_as_id: i32,
        final_count: i32,
        available: i32,
        baseline: i32,
        generation: u64,
    ) -> Self {
        Self {
            bank_item_id,
            name,
            requested: final_count,
            lands_as_id,
            baseline,
            available,
            target: final_count,
            generation,
            policy: CompletePolicy::ExactAtLeast,
        }
    }

    /// Native `WithdrawTo`: exact equality; overshoot is rejected.
    pub fn exact_equal(
        bank_item_id: i32,
        name: Arc<str>,
        lands_as_id: i32,
        final_count: i32,
        available: i32,
        baseline: i32,
        generation: u64,
    ) -> Self {
        Self {
            policy: CompletePolicy::ExactEqual,
            ..Self::exact_at_least(
                bank_item_id,
                name,
                lands_as_id,
                final_count,
                available,
                baseline,
                generation,
            )
        }
    }

    /// Frozen `withdrawX*`: add `requested`, clipped once to the stock at
    /// begin. Later stock only bounds a click.
    pub fn available_limited(
        bank_item_id: i32,
        name: Arc<str>,
        lands_as_id: i32,
        requested: i32,
        available: i32,
        baseline: i32,
        generation: u64,
    ) -> Self {
        Self {
            bank_item_id,
            name,
            requested,
            lands_as_id,
            baseline,
            available,
            target: baseline.saturating_add(requested.min(available).max(0)),
            generation,
            policy: CompletePolicy::AvailableLimited,
        }
    }
}

/// What the next click still needs. Not clipped to stock: [`withdraw_click`]
/// bounds the emitted count by the live row.
pub fn withdraw_remaining(goal: &WithdrawGoal, now_held: i32) -> i32 {
    goal.target.saturating_sub(now_held).max(0)
}

/// The next withdraw click on `row` for `remaining` of `goal`: an exact
/// Withdraw-1/5/10, else Withdraw-X, else the largest fixed op not above the
/// need. The click's count may be smaller than `remaining` (fixed-only 7 is
/// 5, live stock 5 is 5); it is never the request's completion. `None`: the
/// row is another item, empty, or has no compatible op.
pub fn withdraw_click<R: BankRow>(
    row: &R,
    remaining: i32,
    goal: &WithdrawGoal,
) -> Option<InteractReq> {
    if row.id() != goal.bank_item_id {
        return None;
    }
    let requested = remaining.min(row.count().max(0));
    if requested <= 0 {
        return None;
    }
    let exact = match requested {
        1 => Some("Withdraw-1"),
        5 => Some("Withdraw-5"),
        10 => Some("Withdraw-10"),
        _ => None,
    };
    let (count, op) = exact
        .and_then(|label| action_op(row, label).map(|op| (requested, op)))
        .or_else(|| action_op(row, "Withdraw-X").map(|op| (requested, op)))
        .or_else(|| {
            [(10, "Withdraw-10"), (5, "Withdraw-5"), (1, "Withdraw-1")]
                .into_iter()
                .filter(|(count, _)| *count <= requested)
                .find_map(|(count, label)| action_op(row, label).map(|op| (count, op)))
        })?;
    Some(InteractReq::WithdrawX {
        name: goal.name.to_string(),
        count,
        bank_item_id: row.id(),
        lands_as_id: goal.lands_as_id,
        action: row.op_label(op)?.to_owned(),
        bank_generation: goal.generation,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    Incomplete,
    Complete,
    /// [`CompletePolicy::AvailableLimited`] only: progressed, and the pack is full.
    PackFull,
    /// [`CompletePolicy::ExactEqual`] only: more than the target is held.
    OverTarget,
    /// The bank closed or another session opened.
    SessionGone,
}

/// Whether `goal` is done with `now_held` of its landing id held.
pub fn withdraw_progress(
    goal: &WithdrawGoal,
    now_held: i32,
    pack_full: bool,
    same_session: bool,
) -> Progress {
    if !same_session {
        return Progress::SessionGone;
    }
    match goal.policy {
        CompletePolicy::ExactEqual => {
            if now_held == goal.target {
                Progress::Complete
            } else if now_held > goal.target {
                Progress::OverTarget
            } else {
                Progress::Incomplete
            }
        }
        CompletePolicy::ExactAtLeast => {
            if now_held >= goal.target {
                Progress::Complete
            } else {
                Progress::Incomplete
            }
        }
        CompletePolicy::AvailableLimited => {
            if now_held >= goal.target {
                Progress::Complete
            } else if now_held > goal.baseline && pack_full {
                Progress::PackFull
            } else {
                Progress::Incomplete
            }
        }
    }
}

/// One `withdrawLoad` click.
#[derive(Clone, Debug, PartialEq)]
pub enum LoadClick {
    /// The row's Withdraw-All, identity-fenced; no count dialog.
    All(InteractReq),
    /// No All op: the withdraw ladder for the free slots.
    Fill(InteractReq),
}

/// The `withdrawLoad` click on `row`: Withdraw-All when the row has it, else
/// [`withdraw_click`] for `free_slots` of `goal` (an available-limited goal).
pub fn load_click<R: BankRow>(row: &R, free_slots: i32, goal: &WithdrawGoal) -> Option<LoadClick> {
    if row.id() != goal.bank_item_id || row.count() <= 0 {
        return None;
    }
    if let Some(operation) = action_op(row, "Withdraw-All") {
        let (slot, component) = posted_position(row)?;
        return Some(LoadClick::All(InteractReq::InvButton {
            id: row.id(),
            slot,
            component,
            operation,
            bank_generation: goal.generation,
        }));
    }
    withdraw_click(row, free_slots, goal).map(LoadClick::Fill)
}

/// The All branch's posted-observation success. `bank_count_now` is this
/// row's id in the bank, not a name total.
pub fn load_all_progress(
    used_before: i32,
    used_now: i32,
    pack_full: bool,
    bank_count_now: i32,
    same_session: bool,
) -> Progress {
    if !same_session {
        Progress::SessionGone
    } else if used_now > used_before || pack_full || bank_count_now == 0 {
        Progress::Complete
    } else {
        Progress::Incomplete
    }
}

// --- deposit identity and settlement ----------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepositKind {
    /// Deposit every candidate; a settled empty side is success.
    UntilEmpty,
    /// The candidates must leave the pack; a posted side without them fails.
    Required,
}

/// Which pack ids a deposit moves. Borrows the caller's id lists (built
/// once at begin, at most [`MAX_MEMO`] ids each), so a poll copies nothing
/// and a machine stores nothing extra.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DepositSpec<'a> {
    pub kind: DepositKind,
    /// Never deposited.
    pub keep: &'a [i32],
    /// When set, only these ids are candidates.
    pub only: Option<&'a [i32]>,
}

impl DepositSpec<'_> {
    fn candidate(&self, id: i32) -> bool {
        !self.keep.contains(&id) && self.only.is_none_or(|only| only.contains(&id))
    }
}

/// A one-row labelled op (`Bank.deposit(name, op)`) or the All sweep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepositRequest<'a> {
    Label(&'a str),
    Sweep,
}

/// One exact bank-side row and its 1-based op. Not a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DepositClick {
    pub id: i32,
    pub slot: i32,
    pub component: i32,
    pub operation: i32,
}

fn posted_position<R: BankRow>(row: &R) -> Option<(i32, i32)> {
    let slot = row.slot().filter(|slot| *slot >= 0)?;
    let component = row.component_id().filter(|component| *component >= 0)?;
    Some((slot, component))
}

/// The click for `row`. `None` when the row's slot or component is missing
/// or negative, or it has no such op (a missing label is not a sweep).
pub fn deposit_click<R: BankRow>(row: &R, req: DepositRequest<'_>) -> Option<DepositClick> {
    let (slot, component) = posted_position(row)?;
    let operation = match req {
        DepositRequest::Label(label) => action_op(row, label)?,
        DepositRequest::Sweep => sweep_op(row)?,
    };
    Some(DepositClick {
        id: row.id(),
        slot,
        component,
        operation,
    })
}

/// The host re-resolves this exact row under `generation`; it never re-finds
/// a row by name.
pub fn deposit_req(click: DepositClick, generation: u64) -> InteractReq {
    InteractReq::InvButton {
        id: click.id,
        slot: click.slot,
        component: click.component,
        operation: click.operation,
        bank_generation: generation,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepositScan {
    /// Not enough observation. UntilEmpty: the machine arms
    /// [`DEPOSIT_VIEW_MS`] once the pack posted. Required: wait for the side.
    WaitView,
    Click(DepositClick),
    Done,
    /// Required: the pack still holds a candidate and the posted side has
    /// no clickable row for one.
    MissingRequired,
    /// The bank closed or another session opened: nothing settles.
    SessionGone,
}

/// The side backpack as a deposit observation: `Some` only while the main
/// bank is open and its side root is up. An empty `rows` is then a posted
/// empty side; vector length is never readiness.
pub fn side_observation<R: BankRow>(main_open: bool, side_root: i32, rows: &[R]) -> Option<&[R]> {
    (main_open && side_root >= 0).then_some(rows)
}

/// The next deposit step. `side` / `pack` `None` is not posted; `Some(&[])`
/// is posted empty. `empty_wait_done` is the UntilEmpty not-ready bound and
/// never turns a Required wait into success. Like [`withdraw_progress`],
/// nothing clicks or completes outside the request's bank session.
pub fn deposit_next<R: BankRow>(
    spec: &DepositSpec<'_>,
    side: Option<&[R]>,
    pack: Option<&[R]>,
    empty_wait_done: bool,
    same_session: bool,
) -> DepositScan {
    if !same_session {
        return DepositScan::SessionGone;
    }
    let Some(pack) = pack else {
        return DepositScan::WaitView;
    };
    let clickable = |side: &[R]| {
        side.iter()
            .filter(|row| row.count() > 0 && spec.candidate(row.id()))
            .find_map(|row| deposit_click(row, DepositRequest::Sweep))
    };
    match spec.kind {
        DepositKind::Required => {
            if !pack
                .iter()
                .any(|row| row.count() > 0 && spec.candidate(row.id()))
            {
                return DepositScan::Done;
            }
            let Some(side) = side else {
                return DepositScan::WaitView;
            };
            clickable(side).map_or(DepositScan::MissingRequired, DepositScan::Click)
        }
        DepositKind::UntilEmpty => match side {
            None if empty_wait_done => DepositScan::Done,
            None => DepositScan::WaitView,
            Some(side) => clickable(side).map_or(DepositScan::Done, DepositScan::Click),
        },
    }
}

// --- close -----------------------------------------------------------------

/// The caller's explicit bound, else [`TRANSFER_BOUND`].
pub fn close_deadline(timeout_ms: Option<u64>) -> Duration {
    timeout_ms.map_or(TRANSFER_BOUND, Duration::from_millis)
}

/// The bank session and side root seen when the Close verb was sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CloseBaseline {
    pub generation: u64,
    /// `-1` when no side root was up.
    pub side: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseScan {
    /// Already shut at begin: true, no verb.
    AlreadyShut,
    Waiting,
    /// Main shut, the old side root gone or changed, and a newer session.
    Complete,
    /// Main shut but the old side root is still up: backpack ops are not
    /// restored yet.
    SideHeld,
    /// Open again in another session, or the login session ended.
    SessionReplaced,
    TimedOut,
}

pub fn close_begin(open: bool) -> Option<CloseScan> {
    (!open).then_some(CloseScan::AlreadyShut)
}

pub fn close_progress(
    open: bool,
    side: i32,
    generation: u64,
    baseline: CloseBaseline,
    same_login: bool,
    deadline_reached: bool,
) -> CloseScan {
    if !same_login || (open && generation != baseline.generation) {
        return CloseScan::SessionReplaced;
    }
    let side_released = baseline.side < 0 || side != baseline.side;
    if !open && side_released && generation > baseline.generation {
        CloseScan::Complete
    } else if deadline_reached {
        CloseScan::TimedOut
    } else if !open && !side_released {
        CloseScan::SideHeld
    } else {
        CloseScan::Waiting
    }
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
