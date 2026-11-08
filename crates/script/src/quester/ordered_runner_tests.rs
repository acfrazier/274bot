//! `order: "ordered"` sequences: the in-memory cursor advances through steps
//! nothing observable distinguishes, replays after a restart, jumps proven
//! skips, holds on a failure, and resets on a stage change. Authored
//! sequences still re-select from the top at every boundary.
use super::tests::status_fixture;
use super::*;
use crate::native::ledger::{Ledger, TickBudget};
use crate::native::NativeActions;
use crate::quester::compile::{PredicateContext, PredicatePlan, StepPlan, StepRun};
use crate::quester::families::tests::{
    local_player, with_tick, with_tick_budget_output, with_tick_output,
};
use crate::quester::path::{PathDocument, PredicateDocument, SequenceOrder, StepDocument};
use api::snapshot::{GameSnapshot, QuestStatusView, StatView};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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
        max_attempts: None,
        stop: None,
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

fn maze_event() -> DetectedRandom {
    DetectedRandom {
        kind: api::random::RandomKind::Maze,
        name: "maze".into(),
        ours: true,
        npc_index: None,
    }
}

/// A step that stays in flight until released and counts the progress reads
/// it was handed.
struct HeldStep {
    release: Arc<AtomicBool>,
    reads: Arc<AtomicUsize>,
}

impl StepPlan for HeldStep {
    fn begin(&self, _: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        Ok(Box::new(HeldRun {
            release: Arc::clone(&self.release),
            reads: Arc::clone(&self.reads),
        }))
    }
}

struct HeldRun {
    release: Arc<AtomicBool>,
    reads: Arc<AtomicUsize>,
}

impl StepRun for HeldRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        if !self.release.load(Ordering::Acquire) {
            return Poll::Pending;
        }
        Poll::Ready(Ok(StepOutcome {
            progress: None,
            evidence: cx.tick.cx.evidence(),
            receipt: None,
        }))
    }

    fn cancel(&mut self, _: &mut NativeActions) {}

    fn progress_read_completed(&mut self, _: Duration) {
        self.reads.fetch_add(1, Ordering::Relaxed);
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

/// A step that completes on the poll after its begin, so one step boundary
/// passes per tick and these cursor tests can act between steps. A step that
/// completes on its first poll hands over within its tick (TICK-FIX #5); the
/// runner tests cover that chaining.
struct OneTick;

impl StepPlan for OneTick {
    fn begin(&self, _: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        Ok(Box::new(OneTickRun { pending: true }))
    }
}

struct OneTickRun {
    pending: bool,
}

impl StepRun for OneTickRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        if std::mem::take(&mut self.pending) {
            return Poll::Pending;
        }
        Poll::Ready(Ok(StepOutcome {
            progress: None,
            evidence: cx.tick.cx.evidence(),
            receipt: None,
        }))
    }

    fn cancel(&mut self, _: &mut NativeActions) {}
}
struct CountedStep(Arc<AtomicUsize>);

impl StepPlan for CountedStep {
    fn begin(
        &self,
        _: &mut crate::quester::compile::StepContext<'_, '_>,
    ) -> Result<Box<dyn StepRun>, ActionError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(Box::new(OneTickRun { pending: false }))
    }

    fn settle_timeout(&self) -> std::time::Duration {
        std::time::Duration::ZERO
    }
}

#[derive(Clone, Copy)]
enum HardPollFailure {
    Blocked,
    NeedsEvidence,
    UserInput,
}

impl HardPollFailure {
    fn error(self) -> ActionError {
        match self {
            Self::Blocked => ActionError::Blocked(Arc::from("action blocked")),
            Self::NeedsEvidence => ActionError::NeedsEvidence(Vec::new().into()),
            Self::UserInput => ActionError::UserInput,
        }
    }
}

struct HardPoll(HardPollFailure);

impl StepPlan for HardPoll {
    fn begin(&self, _: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        Ok(Box::new(HardPollRun(self.0)))
    }
}

struct HardPollRun(HardPollFailure);

impl StepRun for HardPollRun {
    fn poll(&mut self, _: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        Poll::Ready(Err(self.0.error()))
    }

