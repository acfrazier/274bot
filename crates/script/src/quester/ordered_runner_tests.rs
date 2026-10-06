//! `order: "ordered"` sequences: the in-memory cursor advances through steps
//! nothing observable distinguishes, replays after a restart, jumps proven
//! skips, holds on a failure, and resets on a stage change. Authored
//! sequences still re-select from the top at every boundary.
use super::tests::status_fixture;
use super::*;
use crate::native::ledger::Ledger;
use crate::quester::families::tests::with_tick_output;
use crate::quester::path::{PathDocument, PredicateDocument, SequenceOrder, StepDocument};
use api::snapshot::{GameSnapshot, QuestStatusView};

const NOT_STARTED: i32 = 0xF80000;
const IN_PROGRESS: i32 = 0xF8F800;
const TICK_CAP: u64 = 400;

fn never() -> PredicateDocument {
    PredicateDocument::Any(vec![])
}

fn always() -> PredicateDocument {
    PredicateDocument::All(vec![])
}

fn has_egg() -> PredicateDocument {
    PredicateDocument::Fact {
        kind: "has_item".into(),
        version: 1,
        args: serde_json::json!({"obj": "egg"}),
    }
}

/// A wait step that completes on its first poll; `settle` decides whether the
/// boundary settles (`All []`) or times out (`Any []`).
fn wait_step(id: &str, skip_if: PredicateDocument, settle: PredicateDocument) -> StepDocument {
    StepDocument {
        id: FactKey::new(id),
        kind: "wait".into(),
        version: 1,
        args: serde_json::json!({"until": {"All": []}, "max_ticks": 10}),
        comment: None,
        advances: Some(false),
        skip_if,
        settle,
    }
}

/// Cook's Assistant with five indistinguishable steps in the not-started
/// sequence and two in the in-progress one. `third_skip` replaces step 3's
/// skip and `second_settle` replaces step 2's settle.
fn document(
    order: SequenceOrder,
    third_skip: PredicateDocument,
    second_settle: PredicateDocument,
) -> PathDocument {
    let mut document = crate::quester::compile::decode_cook().unwrap();
    let role = &mut document.roles[0];
    let first = &mut role.sequences[0];
    first.order = order;
    first.steps = (1..=5)
        .map(|n| {
            let skip = if n == 3 { third_skip.clone() } else { never() };
            let settle = if n == 2 {
                second_settle.clone()
            } else {
                always()
            };
            wait_step(&format!("step-{n}"), skip, settle)
        })
        .collect();
    let second = &mut role.sequences[1];
    second.order = order;
    second.steps = (1..=2)
        .map(|n| wait_step(&format!("later-{n}"), never(), always()))
        .collect();
    document
}

#[derive(Default)]
struct Logs(Vec<String>);

impl NativeOutput for Logs {
    fn status(&mut self, _: ScriptStatus) {}
    fn paint(&mut self, _: Arc<crate::shim::ScriptPaint>) {}
    fn log(&mut self, _: api::hostlog::Level, message: &str) {
        self.0.push(message.to_owned());
    }
    fn settings_applied(&mut self, _: u64) {}
}

/// Drives the runner tick by tick and records every step that begins, in
/// order, by watching the settle window open for it.
struct Harness {
    script: Quester,
    snapshot: GameSnapshot,
    ledger: Option<Box<Ledger>>,
    logs: Logs,
    tick: u64,
    was_settling: bool,
    begun: Vec<String>,
}

impl Harness {
    fn new(document: PathDocument) -> Self {
        let (mut script, snapshot) = status_fixture(document);
        // Production starts with a progress read; the first adoption is the
        // fresh one that places the cursor at the top.
        script.needs_read = true;
        Self {
            script,
            snapshot,
            ledger: None,
            logs: Logs::default(),
            tick: 0,
            was_settling: false,
            begun: Vec::new(),
        }
    }

    fn colour(&mut self, colour: i32) {
        self.snapshot.seed_quest_statuses(
            vec![QuestStatusView {
                name: "Cook's Assistant".into(),
                component_id: 42,
                colour,
            }],
            true,
        );
    }

    fn hold_egg(&mut self) {
        let egg = self.script.selected.item_by_alias("egg").unwrap().id;
        self.snapshot.seed_inventory(
            vec![api::snapshot::ItemView {
                def: api::obj_names::ItemDefView {
                    id: egg,
                    name: Some("Egg".into()),
                    stackable: false,
                    members: false,
                    base_value: 0,
                    noted: false,
                    certificate_link: -1,
                    certificate_template: -1,
                },
                container: api::snapshot::ItemContainer::Inventory,
                action_family: api::snapshot::ItemActionFamily::Held,
                slot: 0,
                count: 1,
                actions: Vec::new(),
                component_id: 0,
            }],
            28,
        );
    }

