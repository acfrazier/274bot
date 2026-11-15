//! Compact journal/progress evidence shared by the runner, host and nav provider.

use crate::game_data::Transmission;
use crate::prereq_knowledge::Observation;
use crate::quest_facts::{QuestCatalog, QuestFacts};
use crate::selected::{FactKey, InclusiveRange, Knowledge, QuestGate, RunKey, SignalRange, Truth};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalRead {
    pub quest: FactKey,
    pub root: i32,
    pub lines: Arc<[Arc<str>]>,
    pub colour: Option<i32>,
    pub acquired: EvidenceStamp,
    pub closed: EvidenceStamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressError {
    WrongQuest,
    WrongRole,
    Stale,
    UnknownTemplate,
    Ambiguous,
    Contradiction,
}

pub trait EvidenceProvider: Send + Sync {
    fn observe_gate(&self, gate: &QuestGate, required_after: EvidenceStamp) -> Option<Observation>;

    fn test_gate(&self, gate: &QuestGate, required_after: EvidenceStamp) -> Truth {
        self.observe_gate(gate, required_after)
            .map_or(Truth::Unknown, |observation| observation.truth)
    }
}

/// Resolve one canonical quest identity against the observed quest-list rows.
///
/// A missing catalog identity, unavailable quest list, or missing row returns
/// `None`. An observed row with an unsupported colour returns `Unknown`.
pub fn quest_list_status(
    quests: &QuestCatalog,
    quest: &str,
    snapshot: SnapshotView<'_>,
) -> Option<Observed<QuestListStatus>> {
    let facts = quests.quest(quest).ok()?;
    let rows = snapshot.quest_statuses()?;
    let (_, status) = status_for_display(&facts.display, rows.value)?;
    Some(Observed {
        value: status,
        stamp: rows.stamp,
    })
}

fn status_for_display(display: &str, rows: &[QuestStatusView]) -> Option<(usize, QuestListStatus)> {
    rows.iter()
        .enumerate()
        .find(|(_, row)| row.name.eq_ignore_ascii_case(display))
        .map(|(index, row)| (index, row.status()))
}

/// Borrowed ordinary quest-list evaluation for a runner poll. The caller must
/// validate that the catalog family is the selected family before using it.
pub fn observe_quest_gate(
    quests: &QuestCatalog,
    gate: &QuestGate,
    snapshot: SnapshotView<'_>,
    required_after: EvidenceStamp,
) -> Option<Observation> {
    let rows = snapshot.quest_statuses()?;
    let quest_id = match gate {
        QuestGate::Complete(quest) => quest.0.as_ref(),
        QuestGate::Window(window) => window.quest.0.as_ref(),
    };
    let facts = quests.quest(quest_id).ok()?;
    let (index, status) = status_for_display(&facts.display, rows.value)
        .unwrap_or((usize::MAX, QuestListStatus::Unknown));
    let truth = if !rows.stamp.meets(required_after) {
        Truth::Unknown
    } else {
        match gate {
            QuestGate::Complete(_) => status_truth(status),
            QuestGate::Window(window) if window.signal == facts.varp => {
                stage_colour_range(status, facts, snapshot.quest_green_since_fresh_login(index))
                    .map_or(Truth::Unknown, |possible| window.values.test(&possible))
            }
            QuestGate::Window(_) => Truth::Unknown,
        }
    };
    Some(Observation {
        truth,
        observed: Some(rows.stamp),
    })
}

fn status_truth(status: QuestListStatus) -> Truth {
    match status {
        QuestListStatus::Complete => Truth::True,
        QuestListStatus::NotStarted | QuestListStatus::InProgress => Truth::False,
        QuestListStatus::Unknown => Truth::Unknown,
    }
}

fn stage_colour_range(
    status: QuestListStatus,
    facts: &QuestFacts,
    green_since_login: bool,
) -> Option<InclusiveRange> {
    let complete = facts.complete;
    if facts.transmission != Transmission::NotTransmitted || complete <= 0 {
        return None;
    }
    if status == QuestListStatus::Complete {
        let range = if green_since_login {
            facts
                .completion_range_after_login
                .or(facts.completion_range)
        } else {
            facts.completion_range
        };
        if let Some(range) = range {
            return Some(range);
        }
    }
    if !facts.colour_authoritative {
        return None;
    }
    let domain = facts.stage_domain?;
    let (Some(domain_min), Some(domain_max)) = (domain.min, domain.max) else {
        return None;
    };
    if domain_min != 0 || domain_max < complete {
        return None;
    }
    let colour_range = match status {
        QuestListStatus::NotStarted => InclusiveRange {
            min: Some(0),
            max: Some(0),
        },
        QuestListStatus::InProgress => InclusiveRange {
            min: Some(1),
            max: Some(complete - 1),
        },
        QuestListStatus::Complete => InclusiveRange {
            min: Some(complete),
            max: Some(i32::MAX),
        },
        QuestListStatus::Unknown => return None,
    };
    intersect_ranges(colour_range, domain)
}

fn intersect_ranges(left: InclusiveRange, right: InclusiveRange) -> Option<InclusiveRange> {
    let min = left
        .min
        .unwrap_or(i32::MIN)
        .max(right.min.unwrap_or(i32::MIN));
    let max = left
        .max
        .unwrap_or(i32::MAX)
        .min(right.max.unwrap_or(i32::MAX));
    (min <= max).then_some(InclusiveRange {
        min: Some(min),
        max: Some(max),
    })
}

#[derive(Debug, Clone)]
struct QuestListStatusEvidence {
    quest: FactKey,
    status: QuestListStatus,
    signal: FactKey,
    stage_range: Option<InclusiveRange>,
}
/// Evidence copied from one observed quest-list frame. It retains only
/// canonical quest facts needed for complete and stage-window evaluation.
///
/// This provider retains neither the snapshot nor the quest catalog.
#[derive(Debug, Clone)]
pub struct QuestListEvidenceProvider {
    stamp: Option<EvidenceStamp>,
    statuses: Vec<QuestListStatusEvidence>,
}

impl QuestListEvidenceProvider {
    /// Copy compact quest-list evidence from a frame.
    pub fn from_snapshot(quests: &QuestCatalog, snapshot: SnapshotView<'_>) -> Self {
        let Some(observed) = snapshot.quest_statuses() else {
            return Self {
                stamp: None,
                statuses: Vec::new(),
            };
        };
        let statuses = quests
            .iter()
            .map(|facts| {
                let (index, status) = status_for_display(&facts.display, observed.value)
                    .unwrap_or((usize::MAX, QuestListStatus::Unknown));
                QuestListStatusEvidence {
                    quest: facts.id.clone(),
                    status,
                    signal: facts.varp.clone(),
                    stage_range: stage_colour_range(
                        status,
                        facts,
                        snapshot.quest_green_since_fresh_login(index),
                    ),
                }
            })
            .collect();
        Self {
            stamp: Some(observed.stamp),
            statuses,
        }
    }

    /// Owned status storage; canonical quest-key strings are shared with the
    /// prepared catalog, whose owner already retains them.
    pub fn retained_status_bytes(&self) -> usize {
        self.statuses.capacity() * std::mem::size_of::<QuestListStatusEvidence>()
    }
}

impl EvidenceProvider for QuestListEvidenceProvider {
    fn observe_gate(&self, gate: &QuestGate, required_after: EvidenceStamp) -> Option<Observation> {
        let quest = match gate {
            QuestGate::Complete(quest) => quest,
            QuestGate::Window(window) => &window.quest,
        };
        let status = self.statuses.iter().find(|status| status.quest == *quest);
        let truth = if !self.stamp.is_some_and(|stamp| stamp.meets(required_after)) {
            Truth::Unknown
        } else {
            let Some(status) = status else {
                return Some(Observation {
                    truth: Truth::Unknown,
                    observed: self.stamp,
                });
            };
            match gate {
                QuestGate::Complete(_) => status_truth(status.status),
                QuestGate::Window(window) if window.signal == status.signal => status
                    .stage_range
                    .map_or(Truth::Unknown, |possible| window.values.test(&possible)),
                QuestGate::Window(_) => Truth::Unknown,
            }
        };
        Some(Observation {
            truth,
            observed: self.stamp,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captured_quest_rows_retain_only_one_stage_range() {
        // Measured 56 B plus the default 64 B headroom (AGENTS.md tripwire rule).
        assert!(std::mem::size_of::<QuestListStatusEvidence>() <= 120);
    }

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
    use crate::selected::{ClientRevision, InclusiveRange, RunKey, StageWindow};
    use crate::snapshot::{GameSnapshot, QuestStatusView};

    fn catalog() -> Option<QuestCatalog> {
        let selected = crate::game_data::test_data(ClientRevision::R289)?;
        Some(QuestCatalog::from_identity(selected.quest_identity()).unwrap())
    }

    fn quest(quests: &QuestCatalog) -> (FactKey, String) {
        let facts = quests
            .iter()
            .next()
            .expect("selected quest catalog has rows");
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
    ) -> QuestListEvidenceProvider {
        QuestListEvidenceProvider::from_snapshot(
            quests,
            SnapshotView::new(Some(snapshot), observed),
        )
    }

    fn complete_gate(quest: FactKey) -> QuestGate {
        QuestGate::Complete(quest)
    }
    fn biohazard_window(values: InclusiveRange) -> QuestGate {
        QuestGate::Window(StageWindow {
            quest: FactKey::new("biohazard"),
            signal: FactKey::new("biohazard"),
            values,
        })
    }

    #[test]
    fn observed_statuses_answer_completion_without_other_evidence_sources() {
        let Some(quests) = catalog() else {
            return;
        };
        let (quest, display) = quest(&quests);
        let observed = stamp(run(), 10, 4);
        for (colour, expected) in [
            (0x00F800, Truth::True),
            (0xF80000, Truth::False),
            (0xF8F800, Truth::False),
            (0x123456, Truth::Unknown),
        ] {
            let snapshot = snapshot(vec![row(display.clone(), colour)], true);
            let provider = evidence(&quests, &snapshot, observed);
            assert_eq!(
                provider.test_gate(&complete_gate(quest.clone()), observed),
                expected
            );
        }

        let absent_row = snapshot(Vec::new(), true);
        let provider = evidence(&quests, &absent_row, observed);
        assert_eq!(
            provider.test_gate(&complete_gate(quest.clone()), observed),
            Truth::Unknown
        );

        let unavailable = snapshot(vec![row(display, 0x00F800)], false);
        let provider = evidence(&quests, &unavailable, observed);
        assert_eq!(
            provider.test_gate(&complete_gate(quest.clone()), observed),
            Truth::Unknown
        );

        let empty = snapshot(Vec::new(), true);
        let provider = evidence(&quests, &empty, observed);
        assert_eq!(
            provider.test_gate(&complete_gate(FactKey::new("unknown-quest")), observed),
            Truth::Unknown
        );
    }

    #[test]
    fn evidence_requires_the_causal_floor_and_same_run_session() {
        let Some(quests) = catalog() else {
            return;
        };
        let (quest, display) = quest(&quests);
        let observed = stamp(run(), 10, 7);
        let snapshot = snapshot(vec![row(display, 0x00F800)], true);
        let provider = evidence(&quests, &snapshot, observed);
        let gate = complete_gate(quest);

        assert_eq!(provider.test_gate(&gate, stamp(run(), 10, 6)), Truth::True);
        assert_eq!(provider.test_gate(&gate, stamp(run(), 10, 7)), Truth::True);
        assert_eq!(
            provider.test_gate(&gate, stamp(run(), 10, 8)),
            Truth::Unknown
        );
        // Sequence orders observations within a tick; a later tick satisfies
        // the floor even when the earlier tick had a larger sequence.
        assert_eq!(provider.test_gate(&gate, stamp(run(), 9, 99)), Truth::True);
        assert_eq!(
            provider.test_gate(&gate, stamp(run(), 11, 1)),
            Truth::Unknown
        );
        for other_run in [
            RunKey { slot: 8, ..run() },
            RunKey { run: 2, ..run() },
            RunKey {
                session: 2,
                ..run()
            },
        ] {
            assert_eq!(
                provider.test_gate(&gate, stamp(other_run, 10, 7)),
                Truth::Unknown
            );
        }
    }

    #[test]
    fn not_started_biohazard_cannot_prove_zero_without_colour_authority() {
        let Some(quests) = catalog() else {
            return;
        };
        // Current F-C witness:
        // scripts/areas/area_ardougne_east/scripts/elena.rs2:132.
        // Its enclosing case118 and nested choice126-132 are a recorded
        // producer accuracy follow-up, not evidence of a real reset.
        assert!(!quests.quest("biohazard").unwrap().colour_authoritative);
        let observed = stamp(run(), 10, 4);
        let snapshot = snapshot(vec![row(String::from("Biohazard"), 0xF80000)], true);
        let provider = evidence(&quests, &snapshot, observed);

        for min in [None, Some(0)] {
            assert_eq!(
                provider.test_gate(
                    &biohazard_window(InclusiveRange { min, max: Some(9) }),
                    observed,
                ),
                Truth::Unknown
            );
        }
    }

    #[test]
    fn completed_biohazard_windows_remain_unknown_without_colour_authority() {
        let Some(quests) = catalog() else {
            return;
        };
        // The same conservative elena.rs2:118,126-132 writer analysis applies.
        assert!(!quests.quest("biohazard").unwrap().colour_authoritative);
        let observed = stamp(run(), 10, 4);
        let snapshot = snapshot(vec![row(String::from("Biohazard"), 0x00F800)], true);
        let provider = evidence(&quests, &snapshot, observed);

        assert_eq!(
            provider.test_gate(
                &biohazard_window(InclusiveRange {
                    min: None,
                    max: Some(9),
                }),
                observed,
            ),
            Truth::Unknown
        );
        assert_eq!(
            provider.test_gate(
                &biohazard_window(InclusiveRange {
                    min: Some(14),
                    max: Some(16),
                }),
                observed,
            ),
            Truth::Unknown
        );
    }

    #[test]
    fn in_progress_biohazard_windows_remain_unknown_without_colour_authority() {
        let Some(quests) = catalog() else {
            return;
        };
        // The same conservative elena.rs2:118,126-132 writer analysis applies.
        assert!(!quests.quest("biohazard").unwrap().colour_authoritative);
        let observed = stamp(run(), 10, 4);
        let snapshot = snapshot(vec![row(String::from("Biohazard"), 0xF8F800)], true);
        let provider = evidence(&quests, &snapshot, observed);

        assert_eq!(
            provider.test_gate(
                &biohazard_window(InclusiveRange {
                    min: Some(1),
                    max: Some(15),
                }),
                observed,
            ),
            Truth::Unknown
        );
        assert_eq!(
            provider.test_gate(
                &biohazard_window(InclusiveRange {
                    min: None,
                    max: Some(9),
                }),
                observed,
            ),
            Truth::Unknown
        );
    }

    fn assert_window_from_colour(
        quests: &QuestCatalog,
        quest: &str,
        colour: i32,
        min: i32,
        max: i32,
        expected: Truth,
    ) {
        let facts = quests.quest(quest).unwrap();
        let snapshot = snapshot(vec![row(facts.display.to_string(), colour)], true);
        assert_window_from_snapshot(quests, quest, &snapshot, min, max, expected);
    }

    fn assert_window_from_snapshot(
        quests: &QuestCatalog,
        quest: &str,
        snapshot: &GameSnapshot,
        min: i32,
        max: i32,
        expected: Truth,
    ) {
        let facts = quests.quest(quest).unwrap();
        let observed = stamp(run(), 10, 4);
        let gate = QuestGate::Window(StageWindow {
            quest: facts.id.clone(),
            signal: facts.varp.clone(),
            values: InclusiveRange {
                min: Some(min),
                max: Some(max),
            },
        });
        let provider = evidence(quests, snapshot, observed);
        assert_eq!(provider.test_gate(&gate, observed), expected);
        assert_eq!(
            observe_quest_gate(
                quests,
                &gate,
                SnapshotView::new(Some(snapshot), observed),
                observed,
            )
            .unwrap()
            .truth,
            expected
        );
    }

    #[test]
    fn completed_hero_window_requires_unchanged_fresh_login_green() {
        let Some(quests) = catalog() else {
            return;
        };
        let facts = quests.quest("hero").unwrap();
        // The queued Grip writer can set stage 5 without a delivery guard.
        // A cold login clears it; direct writes still obey their prerequisites.
        assert!(!facts.colour_authoritative);
        assert!(facts.completion_range.unwrap().min.unwrap() <= 5);
        assert_eq!(
            facts.completion_range_after_login,
            Some(InclusiveRange {
                min: Some(15),
                max: Some(15)
            })
        );
        for colour in [0x00F800, 0xF8F800, 0xF80000] {
            assert_window_from_colour(&quests, "hero", colour, 15, 15, Truth::Unknown);
            let mut snapshot = snapshot(vec![row(facts.display.to_string(), colour)], true);
            snapshot.seed_tick(1);
            snapshot.begin_fresh_login_quest_colours();
            let expected = if colour == 0x00F800 {
                Truth::True
            } else {
                Truth::Unknown
            };
            assert_window_from_snapshot(&quests, "hero", &snapshot, 15, 15, expected);
        }
    }

    #[test]
    fn later_green_cannot_create_or_restore_the_login_proof() {
        let Some(quests) = catalog() else {
            return;
        };
        let display = quests.quest("hero").unwrap().display.to_string();
        let mut snapshot = snapshot(vec![row(display.clone(), 0xF8F800)], true);
        snapshot.seed_tick(1);
        snapshot.begin_fresh_login_quest_colours();
        snapshot.seed_quest_statuses(vec![row(display.clone(), 0x00F800)], true);
        assert_window_from_snapshot(&quests, "hero", &snapshot, 15, 15, Truth::Unknown);
        snapshot.begin_fresh_login_quest_colours();
        assert_window_from_snapshot(&quests, "hero", &snapshot, 15, 15, Truth::True);
        snapshot.seed_quest_statuses(vec![row(display.clone(), 0xF80000)], true);
        snapshot.seed_quest_statuses(vec![row(display, 0x00F800)], true);
        assert_window_from_snapshot(&quests, "hero", &snapshot, 15, 15, Truth::Unknown);
    }

    #[test]
    fn login_proof_waits_for_player_info_and_does_not_cross_a_session_reset() {
        let Some(quests) = catalog() else {
            return;
        };
        let display = quests.quest("hero").unwrap().display.to_string();
        let mut snapshot = snapshot(vec![row(display.clone(), 0x00F800)], true);
        snapshot.begin_fresh_login_quest_colours();
        assert_window_from_snapshot(&quests, "hero", &snapshot, 15, 15, Truth::Unknown);
        snapshot.seed_tick(1);
        snapshot.seed_quest_statuses(vec![row(display.clone(), 0x00F800)], true);
        assert_window_from_snapshot(&quests, "hero", &snapshot, 15, 15, Truth::True);
        snapshot.reset_session(client::client::ClientGens::default());
        snapshot.seed_ingame(2);
        snapshot.seed_tick(1);
        snapshot.seed_quest_statuses(vec![row(display, 0x00F800)], true);
        assert_window_from_snapshot(&quests, "hero", &snapshot, 15, 15, Truth::Unknown);
    }

    #[test]
    fn retained_pre_login_green_cannot_seed_a_later_green_proof() {
        let Some(quests) = catalog() else {
            return;
        };
        let display = quests.quest("hero").unwrap().display.to_string();
        let mut snapshot = snapshot(vec![row(display.clone(), 0x00F800)], true);
        snapshot.begin_fresh_login_quest_colours();
        snapshot.seed_quest_statuses(vec![row(display.clone(), 0xF80000)], true);
        snapshot.seed_tick(1);
        snapshot.seed_quest_statuses(vec![row(display.clone(), 0xF80000)], true);
        snapshot.seed_quest_statuses(vec![row(display, 0x00F800)], true);
        assert_window_from_snapshot(&quests, "hero", &snapshot, 15, 15, Truth::Unknown);
    }

    #[test]
    fn weak_colour_proves_only_windows_covering_every_later_value() {
        let Some(selected) = crate::game_data::test_data(ClientRevision::R289) else {
            return;
        };
        let mut identity = selected.quest_identity().unwrap().clone();
        let row = identity
            .rows
            .iter_mut()
            .find(|row| row.id == "hero")
            .unwrap();
        row.colour_authoritative = false;
        row.completion_range = Some(InclusiveRange {
            min: Some(8),
            max: Some(15),
        });
        let quests = QuestCatalog::from_identity(Some(&identity)).unwrap();
        assert_window_from_colour(&quests, "hero", 0x00F800, 8, 15, Truth::True);
        assert_window_from_colour(&quests, "hero", 0x00F800, 15, 15, Truth::Unknown);
        assert_window_from_colour(&quests, "hero", 0xF8F800, 15, 15, Truth::Unknown);
        assert_window_from_colour(&quests, "hero", 0xF80000, 15, 15, Truth::Unknown);
    }

    #[test]
    fn window_gate_mort_myre_true_on_yellow() {
        let Some(quests) = catalog() else {
            return;
        };
        assert_window_from_colour(&quests, "druidspirit", 0xF8F800, 1, 110, Truth::True);
    }

    #[test]
    fn completed_priest_window_stays_weak_after_reachable_writer() {
        let Some(quests) = catalog() else {
            return;
        };
        // drezel.rs2:8-12,138-147 can change the completed stage from 60 to 61.
        // temple_guardian.rs2:1-12 also has a death-event reset; its separate
        // attack guard does not prove that event unreachable to the extractor.
        assert!(!quests.quest("priestperil").unwrap().colour_authoritative);
        assert_window_from_colour(&quests, "priestperil", 0x00F800, 0, 61, Truth::True);
        assert_window_from_colour(&quests, "priestperil", 0x00F800, 61, 61, Truth::Unknown);
    }

    #[test]
    fn red_window_answers_follow_colour_authority() {
        // drezel.rs2:147 can write Priest stage61 after completion60.
        // Keep the authoritative Druid Spirit false control separately.
        let Some(quests) = catalog() else {
            return;
        };
        for (quest, min, max, expected) in [
            ("priestperil", 61, 61, Truth::Unknown),
            ("druidspirit", 1, 110, Truth::False),
        ] {
            assert_window_from_colour(&quests, quest, 0xF80000, min, max, expected);
        }
    }

    #[test]
    fn authoritative_colour_windows_keep_positive_and_negative_bounds() {
        let Some(quests) = catalog() else {
            return;
        };
        assert!(quests.quest("druidspirit").unwrap().colour_authoritative);
        for (colour, min, max, expected) in [
            (0xF80000, 0, 9, Truth::True),
            (0xF80000, 1, 110, Truth::False),
            (0xF8F800, 1, 109, Truth::True),
            (0xF8F800, 1, 9, Truth::Unknown),
            (0x00F800, 1, 109, Truth::False),
            (0x00F800, 108, 110, Truth::True),
        ] {
            assert_window_from_colour(&quests, "druidspirit", colour, min, max, expected);
        }
    }

    #[test]
    fn window_gate_unknown_for_upass_row() {
        let Some(quests) = catalog() else {
            return;
        };
        assert_window_from_colour(&quests, "upass", 0xF8F800, 1, 21, Truth::Unknown);
    }

    #[test]
    fn window_gate_unknown_for_transmitted_misc_quest() {
        let Some(selected) = crate::game_data::test_data(ClientRevision::R289) else {
            return;
        };
        let mut facts = selected.quest_identity().unwrap().clone();
        let misc = facts.rows.iter_mut().find(|row| row.id == "misc").unwrap();
        assert_eq!(misc.transmission, Transmission::PostedByContent);
        // Isolate transmission from the independent colour-authority guard.
        misc.colour_authoritative = true;
        let quests = QuestCatalog::from_identity(Some(&facts)).unwrap();
        assert_window_from_colour(&quests, "misc", 0xF8F800, 1, 100, Truth::Unknown);
    }

    #[test]
    fn window_gate_unknown_without_domain() {
        let Some(selected) = crate::game_data::test_data(ClientRevision::R289) else {
            return;
        };
        let mut facts = selected.quest_identity().unwrap().clone();
        facts
            .rows
            .iter_mut()
            .find(|row| row.id == "druidspirit")
            .unwrap()
            .stage_domain = None;
        let quests = QuestCatalog::from_identity(Some(&facts)).unwrap();
        assert_window_from_colour(&quests, "druidspirit", 0xF8F800, 1, 110, Truth::Unknown);
    }

    #[test]
    fn complete_gate_unchanged() {
        let Some(quests) = catalog() else {
            return;
        };
        let observed = stamp(run(), 10, 4);
        for quest in ["priestperil", "druidspirit", "upass", "misc"] {
            let facts = quests.quest(quest).unwrap();
            for (colour, expected) in [
                (0x00F800, Truth::True),
                (0xF8F800, Truth::False),
                (0xF80000, Truth::False),
            ] {
                let snapshot = snapshot(vec![row(facts.display.to_string(), colour)], true);
                let provider = evidence(&quests, &snapshot, observed);
                assert_eq!(
                    provider.test_gate(&complete_gate(facts.id.clone()), observed),
                    expected
                );
            }
        }
    }

    #[test]
    fn shared_lookup_returns_the_observed_status_and_stamp() {
        let Some(quests) = catalog() else {
            return;
        };
        let (quest, display) = quest(&quests);
        let observed = stamp(run(), 10, 4);
        let snapshot = snapshot(vec![row(display, 0xF8F800)], true);

        let status = quest_list_status(
            &quests,
            quest.0.as_ref(),
            SnapshotView::new(Some(&snapshot), observed),
        )
        .expect("quest row is observed");
        assert_eq!(status.value, QuestListStatus::InProgress);
        assert_eq!(status.stamp, observed);
    }
}