    fn cancel(&mut self, _: &mut NativeActions) {}
}

struct AttemptGate {
    attempts: Arc<AtomicUsize>,
    settle_at: Option<usize>,
}

impl PredicatePlan for AttemptGate {
    fn evaluate(&self, _: &PredicateContext<'_, '_>) -> api::selected::Truth {
        if self
            .settle_at
            .is_some_and(|settle_at| self.attempts.load(Ordering::Relaxed) >= settle_at)
        {
            api::selected::Truth::True
        } else {
            api::selected::Truth::False
        }
    }
}

#[derive(Default)]
struct Logs(Vec<String>, Vec<ScriptStatus>);

impl NativeOutput for Logs {
    fn status(&mut self, status: ScriptStatus) {
        self.1.push(status);
    }
    fn paint(&mut self, _: Arc<crate::paint::ScriptPaint>) {}
    fn log(&mut self, _: api::hostlog::Level, message: &str) {
        self.0.push(message.to_owned());
    }
    fn settings_applied(&mut self, _: u64) {}
}

/// Drives the runner tick by tick and records every step that begins, in
/// order, from the runner's own begin record. A completed step may settle
/// and hand over to the next one within its tick (TICK-FIX #5), so the settle
/// window is no longer visible between ticks.
struct Harness {
    script: Quester,
    snapshot: GameSnapshot,
    ledger: Option<Box<Ledger>>,
    /// The slot's budget, which outlives each tick and carries the guardian
    /// hold the test starts and ends on it.
    budget: TickBudget,
    logs: Logs,
    tick: u64,
    advance_xp: bool,
    begun: Vec<String>,
}

impl Harness {
    fn new(document: PathDocument) -> Self {
        let (mut script, snapshot) = status_fixture(document);
        // Production starts with a progress read; the first adoption is the
        // fresh one that places the cursor at the top.
        script.needs_read = true;
        let path = Arc::get_mut(&mut script.path).expect("unshared path");
        for step in path
            .sequences
            .iter_mut()
            .flat_map(|sequence| sequence.steps.iter_mut())
        {
            step.plan = Arc::new(OneTick);
        }
        Self {
            script,
            snapshot,
            ledger: None,
            budget: TickBudget::default(),
            logs: Logs::default(),
            tick: 0,
            advance_xp: false,
            begun: Vec::new(),
        }
    }

    /// The player stands on `tile` from the next frame on.
    fn stand(&mut self, tile: api::WorldTile) {
        self.snapshot.seed_local_player(local_player(tile));
    }

    /// The host's per-frame position report (`SlotScript::observe_player_fix`).
    fn observe_fix(&mut self) {
        let tile = self
            .snapshot
            .local_player()
            .map(|player| player.player.network);
        self.budget.observe_fix(self.tick, tile);
    }

    /// A guardian hold: the script does not run, the host reports one frame
    /// per tile in `frames` (one tick each), and the hold ends.
    fn guardian_hold(&mut self, frames: &[api::WorldTile]) {
        self.budget.hold_begin();
        for tile in frames {
            self.tick += 1;
            self.stand(*tile);
            self.observe_fix();
        }
        self.budget.hold_end();
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
                .map(|&colour| resolve_colour(&self.script.path, colour, tick.cx.evidence()))
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
            attempt_budget: None,
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

        if self.advance_xp {
            self.snapshot.seed_stats(vec![StatView {
                index: 0,
                name: api::snapshot::stat_name(0).into(),
                effective: 1,
                base: 1,
                xp: self.tick as i32,
                used: api::snapshot::stat_used(0),
            }]);
        }
        self.observe_fix();
        with_tick_budget_output(
            &self.snapshot,
            &mut self.ledger,
            &mut self.budget,
            self.tick,
            &mut self.logs,
            |t| {
                self.script.tick(t).unwrap();
            },
        );
        self.begun = self.script.begun.iter().map(|id| id.to_string()).collect();
    }