    fn step(&mut self) {
        self.tick += 1;
        assert!(
            self.tick <= TICK_CAP,
            "the run did not reach the expected state"
        );
        with_tick_output(
            &self.snapshot,
            &mut self.ledger,
            self.tick,
            &mut self.logs,
            |t| {
                self.script.tick(t).unwrap();
            },
        );
        if self.script.settling && !self.was_settling {
            let step = self.script.current_step().expect("settling step");
            self.begun.push(step.id.0.to_string());
        }
        self.was_settling = self.script.settling;
    }

    fn run_until_begun(&mut self, count: usize) {
        while self.begun.len() < count {
            assert!(!self.script.parked, "parked before {count} steps began");
            self.step();
        }
    }

    fn run_until_parked(&mut self) {
        while !self.script.parked {
            self.step();
        }
    }
}

#[test]
fn ordered_sequence_advances_through_indistinguishable_steps_once() {
    let mut harness = Harness::new(document(SequenceOrder::Ordered, never(), always()));
    harness.run_until_parked();
    assert_eq!(
        harness.begun,
        ["step-1", "step-2", "step-3", "step-4", "step-5"],
        "every step runs exactly once, in order, without re-selecting step 1"
    );
    assert_eq!(
        harness.script.park_reason, "no step for stage",
        "a sequence that runs off its end without a stage change parks"
    );
    assert!(
        harness
            .logs
            .0
            .iter()
            .any(|line| line.contains("ordered sequence has no step left at cursor 5")),
        "the exhausted cursor is traced: {:?}",
        harness.logs.0
    );
}

#[test]
fn authored_sequence_still_reselects_from_the_top() {
    let mut harness = Harness::new(document(SequenceOrder::Authored, never(), always()));
    harness.run_until_begun(3);
    assert_eq!(harness.begun, ["step-1", "step-1", "step-1"]);
}

#[test]
fn restart_replays_an_ordered_sequence_from_its_first_step() {
    let mut harness = Harness::new(document(SequenceOrder::Ordered, never(), always()));
    harness.run_until_begun(2);
    assert_eq!(harness.begun, ["step-1", "step-2"]);
    harness.script.interrupt(Interrupt::Resume);
    harness.run_until_begun(4);
    assert_eq!(
        harness.begun,
        ["step-1", "step-2", "step-1", "step-2"],
        "Resume drops progress, so the sequence replays from step 1"
    );
    harness.script.interrupt(Interrupt::SessionReady);
    harness.run_until_begun(5);
    assert_eq!(harness.begun[4], "step-1");
}

#[test]
fn skip_if_jumps_the_ordered_cursor_forward() {
    let mut harness = Harness::new(document(SequenceOrder::Ordered, has_egg(), always()));
    harness.hold_egg();
    harness.run_until_parked();
    assert_eq!(harness.begun, ["step-1", "step-2", "step-4", "step-5"]);
    assert!(
        harness
            .logs
            .0
            .iter()
            .any(|line| line.contains("step step-3 skipped")),
        "the skipped step is traced: {:?}",
        harness.logs.0
    );
}

#[test]
fn stage_change_resets_the_ordered_cursor() {
    let mut harness = Harness::new(document(SequenceOrder::Ordered, never(), always()));
    harness.run_until_begun(2);
    harness.colour(IN_PROGRESS);
    harness.run_until_begun(3);
    assert_eq!(harness.begun, ["step-1", "step-2", "later-1"]);
    harness.colour(NOT_STARTED);
    harness.run_until_begun(4);
    assert_eq!(
        harness.begun[3], "step-1",
        "returning to a stage starts its sequence over, not at the old cursor"
    );
}

#[test]
fn failed_ordered_step_is_retried_in_place() {
    let mut harness = Harness::new(document(SequenceOrder::Ordered, never(), never()));
    harness.run_until_parked();
    assert_eq!(
        harness.begun,
        ["step-1", "step-2", "step-2", "step-2", "step-2", "step-2"],
        "a settle timeout retries the same step until the failure streak parks"
    );
    assert_eq!(
        harness.script.last_error.as_deref(),
        Some("step settle timeout")
    );
}
