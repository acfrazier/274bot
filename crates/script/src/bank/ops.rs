//! The one bank transfer kernel. Compiled cards ([`crate::native_bank`]),
//! native bank seats, and the host dispatch paths for their requests all
//! decide through here.
//!
//! The kernel is stateless and copies nothing. Callers pass the rows they
//! already hold through [`BankRow`]: native `ItemView` and isolate `ItemRow`
//! both implement it, so a scan never converts or collects a row. Machines
//! own the clocks, the emits and the loops; the kernel only decides.
//!
//! Where callers' historic semantics differ — the kernel withdraw ladder
//! vs the frozen [`next_chunk`] ladder, the fill rules of `withdrawLoad`
//! vs the host's [`fill_click_in`], folded vs normalized label matching —
//! each keeps its own named policy below. They decide alike wherever the
//! content labels (bank `option1..5`) reach.

use crate::shim::InteractReq;
use api::snapshot::{ItemView, LocView, NpcView, WorldTile};
use std::sync::Arc;
use std::time::Duration;

/// The production world tick duration. Server content advances on the 600 ms
/// engine tick; observed packet bursts can contain several such ticks at once.
pub const SERVER_TICK_MS: u64 = 600;
/// Minimum distinct fresh observed ticks before a transfer may expire; the
/// elapsed-time floor also requires one 600 ms server interval per tick.
pub const TRANSFER_OBSERVED_TICK_LIMIT: u64 = 8;
/// Round a duration up to the first complete production server tick.
pub const fn observed_ticks_for_millis(millis: u64) -> u64 {
    millis.saturating_add(SERVER_TICK_MS - 1) / SERVER_TICK_MS
}

/// A bank wait expires only after both its observed-tick budget and the
/// corresponding amount of server time have elapsed.
pub fn observed_wait_expired(
    started_tick: u64,
    current_tick: u64,
    elapsed: Duration,
    tick_limit: u64,
) -> bool {
    observed_wait_expired_with_time_bound(
        started_tick,
        current_tick,
        elapsed >= Duration::from_millis(tick_limit.saturating_mul(SERVER_TICK_MS)),
        tick_limit,
    )
}

/// The same dual bound when a pause-aware clock owns the elapsed-time floor.
pub fn observed_wait_expired_with_time_bound(
    started_tick: u64,
    current_tick: u64,
    time_bound_reached: bool,
    tick_limit: u64,
) -> bool {
    current_tick.saturating_sub(started_tick) >= tick_limit && time_bound_reached
}
/// Frozen wait for a usable side backpack before an until-empty deposit fails.
pub const DEPOSIT_VIEW_MS: u64 = 1_200;
/// Frozen `for (let guard = 0; guard < 32; guard++)` deposit rounds.
pub const MAX_DEPOSITS: u8 = 32;
/// Cap on caller id lists (keep, only, memo, withdrawals).
pub const MAX_MEMO: usize = 64;

/// A deposit dispatch or content refusal, distinct from an observed transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DepositFailure {
    #[error("{}", self.message())]
    DispatchRefused,
    #[error("{}", self.message())]
    DispatchMissing,
    #[error("{}", self.message())]
    NoCountFall,
    #[error("{}", self.message())]
    FullStack,
    #[error("{}", self.message())]
    BankFull,
    #[error("{}", self.message())]
    Unbankable,
}

impl DepositFailure {
    /// Shared user-facing text for status, traces and `Display`, without allocation.
    pub const fn message(self) -> &'static str {
        match self {
            Self::DispatchRefused => "bank deposit dispatch was refused",
            Self::DispatchMissing => "bank deposit dispatch receipt was not observed",
            Self::NoCountFall => "bank deposit did not move inventory",
            Self::FullStack => "bank deposit refused: full stack in bank",
            Self::BankFull => "bank deposit refused: bank is full",
            Self::Unbankable => "bank deposit refused: item cannot be banked",
        }
    }
}