    fn run_until_begun(&mut self, count: usize) {
        while self.begun.len() < count {
            assert!(
                !self.script.parked,
                "parked before {count} steps began: {:#?}",
                self.logs.0
            );
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
fn retry_harness(settle_at: Option<usize>) -> (Harness, Arc<AtomicUsize>) {
    let mut document = document(SequenceOrder::Ordered, |_| never(), never());
    let sequence = &mut document.roles[0].sequences[0];
    sequence.steps.truncate(1);
    sequence.steps[0].max_attempts = Some(32);
    sequence.steps[0].stop = Some("retry-limit-reached".into());

    let mut harness = Harness::new(document);
    harness.script_reads(&[QuestListStatus::NotStarted]);
    harness.advance_xp = true;
    let attempts = Arc::new(AtomicUsize::new(0));
    let path = Arc::get_mut(&mut harness.script.path).expect("unshared path");
    let step = &mut path.sequences[0].steps[0];
    step.plan = Arc::new(CountedStep(Arc::clone(&attempts)));
    step.settle = Arc::new(AttemptGate {
        attempts: Arc::clone(&attempts),
        settle_at,
    });
    (harness, attempts)
}

#[test]
fn ordered_sequence_advances_through_indistinguishable_steps_once() {
    if !api::test_support::game_data_available() {
        return;
    }
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
    if !api::test_support::game_data_available() {
        return;
    }
    let mut harness = Harness::new(document(SequenceOrder::Authored, |_| never(), always()));
    harness.run_until_begun(3);
    assert_eq!(harness.begun, ["step-1", "step-1", "step-1"]);
}

#[test]
fn restart_replays_an_ordered_sequence_from_its_first_step() {
    if !api::test_support::game_data_available() {
        return;
    }
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
    if !api::test_support::game_data_available() {
        return;
    }
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
    if !api::test_support::game_data_available() {
        return;
    }
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
    if !api::test_support::game_data_available() {
        return;
    }
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
    if !api::test_support::game_data_available() {
        return;
    }
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

/// QUESTER-READ-KEEPS-STEP: the engine's 500-tick login timer spawns a random
/// event while a step runs (`login.rs2:40`, `macro_events.rs2:2-12`), and the
/// quest journal read that follows must find the same stage with the same
/// step still in flight. Dig Site `steal-green-sample` and Ernest
/// `unlock-closet` began again from cursor 0 here, with no failure.
#[test]
fn random_event_then_a_same_stage_read_keeps_the_step_in_flight() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut harness = Harness::new(document(SequenceOrder::Ordered, |_| never(), always()));
    let release = Arc::new(AtomicBool::new(false));
    let reads = Arc::new(AtomicUsize::new(0));
    Arc::get_mut(&mut harness.script.path)
        .expect("unshared path")
        .sequences[0]
        .steps[1]
        .plan = Arc::new(HeldStep {
        release: Arc::clone(&release),
        reads: Arc::clone(&reads),
    });
    harness.run_until_begun(2);
    assert!(harness.script.step.is_some());
    assert_eq!(harness.script.cursor, 1);
    assert_eq!(harness.script.on_random(&random_event()), RandomClaim::Host);
    assert!(
        harness.script.step.is_some(),
        "the event does not end step 2"
    );
    assert!(harness.script.progress.is_some());
    // The journal read of the same stage, as the live runs sent it.
    harness.script.needs_read = true;
    for _ in 0..6 {
        harness.step();
    }
    assert_eq!(
        harness.begun,
        ["step-1", "step-2"],
        "the read of an unchanged stage must not begin step 2 again"
    );
    assert!(harness.script.step.is_some());
    assert!(harness.script.progress.is_some());
    assert_eq!(harness.script.cursor, 1);
    assert!(!harness.script.needs_read);
    assert_eq!(
        reads.load(Ordering::Relaxed),
        1,
        "the in-flight step takes the finished read through progress_read_completed"
    );
    release.store(true, Ordering::Release);
    harness.run_until_begun(3);
    assert_eq!(
        harness.begun[2], "step-3",
        "the sequence carries on past the kept step"
    );
}

fn tile(x: i32, z: i32) -> api::WorldTile {
    api::WorldTile { x, z, level: 0 }
}

/// A harness whose second step stays in flight until its plan is released,
/// with the player standing at `(3200, 3200)`.
fn held_second_step() -> (Harness, Arc<AtomicBool>) {
    let mut harness = Harness::new(document(SequenceOrder::Ordered, |_| never(), always()));
    let release = Arc::new(AtomicBool::new(false));
    Arc::get_mut(&mut harness.script.path)
        .expect("unshared path")
        .sequences[0]
        .steps[1]
        .plan = Arc::new(HeldStep {
        release: Arc::clone(&release),
        reads: Arc::new(AtomicUsize::new(0)),
    });
    harness.stand(tile(3200, 3200));
    harness.run_until_begun(2);
    assert!(harness.script.step.is_some());
    (harness, release)
}

/// The classification is by what the frames of the hold show, never by the
/// event's name. A teleport ends the step handle only: progress and the
/// cursor stay, so selection resumes at the same step (R2). The Genie's
/// ignored-event teleport (`macro_event_genie.rs2:29-35`) is not a Maze or a
/// Mime event, and moves the player the same way (`macro_events.rs2:120-137`).
#[test]
fn a_teleport_seen_during_a_guardian_hold_drops_only_the_step_whatever_the_event() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut harness, _release) = held_second_step();
    harness.script.on_random(&random_event());
    harness.guardian_hold(&[tile(3200, 3200), tile(3280, 3290), tile(3281, 3290)]);
    harness.step();
    assert!(harness.script.progress.is_some(), "progress stays");
    assert_eq!(harness.script.cursor, 1, "the cursor stays");
    harness.run_until_begun(3);
    assert_eq!(
        harness.begun,
        ["step-1", "step-2", "step-2"],
        "the sequence resumes at the kept cursor, not at step 1"
    );
}

/// A plane change is a teleport whatever its distance (stairs are the
/// player's own click; the guardian climbs none).
#[test]
fn a_plane_change_during_a_guardian_hold_is_a_teleport() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut harness, _release) = held_second_step();
    harness.script.on_random(&random_event());
    harness.guardian_hold(&[api::WorldTile {
        level: 1,
        ..tile(3200, 3200)
    }]);
    harness.step();
    harness.run_until_begun(3);
    assert_eq!(harness.begun, ["step-1", "step-2", "step-2"]);
}

