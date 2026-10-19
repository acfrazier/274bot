//! Compact journal/progress evidence shared by the runner, host and nav provider.

use crate::game_data::QuestFamilyIdentity;
use crate::quest_facts::QuestCatalog;
use crate::selected::{FactKey, Knowledge, QuestGate, RunKey, SelectedPin, SignalRange, Truth};
use crate::snapshot::{Observed, QuestListStatus, QuestStatusView, SnapshotView};
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

/// Resolve one canonical quest identity against the observed quest-list rows.
///
/// A missing catalog identity, unavailable quest list, or missing row returns
/// `None`. An observed row with an unsupported colour returns `Unknown`.
pub fn quest_list_status(
    quests: &QuestCatalog,
    quest: &FactKey,
    snapshot: SnapshotView<'_>,
) -> Option<Observed<QuestListStatus>> {
    let facts = quests.quest(quest.0.as_ref()).ok()?;
    let rows = snapshot.quest_statuses()?;
    let status = status_for_display(&facts.display, rows.value)?;
    Some(Observed {
        value: status,
        stamp: rows.stamp,
    })
}

fn status_for_display(display: &str, rows: &[QuestStatusView]) -> Option<QuestListStatus> {
    rows.iter()
        .find(|row| row.name.eq_ignore_ascii_case(display))
        .map(QuestStatusView::status)
}

#[derive(Debug, Clone)]
struct QuestListStatusEvidence {
    quest: FactKey,
    status: QuestListStatus,
}

/// Completion evidence copied from one observed quest-list frame.
///
/// This provider retains only canonical quest IDs, their observed list status,
/// and the frame stamp. It does not retain the snapshot or the quest catalog.
#[derive(Debug, Clone)]
pub struct QuestListEvidenceProvider {
    stamp: Option<EvidenceStamp>,
    family_matches: bool,
    statuses: Vec<QuestListStatusEvidence>,
}

impl QuestListEvidenceProvider {
    /// Copy compact quest-list evidence from a frame.
    ///
    /// `catalog_family` identifies the verified family used to create
    /// `quests`. `selected_family` identifies the currently selected family.
    /// Both identities must be available and equal. Each identity contains the
    /// family SHA-256 and schema, so a different revision family fails closed.
    pub fn from_snapshot(
        quests: &QuestCatalog,
        catalog_family: Option<QuestFamilyIdentity>,
        selected_family: Option<QuestFamilyIdentity>,
        snapshot: SnapshotView<'_>,
    ) -> Self {
        let family_matches = matches!(
            (catalog_family, selected_family),
            (Some(catalog), Some(selected)) if catalog == selected
        );
        let Some(observed) = snapshot.quest_statuses() else {
            return Self {
                stamp: None,
                family_matches,
                statuses: Vec::new(),
            };
        };
        let statuses = if family_matches {
            quests
                .iter()
                .map(|facts| QuestListStatusEvidence {
                    quest: facts.id.clone(),
                    status: status_for_display(&facts.display, observed.value)
                        .unwrap_or(QuestListStatus::Unknown),
                })
                .collect()
        } else {
            Vec::new()
        };
        Self {
            stamp: Some(observed.stamp),
            family_matches,
            statuses,
        }
    }
}

impl EvidenceProvider for QuestListEvidenceProvider {
    fn test_gate(&self, gate: &QuestGate, required_after: EvidenceStamp) -> Truth {
        let QuestGate::Complete(quest) = gate else {
            return Truth::Unknown;
        };
        if !self.family_matches
            || !self
                .stamp
                .is_some_and(|stamp| stamp.meets(required_after))
        {
            return Truth::Unknown;
        }
        match self
            .statuses
            .iter()
            .find(|status| status.quest == *quest)
            .map(|status| status.status)
            .unwrap_or(QuestListStatus::Unknown)
        {
            QuestListStatus::Complete => Truth::True,
            QuestListStatus::NotStarted | QuestListStatus::InProgress => Truth::False,
            QuestListStatus::Unknown => Truth::Unknown,
        }
    }
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

#[cfg(test)]
mod quest_list_tests {
    use super::*;
    use crate::game_data::QuestFamilyIdentity;
    use crate::selected::{ClientRevision, InclusiveRange, RunKey, StageWindow};
    use crate::snapshot::{GameSnapshot, QuestStatusView};
    use std::num::NonZeroU16;

    fn catalog() -> QuestCatalog {
        let selected = crate::game_data::for_revision(ClientRevision::R289).unwrap();
        QuestCatalog::from_identity(selected.quest_identity()).unwrap()
    }

    fn quest(quests: &QuestCatalog) -> (FactKey, String) {
        let facts = quests.iter().next().expect("selected quest catalog has rows");
        (facts.id.clone(), facts.display.to_string())
    }

    fn run() -> RunKey {
        RunKey {
            slot: 7,
            run: 1,
            session: 1,
        }
    }

    fn stamp(run: RunKey, tick: u64, sequence: u64) -> EvidenceStamp {
        EvidenceStamp {
            run,
            tick,
            sequence,
        }
    }

    fn family(sha: u8, schema: u16) -> QuestFamilyIdentity {
        QuestFamilyIdentity {
            sha256: [sha; 32],
            schema: NonZeroU16::new(schema).unwrap(),
        }
    }

    fn snapshot(rows: Vec<QuestStatusView>, available: bool) -> GameSnapshot {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_quest_statuses(rows, available);
        snapshot
    }

    fn row(name: String, colour: i32) -> QuestStatusView {
        QuestStatusView {
            component_id: 1,
            name,
            colour,
        }
    }

