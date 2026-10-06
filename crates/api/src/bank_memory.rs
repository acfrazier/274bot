//! The last-seen whole bank of one account (design-bank-snapshot §1.5).
//!
//! The host owns one memory per account and fills it from the open bank;
//! this module is the type alone. Rows are `(obj id, count)` pairs sorted
//! by id, unique, unnoted, every count positive, so a count is a binary
//! search and never a copy. A memory that was never filled is `Unknown`:
//! `count` answers `None`, never an observed zero.

/// Where the rows came from (design-bank-snapshot §2.4 keys every negative
/// decision on this).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Never observed in this process and no hint loaded.
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

#[derive(Debug)]
pub struct BankMemory {
    /// Sorted unique ids, unnoted, every count > 0.
    rows: Vec<(i32, i32)>,
    /// `None` while never observed (`Unknown`).
    observed_at: Option<ObservedAt>,
    origin: Origin,
    /// Bumps on every fill so a reader can detect change.
    generation: u64,
}

impl Default for BankMemory {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            observed_at: None,
            origin: Origin::Unknown,
            generation: 0,
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

    /// An offline memory with these rows and origin. The rows are folded the
    /// way the host observes them: sorted, unique, positive counts only.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn seeded(rows: &[(i32, i32)], origin: Origin) -> Self {
        let mut folded: Vec<(i32, i32)> = rows
            .iter()
            .copied()
            .filter(|&(_, count)| count > 0)
            .collect();
        folded.sort_unstable_by_key(|&(id, _)| id);
        folded.dedup_by(|next, kept| {
            if next.0 == kept.0 {
                kept.1 = kept.1.saturating_add(next.1);
                true
            } else {
                false
            }
        });
        Self {
            rows: folded,
            observed_at: (origin != Origin::Unknown).then_some(ObservedAt {
                unix_secs: 0,
                tick: 0,
                bank_session: 0,
            }),
            origin,
            generation: u64::from(origin != Origin::Unknown),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_memory_answers_none_and_known_absent_is_zero() {
        let unknown = BankMemory::default();
        assert!(!unknown.known());
        assert_eq!(unknown.origin(), Origin::Unknown);
        assert_eq!(unknown.count(379), None);
        assert_eq!(unknown.observed_at(), None);
        assert!(unknown.rows().is_empty());

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
}