/// The Maze and Mime events return the player to the tile they left
/// (`macro_event_maze.rs2:15-18`, `macro_event_mime.rs2:9-11`), so the hold's
/// last frame shows the tile the first one did. The jump is in between.
#[test]
fn a_teleport_out_and_back_to_the_same_tile_inside_the_hold_is_still_a_teleport() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut harness, _release) = held_second_step();
    harness.script.on_random(&maze_event());
    harness.guardian_hold(&[tile(2900, 4555), tile(2900, 4556), tile(3200, 3200)]);
    harness.step();
    harness.run_until_begun(3);
    assert_eq!(harness.begun, ["step-1", "step-2", "step-2"]);
}

/// A guardian walk away from a hostile event covers a few tiles at running
/// speed: it keeps the step, and the step's action resumes by itself. The
/// event's name changes nothing: a Maze-named event that shows no jump keeps
/// the step as well.
#[test]
fn a_guardian_walk_of_a_few_tiles_keeps_the_step_for_every_event_name() {
    if !api::test_support::game_data_available() {
        return;
    }
    for event in [random_event(), maze_event()] {
        let (mut harness, release) = held_second_step();
        harness.script.on_random(&event);
        harness.guardian_hold(&[tile(3202, 3200), tile(3204, 3200), tile(3206, 3201)]);
        for _ in 0..4 {
            harness.step();
        }
        assert!(harness.script.step.is_some(), "{event:?}: the step stays");
        assert!(harness.script.progress.is_some());
        assert_eq!(harness.script.cursor, 1);
        assert_eq!(
            harness.begun,
            ["step-1", "step-2"],
            "{event:?}: no new begin, so the ordered guide is not restarted"
        );
        release.store(true, Ordering::Release);
        harness.run_until_begun(3);
        assert_eq!(harness.begun[2], "step-3");
    }
}

