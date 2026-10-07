//! Path authoring probes for tests: shadowed journal rules and the step
//! `select` would start for a stage. Compiled for unit tests and for the
//! `test-hooks` self dev-dependency that integration tests use.
use super::compile::{CompiledPath, PredicateContext};
use super::progress::ShadowedRule;
use super::select::{select, sequence_for_stage, SelectionDecision};
use crate::native::ledger::TickBudget;
use crate::native::{ActionContext, RetainedMemory};
use api::bank_memory::{BankMemory, Origin};
use api::game_data::SelectedGameData;
use api::quest_facts::QuestCatalog;
use api::quest_progress::{EvidenceStamp, ProgressFlag, QuestProgress};
use api::selected::{FactKey, Knowledge, RunKey, Truth};
use api::snapshot::{GameSnapshot, SnapshotView};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Every journal rule, fed exactly its own needles, must resolve to its own
/// stage; see [`super::progress::CompiledProgress::shadowed_rules`].
pub fn shadowed_rules(path: &CompiledPath) -> Vec<ShadowedRule> {
    path.progress.shadowed_rules()
}

/// Panics with every shadowed rule of `path`, newest-first as authored.
pub fn assert_no_shadowed_rules(path: &CompiledPath) {
    let shadowed = shadowed_rules(path);
    assert!(
        shadowed.is_empty(),
        "{} has {} shadowed journal rule(s):\n{}",
        path.id.0,
        shadowed.len(),
        shadowed
            .iter()
            .map(|rule| {
                format!(
                    "  rule[{}] {} resolves to {} with needles {:?}",
                    rule.index,
                    rule.stage.0,
                    rule.resolved
                        .as_ref()
                        .map_or("no rule", |stage| stage.0.as_ref()),
                    rule.needles
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// What `select` would do for a stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// This step would start.
    Step(FactKey),
    /// This step's `skip_if` is unknown, so nothing below it may start.
    Unknown(FactKey),
    /// Every candidate was skipped.
    Exhausted,
}

/// The evidence a selection probe runs against: the runner's own selection
/// inputs minus the live tick.
pub struct Probe<'a> {
    pub path: &'a CompiledPath,
    pub selected: &'a SelectedGameData,
    pub quests: &'a QuestCatalog,
    /// Resolved journal evidence, usually one [`progress_for_stage`] value.
    pub progress: &'a [QuestProgress],
    /// The host bank memory the runner's snapshot view carries (S3).
    pub bank: &'a BankMemory,
}

impl Probe<'_> {
    /// The step `select` starts for `stage` from `cursor` against `snapshot`:
    /// prelude first, then the stage's sequence. `cursor` only matters for an
    /// `ordered` sequence. Seed the snapshot's local player for `nearest`
    /// sequences and for `near`/`in_area` facts.
    pub fn choice(&self, stage: &str, cursor: usize, snapshot: &GameSnapshot) -> Choice {
        let sequence = sequence_for_stage(self.path, stage)
            .unwrap_or_else(|| panic!("{}: stage {stage} has no sequence", self.path.id.0));
        let pin = self.selected.selected_pin().expect("selected pin");
        let evidence = probe_stamp();
        let mut retained = RetainedMemory::default();
        let mut ledger = None;
        let mut budget = TickBudget::default();
        budget.observe(evidence.tick);
        let cx = ActionContext {
            evidence,
            pin: &pin,
            snapshot: SnapshotView::new(Some(snapshot), evidence).with_bank_memory(Some(self.bank)),
            retained: &mut retained,
            action_id: 0,
            active_now: Duration::from_millis(evidence.tick * 600),
            wall_now: Instant::now(),
            ledger: &mut ledger,
            budget: &mut budget,
            eligible: true,
            observed_walk_outcome_seq: 0,
        };
        let pred = PredicateContext {
            cx: &cx,
            quests: self.quests,
            progress: self.progress,
            required_after: evidence,
            chat_since: 0,
            outcome: None,
            pairs: None,
        };
        match select(self.path, sequence, cursor, &pred) {
            SelectionDecision::Selected(selection) => Choice::Step(selection.step.id.clone()),
            SelectionDecision::Unknown(selection) => Choice::Unknown(selection.step.id.clone()),
            SelectionDecision::Exhausted => Choice::Exhausted,
        }
    }
}

/// Journal evidence resolved to `stage` with the given `(flag, truth, count)`
/// rows, as the runner would adopt it.
pub fn progress_for_stage(
    path: &CompiledPath,
    selected: &SelectedGameData,
    stage: &str,
    flags: &[(&str, Truth, Option<u32>)],
) -> QuestProgress {
    let stage = FactKey::new(stage);
    QuestProgress {
        quest: path.id.clone(),
        stage: Knowledge::Known(stage.clone()),
        complete: if stage == path.colour_complete {
            Truth::True
        } else {
            Truth::False
        },
        signals: Arc::from(Vec::new()),
        flags: Arc::from(
            flags
                .iter()
                .map(|(flag, truth, count)| ProgressFlag {
                    flag: FactKey::new(flag),
                    truth: *truth,
                    count: *count,
                })
                .collect::<Vec<_>>(),
        ),
        evidence: probe_stamp(),
        binding: path.progress.binding.clone(),
        role: path.progress.role.clone(),
        rule: Knowledge::Known(stage),
        pin: selected.selected_pin().expect("selected pin"),
    }
}

/// A `Session` bank memory with no rows, so `bank_has` facts are proven
/// false instead of unknown.
pub fn known_empty_bank() -> BankMemory {
    BankMemory::seeded(&[], Origin::Session)
}

fn probe_stamp() -> EvidenceStamp {
    EvidenceStamp {
        run: RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        tick: 1,
        sequence: 1,
    }
}
