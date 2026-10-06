//! The last-seen whole bank of one account (design-bank-snapshot §1.3–§1.5).
//!
//! The host owns one memory per account and fills it from the open bank
//! through [`BankMemory::track`]; the hint file (`script::bank_hints`)
//! fills it at login through [`BankMemory::load_hint`]. Rows are
//! `(obj id, count)` pairs sorted by id, unique, unnoted, every count
//! positive, so a count is a binary search and never a copy. A memory that
//! was never filled is `Unknown`: `count` answers `None`, never an observed
//! zero. A loaded **empty** bank is `Session` with every count `Some(0)`.

use crate::snapshot::{GameSnapshot, ItemView};

/// The most rows a persisted hint may carry; a longer file is rejected and
/// the memory stays `Unknown` (design-bank-snapshot §1.4).
pub const MAX_HINT_ROWS: usize = 1024;

/// Where the rows came from (design-bank-snapshot §2.4 keys every negative
/// decision on this).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Origin {
    /// Never observed in this process and no hint loaded.
    #[default]
    Unknown,
    /// Loaded from the persisted hint, or carried across a relog: advisory.
    Hint,
    /// Observed at an open bank this login: final.
    Session,
}

/// When the rows were observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedAt {
    pub unix_secs: u64,
    pub tick: u64,
    pub bank_session: u64,
}

/// Why [`BankMemory::load_hint`] refused a row set (design-bank-snapshot
/// §1.4): the rows must be at most [`MAX_HINT_ROWS`], every count positive,
/// and the ids strictly increasing, or `count`'s binary search is wrong.
/// File-level rejects (schema, identity, unsafe account, io) are
/// `script::bank_hints::HintError`, which wraps this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintRowsError {
    TooManyRows,
    NonPositiveCount,
    Unsorted,
    DuplicateId,
}

impl std::fmt::Display for HintRowsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            HintRowsError::TooManyRows => "more than the row cap",
            HintRowsError::NonPositiveCount => "a count that is not positive",
            HintRowsError::Unsorted => "ids out of order",
            HintRowsError::DuplicateId => "a duplicate id",
        })
    }
}

/// What one frame did to the memory ([`BankMemory::track`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameEvent {
    /// No loaded bank moved and no bank closed.
    Unchanged,
    /// The first loaded table of a bank session: the rows were replaced.
    /// The owner logs this one, once per bank session.
    Opened,
    /// The loaded bank's packet generation moved again: the rows follow.
    Observed,
    /// The loaded → not-loaded edge: the bank session ended. The owner
    /// saves the hint now, once per bank session.
    Closed,
}

#[derive(Debug)]
pub struct BankMemory {
    /// Sorted unique ids, unnoted, every count > 0. Capacity is reserved
    /// from the bank's slot count on the first loaded observe and never
    /// shrinks, so a later observe allocates nothing.
    rows: Vec<(i32, i32)>,
    /// `None` while never observed (`Unknown`).
    observed_at: Option<ObservedAt>,
    origin: Origin,
    /// Bumps on every observe, load and relog so a reader can detect change.
    generation: u64,
    /// `bank_snapshot_generation` mirrored while the bank is loaded; `None`
    /// while it is not. The loaded → `None` edge is the close.
    live_generation: Option<u64>,
    /// Rows observed since the last hint save.
    dirty: bool,
}

impl Default for BankMemory {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            observed_at: None,
            origin: Origin::Unknown,
            generation: 0,
            live_generation: None,
            dirty: false,
        }
    }
}

impl BankMemory {
    /// Whether the rows mean anything: `Hint` or `Session`.
    pub fn known(&self) -> bool {
        self.origin != Origin::Unknown
    }

    pub fn origin(&self) -> Origin {
        self.origin
    }