/// Partner work pending keeps the step even through a teleport: replaying
/// mid-pairing would desync the partners.
#[test]
fn a_teleport_keeps_the_step_while_partner_work_is_pending() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut harness, release) = held_second_step();
    harness.script.pair_admission_since = Some(harness.active_now());
    harness.script.on_random(&random_event());
    harness.guardian_hold(&[tile(3280, 3290)]);
    harness.step();
    assert!(harness.script.step.is_some(), "the step stays");
    assert!(harness.script.progress.is_some());
    assert_eq!(harness.script.cursor, 1);
    release.store(true, Ordering::Release);
    harness.run_until_begun(3);
    assert_eq!(
        harness.begun,
        ["step-1", "step-2", "step-3"],
        "the next begin is step 3, not a replay of step 2"
    );
}

/// With `settings.random_events` off the guardian neither holds nor acts
/// (`RandomStatus::hold` and `ours` need the setting), so the slot sees no
/// hold edge and the event interrupts nothing. The Quester keeps the step,
/// progress and cursor and does not classify a teleport that no hold
/// surrounds, even one the position reports show.
#[test]
fn with_random_events_off_the_event_changes_nothing_for_the_step() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut harness, release) = held_second_step();
    assert_eq!(harness.script.on_random(&random_event()), RandomClaim::Host);
    // No `hold_begin`: the guardian is off. The player is moved anyway.
    for z in [3201, 3290, 3291] {
        harness.step();
        harness.stand(tile(3200, z));
    }
    harness.step();
    assert!(harness.script.step.is_some());
    assert!(harness.script.progress.is_some());
    assert_eq!(harness.script.cursor, 1);
    assert_eq!(harness.begun, ["step-1", "step-2"]);
    release.store(true, Ordering::Release);
    harness.run_until_begun(3);
    assert_eq!(harness.begun[2], "step-3");
}

#[test]
fn stage_change_resets_the_ordered_cursor() {
    if !api::test_support::game_data_available() {
        return;
    }
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
    if !api::test_support::game_data_available() {
        return;
    }
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

#[test]
fn budgeted_step_can_settle_after_31_failures_on_attempt_32() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut harness, attempts) = retry_harness(Some(32));
    while harness
        .script
        .last_attempt_receipt
        .as_ref()
        .and_then(|receipt| receipt.attempt_count())
        != Some(32)
    {
        assert!(!harness.script.parked, "budgeted step parked early");
        harness.step();
    }

    assert_eq!(attempts.load(Ordering::Relaxed), 32);
    assert!(!harness.script.parked);
    let receipt = harness
        .script
        .last_attempt_receipt
        .as_ref()
        .expect("successful final attempt is receipted");
    assert_eq!(receipt.attempt_count(), Some(32));
    assert_eq!(
        receipt.attempt_stop().map(|stop| stop.as_ref()),
        Some("retry-limit-reached")
    );
    let status = harness
        .logs
        .1
        .last()
        .expect("final attempt publishes status");
    assert!(status.fields.iter().any(|field| {
        field.key == "step_attempts" && matches!(&field.value, StatusValue::Integer(32))
    }));
    assert!(
        !status.fields.iter().any(|field| field.key == "step_stop"),
        "a successful settle did not reach the configured stop"
    );
}

#[test]
fn budgeted_step_parks_on_its_named_stop_after_32_failures() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut harness, attempts) = retry_harness(None);
    harness.run_until_parked();

    assert_eq!(attempts.load(Ordering::Relaxed), 32);
    assert_eq!(harness.script.step_attempts, 32);
    assert_eq!(
        harness
            .script
            .parked_step
            .as_ref()
            .map(|step| step.id.as_ref()),
        Some("step-1")
    );
    assert!(harness
        .script
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("retry-limit-reached")));
    let receipt = harness
        .script
        .last_attempt_receipt
        .as_ref()
        .expect("budget exhaustion is receipted");
    assert_eq!(receipt.attempt_count(), Some(32));
    assert_eq!(
        receipt.attempt_stop().map(|stop| stop.as_ref()),
        Some("retry-limit-reached")
    );
    let status = harness.logs.1.last().expect("parked status is published");
    assert_eq!(status.phase, NativePhase::Blocked);
    assert!(status.fields.iter().any(|field| {
        field.key == "step_attempts" && matches!(&field.value, StatusValue::Integer(32))
    }));
    assert!(status.fields.iter().any(|field| {
        field.key == "step_stop"
            && matches!(&field.value, StatusValue::Text(stop) if stop.as_ref() == "retry-limit-reached")
    }));
}

