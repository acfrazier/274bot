//! `order: "ordered"` sequences: the in-memory cursor advances through steps
//! nothing observable distinguishes, replays after a restart, jumps proven
//! skips, holds on a failure, and resets on a stage change. Authored
//! sequences still re-select from the top at every boundary.
use super::tests::status_fixture;
use super::*;
use crate::native::ledger::Ledger;
use crate::native::NativeActions;
use crate::quester::compile::StepPlan;
use crate::quester::families::tests::{with_tick, with_tick_output};
use crate::quester::path::{PathDocument, PredicateDocument, SequenceOrder, StepDocument};
use api::snapshot::{GameSnapshot, QuestStatusView};
use std::sync::atomic::{AtomicUsize, Ordering};

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

/// Unknown until the equipment is observed, then false.
fn wears_egg() -> PredicateDocument {
    PredicateDocument::Fact {
        kind: "worn".into(),
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
/// sequence and two in the in-progress one. `skip` supplies step `n`'s skip
/// (1-based) and `second_settle` replaces step 2's settle.
fn document(
    order: SequenceOrder,
    skip: impl Fn(usize) -> PredicateDocument,
    second_settle: PredicateDocument,
) -> PathDocument {
    let mut document = crate::quester::compile::decode_cook().unwrap();
    let role = &mut document.roles[0];
    let first = &mut role.sequences[0];
    first.order = order;
    first.steps = (1..=5)
        .map(|n| {
            let settle = if n == 2 {
                second_settle.clone()
            } else {
                always()
            };
            wait_step(&format!("step-{n}"), skip(n), settle)
        })
        .collect();
    let second = &mut role.sequences[1];
    second.order = order;
    second.steps = (1..=2)
        .map(|n| wait_step(&format!("later-{n}"), never(), always()))
        .collect();
    document
}

fn random_event() -> DetectedRandom {
    DetectedRandom {
        kind: api::random::RandomKind::Dialog,
        name: "genie".into(),
        ours: true,
        npc_index: Some(0),
    }
}

/// A `progress_reader` whose reads resolve, in order, to scripted colours and
/// then repeat the last one. Each read stays pending for one tick so its
/// evidence is strictly newer than the read's start, as `valid_progress`
/// requires of an owned read.
struct ScriptedReader {
    reads: Vec<QuestProgress>,
    next: AtomicUsize,
}

impl StepPlan for ScriptedReader {
    fn begin(&self, _: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        let index = self
            .next
            .fetch_add(1, Ordering::Relaxed)
            .min(self.reads.len() - 1);
        Ok(Box::new(ScriptedRead {
            progress: self.reads[index].clone(),
            pending: true,
        }))
    }
}

struct ScriptedRead {
    progress: QuestProgress,
    pending: bool,
}

impl StepRun for ScriptedRead {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        if std::mem::take(&mut self.pending) {
            return Poll::Pending;
        }
        let evidence = cx.tick.cx.evidence();
        let mut progress = self.progress.clone();
        progress.evidence = evidence;
        Poll::Ready(Ok(StepOutcome {
            evidence,
            progress: Some(Arc::new(progress)),
            receipt: None,
        }))
    }

    fn cancel(&mut self, _: &mut NativeActions) {}
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

    fn drop_egg(&mut self) {
        self.snapshot.seed_inventory(Vec::new(), 28);
    }

    fn observe_empty_equipment(&mut self) {
        self.snapshot.seed_equipment(Vec::new());
    }

    /// Replaces colour reads with a `progress_reader` that answers `colours`
    /// in order, repeating the last one.
    fn script_reads(&mut self, colours: &[QuestListStatus]) {
        let reads = with_tick(&self.snapshot, &mut None, 0, |tick| {
            colours
                .iter()
                .map(|&colour| {
                    resolve_colour(
                        &self.script.path,
                        colour,
                        tick.cx.evidence(),
                        Arc::new(tick.cx.pin().clone()),
                    )
                })
                .collect()
        });
        let path = Arc::get_mut(&mut self.script.path).expect("unshared path");
        let template = &path.sequences[0].steps[0];
        let reader = CompiledStep {
            id: FactKey::new("scripted-reader"),
            kind: Arc::clone(&template.kind),
            comment: None,
            loadout: None,
            tactic: None,
            advances: false,
            skip_if: Arc::clone(&template.skip_if),
            skip_if_summary: Arc::clone(&template.skip_if_summary),
            settle: Arc::clone(&template.settle),
            plan: Arc::new(ScriptedReader {
                reads,
                next: AtomicUsize::new(0),
            }),
        };
        path.progress_reader = Some(Box::new(reader));
    }

    fn active_now(&self) -> Duration {
        Duration::from_millis(self.tick * 600)
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

    fn run_until_logged(&mut self, needle: &str) {
        while !self.logged(needle) {
            assert!(!self.script.parked, "parked before logging {needle:?}");
            self.step();
        }
    }

    fn logged(&self, needle: &str) -> bool {
        self.logs.0.iter().any(|line| line.contains(needle))
    }
}

#[test]
fn ordered_sequence_advances_through_indistinguishable_steps_once() {
    let mut harness = Harness::new(document(SequenceOrder::Ordered, |_| never(), always()));
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
        harness.logged("ordered sequence has no step left at cursor 5"),
        "the exhausted cursor is traced: {:?}",
        harness.logs.0
    );
}

#[test]
fn authored_sequence_still_reselects_from_the_top() {
    let mut harness = Harness::new(document(SequenceOrder::Authored, |_| never(), always()));
    harness.run_until_begun(3);
    assert_eq!(harness.begun, ["step-1", "step-1", "step-1"]);
}

#[test]
fn restart_replays_an_ordered_sequence_from_its_first_step() {
    let mut harness = Harness::new(document(SequenceOrder::Ordered, |_| never(), always()));
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
    let mut harness = Harness::new(document(
        SequenceOrder::Ordered,
        |n| if n == 3 { has_egg() } else { never() },
        always(),
    ));
    harness.hold_egg();
    harness.run_until_parked();
    assert_eq!(harness.begun, ["step-1", "step-2", "step-4", "step-5"]);
    assert!(
        harness.logged("step step-3 skipped"),
        "the skipped step is traced: {:?}",
        harness.logs.0
    );
}

#[test]
fn proven_skip_stays_committed_while_a_later_skip_is_unknown() {
    // Step 1 is proven done; step 2's skip is unknown until the equipment is
    // observed. Selection waits on step 2, and that wait must not forget
    // step 1's proven skip.
    let mut harness = Harness::new(document(
        SequenceOrder::Ordered,
        |n| match n {
            1 => has_egg(),
            2 => wears_egg(),
            _ => never(),
        },
        always(),
    ));
    harness.hold_egg();
    harness.run_until_logged("step step-2 skip predicate waiting");
    assert_eq!(
        harness.script.cursor, 1,
        "the proven skip is committed before the wait"
    );
    // Step 1's evidence flips while step 2 is still unknown: the skipped
    // prefix must not become runnable again.
    harness.drop_egg();
    for _ in 0..3 {
        harness.step();
    }
    assert!(
        harness.begun.is_empty(),
        "no step may start while step 2 is unknown"
    );
    assert_eq!(
        harness.script.cursor, 1,
        "the flipped evidence does not move the cursor back"
    );
    harness.observe_empty_equipment();
    harness.run_until_parked();
    assert_eq!(
        harness.begun,
        ["step-2", "step-3", "step-4", "step-5"],
        "step 1 was proven done once and never re-runs without a reset"
    );
}

#[test]
fn all_skipped_suffix_commits_the_cursor_to_the_end() {
    // Steps 3-5 are proven done once steps 1-2 settle, so selection exhausts
    // the sequence; the confirming read must not re-evaluate them.
    let mut harness = Harness::new(document(
        SequenceOrder::Ordered,
        |n| if n >= 3 { has_egg() } else { never() },
        always(),
    ));
    harness.hold_egg();
    harness.run_until_begun(2);
    harness.run_until_logged("ordered sequence has no step left at cursor 5");
    assert_eq!(harness.script.cursor, 5);
    // The evidence flips between the exhausted selection and its confirming
    // read; the skipped suffix stays skipped.
    harness.drop_egg();
    harness.run_until_parked();
    assert_eq!(
        harness.begun,
        ["step-1", "step-2"],
        "an all-skipped suffix never runs after the cursor passed it"
    );
    assert_eq!(harness.script.park_reason, "no step for stage");
}

#[test]
fn unresolved_first_read_after_resume_still_resets_the_cursor() {
    // After Resume the first owned read is unresolved; the Known read that
    // follows must still replay the sequence from step 1.
    let mut harness = Harness::new(document(SequenceOrder::Ordered, |_| never(), always()));
    harness.script_reads(&[
        QuestListStatus::NotStarted,
        QuestListStatus::Unknown,
        QuestListStatus::NotStarted,
    ]);
    harness.run_until_begun(2);
    assert_eq!(harness.begun, ["step-1", "step-2"]);
    harness.script.interrupt(Interrupt::Resume);
    while !harness
        .script
        .progress
        .as_ref()
        .is_some_and(|progress| matches!(progress.stage, Knowledge::Unknown(_)))
    {
        assert!(!harness.script.parked);
        harness.step();
    }
    assert_eq!(
        harness.script.cursor, 0,
        "the fresh adoption resets the cursor even while its stage is unresolved"
    );
    harness.run_until_begun(3);
    assert!(
        harness.logged("stage unknown → "),
        "the Known read follows the unresolved one: {:?}",
        harness.logs.0
    );
    assert_eq!(
        harness.begun[2], "step-1",
        "an Unknown-then-Known resume replays from step 1, not the stale cursor"
    );
}

#[test]
fn random_event_replays_an_ordered_sequence_unless_paired_work_is_pending() {
    let mut harness = Harness::new(document(SequenceOrder::Ordered, |_| never(), always()));
    harness.run_until_begun(2);
    assert_eq!(harness.script.on_random(&random_event()), RandomClaim::Host);
    assert!(harness.script.progress.is_none());
    harness.run_until_begun(4);
    assert_eq!(
        harness.begun,
        ["step-1", "step-2", "step-1", "step-2"],
        "an ordinary random event drops progress, so the sequence replays from step 1"
    );
    // Let step 2 settle so the cursor sits past it, then model partner
    // admission in flight, which is pending paired work.
    while harness.script.settling {
        harness.step();
    }
    assert_eq!(harness.script.cursor, 2);
    harness.script.pair_admission_since = Some(harness.active_now());
    assert_eq!(harness.script.on_random(&random_event()), RandomClaim::Host);
    assert!(harness.script.progress.is_some());
    assert_eq!(harness.script.cursor, 2);
    harness.run_until_begun(5);
    assert_eq!(
        harness.begun[4], "step-3",
        "while paired work is pending a random event keeps the cursor: replaying mid-pairing would desync the partners"
    );
}

#[test]
fn stage_change_resets_the_ordered_cursor() {
    let mut harness = Harness::new(document(SequenceOrder::Ordered, |_| never(), always()));
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
    let mut harness = Harness::new(document(SequenceOrder::Ordered, |_| never(), never()));
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