    pub fn observed_at(&self) -> Option<ObservedAt> {
        self.observed_at
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Rows observed since the last hint save.
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// The hint owner saved the rows: nothing is pending until the next
    /// observe.
    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    /// The reserved row capacity (the FLOOR pin: it grows once, on the
    /// first loaded observe, and never on a later one).
    pub fn capacity(&self) -> usize {
        self.rows.capacity()
    }

    /// The stock of `id`: `None` while `Unknown`, `Some(0)` for an id the
    /// known bank does not hold. A note is another id and is never stored.
    pub fn count(&self, id: i32) -> Option<i32> {
        self.known().then(|| {
            self.rows
                .binary_search_by_key(&id, |&(row, _)| row)
                .map_or(0, |index| self.rows[index].1)
        })
    }

    /// Every known row, sorted by id: the planner input, borrowed.
    pub fn rows(&self) -> &[(i32, i32)] {
        &self.rows
    }

    /// Replace the rows with a loaded bank table (design-bank-snapshot
    /// §1.3): the per-slot rows are folded by id (a duplicate id's count
    /// joins the kept row with `saturating_add`, `count <= 0` rows are
    /// dropped), sorted and unique. The capacity is reserved from
    /// `bank_size`, the withdraw component's slot count, so a 3-row first
    /// open never reallocates on a 40-row second one. An empty `rows` is a
    /// loaded empty bank: `Session`, every count `Some(0)`.
    pub fn observe(&mut self, rows: &[ItemView], bank_size: i32, at: ObservedAt) {
        self.rows.clear();
        self.rows.reserve(usize::try_from(bank_size).unwrap_or(0));
        self.rows.extend(
            rows.iter()
                .filter(|row| row.count > 0)
                .map(|row| (row.def.id, row.count)),
        );
        fold_sorted(&mut self.rows);
        self.origin = Origin::Session;
        self.observed_at = Some(at);
        self.generation = self.generation.wrapping_add(1);
        self.dirty = true;
    }

    /// Fill an `Unknown` memory from the persisted hint (design-bank-snapshot
    /// §1.4): `Hint` origin, the file's rows as they are, nothing to save.
    /// The rows must already be sorted, unique and positive, or the file is
    /// refused whole and the memory is left as it was.
    pub fn load_hint(
        &mut self,
        rows: Vec<(i32, i32)>,
        unix_secs: u64,
    ) -> Result<(), HintRowsError> {
        check_hint_rows(&rows)?;
        self.rows = rows;
        self.origin = Origin::Hint;
        self.observed_at = Some(ObservedAt {
            unix_secs,
            tick: 0,
            bank_session: 0,
        });
        self.generation = self.generation.wrapping_add(1);
        self.live_generation = None;
        self.dirty = false;
        Ok(())
    }

    /// A new login of the same account in this process (design-bank-snapshot
    /// §1.4): the rows stay, `dirty` stays (an unsaved session is saved at
    /// the next close or end), and a known memory becomes `Hint` — nothing
    /// proves the bank is unchanged since logout. The first loaded observe
    /// of the new session flips it back to `Session`.
    pub fn relogged(&mut self) {
        self.live_generation = None;
        if self.known() {
            self.origin = Origin::Hint;
            self.generation = self.generation.wrapping_add(1);
        }
    }

    /// Whether [`Self::track`] would change anything on this frame: a loaded
    /// bank whose packet generation moved, or the loaded → closed edge. A
    /// read, so the slot thread takes no write lock on an idle frame.
    pub fn frame_due(&self, snapshot: &GameSnapshot) -> bool {
        if snapshot.bank_loaded() {
            self.live_generation != snapshot.bank_snapshot_generation()
        } else {
            self.live_generation.is_some()
        }
    }

    /// Mirror one frame of the open bank (design-bank-snapshot §1.3): a
    /// loaded table whose packet generation moved replaces the rows, and the
    /// loaded → not-loaded edge is the close the owner saves on. Only a
    /// loaded table is observed — an open bank still transmitting is not.
    pub fn track(&mut self, snapshot: &GameSnapshot, unix_secs: u64) -> FrameEvent {
        if snapshot.bank_loaded() {
            let generation = snapshot.bank_snapshot_generation();
            if self.live_generation == generation {
                return FrameEvent::Unchanged;
            }
            self.observe(
                snapshot.bank(),
                snapshot.bank_size(),
                ObservedAt {
                    unix_secs,
                    tick: u64::from(snapshot.tick()),
                    bank_session: snapshot.bank_session_generation(),
                },
            );
            let first = self.live_generation.is_none();
            self.live_generation = generation;
            if first {
                FrameEvent::Opened
            } else {
                FrameEvent::Observed
            }
        } else if self.live_generation.take().is_some() {
            FrameEvent::Closed
        } else {
            FrameEvent::Unchanged
        }
    }

    /// An offline memory with these rows and origin. The rows are folded the
    /// way the host observes them: sorted, unique, positive counts only.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn seeded(rows: &[(i32, i32)], origin: Origin) -> Self {
        let mut folded: Vec<(i32, i32)> = rows
            .iter()
            .copied()
            .filter(|&(_, count)| count > 0)
            .collect();
        fold_sorted(&mut folded);
        Self {
            rows: folded,
            observed_at: (origin != Origin::Unknown).then_some(ObservedAt {
                unix_secs: 0,
                tick: 0,
                bank_session: 0,
            }),
            origin,
            generation: u64::from(origin != Origin::Unknown),
            live_generation: None,
            dirty: false,
        }
    }
}

