//! Session-scoped facts learned from prerequisite crossings and dialogue.
//!
//! Durable storage is deliberately only a boundary trait here. The route view
//! composes immutable session memory, explicit evidence, and ordinary colour
//! evidence without consulting a persistence service.

use crate::local_state::{AccountKey, ContentScope, ErrorCode, SubmitReceipt};
use crate::prereq_steps::{ActionableGate, CrossingId};
use crate::quest_progress::{EvidenceProvider, EvidenceStamp};
use crate::selected::{FactKey, InclusiveRange, QuestGate, Truth};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observation {
    pub truth: Truth,
    /// `None` is reserved for explicit caller evidence that cannot be ordered.
    pub observed: Option<EvidenceStamp>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactSource {
    Crossing {
        loc_id: i32,
        option: u8,
        passed: bool,
    },
    Page {
        row: u16,
        line: u16,
    },
    Refusal {
        row: u16,
        entry: u8,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    pub quest: FactKey,
    pub signal: FactKey,
    pub possible: InclusiveRange,
    pub after: InclusiveRange,
    pub refused: Option<InclusiveRange>,
    pub source: FactSource,
    pub observed: EvidenceStamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogRef {
    pub row: u16,
    pub entry: u8,
}

/// The immutable description of one server refusal, shared by every terminal
/// and handover carrier. Its fact is recorded before a runner or owner starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefusalObservation {
    pub crossing: CrossingId,
    pub leg: u16,
    pub gates: Arc<[ActionableGate]>,
    pub matched: CatalogRef,
    pub fact: Fact,
}

/// At most one compact fact for a `(quest, signal)` pair.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionMemory {
    facts: Option<Box<[Fact]>>,
}

impl SessionMemory {
    /// Record a live observation. Returns true only when the retained fact changes.
    pub fn record(&mut self, mut fact: Fact) -> bool {
        let Some(facts) = self.facts.as_mut() else {
            self.facts = Some(vec![fact].into_boxed_slice());
            return true;
        };
        let Some(index) = facts
            .iter()
            .position(|old| old.quest == fact.quest && old.signal == fact.signal)
        else {
            let mut next = Vec::with_capacity(facts.len() + 1);
            next.extend(facts.iter().cloned());
            next.push(fact);
            self.facts = Some(next.into_boxed_slice());
            return true;
        };

        let old = &facts[index];
        if old.observed.run == fact.observed.run {
            let old_order = (old.observed.tick, old.observed.sequence);
            let new_order = (fact.observed.tick, fact.observed.sequence);
            if new_order < old_order {
                return false;
            }
            if new_order == old_order && old.source == fact.source {
                return false;
            }
            if new_order >= old_order && old.refused.is_none() && fact.refused.is_none() {
                if let Some(possible) = intersect(old.possible, fact.possible) {
                    fact.possible = possible;
                    fact.after = intersect(old.after, fact.after).unwrap_or(fact.after);
                }
            }
        }
        facts[index] = fact;
        true
    }

    pub fn facts(&self) -> &[Fact] {
        self.facts.as_deref().unwrap_or(&[])
    }

    /// Owned storage estimate, including the inline `Fact` array but not Arc targets.
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.facts.as_deref().map_or(0, std::mem::size_of_val)
    }

    /// Current-run facts answer authoritatively; facts from another run remain hints.
    pub fn observe(&self, gate: &QuestGate, floor: EvidenceStamp) -> Option<Observation> {
        let QuestGate::Window(window) = gate else {
            return None;
        };
        let fact = self
            .facts()
            .iter()
            .find(|fact| fact.quest == window.quest && fact.signal == window.signal)?;
        if fact.observed.run != floor.run {
            return Some(Observation {
                truth: Truth::Unknown,
                observed: Some(fact.observed),
            });
        }
        let range = if fact.observed.meets(floor) {
            fact.possible
        } else {
            fact.after
        };
        let truth = if fact
            .refused
            .is_some_and(|refused| refused.test(&window.values) == Truth::True)
        {
            Truth::False
        } else {
            window.values.test(&range)
        };
        Some(Observation {
            truth,
            observed: Some(fact.observed),
        })
    }
}

fn intersect(left: InclusiveRange, right: InclusiveRange) -> Option<InclusiveRange> {
    let left_min = left.min.unwrap_or(i32::MIN);
    let right_min = right.min.unwrap_or(i32::MIN);
    let left_max = left.max.unwrap_or(i32::MAX);
    let right_max = right.max.unwrap_or(i32::MAX);
    let min = left_min.max(right_min);
    let max = left_max.min(right_max);
    (min <= max).then_some(InclusiveRange {
        min: (min != i32::MIN).then_some(min),
        max: (max != i32::MAX).then_some(max),
    })
}