#[test]
fn budgeted_hard_poll_failure_does_not_report_an_unspent_stop() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut document = document(SequenceOrder::Ordered, |_| never(), never());
    document.roles[0].sequences[0].steps.truncate(1);
    document.roles[0].sequences[0].steps[0].max_attempts = Some(2);
    document.roles[0].sequences[0].steps[0].stop = Some("blocked-stop".into());

    let mut harness = Harness::new(document);
    harness.script_reads(&[QuestListStatus::NotStarted]);
    let path = Arc::get_mut(&mut harness.script.path).expect("unshared path");
    path.sequences[0].steps[0].plan = Arc::new(HardPoll(HardPollFailure::Blocked));
    harness.run_until_parked();

    assert_eq!(harness.script.step_attempts, 1);
    assert_eq!(
        harness.script.blocked_failure().message.as_ref(),
        "action blocked"
    );
    let receipt = harness
        .script
        .last_attempt_receipt
        .as_ref()
        .expect("blocked attempt is receipted");
    assert_eq!(receipt.attempt_count(), Some(1));
    assert_eq!(
        receipt.attempt_stop().map(|stop| stop.as_ref()),
        Some("blocked-stop")
    );
    let status = harness.logs.1.last().expect("blocked status is published");
    assert!(
        !status.fields.iter().any(|field| field.key == "step_stop"),
        "a hard failure before the limit does not report the configured stop"
    );
}

#[test]
fn budgeted_needs_evidence_failure_keeps_its_failure_kind() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut document = document(SequenceOrder::Ordered, |_| never(), never());
    document.roles[0].sequences[0].steps.truncate(1);
    document.roles[0].sequences[0].steps[0].max_attempts = Some(1);
    document.roles[0].sequences[0].steps[0].stop = Some("evidence-stop".into());

    let mut harness = Harness::new(document);
    harness.script_reads(&[QuestListStatus::NotStarted]);
    let path = Arc::get_mut(&mut harness.script.path).expect("unshared path");
    path.sequences[0].steps[0].plan = Arc::new(HardPoll(HardPollFailure::NeedsEvidence));
    harness.run_until_parked();

    let failure = harness.script.blocked_failure();
    assert_eq!(failure.code.as_ref(), "needs-evidence");
    assert!(failure.message.contains("evidence-stop"));
}

#[test]
fn user_input_cancel_does_not_spend_budget_or_release_queued_quest() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut document = document(SequenceOrder::Ordered, |_| never(), never());
    document.roles[0].sequences[0].steps.truncate(1);
    document.roles[0].sequences[0].steps[0].max_attempts = Some(1);
    document.roles[0].sequences[0].steps[0].stop = Some("movement-stop".into());

    let mut harness = Harness::new(document);
    harness.script_reads(&[QuestListStatus::NotStarted]);
    let path = Arc::get_mut(&mut harness.script.path).expect("unshared path");
    path.sequences[0].steps[0].plan = Arc::new(HardPoll(HardPollFailure::UserInput));
    harness.run_until_parked();

    assert_eq!(harness.script.step_attempts, 0);
    assert!(harness.script.last_attempt_receipt.is_none());
    assert_eq!(
        harness.script.blocked_failure().code.as_ref(),
        "manual-movement",
        "QueuedQuester holds a queue row when the active quest reports manual movement"
    );
    let status = harness
        .logs
        .1
        .last()
        .expect("manual-movement status is published");
    assert!(
        !status.fields.iter().any(|field| field.key == "step_stop"),
        "user input did not spend the step budget"
    );
}