/// Exact messages emitted by `interface_bank/scripts/bank.rs2:87-98,129-159`.
/// Callers must scope these to game messages newer than the dispatch baseline.
pub fn deposit_content_refusal(message: &str) -> Option<DepositFailure> {
    if message.eq_ignore_ascii_case("You already have a full stack of that item in the bank.") {
        Some(DepositFailure::FullStack)
    } else if message.eq_ignore_ascii_case("You don't have enough space in your bank account.")
        || message
            .eq_ignore_ascii_case("You need to subscribe for a members account if you want to")
        || message.eq_ignore_ascii_case("store any more items in the bank.")
    {
        Some(DepositFailure::BankFull)
    } else if message.eq_ignore_ascii_case("A magical force prevents you from banking this item!")
        || message.eq_ignore_ascii_case("You have more urgent things to do with this anthem!")
        || message.eq_ignore_ascii_case("Sorry, you can't bank the monkey!")
    {
        Some(DepositFailure::Unbankable)
    } else {
        None
    }
}

// --- borrowed row ----------------------------------------------------------

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
    /// `None` when a row representation omits item stackability.
    fn stackable(&self) -> Option<bool> {
        None
    }
}

/// The 0-based index of 1-based op slot `one_based`, if it is one.
pub fn op_index(one_based: i32) -> Option<usize> {
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
    fn stackable(&self) -> Option<bool> {
        Some(self.def.stackable)
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

/// Whether `pack` still holds this positive item row. When both components
/// expose a slot, identity includes the slot as well as the object id.
pub fn pack_has_current_row<R: BankRow>(pack: &[R], id: i32, slot: Option<i32>) -> bool {
    pack.iter().any(|main| {
        main.count() > 0
            && main.id() == id
            && match (main.slot(), slot) {
                (Some(main), Some(side)) => main == side,
                _ => true,
            }
    })
}

/// Taken slots: every row with a positive count.
pub fn occupied<R: BankRow>(rows: &[R]) -> i32 {
    i32::try_from(rows.iter().filter(|row| row.count() > 0).count()).unwrap_or(i32::MAX)
}

/// Whether every slot of a posted pack of `inv_size` slots is taken; an
/// unposted size (`<= 0`) is never full.
pub fn pack_full<R: BankRow>(inv: &[R], inv_size: i32) -> bool {
    inv_size > 0 && occupied(inv) >= inv_size
}

/// Label chars as the deleted host `norm_action` folded them: the full
/// Unicode lowercase of `value` (exactly `str::to_lowercase`), with every
/// Unicode whitespace char (VT and NBSP included), `-` and `_` gone.
///
/// Lazy and allocation-free for every label without a capital sigma: the
/// standard lowercase mapping is per-char except for `Σ`, whose final-sigma
/// rule depends on the Unicode `Cased`/`Case_Ignorable` tables that `char`
/// does not expose. A label containing `Σ` is folded through
/// `str::to_lowercase` itself (allocating; such labels are rare and come
/// only from scripts), so it matches exactly.
pub fn normalized_action_chars(value: &str) -> impl Iterator<Item = char> + '_ {
    let chars = if value.contains('\u{3a3}') {
        FoldSource::Lowered(value.to_lowercase().chars().collect::<Vec<_>>().into_iter())
    } else {
        FoldSource::PerChar {
            chars: value.chars(),
            pending: None,
        }
    };
    chars.filter(|&c| kept(c))
}

enum FoldSource<'a> {
    /// Context-free path: each source char's own lowercase expansion
    /// (full Unicode lowercase can expand one char into several, e.g.
    /// `İ` → `i̇`).
    PerChar {
        chars: std::str::Chars<'a>,
        pending: Option<std::char::ToLowercase>,
    },
    /// A label with a capital sigma, lowered by `str::to_lowercase`.
    Lowered(std::vec::IntoIter<char>),
}

impl Iterator for FoldSource<'_> {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        match self {
            Self::Lowered(chars) => chars.next(),
            Self::PerChar { chars, pending } => loop {
                if let Some(c) = pending.as_mut().and_then(Iterator::next) {
                    return Some(c);
                }
                *pending = Some(chars.next()?.to_lowercase());
            },
        }
    }
}

/// The fold keeps a lowered char unless it is Unicode whitespace (per
/// `char::is_whitespace`, so VT counts), `-` or `_`.
fn kept(c: char) -> bool {
    !c.is_whitespace() && c != '-' && c != '_'
}

/// `Withdraw-1`, `withdraw 1` and `Withdraw_1` are one label.
pub fn normalized_action_eq(actual: &str, expected: &str) -> bool {
    normalized_action_chars(actual).eq(normalized_action_chars(expected))
}