/// Compose one captured route view with explicit evidence and quest-list colour.
pub fn compose(
    gate: &QuestGate,
    floor: EvidenceStamp,
    memory: &SessionMemory,
    explicit: Option<&dyn EvidenceProvider>,
    ordinary: &dyn EvidenceProvider,
) -> Observation {
    let memory = memory
        .observe(gate, floor)
        .filter(|observation| observation.truth != Truth::Unknown);
    let explicit = explicit
        .and_then(|provider| provider.observe_gate(gate, floor))
        .filter(|observation| observation.truth != Truth::Unknown);

    match (memory, explicit) {
        (Some(memory), Some(explicit)) => match (memory.observed, explicit.observed) {
            (Some(memory_stamp), Some(explicit_stamp)) => {
                if memory_stamp.run != explicit_stamp.run {
                    Observation {
                        truth: Truth::Unknown,
                        observed: None,
                    }
                } else if memory_stamp == explicit_stamp {
                    Observation {
                        truth: if memory.truth == explicit.truth {
                            memory.truth
                        } else {
                            Truth::False
                        },
                        observed: Some(memory_stamp),
                    }
                } else if (memory_stamp.tick, memory_stamp.sequence)
                    > (explicit_stamp.tick, explicit_stamp.sequence)
                {
                    memory
                } else {
                    explicit
                }
            }
            (Some(_), None) => memory,
            (None, Some(_)) => explicit,
            (None, None) => memory,
        },
        (Some(memory), None) => memory,
        (None, Some(explicit)) => explicit,
        (None, None) => ordinary.observe_gate(gate, floor).unwrap_or(Observation {
            truth: Truth::Unknown,
            observed: None,
        }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hint {
    pub possible: InclusiveRange,
    pub refused: Option<InclusiveRange>,
    pub observed_at: crate::identity::DurableTimestamp,
}

/// Durable knowledge boundary. This slice has no persistence implementation.
pub trait PrereqKnowledge: Send + Sync {
    fn hint(
        &self,
        account: &AccountKey,
        scope: &ContentScope,
        signal: &(FactKey, FactKey),
    ) -> Result<Option<Hint>, ErrorCode>;

    fn record(
        &self,
        expected: Option<crate::local_state::StoreVersion>,
        fact: &Fact,
        observed_at: crate::identity::DurableTimestamp,
    ) -> Result<SubmitReceipt, ErrorCode>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selected::{RunKey, StageWindow};

    struct FixedEvidence(Observation);

    impl EvidenceProvider for FixedEvidence {
        fn observe_gate(
            &self,
            _gate: &QuestGate,
            _required_after: EvidenceStamp,
        ) -> Option<Observation> {
            Some(self.0)
        }
    }

    #[test]
    fn equal_memory_and_explicit_stamps_preserve_agreement_and_reject_conflict() {
        let at = stamp(10, 2);
        let mut memory = SessionMemory::default();
        assert!(memory.record(fact((5, 5), (5, 110), at, false)));
        let gate = gate(1, None);
        let agreeing = FixedEvidence(Observation {
            truth: Truth::True,
            observed: Some(at),
        });
        let ordinary = FixedEvidence(Observation {
            truth: Truth::Unknown,
            observed: Some(at),
        });
        assert_eq!(
            compose(&gate, at, &memory, Some(&agreeing), &ordinary).truth,
            Truth::True
        );

        let conflicting = FixedEvidence(Observation {
            truth: Truth::False,
            observed: Some(at),
        });
        assert_eq!(
            compose(&gate, at, &memory, Some(&conflicting), &ordinary).truth,
            Truth::False
        );
    }

    fn run() -> RunKey {
        RunKey {
            slot: 2,
            run: 1,
            session: 1,
        }
    }

    fn stamp(tick: u64, sequence: u64) -> EvidenceStamp {
        EvidenceStamp {
            run: run(),
            tick,
            sequence,
        }
    }

    fn fact(possible: (i32, i32), after: (i32, i32), at: EvidenceStamp, refused: bool) -> Fact {
        Fact {
            quest: FactKey::new("druidspirit"),
            signal: FactKey::new("druidspirit"),
            possible: InclusiveRange {
                min: Some(possible.0),
                max: Some(possible.1),
            },
            after: InclusiveRange {
                min: Some(after.0),
                max: Some(after.1),
            },
            refused: refused.then_some(InclusiveRange {
                min: Some(1),
                max: None,
            }),
            source: FactSource::Page { row: 1, line: 0 },
            observed: at,
        }
    }

    fn gate(min: i32, max: Option<i32>) -> QuestGate {
        QuestGate::Window(StageWindow {
            quest: FactKey::new("druidspirit"),
            signal: FactKey::new("druidspirit"),
            values: InclusiveRange {
                min: Some(min),
                max,
            },
        })
    }

    #[test]
    fn refusal_falsifies_only_windows_inside_its_refused_range() {
        let mut memory = SessionMemory::default();
        assert!(memory.record(fact((0, 0), (0, 110), stamp(5, 1), true)));
        assert_eq!(
            memory.observe(&gate(1, None), stamp(5, 1)).unwrap().truth,
            Truth::False
        );
        assert_eq!(
            memory.observe(&gate(0, None), stamp(5, 1)).unwrap().truth,
            Truth::True
        );
    }

    #[test]
    fn newer_positive_replaces_refusal_and_equal_refusal_is_idempotent() {
        let mut memory = SessionMemory::default();
        let refusal = fact((0, 0), (0, 110), stamp(5, 1), true);
        assert!(memory.record(refusal.clone()));
        assert!(!memory.record(refusal));
        assert!(memory.record(fact((5, 5), (5, 110), stamp(6, 1), false)));
        assert_eq!(
            memory.observe(&gate(1, None), stamp(6, 1)).unwrap().truth,
            Truth::True
        );
    }

    #[test]
    fn older_write_is_dropped_and_positive_ranges_intersect() {
        let mut memory = SessionMemory::default();
        assert!(memory.record(fact((5, 5), (5, 110), stamp(8, 1), false)));
        assert!(!memory.record(fact((0, 110), (0, 110), stamp(7, 1), false)));
        assert!(memory.record(fact((1, 110), (1, 110), stamp(9, 1), false)));
        assert_eq!(
            memory.facts()[0].possible,
            InclusiveRange {
                min: Some(5),
                max: Some(5)
            }
        );
        assert_eq!(
            memory.facts()[0].after,
            InclusiveRange {
                min: Some(5),
                max: Some(110)
            }
        );
    }
}
