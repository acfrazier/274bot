//! Compact journal/progress evidence shared by the runner, host and nav provider.

use crate::selected::{FactKey, Knowledge, QuestGate, RunKey, SelectedPin, SignalRange, Truth};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvidenceStamp {
    pub run: RunKey,
    pub tick: u64,
    pub sequence: u64,
}

impl EvidenceStamp {
    /// Same run/session and at least the causal floor. Sequence orders same-tick
    /// observations; ordinary later ticks do not invalidate an older proof.
    pub fn meets(self, required_after: Self) -> bool {
        self.run == required_after.run
            && (self.tick, self.sequence) >= (required_after.tick, required_after.sequence)
    }
}

#[derive(Debug, Clone)]
pub struct QuestProgress {
    pub quest: FactKey,
    pub stage: Knowledge<FactKey>,
    pub complete: Truth,
    pub signals: Arc<[SignalRange]>,
    pub flags: Arc<[ProgressFlag]>,
    pub evidence: EvidenceStamp,
    pub binding: FactKey,
    pub role: Option<FactKey>,
    pub rule: Knowledge<FactKey>,
    pub pin: Arc<SelectedPin>,
}

/// One typed flag result from an examined quest journal.
///
/// Boolean flags use `truth` and leave `count` empty. Counted flags use the
/// optional count capture and set `truth` to `True` when a bounded capture was
/// found, `False` when the flag rule did not match, or `Unknown` when the
/// journal evidence was incomplete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressFlag {
    pub flag: FactKey,
    pub truth: Truth,
    pub count: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct JournalRead {
    pub quest: FactKey,
    pub root: i32,
    pub lines: Arc<[Arc<str>]>,
    pub colour: Option<i32>,
    pub acquired: EvidenceStamp,
    pub closed: EvidenceStamp,
    pub pin: Arc<SelectedPin>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressError {
    WrongPin,
    WrongQuest,
    WrongRole,
    Stale,
    UnknownTemplate,
    Ambiguous,
    Contradiction,
}

pub trait EvidenceProvider: Send + Sync {
    fn test_gate(&self, gate: &QuestGate, required_after: EvidenceStamp) -> Truth;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_and_causal_changes_invalidate_but_idle_ticks_do_not() {
        let run = RunKey {
            slot: 7,
            run: 1,
            session: 1,
        };
        let proof = EvidenceStamp {
            run,
            tick: 10,
            sequence: 20,
        };
        assert!(proof.meets(proof));
        assert!(proof.meets(EvidenceStamp {
            tick: 1,
            sequence: 1,
            ..proof
        }));
        assert!(EvidenceStamp {
            tick: 11,
            sequence: 1,
            ..proof
        }
        .meets(proof));
        assert!(!EvidenceStamp {
            sequence: 19,
            ..proof
        }
        .meets(proof));
        assert!(!EvidenceStamp {
            tick: 9,
            sequence: 99,
            ..proof
        }
        .meets(proof));
        assert!(!EvidenceStamp {
            run: RunKey { slot: 8, ..run },
            ..proof
        }
        .meets(proof));
        assert!(!proof.meets(EvidenceStamp {
            sequence: 21,
            ..proof
        }));
        assert!(!proof.meets(EvidenceStamp {
            run: run.next_session().unwrap(),
            ..proof
        }));
        assert!(!proof.meets(EvidenceStamp {
            run: run.next_run().unwrap(),
            ..proof
        }));
    }
}