/// The 1-based op slot whose normalized label equals `wanted`.
pub fn action_op<R: BankRow>(row: &R, wanted: &str) -> Option<i32> {
    (1..=row.op_len()).find(|&op| {
        row.op_label(op)
            .is_some_and(|label| normalized_action_eq(label, wanted))
    })
}

/// The All op: the first label whose normalized form contains `all`
/// (even inside another word), 1-based. No fallback: a row without such
/// a label has no All op.
pub fn all_op<R: BankRow>(row: &R) -> Option<i32> {
    (1..=row.op_len()).find(|&op| row.op_label(op).is_some_and(contains_all))
}

/// The sweep op: the All op, else the last non-empty label (frozen
/// `depositAllMatching` / `bestOpIndex`).
pub fn sweep_op<R: BankRow>(row: &R) -> Option<i32> {
    all_op(row).or_else(|| {
        (1..=row.op_len())
            .rev()
            .find(|&op| row.op_label(op).is_some())
    })
}

/// The hunt's X-label spelling: `-` and space are one separator (other
/// separators are not), the ends are trimmed after that replacement, and
/// the rest folds ASCII case. `Withdraw-X` and `withdraw x` match
/// `Withdraw X`; `Withdraw_X` and `Withdraw  X` do not.
pub fn dash_space_eq(actual: &str, expected: &str) -> bool {
    let edge = |c: char| c.is_whitespace() || c == '-';
    let start = actual.find(|c: char| !edge(c)).unwrap_or(actual.len());
    let end = actual.rfind(|c: char| !edge(c)).map_or(start, |at| {
        at + actual[at..].chars().next().map_or(0, char::len_utf8)
    });
    let actual = &actual.as_bytes()[start..end];
    let expected = expected.as_bytes();
    actual.len() == expected.len()
        && actual.iter().zip(expected).all(|(&got, &want)| {
            let got = if got == b'-' { b' ' } else { got };
            got.eq_ignore_ascii_case(&want)
        })
}

