//! Bounded, opt-in fixed-window responsiveness cohort journal.
//!
//! This is additive to the legacy responsiveness aggregates.  A cohort is a
//! single process-local observation with an explicit producer-close barrier;
//! timestamps alone never prove that all pre-end producers have published.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

pub const SCHEMA_VERSION: u16 = 1;
pub const DEFAULT_CAPACITY: usize = 512;
pub const DEFAULT_TAIL_NS: u64 = 5_000_000_000;

static OPT_IN: AtomicBool = AtomicBool::new(false);
static STATE: OnceLock<Mutex<State>> = OnceLock::new();

fn state() -> &'static Mutex<State> {
    STATE.get_or_init(|| Mutex::new(State::default()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Boundaries {
    pub start_mono_ns: u64,
    pub end_mono_ns: u64,
    pub tail_ns: u64,
}
impl Boundaries {
    fn validate(self) -> Result<(), CohortError> {
        if self.start_mono_ns >= self.end_mono_ns
            || self.tail_ns == 0
            || self.end_mono_ns.checked_add(self.tail_ns).is_none()
        {
            return Err(CohortError::MalformedBoundaries);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventId {
    pub slot_id: u64,
    pub generation: u64,
    pub sequence: u64,
    pub start_mono_ns: u64,
    pub surface: Surface,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Surface {
    Decode,
    Panel,
    Tui,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Completed,
    Canceled,
    Lost,
    Dropped,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRecord {
    pub id: EventId,
    pub complete_mono_ns: Option<u64>,
    pub outcome: Outcome,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LossReason {
    Capacity,
    LateEvent,
    GenerationMismatch,
    MissingIdentity,
    Incomplete,
    MalformedTimestamp,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LossReceipt {
    pub sequence: u64,
    pub slot_id: u64,
    pub generation: u64,
    pub start_mono_ns: Option<u64>,
    pub surface: Surface,
    pub outcome: Outcome,
    pub reason: LossReason,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CohortBatch {
    pub schema_version: u16,
    pub boundaries: Boundaries,
    pub records: Vec<EventRecord>,
    pub losses: Vec<LossReceipt>,
    /// Cursor is the largest journal sequence included in this batch.
    pub next_cursor: u64,
    pub complete: bool,
    pub loss_count: u64,
    /// Number of terminal entries rejected after the bounded journal filled.
    pub journal_overflow_n: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSummary {
    pub schema_version: u16,
    pub boundaries: Boundaries,
    pub terminal: bool,
    pub available: bool,
    pub records_n: u64,
    pub losses_n: u64,
    pub pending_n: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CohortError {
    AlreadyActive,
    AlreadyFinalized,
    MalformedBoundaries,
    StaleOrOverlappingBoundaries,
    NotActive,
    InvalidCursor,
    LateEvent,
    GenerationMismatch,
    MissingIdentity,
    Terminal,
    ProducersNotClosed,
}
struct Pending {
    id: EventId,
}
struct JournalEntry {
    journal_sequence: u64,
    entry: Entry,
}
enum Entry {
    Record(EventRecord),
    Loss(LossReceipt),
}
struct State {
    boundaries: Option<Boundaries>,
    journal: VecDeque<JournalEntry>,
    pending: VecDeque<Pending>,
    next_sequence: u64,
    next_journal_sequence: u64,
    cursor_floor: u64,
    available: bool,
    finalized: bool,
    producers_closed: bool,
    capacity: usize,
    last_end: Option<u64>,
    records_n: u64,
    losses_n: u64,
    journal_overflow_n: u64,
}
impl Default for State {
    fn default() -> Self {
        Self {
            boundaries: None,
            journal: VecDeque::new(),
            pending: VecDeque::new(),
            next_sequence: 1,
            next_journal_sequence: 1,
            cursor_floor: 0,
            available: true,
            finalized: false,
            producers_closed: false,
            capacity: DEFAULT_CAPACITY,
            last_end: None,
            records_n: 0,
            losses_n: 0,
            journal_overflow_n: 0,
        }
    }
}
pub fn enabled() -> bool {
    OPT_IN.load(Ordering::Acquire)
}

pub fn begin(boundaries: Boundaries, capacity: usize) -> Result<(), CohortError> {
    boundaries.validate()?;
    if capacity == 0 {
        return Err(CohortError::MalformedBoundaries);
    }
    let mut s = state().lock().unwrap();
    if s.finalized {
        return Err(CohortError::AlreadyFinalized);
    }
    if s.boundaries.is_some() {
        return Err(CohortError::AlreadyActive);
    }
    if s.last_end.is_some_and(|end| boundaries.start_mono_ns < end) {
        return Err(CohortError::StaleOrOverlappingBoundaries);
    }
    s.boundaries = Some(boundaries);
    s.capacity = capacity;
    s.available = true;
    s.producers_closed = false;
    OPT_IN.store(true, Ordering::Release);
    Ok(())
}
pub fn begin_default(boundaries: Boundaries) -> Result<(), CohortError> {
    begin(boundaries, DEFAULT_CAPACITY)
}

/// Establish the producer synchronization barrier. The caller must invoke
/// this only after all producer threads/slot generations have stopped
/// publishing starts and terminal outcomes. It never blocks gameplay.
pub fn acknowledge_producers_closed() -> Result<(), CohortError> {
    let mut s = state().lock().unwrap();
    if s.boundaries.is_none() {
        return Err(CohortError::NotActive);
    }
    if s.finalized {
        return Err(CohortError::AlreadyFinalized);
    }
    s.producers_closed = true;
    Ok(())
}
pub fn producers_closed() -> bool {
    state().lock().unwrap().producers_closed
}

fn occupied(s: &State) -> usize {
    s.pending.len().saturating_add(s.journal.len())
}

fn append(s: &mut State, entry: Entry) -> bool {
    // Shared bound: pending identities + undrained journal entries together.
    // Never retain a receipt that would push occupied memory past capacity.
    if occupied(s) >= s.capacity {
        s.available = false;
        s.journal_overflow_n = s.journal_overflow_n.saturating_add(1);
        return false;
    }
    let journal_sequence = s.next_journal_sequence;
    s.next_journal_sequence = journal_sequence.wrapping_add(1);
    s.journal.push_back(JournalEntry {
        journal_sequence,
        entry,
    });
    true
}
fn loss(s: &mut State, r: LossReceipt) -> bool {
    s.losses_n = s.losses_n.saturating_add(1);
    let appended = append(s, Entry::Loss(r));
    s.available = false;
    appended
}

fn capacity_overflow(s: &mut State) {
    s.losses_n = s.losses_n.saturating_add(1);
    s.journal_overflow_n = s.journal_overflow_n.saturating_add(1);
    s.available = false;
}

pub fn start(
    slot_id: u64,
    generation: u64,
    start_mono_ns: u64,
    surface: Surface,
) -> Option<EventId> {
    if !enabled() {
        return None;
    }
    let mut s = state().lock().unwrap();
    let b = s.boundaries?;
    if s.finalized || s.producers_closed {
        if start_mono_ns >= b.start_mono_ns && start_mono_ns < b.end_mono_ns {
            let seq = s.next_sequence;
            s.next_sequence = seq.wrapping_add(1);
            loss(
                &mut s,
                LossReceipt {
                    sequence: seq,
                    slot_id,
                    generation,
                    start_mono_ns: Some(start_mono_ns),
                    surface,
                    outcome: Outcome::Dropped,
                    reason: LossReason::LateEvent,
                },
            );
        }
        return None;
    }
    if start_mono_ns < b.start_mono_ns || start_mono_ns >= b.end_mono_ns {
        return None;
    }
    if occupied(&s) >= s.capacity {
        s.next_sequence = s.next_sequence.wrapping_add(1);
        capacity_overflow(&mut s);
        return None;
    }
    let id = EventId {
        slot_id,
        generation,
        sequence: s.next_sequence,
        start_mono_ns,
        surface,
    };
    s.next_sequence = s.next_sequence.wrapping_add(1);
    s.pending.push_back(Pending { id });
    Some(id)
}
pub fn dropped_start(slot_id: u64, generation: u64, start_mono_ns: u64, surface: Surface) {
    if !enabled() {
        return;
    }
    let mut s = state().lock().unwrap();
    let Some(b) = s.boundaries else { return };
    if start_mono_ns < b.start_mono_ns || start_mono_ns >= b.end_mono_ns {
        return;
    }
    let seq = s.next_sequence;
    s.next_sequence = seq.wrapping_add(1);
    // When the shared pending+journal bound is already full, count the drop as
    // an overflow gap without retaining an extra journal slot.
    if occupied(&s) >= s.capacity {
        capacity_overflow(&mut s);
        return;
    }
    let reason = if s.finalized || s.producers_closed {
        LossReason::LateEvent
    } else {
        LossReason::Capacity
    };
    loss(
        &mut s,
        LossReceipt {
            sequence: seq,
            slot_id,
            generation,
            start_mono_ns: Some(start_mono_ns),
            surface,
            outcome: Outcome::Dropped,
            reason,
        },
    );
}
fn terminal(id: EventId, complete: Option<u64>, outcome: Outcome) {
    let mut s = state().lock().unwrap();
    let Some(pos) = s.pending.iter().position(|p| p.id == id) else {
        let reason = if s.finalized {
            LossReason::LateEvent
        } else {
            LossReason::MissingIdentity
        };
        loss(
            &mut s,
            LossReceipt {
                sequence: id.sequence,
                slot_id: id.slot_id,
                generation: id.generation,
                start_mono_ns: Some(id.start_mono_ns),
                surface: id.surface,
                outcome,
                reason,
            },
        );
        return;
    };
    let pending = s.pending.remove(pos).unwrap();
    if outcome != Outcome::Completed {
        s.available = false;
    }
    if let Some(end) = complete {
        let Some(b) = s.boundaries else { return };
        if end < pending.id.start_mono_ns {
            loss(
                &mut s,
                LossReceipt {
                    sequence: id.sequence,
                    slot_id: id.slot_id,
                    generation: id.generation,
                    start_mono_ns: Some(id.start_mono_ns),
                    surface: id.surface,
                    outcome,
                    reason: LossReason::MalformedTimestamp,
                },
            );
            return;
        }
        if end > b.end_mono_ns.saturating_add(b.tail_ns) {
            loss(
                &mut s,
                LossReceipt {
                    sequence: id.sequence,
                    slot_id: id.slot_id,
                    generation: id.generation,
                    start_mono_ns: Some(id.start_mono_ns),
                    surface: id.surface,
                    outcome,
                    reason: LossReason::LateEvent,
                },
            );
            return;
        }
    }
    if append(
        &mut s,
        Entry::Record(EventRecord {
            id: pending.id,
            complete_mono_ns: complete,
            outcome,
        }),
    ) {
        s.records_n = s.records_n.saturating_add(1);
    } else {
        s.available = false;
    }
}
pub fn complete(id: EventId, at: u64) {
    if enabled() {
        terminal(id, Some(at), Outcome::Completed);
    }
}
pub fn cancel(id: EventId) {
    if enabled() {
        terminal(id, None, Outcome::Canceled);
    }
}
pub fn lost(id: EventId) {
    if enabled() {
        terminal(id, None, Outcome::Lost);
    }
}
pub fn dropped(id: EventId) {
    if enabled() {
        terminal(id, None, Outcome::Dropped);
    }
}
pub fn generation_lost(id: EventId, generation: u64) {
    if generation == id.generation {
        lost(id)
    } else if enabled() {
        let mut s = state().lock().unwrap();
        if let Some(pos) = s.pending.iter().position(|p| p.id == id) {
            let _ = s.pending.remove(pos);
        }
        let seq = id.sequence;
        loss(
            &mut s,
            LossReceipt {
                sequence: seq,
                slot_id: id.slot_id,
                generation,
                start_mono_ns: Some(id.start_mono_ns),
                surface: id.surface,
                outcome: Outcome::Lost,
                reason: LossReason::GenerationMismatch,
            },
        );
    }
}

/// Drain journal entries after `cursor`; extraction frees bounded storage.
pub fn extract_since(cursor: u64) -> Result<CohortBatch, CohortError> {
    let mut s = state().lock().unwrap();
    let b = s.boundaries.ok_or(CohortError::NotActive)?;
    // Extraction is an acknowledgement protocol: only the currently exposed
    // floor may be acknowledged.  Accepting a future cursor would drain and
    // silently discard entries the reader has never observed.
    if cursor != s.cursor_floor {
        return Err(CohortError::InvalidCursor);
    }
    let entries: Vec<_> = s.journal.drain(..).collect();
    let mut records = Vec::new();
    let mut losses = Vec::new();
    let mut next = cursor;
    for entry in entries {
        let sequence = entry.journal_sequence;
        if sequence > cursor {
            match entry.entry {
                Entry::Record(record) => records.push(record),
                Entry::Loss(receipt) => losses.push(receipt),
            }
            next = next.max(sequence);
        }
    }
    s.cursor_floor = s.cursor_floor.max(next);
    Ok(CohortBatch {
        schema_version: SCHEMA_VERSION,
        boundaries: b,
        records,
        losses,
        next_cursor: next,
        complete: s.finalized,
        loss_count: s.losses_n,
        journal_overflow_n: s.journal_overflow_n,
    })
}

pub fn finalize(now_mono_ns: u64) -> Result<TerminalSummary, CohortError> {
    let mut s = state().lock().unwrap();
    let b = s.boundaries.ok_or(CohortError::NotActive)?;
    if s.finalized {
        return Err(CohortError::AlreadyFinalized);
    }
    if now_mono_ns < b.end_mono_ns.saturating_add(b.tail_ns) || !s.producers_closed {
        return Err(if !s.producers_closed {
            CohortError::ProducersNotClosed
        } else {
            CohortError::Terminal
        });
    }
    let pending: Vec<_> = s.pending.drain(..).collect();
    for p in pending {
        loss(
            &mut s,
            LossReceipt {
                sequence: p.id.sequence,
                slot_id: p.id.slot_id,
                generation: p.id.generation,
                start_mono_ns: Some(p.id.start_mono_ns),
                surface: p.id.surface,
                outcome: Outcome::Lost,
                reason: LossReason::Incomplete,
            },
        );
    }
    s.finalized = true;
    s.last_end = Some(b.end_mono_ns);
    Ok(TerminalSummary {
        schema_version: SCHEMA_VERSION,
        boundaries: b,
        terminal: true,
        available: s.available && s.pending.is_empty(),
        records_n: s.records_n,
        losses_n: s.losses_n,
        pending_n: s.pending.len() as u64,
    })
}

#[cfg(test)]
pub(crate) fn test_reset() {
    *state().lock().unwrap() = State::default();
    OPT_IN.store(false, Ordering::Release);
}

/// Test-only occupied count for capacity regressions.
#[cfg(test)]
fn test_occupied() -> usize {
    let s = state().lock().unwrap();
    occupied(&s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn isolate() -> std::sync::MutexGuard<'static, ()> {
        let g = crate::responsiveness_profile::tests::lock_tests();
        // lock_tests already clears profile queues and cohort via test_reset.
        g
    }

    // Clear decode pending without going through lost accounting after test body.
    fn silence_local_drop(local: &mut crate::responsiveness_profile::Local) {
        // Use public cancel path until empty, then drop is quiet on decode side.
        // Local::decode_pending is private — cancel repeatedly via note_script_canceled.
        for _ in 0..64 {
            local.note_script_canceled();
        }
    }

    #[test]
    fn window_tail_and_barrier() {
        let _g = isolate();
        let b = Boundaries {
            start_mono_ns: 100,
            end_mono_ns: 200,
            tail_ns: 50,
        };
        begin(b, 8).unwrap();
        assert!(start(1, 2, 99, Surface::Decode).is_none()); // pre-start
        assert!(start(1, 2, 200, Surface::Decode).is_none()); // post-end (half-open)
        assert!(start(1, 2, 250, Surface::Decode).is_none()); // post-end
        let id = start(1, 2, 150, Surface::Decode).unwrap();
        complete(id, 240); // completion in tail
        assert!(finalize(250).is_err());
        acknowledge_producers_closed().unwrap();
        assert_eq!(finalize(250).unwrap().records_n, 1);
        test_reset();
    }

    #[test]
    fn extraction_drains_and_capacity_loss_is_bounded() {
        let _g = isolate();
        begin(
            Boundaries {
                start_mono_ns: 1,
                end_mono_ns: 10,
                tail_ns: 2,
            },
            1,
        )
        .unwrap();
        let id = start(1, 1, 2, Surface::Panel).unwrap();
        assert!(start(1, 1, 3, Surface::Panel).is_none());
        assert!(test_occupied() <= 1);
        let a = extract_since(0).unwrap();
        assert!(a.losses.is_empty());
        assert_eq!(a.journal_overflow_n, 1);
        lost(id);
        let b = extract_since(a.next_cursor).unwrap();
        assert_eq!(b.records.len(), 1);
        assert_eq!(b.loss_count, 1);
        assert_eq!(b.journal_overflow_n, 1);
        test_reset();
    }

    #[test]
    fn drop_before_admit_at_full_pending_does_not_exceed_capacity() {
        let _g = isolate();
        begin(
            Boundaries {
                start_mono_ns: 1,
                end_mono_ns: 100,
                tail_ns: 2,
            },
            1,
        )
        .unwrap();
        let id = start(1, 1, 2, Surface::Decode).unwrap();
        assert_eq!(test_occupied(), 1);
        // Pending holds the only slot; drop-before-admit must not retain a journal entry.
        dropped_start(9, 1, 3, Surface::Decode);
        assert!(test_occupied() <= 1);
        let batch = extract_since(0).unwrap();
        assert!(batch.records.is_empty());
        assert!(batch.losses.is_empty());
        assert!(batch.journal_overflow_n >= 1);
        lost(id);
        let batch = extract_since(batch.next_cursor).unwrap();
        assert_eq!(batch.records.len(), 1);
        test_reset();
    }

    #[test]
    fn extraction_does_not_wedge_on_long_pending_first_event() {
        let _g = isolate();
        begin(
            Boundaries {
                start_mono_ns: 1,
                end_mono_ns: 10,
                tail_ns: 2,
            },
            8,
        )
        .unwrap();
        let first = start(1, 1, 2, Surface::Decode).unwrap();
        let second = start(1, 1, 3, Surface::Decode).unwrap();
        complete(second, 4);
        let batch = extract_since(0).unwrap();
        assert_eq!(batch.records.len(), 1);
        assert_eq!(batch.records[0].id.sequence, second.sequence);
        complete(first, 5);
        let batch = extract_since(batch.next_cursor).unwrap();
        assert_eq!(batch.records.len(), 1);
        assert_eq!(batch.records[0].id.sequence, first.sequence);
        test_reset();
    }

    #[test]
    fn incomplete_and_noncompleted_fail_closed() {
        let _g = isolate();
        begin(
            Boundaries {
                start_mono_ns: 1,
                end_mono_ns: 10,
                tail_ns: 2,
            },
            8,
        )
        .unwrap();
        let id = start(1, 1, 2, Surface::Panel).unwrap();
        cancel(id);
        acknowledge_producers_closed().unwrap();
        let s = finalize(12).unwrap();
        assert!(!s.available);
        test_reset();
    }

    #[test]
    fn forged_cursor_preserves_unacknowledged_journal() {
        let _g = isolate();
        begin(
            Boundaries {
                start_mono_ns: 1,
                end_mono_ns: 10,
                tail_ns: 2,
            },
            8,
        )
        .unwrap();
        let id = start(1, 1, 2, Surface::Decode).unwrap();
        complete(id, 3);
        assert_eq!(extract_since(1), Err(CohortError::InvalidCursor));
        let batch = extract_since(0).unwrap();
        assert_eq!(batch.records.len(), 1);
        assert_eq!(batch.records[0].id.sequence, id.sequence);
        test_reset();
    }

    #[test]
    fn disabled_legacy_entrypoints_do_not_mutate_cohort() {
        let _g = isolate();
        assert!(!enabled());
        crate::responsiveness_profile::enable();
        let t0 = std::time::Instant::now();
        let mut local = crate::responsiveness_profile::Local::new(0xd150).unwrap();
        local.note_decode_edge(t0);
        local.note_script_dispatch(t0 + Duration::from_millis(1));
        crate::responsiveness_profile::set_input_surface(
            crate::responsiveness_profile::InputSurface::Tui,
        );
        assert!(crate::responsiveness_profile::note_input_start(
            local.slot_id(),
            t0,
            0
        ));
        crate::responsiveness_profile::note_tui_draw_flush(
            local.slot_id(),
            t0 + Duration::from_millis(2),
        );
        // Cohort never began: still disabled, extract refuses, no journal growth possible.
        assert!(!enabled());
        assert_eq!(extract_since(0), Err(CohortError::NotActive));
        // Starting a fresh cohort after disabled traffic must see a clean sequence space.
        let start_ns = crate::responsiveness_profile::mono_ns(t0);
        begin(
            Boundaries {
                start_mono_ns: start_ns + 10_000_000,
                end_mono_ns: start_ns + 20_000_000,
                tail_ns: 1_000_000,
            },
            8,
        )
        .unwrap();
        let batch = extract_since(0).unwrap();
        assert!(batch.records.is_empty());
        assert!(batch.losses.is_empty());
        assert_eq!(batch.loss_count, 0);
        silence_local_drop(&mut local);
        test_reset();
    }

    #[test]
    fn existing_decode_entrypoints_publish_cohort_identity() {
        let _g = isolate();
        let start_at = std::time::Instant::now();
        let start_ns = crate::responsiveness_profile::mono_ns(start_at);
        begin(
            Boundaries {
                start_mono_ns: start_ns.saturating_sub(1),
                end_mono_ns: start_ns + 1_000_000_000,
                tail_ns: 50_000_000,
            },
            8,
        )
        .unwrap();
        crate::responsiveness_profile::enable();
        let mut local = crate::responsiveness_profile::Local::new(0xabc).unwrap();
        local.note_decode_edge(start_at);
        local.note_script_dispatch(start_at + Duration::from_millis(1));
        let batch = extract_since(0).unwrap();
        assert_eq!(batch.records.len(), 1);
        assert_eq!(batch.records[0].id.surface, Surface::Decode);
        assert_eq!(batch.records[0].outcome, Outcome::Completed);
        silence_local_drop(&mut local);
        test_reset();
    }

    #[test]
    fn actual_input_tui_complete_and_cancel_entrypoints() {
        let _g = isolate();
        crate::responsiveness_profile::enable();
        let t0 = std::time::Instant::now();
        let start_ns = crate::responsiveness_profile::mono_ns(t0);
        begin(
            Boundaries {
                start_mono_ns: start_ns.saturating_sub(1),
                end_mono_ns: start_ns + 1_000_000_000,
                tail_ns: 50_000_000,
            },
            8,
        )
        .unwrap();
        crate::responsiveness_profile::set_input_surface(
            crate::responsiveness_profile::InputSurface::Tui,
        );
        let mut local = crate::responsiveness_profile::Local::new(0xdef).unwrap();
        assert!(crate::responsiveness_profile::note_input_start(
            local.slot_id(),
            t0,
            0
        ));
        crate::responsiveness_profile::note_tui_draw_flush(
            local.slot_id(),
            t0 + Duration::from_millis(1),
        );
        let batch = extract_since(0).unwrap();
        assert_eq!(batch.records.len(), 1);
        assert_eq!(batch.records[0].id.surface, Surface::Tui);
        assert_eq!(batch.records[0].outcome, Outcome::Completed);

        // Cancel path on a second start.
        let t1 = std::time::Instant::now();
        assert!(crate::responsiveness_profile::note_input_start(
            local.slot_id(),
            t1,
            0
        ));
        crate::responsiveness_profile::note_input_canceled(local.slot_id());
        let batch = extract_since(batch.next_cursor).unwrap();
        assert_eq!(batch.records.len(), 1);
        assert_eq!(batch.records[0].outcome, Outcome::Canceled);
        assert_eq!(batch.records[0].id.surface, Surface::Tui);
        silence_local_drop(&mut local);
        test_reset();
    }

    #[test]
    fn actual_panel_present_entrypoint_records_surface() {
        let _g = isolate();
        crate::responsiveness_profile::enable();
        let t0 = std::time::Instant::now();
        let start_ns = crate::responsiveness_profile::mono_ns(t0);
        begin(
            Boundaries {
                start_mono_ns: start_ns.saturating_sub(1),
                end_mono_ns: start_ns + 1_000_000_000,
                tail_ns: 50_000_000,
            },
            8,
        )
        .unwrap();
        crate::responsiveness_profile::set_input_surface(
            crate::responsiveness_profile::InputSurface::Panel,
        );
        let mut local = crate::responsiveness_profile::Local::new(0xbabe1).unwrap();
        let sid = local.slot_id();
        assert!(crate::responsiveness_profile::note_input_start(sid, t0, 0));
        crate::responsiveness_profile::bind_input_to_mailbox_gen(sid, 7);
        crate::responsiveness_profile::note_panel_present(sid, 7, t0 + Duration::from_millis(40));
        let batch = extract_since(0).unwrap();
        assert_eq!(batch.records.len(), 1);
        assert_eq!(batch.records[0].id.surface, Surface::Panel);
        assert_eq!(batch.records[0].outcome, Outcome::Completed);
        silence_local_drop(&mut local);
        test_reset();
    }

    #[test]
    fn local_drop_teardown_publishes_lost_via_actual_drop() {
        let _g = isolate();
        crate::responsiveness_profile::enable();
        let t0 = std::time::Instant::now();
        let start_ns = crate::responsiveness_profile::mono_ns(t0);
        begin(
            Boundaries {
                start_mono_ns: start_ns.saturating_sub(1),
                end_mono_ns: start_ns + 1_000_000_000,
                tail_ns: 50_000_000,
            },
            8,
        )
        .unwrap();
        {
            let mut local = crate::responsiveness_profile::Local::new(0xd70b).unwrap();
            local.note_decode_edge(t0);
            crate::responsiveness_profile::set_input_surface(
                crate::responsiveness_profile::InputSurface::Tui,
            );
            assert!(crate::responsiveness_profile::note_input_start(
                local.slot_id(),
                t0,
                0
            ));
            // Drop without complete — decode + input lost via Drop wiring.
        }
        let batch = extract_since(0).unwrap();
        assert_eq!(batch.records.len(), 2);
        assert!(batch.records.iter().all(|r| r.outcome == Outcome::Lost));
        let surfaces: Vec<_> = batch.records.iter().map(|r| r.id.surface).collect();
        assert!(surfaces.contains(&Surface::Decode));
        assert!(surfaces.contains(&Surface::Tui));
        test_reset();
    }

    #[test]
    fn finalize_race_after_barrier_records_late_producer_start() {
        let _g = isolate();
        crate::responsiveness_profile::enable();
        let t0 = std::time::Instant::now();
        let start_ns = crate::responsiveness_profile::mono_ns(t0);
        let end_ns = start_ns + 1_000_000_000;
        let tail_ns = 1_000_000;
        begin(
            Boundaries {
                start_mono_ns: start_ns.saturating_sub(1),
                end_mono_ns: end_ns,
                tail_ns,
            },
            8,
        )
        .unwrap();
        let mut local = crate::responsiveness_profile::Local::new(0xface).unwrap();
        // Barrier closed while a producer could still publish with in-window time.
        acknowledge_producers_closed().unwrap();
        // Actual decode entrypoint after barrier: in-window start must be LateEvent, not silent.
        local.note_decode_edge(t0);
        let batch = extract_since(0).unwrap();
        assert_eq!(batch.records.len(), 0);
        assert_eq!(batch.losses.len(), 1);
        assert_eq!(batch.losses[0].reason, LossReason::LateEvent);
        assert_eq!(batch.losses[0].surface, Surface::Decode);
        // Finalize still requires tail time; after tail, unavailable due to late loss.
        let now = end_ns + tail_ns;
        let summary = finalize(now).unwrap();
        assert!(!summary.available);
        silence_local_drop(&mut local);
        test_reset();
    }

    #[test]
    fn missing_generation_input_admits_legacy_without_cohort_identity() {
        let _g = isolate();
        crate::responsiveness_profile::enable();
        let t0 = std::time::Instant::now();
        let start_ns = crate::responsiveness_profile::mono_ns(t0);
        begin(
            Boundaries {
                start_mono_ns: start_ns.saturating_sub(1),
                end_mono_ns: start_ns + 1_000_000_000,
                tail_ns: 50_000_000,
            },
            8,
        )
        .unwrap();
        crate::responsiveness_profile::set_input_surface(
            crate::responsiveness_profile::InputSurface::Tui,
        );
        // No Local/registry row for this slot → live_generation_for is None.
        let orphan = 0x0f_fa_u64;
        assert!(crate::responsiveness_profile::note_input_start(
            orphan, t0, 0
        ));
        crate::responsiveness_profile::note_tui_draw_flush(orphan, t0 + Duration::from_millis(1));
        let batch = extract_since(0).unwrap();
        assert!(batch.records.is_empty());
        assert!(batch.losses.is_empty());
        test_reset();
    }

    #[test]
    fn post_end_start_excluded_on_real_decode_entrypoint() {
        let _g = isolate();
        crate::responsiveness_profile::enable();
        let t0 = std::time::Instant::now();
        // End at the mono of t0 (at least 1). A start stamped 1ms later is post-end.
        let end_ns = crate::responsiveness_profile::mono_ns(t0).max(1);
        begin(
            Boundaries {
                start_mono_ns: 0,
                end_mono_ns: end_ns,
                tail_ns: 50_000_000,
            },
            8,
        )
        .unwrap();
        let mut local = crate::responsiveness_profile::Local::new(0xb057).unwrap();
        let later = t0 + Duration::from_millis(1);
        local.note_decode_edge(later); // start_mono >= end → excluded
        local.note_script_dispatch(later + Duration::from_millis(1));
        let batch = extract_since(0).unwrap();
        assert!(batch.records.is_empty());
        assert!(batch.losses.is_empty());
        silence_local_drop(&mut local);
        test_reset();
    }
}