#[test]
fn terminal_timed_ground_step_publishes_its_receipt() {
    if !api::test_support::game_data_available() {
        return;
    }
    #[derive(Default)]
    struct Capture(Option<ScriptStatus>);

    impl NativeOutput for Capture {
        fn status(&mut self, status: ScriptStatus) {
            self.0 = Some(status);
        }
        fn paint(&mut self, _: Arc<crate::paint::ScriptPaint>) {}
        fn log(&mut self, _: api::hostlog::Level, _: &str) {}
        fn settings_applied(&mut self, _: u64) {}
    }

    let mut document = document(SequenceOrder::Ordered, |_| never(), always());
    let sequence = &mut document.roles[0].sequences[0];
    sequence.terminal = true;
    sequence.steps = vec![StepDocument {
        id: FactKey::new("take-terminal-egg"),
        kind: "interact".into(),
        version: 1,
        args: serde_json::json!({
            "target": {"ground": "egg"}, "op": "Take", "radius": 2, "max_ticks": 100
        }),
        comment: None,
        advances: Some(false),
        max_attempts: None,
        stop: None,
        skip_if: never(),
        settle: has_egg(),
    }];
    // Keep the real compiled timed-pickup plan; Harness::new replaces plans.
    let (mut script, mut snapshot) = status_fixture(document);
    let egg = script.selected.item_by_alias("egg").unwrap();
    let egg_def = crate::quester::families::tests::def(egg.id, "Egg");
    let egg_id = egg.id;
    snapshot.seed_inventory(Vec::new(), 28);
    crate::quester::families::tests::seed_dialogue_combat(&mut snapshot, false);
    snapshot.seed_ground_items(vec![api::snapshot::GroundItemView {
        def: egg_def.clone(),
        count: 1,
        actions: vec![Some("Take".into())],
        tile: api::WorldTile {
            x: 3253,
            z: 3401,
            level: 0,
        },
        distance: 0,
    }]);
    let mut ledger = None;
    let mut capture = Capture::default();
    let mut began_at = None;
    let mut completed_at = None;
    let mut take_observed = false;
    for tick in 1..=20 {
        let flow = with_tick_output(&snapshot, &mut ledger, tick, &mut capture, |native| {
            script.tick(native).unwrap()
        });
        if !script.begun.is_empty() {
            began_at.get_or_insert(tick);
        }
        if let Some(authority) = ledger
            .as_ref()
            .filter(|_| !take_observed)
            .and_then(|ledger| {
                ledger
                    .outbox
                    .last()
                    .filter(|action| {
                        matches!(&action.effect, crate::native::HostEffect::Interaction(
                    crate::interact::InteractReq::Obj { id: Some(id), action, .. }
                ) if *id == egg_id && action == "Take")
                    })
                    .map(|action| action.authority())
            })
        {
            ledger.as_mut().unwrap().complete_interaction(
                &authority,
                crate::native::InteractionReceipt {
                    request_id: authority.request_id().get(),
                    evidence: api::quest_progress::EvidenceStamp {
                        run: authority.run(),
                        tick,
                        sequence: tick,
                    },
                    accepted: true,
                    chat_since: 0,
                },
            );
            snapshot.seed_inventory(
                vec![api::snapshot::ItemView {
                    def: egg_def.clone(),
                    container: api::snapshot::ItemContainer::Inventory,
                    action_family: api::snapshot::ItemActionFamily::Held,
                    slot: 0,
                    count: 1,
                    actions: Vec::new(),
                    component_id: 3214,
                }],
                28,
            );
            snapshot.seed_ground_items(Vec::new());
            take_observed = true;
        }
        if flow == ScriptFlow::Complete {
            completed_at = Some(tick);
            break;
        }
        assert!(
            !script.parked,
            "the terminal pickup parked: {:?}",
            script.last_error
        );
    }
    let completed_at = completed_at.expect("the terminal pickup completes");
    assert!(take_observed, "the real timed pickup must dispatch Take");
    let waited = i64::try_from(completed_at - began_at.unwrap()).unwrap();
    assert!(waited > 0);
    let status = capture.0.expect("the terminal runner emits a status");
    assert_eq!(status.phase, NativePhase::Complete);
    for (key, expected) in [("ground_waited_ticks", waited), ("ground_held", 1)] {
        assert!(
            status.fields.iter().any(|field| {
                field.key == key
                    && matches!(field.value, StatusValue::Integer(value) if value == expected)
            }),
            "terminal status must retain {key}={expected}: {status:?}"
        );
    }
}