    fn evidence(
        quests: &QuestCatalog,
        snapshot: &GameSnapshot,
        observed: EvidenceStamp,
        catalog_family: Option<QuestFamilyIdentity>,
        selected_family: Option<QuestFamilyIdentity>,
    ) -> QuestListEvidenceProvider {
        QuestListEvidenceProvider::from_snapshot(
            quests,
            catalog_family,
            selected_family,
            SnapshotView::new(Some(snapshot), observed),
        )
    }

    fn complete_gate(quest: FactKey) -> QuestGate {
        QuestGate::Complete(quest)
    }

    #[test]
    fn observed_statuses_answer_completion_without_other_evidence_sources() {
        let quests = catalog();
        let (quest, display) = quest(&quests);
        let family = family(1, 1);
        let observed = stamp(run(), 10, 4);
        for (colour, expected) in [
            (0x00F800, Truth::True),
            (0xF80000, Truth::False),
            (0xF8F800, Truth::False),
            (0x123456, Truth::Unknown),
        ] {
            let snapshot = snapshot(vec![row(display.clone(), colour)], true);
            let provider = evidence(
                &quests,
                &snapshot,
                observed,
                Some(family),
                Some(family),
            );
            assert_eq!(
                provider.test_gate(&complete_gate(quest.clone()), observed),
                expected
            );
        }

        let absent_row = snapshot(Vec::new(), true);
        let provider = evidence(
            &quests,
            &absent_row,
            observed,
            Some(family),
            Some(family),
        );
        assert_eq!(
            provider.test_gate(&complete_gate(quest.clone()), observed),
            Truth::Unknown
        );

        let unavailable = snapshot(vec![row(display, 0x00F800)], false);
        let provider = evidence(
            &quests,
            &unavailable,
            observed,
            Some(family),
            Some(family),
        );
        assert_eq!(
            provider.test_gate(&complete_gate(quest.clone()), observed),
            Truth::Unknown
        );

        let provider = evidence(
            &quests,
            &snapshot(Vec::new(), true),
            observed,
            Some(family),
            Some(family),
        );
        assert_eq!(
            provider.test_gate(&complete_gate(FactKey::new("unknown-quest")), observed),
            Truth::Unknown
        );
    }

    #[test]
    fn evidence_requires_the_causal_floor_and_same_run_session() {
        let quests = catalog();
        let (quest, display) = quest(&quests);
        let family = family(1, 1);
        let observed = stamp(run(), 10, 7);
        let snapshot = snapshot(vec![row(display, 0x00F800)], true);
        let provider = evidence(
            &quests,
            &snapshot,
            observed,
            Some(family),
            Some(family),
        );
        let gate = complete_gate(quest);

        assert_eq!(
            provider.test_gate(&gate, stamp(run(), 10, 6)),
            Truth::True
        );
        assert_eq!(
            provider.test_gate(&gate, stamp(run(), 10, 7)),
            Truth::True
        );
        assert_eq!(
            provider.test_gate(&gate, stamp(run(), 10, 8)),
            Truth::Unknown
        );
        assert_eq!(
            provider.test_gate(&gate, stamp(run(), 9, 99)),
            Truth::Unknown
        );
        assert_eq!(
            provider.test_gate(&gate, stamp(run(), 11, 1)),
            Truth::Unknown
        );
        for other_run in [
            RunKey { slot: 8, ..run() },
            RunKey { run: 2, ..run() },
            RunKey { session: 2, ..run() },
        ] {
            assert_eq!(
                provider.test_gate(&gate, stamp(other_run, 10, 7)),
                Truth::Unknown
            );
        }
    }

    #[test]
    fn evidence_requires_the_same_verified_family_sha_and_schema() {
        let quests = catalog();
        let (quest, display) = quest(&quests);
        let selected_family = family(1, 1);
        let observed = stamp(run(), 10, 4);
        let snapshot = snapshot(vec![row(display, 0x00F800)], true);
        let gate = complete_gate(quest);

        for (catalog_family, current_family, expected) in [
            (Some(selected_family), Some(selected_family), Truth::True),
            (Some(family(2, 1)), Some(selected_family), Truth::Unknown),
            (Some(family(1, 2)), Some(selected_family), Truth::Unknown),
            (None, Some(selected_family), Truth::Unknown),
            (Some(selected_family), None, Truth::Unknown),
        ] {
            let provider = evidence(
                &quests,
                &snapshot,
                observed,
                catalog_family,
                current_family,
            );
            assert_eq!(provider.test_gate(&gate, observed), expected);
        }
    }

    #[test]
    fn window_gates_remain_unknown_even_when_the_quest_is_complete() {
        let quests = catalog();
        let (quest, display) = quest(&quests);
        let family = family(1, 1);
        let observed = stamp(run(), 10, 4);
        let snapshot = snapshot(vec![row(display, 0x00F800)], true);
        let provider = evidence(
            &quests,
            &snapshot,
            observed,
            Some(family),
            Some(family),
        );
        let gate = QuestGate::Window(StageWindow {
            quest,
            signal: FactKey::new("signal"),
            values: InclusiveRange {
                min: Some(1),
                max: Some(2),
            },
        });

        assert_eq!(provider.test_gate(&gate, observed), Truth::Unknown);
    }

    #[test]
    fn shared_lookup_returns_the_observed_status_and_stamp() {
        let quests = catalog();
        let (quest, display) = quest(&quests);
        let observed = stamp(run(), 10, 4);
        let snapshot = snapshot(vec![row(display, 0xF8F800)], true);

        let status = quest_list_status(
            &quests,
            &quest,
            SnapshotView::new(Some(&snapshot), observed),
        )
        .expect("quest row is observed");
        assert_eq!(status.value, QuestListStatus::InProgress);
        assert_eq!(status.stamp, observed);
    }
}