fn contains_all(label: &str) -> bool {
    let mut window = ['\0'; 3];
    normalized_action_chars(label).any(|c| {
        window = [window[1], window[2], c];
        window == ['a', 'l', 'l']
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
    /// The caller's argument as given; its meaning follows `policy`: a native
    /// held target under `ExactAtLeast` / `ExactEqual` (`requested ==
    /// target`), a frozen `withdrawX` add-count under `AvailableLimited`
    /// (`target = baseline + min(requested, available)`). Nothing plans from
    /// it; the kernel reads `target` only.
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
    /// Deposit every candidate; an unavailable side view at its bound is an error.
    UntilEmpty,
    /// The candidates must leave the pack.
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

/// The exact posted label selected by `click`, if its row still appears in
/// `side`. The label is borrowed so callers need not copy it for diagnostics.
pub fn deposit_option<R: BankRow>(side: &[R], click: DepositClick) -> Option<&str> {
    side.iter()
        .find(|row| {
            row.id() == click.id
                && row.slot() == Some(click.slot)
                && row.component_id() == Some(click.component)
        })?
        .op_label(click.operation)
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
    /// The current main candidates have no usable matching side row yet.
    WaitView,
    /// The until-empty view bound expired before a usable deposit view was available.
    ViewExpired,
    Click(DepositClick),
    Done,
    /// Capacity release: a matching row exists, but no safe operation can be
    /// selected for its stackability or posted actions.
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
/// is posted. `view_wait_expired` expires only the UntilEmpty view wait;
/// it never turns a deposit with current candidates into success. Like
/// [`withdraw_progress`], nothing clicks or completes outside the request's
/// bank session.
pub fn deposit_next<R: BankRow>(
    spec: &DepositSpec<'_>,
    side: Option<&[R]>,
    pack: Option<&[R]>,
    view_wait_expired: bool,
    same_session: bool,
) -> DepositScan {
    if !same_session {
        return DepositScan::SessionGone;
    }
    let view_expired = spec.kind == DepositKind::UntilEmpty && view_wait_expired;
    let Some(pack) = pack else {
        return if view_expired {
            DepositScan::ViewExpired
        } else {
            DepositScan::WaitView
        };
    };
    let has_candidate = pack
        .iter()
        .any(|row| row.count() > 0 && spec.candidate(row.id()));
    if !has_candidate {
        return DepositScan::Done;
    }
    let Some(side) = side else {
        return if view_expired {
            DepositScan::ViewExpired
        } else {
            DepositScan::WaitView
        };
    };
    let click = side
        .iter()
        .filter(|row| row.count() > 0 && spec.candidate(row.id()))
        .filter(|side_row| pack_has_current_row(pack, side_row.id(), side_row.slot()))
        .find_map(|row| deposit_click(row, DepositRequest::Sweep));
    match click {
        Some(click) => DepositScan::Click(click),
        None if view_expired => DepositScan::ViewExpired,
        None => DepositScan::WaitView,
    }
}

fn same_item_slot<R: BankRow>(main: &R, side: &R) -> bool {
    match (main.slot(), side.slot()) {
        (Some(main), Some(side)) => main == side,
        _ => true,
    }
}

/// The next safe row deposit for an exact capacity release. Stackables must
/// use Deposit-All; unstackables use Deposit-1 so one inventory row is freed.
pub fn deposit_capacity_next<R: BankRow>(
    keep: &[i32],
    side: Option<&[R]>,
    pack: Option<&[R]>,
    same_session: bool,
) -> DepositScan {
    if !same_session {
        return DepositScan::SessionGone;
    }
    let Some(pack) = pack else {
        return DepositScan::WaitView;
    };
    let Some(side) = side else {
        return DepositScan::WaitView;
    };
    let mut candidate = false;
    let mut matching_side_row = false;
    for item in pack
        .iter()
        .filter(|item| item.count() > 0 && !keep.contains(&item.id()))
    {
        candidate = true;
        let has_side_row = side
            .iter()
            .any(|row| row.count() > 0 && row.id() == item.id() && same_item_slot(item, row));
        matching_side_row |= has_side_row;
        let operation = match item.stackable() {
            Some(true) => "Deposit-All",
            Some(false) => "Deposit-1",
            None => continue,
        };
        if let Some(click) = side
            .iter()
            .filter(|row| row.count() > 0 && row.id() == item.id() && same_item_slot(item, row))
            .find_map(|row| deposit_click(row, DepositRequest::Label(operation)))
        {
            return DepositScan::Click(click);
        }
    }
    if candidate && matching_side_row {
        DepositScan::MissingRequired
    } else if candidate {
        DepositScan::WaitView
    } else {
        DepositScan::Done
    }
}

// --- close -----------------------------------------------------------------

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

// --- frozen chunk ladder ---------------------------------------------------

/// One frozen `nextWithdrawChunk` step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Chunk {
    /// Withdraw-X for this many.
    X(f64),
    /// `Withdraw-10` / `Withdraw-5` / `Withdraw-1`.
    Op(&'static str),
}

/// Frozen `nextWithdrawChunk(need)`: Withdraw-X above 10, else the 10/5/1
/// ladder; nothing for no need. This published compatibility policy is
/// deliberately not [`withdraw_click`]'s ladder (need 7 is Withdraw-5
/// here, Withdraw-X/7 there); the callers that chunk by it — compat
/// `withdrawTo` and the hunt bank — keep their recorded emits.
pub fn next_chunk(need: f64) -> Option<Chunk> {
    if need <= 0.0 {
        None
    } else if need > 10.0 {
        Some(Chunk::X(need))
    } else if need >= 10.0 {
        Some(Chunk::Op("Withdraw-10"))
    } else if need >= 5.0 {
        Some(Chunk::Op("Withdraw-5"))
    } else {
        Some(Chunk::Op("Withdraw-1"))
    }
}

// --- host actions-slice policy ---------------------------------------------
//
// The host re-resolves a request against the live snapshot rows, whose
// `actions` tables it already holds as `Vec<Option<String>>`. These are
// the same normalized decisions over that slice, holes keeping their
// 1-based positions.

/// The 1-based op slot in `actions` whose normalized label equals
/// `wanted`, first match wins.
pub fn action_op_in(actions: &[Option<String>], wanted: &str) -> Option<i32> {
    actions
        .iter()
        .position(|label| {
            label
                .as_deref()
                .is_some_and(|label| normalized_action_eq(label, wanted))
        })
        .map(|index| index as i32 + 1)
}

/// The 1-based All slot in `actions`: the first label whose normalized
/// form contains `all`, no fallback ([`all_op`] over the slice).
pub fn all_op_in(actions: &[Option<String>]) -> Option<i32> {
    actions
        .iter()
        .position(|label| label.as_deref().is_some_and(contains_all))
        .map(|index| index as i32 + 1)
}

/// The slot of the label for exactly `count` (`Withdraw {count}`, any
/// separator spelling). The `-` of a negative count normalizes away, as
/// it does for a `Withdraw-10` label.
pub fn withdraw_count_op_in(actions: &[Option<String>], count: i32) -> Option<i32> {
    let mut digits = ['\0'; 10];
    let mut at = digits.len();
    let mut n = i64::from(count).unsigned_abs();
    loop {
        at -= 1;
        digits[at] = char::from_digit(u32::try_from(n % 10).unwrap_or(0), 10).unwrap_or('0');
        n /= 10;
        if n == 0 {
            break;
        }
    }
    let digits = &digits[at..];
    actions
        .iter()
        .position(|label| {
            label.as_deref().is_some_and(|label| {
                normalized_action_chars(label).eq("withdraw".chars().chain(digits.iter().copied()))
            })
        })
        .map(|index| index as i32 + 1)
}

/// The compiled host fill policy (BankBudget's bounded fill): an exact
/// `Withdraw {count}` label, then
/// All only when the fill consumes the current stock, then the
/// Withdraw-X continuation. The boolean is "needs the amount dialog".
/// Distinct from [`load_click`], the compat `withdrawLoad` policy.
pub fn fill_click_in(actions: &[Option<String>], count: i32, stock: i32) -> Option<(i32, bool)> {
    withdraw_count_op_in(actions, count)
        .map(|op| (op, false))
        .or_else(|| {
            (count == stock)
                .then(|| all_op_in(actions).map(|op| (op, false)))
                .flatten()
        })
        .or_else(|| action_op_in(actions, "Withdraw X").map(|op| (op, true)))
}

// --- open: booth and teller rows -------------------------------------------

/// The op table of a scene row, as the row's representation stores it.
/// `op_label` is the raw label at a 1-based slot: `None` for a hole the
/// representation stores as absent (native views), `Some("")` for one it
/// posts as an empty label (isolate rows).
pub trait OpLabels {
    fn op_len(&self) -> i32;
    fn op_label(&self, one_based: i32) -> Option<&str>;
}

/// A loc row a bank-open decision scans.
pub trait LocRow: OpLabels {
    fn id(&self) -> i32;
    fn tile(&self) -> WorldTile;
    fn distance(&self) -> i32;
    fn name(&self) -> Option<&str>;
}

/// An npc row a bank-open decision scans.
pub trait NpcRow: OpLabels {
    fn tile(&self) -> WorldTile;
    fn distance(&self) -> i32;
    fn name(&self) -> Option<&str>;
}

impl OpLabels for LocView {
    fn op_len(&self) -> i32 {
        i32::try_from(self.actions.len()).unwrap_or(i32::MAX)
    }

    fn op_label(&self, one_based: i32) -> Option<&str> {
        self.actions.get(op_index(one_based)?)?.as_deref()
    }
}

impl LocRow for LocView {
    fn id(&self) -> i32 {
        self.id
    }

    fn tile(&self) -> WorldTile {
        self.tile
    }

    fn distance(&self) -> i32 {
        self.distance
    }

    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

impl OpLabels for NpcView {
    fn op_len(&self) -> i32 {
        i32::try_from(self.actions.len()).unwrap_or(i32::MAX)
    }

    fn op_label(&self, one_based: i32) -> Option<&str> {
        self.actions.get(op_index(one_based)?)?.as_deref()
    }
}

impl NpcRow for NpcView {
    fn tile(&self) -> WorldTile {
        self.tile
    }

    fn distance(&self) -> i32 {
        self.distance
    }

    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

/// How an op label is compared: folded ASCII case only (the loc/npc
/// config spellings: `Use-quickly`, a catalog operation name), or the
/// normalized menu-label equality of [`normalized_action_eq`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelMatch {
    Folded,
    Normalized,
}

fn label_eq(label: &str, wanted: &str, policy: LabelMatch) -> bool {
    match policy {
        LabelMatch::Folded => label.eq_ignore_ascii_case(wanted),
        LabelMatch::Normalized => normalized_action_eq(label, wanted),
    }
}

/// Whether any label of `row` matches `wanted` under `policy`.
fn any_label<R: OpLabels>(row: &R, wanted: &str, policy: LabelMatch) -> bool {
    (1..=row.op_len()).any(|op| {
        row.op_label(op)
            .is_some_and(|label| label_eq(label, wanted, policy))
    })
}

/// The 1-based op slot of `row` matching `wanted` under `policy`, first
/// match wins.
pub fn op_slot<R: OpLabels>(row: &R, wanted: &str, policy: LabelMatch) -> Option<i32> {
    (1..=row.op_len()).find(|&op| {
        row.op_label(op)
            .is_some_and(|label| label_eq(label, wanted, policy))
    })
}

/// A packed 1-based op slot, kept only while that slot still carries a
/// non-empty label.
pub fn packed_op<R: OpLabels>(row: &R, packed: i32) -> Option<i32> {
    (packed >= 1 && row.op_label(packed).is_some_and(|label| !label.is_empty())).then_some(packed)
}

/// The compat named booth: a live row (id ≥ 0) named `name` carrying
/// `action`, on the player's plane when it is known, nearest first.
pub fn booth_named<'a, R: LocRow>(
    rows: &'a [R],
    name: &str,
    action: &str,
    here_level: Option<i32>,
) -> Option<&'a R> {
    rows.iter()
        .filter(|row| {
            row.id() >= 0
                && row.name().is_some_and(|got| got.eq_ignore_ascii_case(name))
                && any_label(*row, action, LabelMatch::Folded)
                && here_level.is_none_or(|level| level == row.tile().level)
        })
        .min_by_key(|row| row.distance())
}

/// The nearest row on `level` carrying `action` (the native no-stand
/// booth: a catalog operation's op, or `Use-quickly`).
pub fn booth_nearest_action<'a, R: LocRow>(
    rows: &'a [R],
    level: i32,
    action: &str,
) -> Option<&'a R> {
    rows.iter()
        .filter(|row| row.tile().level == level && any_label(*row, action, LabelMatch::Folded))
        .min_by_key(|row| row.distance())
}

/// The first row standing exactly on `tile`, named `name` when one is
/// given.
pub fn booth_at<'a, R: LocRow>(
    rows: &'a [R],
    tile: WorldTile,
    name: Option<&str>,
) -> Option<&'a R> {
    rows.iter().find(|row| {
        row.tile() == tile
            && name.is_none_or(|wanted| {
                row.name()
                    .is_some_and(|got| got.eq_ignore_ascii_case(wanted))
            })
    })
}