/// Sort positive `(id, count)` rows by id and fold duplicate ids into one
/// row, in place: no allocation.
fn fold_sorted(rows: &mut Vec<(i32, i32)>) {
    rows.sort_unstable_by_key(|&(id, _)| id);
    rows.dedup_by(|next, kept| {
        if next.0 == kept.0 {
            kept.1 = kept.1.saturating_add(next.1);
            true
        } else {
            false
        }
    });
}

/// The §1.4 row rules a hint file must already satisfy.
fn check_hint_rows(rows: &[(i32, i32)]) -> Result<(), HintRowsError> {
    if rows.len() > MAX_HINT_ROWS {
        return Err(HintRowsError::TooManyRows);
    }
    if rows.iter().any(|&(_, count)| count <= 0) {
        return Err(HintRowsError::NonPositiveCount);
    }
    for pair in rows.windows(2) {
        match pair[0].0.cmp(&pair[1].0) {
            std::cmp::Ordering::Less => {}
            std::cmp::Ordering::Equal => return Err(HintRowsError::DuplicateId),
            std::cmp::Ordering::Greater => return Err(HintRowsError::Unsorted),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obj_names::ItemDefView;
    use crate::snapshot::{ItemActionFamily, ItemContainer};

    const LOBSTER: i32 = 379;
    const COINS: i32 = 995;
    const BONES: i32 = 526;
    const BANK_COM: i32 = 5292;

    fn bank_row(id: i32, count: i32, slot: i32) -> ItemView {
        ItemView {
            def: ItemDefView {
                id,
                name: Some("fixture".into()),
                stackable: id == COINS,
                members: false,
                base_value: 0,
                noted: false,
                certificate_link: -1,
                certificate_template: -1,
            },
            container: ItemContainer::Bank,
            action_family: ItemActionFamily::Component,
            slot,
            count,
            actions: Vec::new(),
            component_id: BANK_COM,
        }
    }

    fn at(unix_secs: u64) -> ObservedAt {
        ObservedAt {
            unix_secs,
            tick: 7,
            bank_session: 2,
        }
    }

    #[test]
    fn unknown_memory_answers_none_and_known_absent_is_zero() {
        let unknown = BankMemory::default();
        assert!(!unknown.known());
        assert_eq!(unknown.origin(), Origin::Unknown);
        assert_eq!(unknown.count(379), None);
        assert_eq!(unknown.observed_at(), None);
        assert!(unknown.rows().is_empty());
        assert!(!unknown.dirty());

        let seeded = BankMemory::seeded(&[(995, 3_400), (379, 12)], Origin::Session);
        assert!(seeded.known());
        assert_eq!(seeded.origin(), Origin::Session);
        assert_eq!(seeded.count(379), Some(12));
        assert_eq!(seeded.count(995), Some(3_400));
        assert_eq!(
            seeded.count(380),
            Some(0),
            "an absent id in a known bank is zero"
        );
        assert_eq!(
            seeded.rows(),
            &[(379, 12), (995, 3_400)],
            "rows come out sorted"
        );
        assert!(seeded.observed_at().is_some());
        assert_eq!(seeded.generation(), 1);
    }

    #[test]
    fn seeded_folds_duplicate_ids_and_drops_empty_rows() {
        let memory = BankMemory::seeded(&[(379, 5), (379, 7), (995, 0), (1, -3)], Origin::Hint);
        assert_eq!(memory.rows(), &[(379, 12)]);
        assert_eq!(memory.count(995), Some(0));
        assert_eq!(memory.origin(), Origin::Hint);

        let empty = BankMemory::seeded(&[], Origin::Session);
        assert!(empty.known(), "a loaded empty bank is a known empty bank");
        assert_eq!(empty.count(379), Some(0));
    }

    #[test]
    fn memory_is_a_header_not_a_table() {
        assert!(
            std::mem::size_of::<BankMemory>() <= 96,
            "BankMemory is {} bytes",
            std::mem::size_of::<BankMemory>()
        );
    }

    #[test]
    fn observe_replaces_rows_and_folds_slots_by_id() {
        let mut memory = BankMemory::default();
        memory.observe(
            &[bank_row(COINS, 50, 0), bank_row(LOBSTER, 3, 1)],
            64,
            at(10),
        );
        assert_eq!(memory.origin(), Origin::Session);
        assert_eq!(memory.rows(), &[(LOBSTER, 3), (COINS, 50)]);
        assert_eq!(memory.observed_at(), Some(at(10)));
        assert_eq!(memory.generation(), 1);
        assert!(memory.dirty());

        // Two slots of one id fold into one row; an empty or negative slot is
        // dropped; ids come out sorted and unique.
        memory.observe(
            &[
                bank_row(COINS, i32::MAX, 0),
                bank_row(BONES, 0, 1),
                bank_row(LOBSTER, 4, 2),
                bank_row(COINS, 5, 3),
                bank_row(BONES, -1, 4),
                bank_row(LOBSTER, 2, 5),
            ],
            64,
            at(11),
        );
        assert_eq!(memory.rows(), &[(LOBSTER, 6), (COINS, i32::MAX)]);
        assert_eq!(memory.count(BONES), Some(0));
        assert_eq!(memory.count(LOBSTER), Some(6));
        assert_eq!(memory.generation(), 2, "every observe bumps the generation");
        assert_eq!(memory.observed_at(), Some(at(11)));
    }

    #[test]
    fn a_loaded_empty_table_is_a_known_empty_bank() {
        let mut memory = BankMemory::default();
        memory.observe(&[], 64, at(1));
        assert!(memory.known());
        assert_eq!(memory.origin(), Origin::Session);
        assert!(memory.rows().is_empty());
        assert_eq!(memory.count(LOBSTER), Some(0));
        assert!(memory.dirty());
    }

    #[test]
    fn observe_reserves_the_bank_size_once_and_never_reallocates() {
        let mut memory = BankMemory::default();
        let three: Vec<ItemView> = (0..3).map(|slot| bank_row(1 + slot, 1, slot)).collect();
        memory.observe(&three, 64, at(1));
        let reserved = memory.capacity();
        assert!(reserved >= 64, "the reserve is the slot count: {reserved}");

        let forty: Vec<ItemView> = (0..40).map(|slot| bank_row(100 + slot, 2, slot)).collect();
        let second = allocation_counter::measure(|| memory.observe(&forty, 64, at(2)));
        assert_eq!(
            second.bytes_total, 0,
            "the second observe allocates nothing"
        );
        assert_eq!(second.count_total, 0);
        assert_eq!(memory.capacity(), reserved);
        assert_eq!(memory.rows().len(), 40);

        // The fold itself (sort + dedup over the reserved buffer) is in place.
        let duplicates: Vec<ItemView> = (0..40).map(|slot| bank_row(7, 1, slot)).collect();
        let folded = allocation_counter::measure(|| memory.observe(&duplicates, 64, at(3)));
        assert_eq!(folded.bytes_total, 0);
        assert_eq!(memory.rows(), &[(7, 40)]);
    }

    #[test]
    fn load_hint_fills_an_unknown_memory_without_marking_it_dirty() {
        let mut memory = BankMemory::default();
        memory
            .load_hint(vec![(LOBSTER, 12), (COINS, 3_400)], 1_759_700_000)
            .unwrap();
        assert_eq!(memory.origin(), Origin::Hint);
        assert!(memory.known());
        assert_eq!(memory.count(LOBSTER), Some(12));
        assert_eq!(memory.count(BONES), Some(0));
        assert_eq!(
            memory.observed_at(),
            Some(ObservedAt {
                unix_secs: 1_759_700_000,
                tick: 0,
                bank_session: 0
            })
        );
        assert_eq!(memory.generation(), 1);
        assert!(!memory.dirty(), "a loaded file has nothing to save");
    }

    #[test]
    fn load_hint_rejects_every_row_rule_and_leaves_the_memory_unknown() {
        let too_many: Vec<(i32, i32)> = (0..=MAX_HINT_ROWS as i32).map(|id| (id, 1)).collect();
        let cases: [(Vec<(i32, i32)>, HintRowsError); 5] = [
            (too_many, HintRowsError::TooManyRows),
            (vec![(LOBSTER, 0)], HintRowsError::NonPositiveCount),
            (
                vec![(LOBSTER, 1), (COINS, -4)],
                HintRowsError::NonPositiveCount,
            ),
            (vec![(COINS, 1), (LOBSTER, 1)], HintRowsError::Unsorted),
            (vec![(LOBSTER, 1), (LOBSTER, 2)], HintRowsError::DuplicateId),
        ];
        for (rows, expected) in cases {
            let mut memory = BankMemory::default();
            assert_eq!(memory.load_hint(rows, 5), Err(expected));
            assert!(!memory.known(), "{expected:?} leaves the memory Unknown");
            assert_eq!(memory.count(LOBSTER), None);
            assert_eq!(memory.generation(), 0);
        }
        let mut exact = BankMemory::default();
        let cap: Vec<(i32, i32)> = (0..MAX_HINT_ROWS as i32).map(|id| (id, 1)).collect();
        assert_eq!(exact.load_hint(cap, 5), Ok(()));
        assert_eq!(exact.rows().len(), MAX_HINT_ROWS);
    }

    #[test]
    fn relogged_keeps_rows_and_dirty_and_demotes_session_to_hint() {
        let mut memory = BankMemory::default();
        memory.observe(&[bank_row(COINS, 50, 0)], 64, at(1));
        let generation = memory.generation();
        memory.relogged();
        assert_eq!(memory.origin(), Origin::Hint);
        assert_eq!(memory.rows(), &[(COINS, 50)]);
        assert!(memory.dirty(), "an unsaved session is still pending");
        assert_eq!(memory.observed_at(), Some(at(1)));
        assert!(
            memory.generation() > generation,
            "the origin change is visible"
        );

        let mut unknown = BankMemory::default();
        unknown.relogged();
        assert_eq!(unknown.origin(), Origin::Unknown);
        assert_eq!(unknown.generation(), 0);

        let mut saved = BankMemory::default();
        saved.observe(&[bank_row(COINS, 50, 0)], 64, at(1));
        saved.mark_saved();
        saved.relogged();
        assert!(!saved.dirty());
        assert_eq!(saved.origin(), Origin::Hint);
    }

    #[test]
    fn track_observes_a_loaded_bank_once_per_packet_and_closes_once() {
        let mut snapshot = GameSnapshot::new();
        let mut memory = BankMemory::default();
        assert!(!memory.frame_due(&snapshot));
        assert_eq!(memory.track(&snapshot, 1), FrameEvent::Unchanged);
        assert!(!memory.known(), "a closed bank is never observed");

        // Open but not loaded: the table is unread, not empty.
        snapshot.seed_bank_observation(BANK_COM, 3, None, Vec::new());
        assert!(!memory.frame_due(&snapshot));
        assert_eq!(memory.track(&snapshot, 2), FrameEvent::Unchanged);
        assert!(!memory.known());

        snapshot.seed_bank_observation(BANK_COM, 3, Some(vec![bank_row(COINS, 50, 0)]), Vec::new());
        assert!(memory.frame_due(&snapshot));
        assert_eq!(memory.track(&snapshot, 3), FrameEvent::Opened);
        assert_eq!(memory.origin(), Origin::Session);
        assert_eq!(memory.rows(), &[(COINS, 50)]);
        assert_eq!(memory.observed_at().map(|at| at.unix_secs), Some(3));
        assert!(
            memory.capacity() >= 64,
            "reserved from the fixture bank size"
        );

        // Same packet generation: an idle frame.
        assert!(!memory.frame_due(&snapshot));
        assert_eq!(memory.track(&snapshot, 4), FrameEvent::Unchanged);
        assert_eq!(memory.generation(), 1);

        // A withdraw moves the packet generation: the memory follows.
        snapshot.seed_bank_observation(BANK_COM, 4, Some(vec![bank_row(COINS, 43, 0)]), Vec::new());
        assert!(memory.frame_due(&snapshot));
        assert_eq!(memory.track(&snapshot, 5), FrameEvent::Observed);
        assert_eq!(memory.rows(), &[(COINS, 43)]);
        assert_eq!(memory.generation(), 2);

        // Close: exactly one Closed, then idle.
        snapshot.seed_bank_observation(-1, 4, None, Vec::new());
        assert!(memory.frame_due(&snapshot));
        assert_eq!(memory.track(&snapshot, 6), FrameEvent::Closed);
        assert!(!memory.frame_due(&snapshot));
        assert_eq!(memory.track(&snapshot, 7), FrameEvent::Unchanged);
        assert_eq!(
            memory.origin(),
            Origin::Session,
            "the rows outlive the open bank"
        );
        assert_eq!(memory.rows(), &[(COINS, 43)]);

        // A reopen at the same packet generation is a new session: observed.
        snapshot.seed_bank_observation(BANK_COM, 4, Some(vec![bank_row(COINS, 43, 0)]), Vec::new());
        assert_eq!(memory.track(&snapshot, 8), FrameEvent::Opened);
    }

    #[test]
    fn idle_frames_allocate_nothing_on_the_memory_path() {
        let mut snapshot = GameSnapshot::new();
        let mut memory = BankMemory::default();
        snapshot.seed_bank_observation(BANK_COM, 3, Some(vec![bank_row(COINS, 50, 0)]), Vec::new());
        assert_eq!(memory.track(&snapshot, 1), FrameEvent::Opened);
        let open_idle = allocation_counter::measure(|| {
            for frame in 0..1_000u64 {
                assert!(!memory.frame_due(&snapshot));
                assert_eq!(memory.track(&snapshot, frame), FrameEvent::Unchanged);
            }
        });
        assert_eq!(open_idle.bytes_total, 0);
        snapshot.seed_bank_observation(-1, 3, None, Vec::new());
        assert_eq!(memory.track(&snapshot, 2), FrameEvent::Closed);
        let closed_idle = allocation_counter::measure(|| {
            for frame in 0..1_000u64 {
                assert!(!memory.frame_due(&snapshot));
                assert_eq!(memory.track(&snapshot, frame), FrameEvent::Unchanged);
            }
        });
        assert_eq!(closed_idle.bytes_total, 0);
    }
}