/// The exact snapshot-selected booth identity: this id on this tile,
/// still named `name` and still carrying `action`.
pub fn booth_exact<'a, R: LocRow>(
    rows: &'a [R],
    tile: WorldTile,
    id: i32,
    name: &str,
    action: &str,
) -> Option<&'a R> {
    rows.iter().find(|row| {
        row.id() == id
            && row.tile() == tile
            && row.name().is_some_and(|got| got.eq_ignore_ascii_case(name))
            && any_label(*row, action, LabelMatch::Folded)
    })
}

/// The catalog bank object around its declared stand: named `name`,
/// carrying `action`, within Chebyshev `radius` of `stand` on its
/// plane, nearest first and by id on a tie.
pub fn booth_catalog<'a, R: LocRow>(
    rows: &'a [R],
    stand: WorldTile,
    radius: i32,
    name: &str,
    action: &str,
) -> Option<&'a R> {
    let chebyshev = |row: &R| {
        row.tile()
            .x
            .abs_diff(stand.x)
            .max(row.tile().z.abs_diff(stand.z))
    };
    rows.iter()
        .filter(|row| {
            row.tile().level == stand.level
                && chebyshev(row) <= u32::try_from(radius).unwrap_or(u32::MAX)
                && row.name().is_some_and(|got| got.eq_ignore_ascii_case(name))
                && any_label(*row, action, LabelMatch::Folded)
        })
        .min_by_key(|row| (chebyshev(row), row.id()))
}

/// The first npc standing exactly on `tile`, named `name`.
pub fn npc_at<'a, R: NpcRow>(rows: &'a [R], tile: WorldTile, name: &str) -> Option<&'a R> {
    rows.iter().find(|row| {
        row.tile() == tile && row.name().is_some_and(|got| got.eq_ignore_ascii_case(name))
    })
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
