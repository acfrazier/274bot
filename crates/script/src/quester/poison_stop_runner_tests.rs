//! D8 poison stop through the Quester runner (operator decision D8). The
//! host's part is simulated: a stand click is a queued `WalkTo` interaction
//! that the test sends and answers, and a walk out ends with a posted walk
//! receipt. The player's observed tile is seeded per tick, so a test can
//! keep the player moving after a click as the server's queued path does.
use super::tests::{survival_condition, survival_step_fixture, SurvivalTestCounts};
use super::*;
use crate::gatherer::select::tests::TestZone;
use crate::interact::InteractReq;
use crate::native::ledger::Ledger;
use crate::native::{HostEffect, InteractionReceipt, WalkEnd};
use api::WorldTile;
use std::sync::atomic::Ordering;

type TestLedger = Option<Box<Ledger>>;

#[derive(Default)]
struct Capture {
    statuses: Vec<ScriptStatus>,
    logs: Vec<String>,
}

impl NativeOutput for Capture {
    fn status(&mut self, status: ScriptStatus) {
        self.statuses.push(status);
    }
    fn paint(&mut self, _: Arc<crate::paint::ScriptPaint>) {}
    fn log(&mut self, _: api::hostlog::Level, message: &str) {
        self.logs.push(message.to_owned());
    }
    fn settings_applied(&mut self, _: u64) {}
}

impl Capture {
    fn stop_field(&self) -> Option<String> {
        self.statuses.last().and_then(|status| {
            status
                .fields
                .iter()
                .find(|field| field.key == "survival_stop")
                .map(|field| match &field.value {
                    StatusValue::Text(text) => text.to_string(),
                    other => panic!("survival_stop is not text: {other:?}"),
                })
        })
    }

    /// Whether any published stop claims a safe tile.
    fn any_safe_claim(&self) -> bool {
        self.statuses.iter().any(|status| {
            status.fields.iter().any(|field| {
                field.key == "survival_stop"
                    && matches!(&field.value, StatusValue::Text(text) if text.ends_with(" outside the danger area"))
            })
        })
    }

    fn logged(&self, part: &str) -> bool {
        self.logs.iter().any(|line| line.contains(part))
    }
}

const REASON: &str = "poisoned at low HP with no food and no antipoison: stopped at ";

struct WalkTargetPlan {
    counts: Arc<SurvivalTestCounts>,
    target: WorldTile,
    cross: Arc<[Arc<str>]>,
}

impl super::super::compile::StepPlan for WalkTargetPlan {
    fn begin(&self, _: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        self.counts.began.fetch_add(1, Ordering::Relaxed);
        Ok(Box::new(CountedPendingRun(Arc::clone(&self.counts))))
    }
    fn walk_target(&self) -> Option<(WorldTile, &[Arc<str>])> {
        Some((self.target, &self.cross))
    }
}

struct CountedPendingRun(Arc<SurvivalTestCounts>);

impl StepRun for CountedPendingRun {
    fn poll(&mut self, _: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        Poll::Pending
    }
    fn cancel(&mut self, _: &mut crate::native::NativeActions) {
        self.0.cancelled.fetch_add(1, Ordering::Relaxed);
    }
}

const ORIGIN: WorldTile = WorldTile {
    x: 3192,
    z: 3192,
    level: 0,
};
/// The thrower zone of every fixture: 3196..=3204 on both axes.
const THROWERS: (&str, (i32, i32, i32, i32)) = ("throwers", (3196, 3204, 3196, 3204));

fn at(x: i32) -> WorldTile {
    WorldTile {
        x,
        z: 3200,
        level: 0,
    }
}

/// Make the first step a pending walk to `target` that declares `cross`.
fn set_walk(
    script: &mut Quester,
    counts: &Arc<SurvivalTestCounts>,
    target: WorldTile,
    cross: &[&str],
) {
    let index = script.seq_index;
    let path = Arc::get_mut(&mut script.path).expect("unshared compiled path");
    path.sequences[index].steps[0].plan = Arc::new(WalkTargetPlan {
        counts: Arc::clone(counts),
        target,
        cross: cross.iter().map(|id| Arc::<str>::from(*id)).collect(),
    });
}

/// The survival fixture with an empty inventory, its first step a pending
/// walk to `target` across `throwers`, and a pack with `zones`.
fn fixture(
    target: WorldTile,
    zones: &[TestZone<'_>],
) -> (
    Quester,
    api::snapshot::GameSnapshot,
    Arc<SurvivalTestCounts>,
    nav_planner::contract::NavLease,
) {
    let (mut script, mut snapshot, counts) = survival_step_fixture("walk");
    snapshot.seed_inventory(vec![], 28);
    set_walk(&mut script, &counts, target, &["throwers"]);
    let lease = crate::gatherer::select::tests::test_zone_rects_lease(zones, ORIGIN);
    (script, snapshot, counts, lease)
}

/// Poison onset at HP 5 of 99 with no food: the emergency line under the
/// strongest unknown pulse is 1 + 11 (`select::lines`, `with_poison`).
fn stranded() -> api::condition::BotCondition {
    survival_condition(
        api::condition::PoisonState::Onset {
            since: 20,
            q: Some(11),
            hit_since_line: false,
        },
        5,
    )
}

fn clear() -> api::condition::BotCondition {
    survival_condition(api::condition::PoisonState::Clear, 60)
}

fn immune() -> api::condition::BotCondition {
    survival_condition(api::condition::PoisonState::Immune { until_lower: 80 }, 60)
}

struct Run {
    script: Quester,
    snapshot: api::snapshot::GameSnapshot,
    ledger: TestLedger,
    output: Capture,
    lease: Option<nav_planner::contract::NavLease>,
    /// Every walk request seen queued, `(request id, target)`. A later
    /// action's begin drops an older queued request from the outbox.
    walk_log: Vec<(u64, WorldTile)>,
}

impl Run {
    /// One observed tick at `here`; returns the allocations of the
    /// Quester's own tick.
    fn tick(
        &mut self,
        observed: u64,
        here: WorldTile,
        condition: api::condition::BotCondition,
    ) -> u64 {
        self.snapshot
            .seed_tick(u32::try_from(20 + observed).unwrap());
        self.snapshot
            .seed_local_player(super::super::families::tests::local_player(here));
        let script = &mut self.script;
        let allocations = super::super::families::tests::with_tick_output_nav(
            &self.snapshot,
            &mut self.ledger,
            observed,
            &mut self.output,
            self.lease.as_ref(),
            |tick| {
                tick.cx.condition = Some(condition);
                tick.cx.poison_unknown_pulse = 11;
                tick.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
                allocation_counter::measure(|| {
                    script.tick(tick).unwrap();
                })
                .count_total
            },
        );
        if let Some(ledger) = self.ledger.as_ref() {
            for action in &ledger.outbox {
                if let HostEffect::Walk(request) = &action.effect {
                    let id = action.request_id.get();
                    if !self.walk_log.iter().any(|(seen, _)| *seen == id) {
                        self.walk_log.push((id, request.target));
                    }
                }
            }
        }
        allocations
    }

    /// The tiles of the queued stand clicks.
    fn queued_clicks(&self) -> Vec<WorldTile> {
        self.ledger.as_ref().map_or_else(Vec::new, |ledger| {
            ledger
                .outbox
                .iter()
                .filter_map(|action| match &action.effect {
                    HostEffect::Interaction(InteractReq::WalkTo { x, z, level }) => {
                        Some(WorldTile {
                            x: *x,
                            z: *z,
                            level: *level,
                        })
                    }
                    _ => None,
                })
                .collect()
        })
    }

    /// The host takes every queued stand click and answers it at `tick`.
    /// Other queued work stays. Returns the clicked tiles.
    fn send_clicks(&mut self, tick: u64, accepted: bool) -> Vec<WorldTile> {
        answer_stand_clicks(&mut self.ledger, tick, accepted)
    }

    /// Every queued walk request's target, oldest first.
    fn walks(&self) -> Vec<WorldTile> {
        self.walk_log.iter().map(|(_, target)| *target).collect()
    }

    fn post_walk(&mut self, tick: u64, end: WalkEnd, detail: Option<&str>) {
        super::super::families::tests::post_walk_receipt(&mut self.ledger, tick, end, detail);
    }
}

/// The host takes every queued stand click (`WalkTo`) and answers it at
/// `tick`. Other queued work stays. Returns the clicked tiles.
pub(super) fn answer_stand_clicks(
    ledger: &mut TestLedger,
    tick: u64,
    accepted: bool,
) -> Vec<WorldTile> {
    let ledger = ledger.as_mut().expect("native ledger");
    let (taken, rest): (Vec<_>, Vec<_>) =
        std::mem::take(&mut ledger.outbox)
            .into_iter()
            .partition(|action| {
                matches!(
                    &action.effect,
                    HostEffect::Interaction(InteractReq::WalkTo { .. })
                )
            });
    ledger.outbox = rest;
    let mut clicks = Vec::with_capacity(taken.len());
    for mut action in taken {
        if let HostEffect::Interaction(InteractReq::WalkTo { x, z, level }) = &action.effect {
            clicks.push(WorldTile {
                x: *x,
                z: *z,
                level: *level,
            });
        }
        action.release_custody();
        let authority = action.authority();
        ledger.complete_interaction(
            &authority,
            InteractionReceipt {
                request_id: authority.request_id().get(),
                evidence: api::quest_progress::EvidenceStamp {
                    run: authority.run(),
                    tick,
                    sequence: tick,
                },
                accepted,
                chat_since: 0,
            },
        );
    }
    clicks
}

fn run(target: WorldTile, zones: &[TestZone<'_>]) -> (Run, Arc<SurvivalTestCounts>) {
    let (script, snapshot, counts, lease) = fixture(target, zones);
    (
        Run {
            script,
            snapshot,
            ledger: None,
            output: Capture::default(),
            walk_log: Vec::new(),
            lease: Some(lease),
        },
        counts,
    )
}

/// The walk out leaves the zone; the stop must end the server's queued
/// path. The player keeps moving after the first click (a refused or late
/// click), so the stop is not reported, a second click goes out, and the
/// stop names the tile the player stands still on. A later tile change
/// ends the stop's claim and the runner stands still again.
#[test]
fn poison_stop_reports_only_a_server_visible_stand_still() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut run, counts) = run(at(3220), &[THROWERS]);
    run.tick(1, at(3200), clear());
    assert_eq!(counts.began.load(Ordering::Relaxed), 1);
    run.tick(2, at(3202), stranded());
    assert_eq!(
        counts.cancelled.load(Ordering::Relaxed),
        0,
        "the walk out is kept"
    );
    assert!(run.output.logged("the current walk leaves the danger area"));
    assert!(run.queued_clicks().is_empty(), "no stop inside the area");

    // The first tile outside: the walk is cancelled and the stand click
    // goes out on this tile, but nothing is reported yet.
    run.tick(3, at(3205), stranded());
    assert_eq!(counts.cancelled.load(Ordering::Relaxed), 1);
    assert_eq!(run.queued_clicks(), [at(3205)]);
    assert!(run.script.survival.parked().is_none());
    assert_eq!(run.send_clicks(3, true), [at(3205)]);

    // The server's queued path keeps the player moving: no stop claim.
    for (observed, x) in (4..=9).zip(3206..) {
        run.tick(observed, at(x), stranded());
        assert_eq!(run.output.stop_field(), None, "moving at tick {observed}");
    }
    assert!(run
        .output
        .logged("the stand-still was not seen (the player still moved"));
    assert_eq!(
        run.queued_clicks(),
        [at(3211)],
        "a second click on the new tile"
    );
    run.send_clicks(9, true);
    for observed in 10..=11 {
        run.tick(observed, at(3211), stranded());
        assert_eq!(run.output.stop_field(), None, "not yet two settled ticks");
    }
    run.tick(12, at(3211), stranded());
    let reason = format!("{REASON}3211,3200,0 outside the danger area");
    assert_eq!(run.output.stop_field().as_deref(), Some(reason.as_str()));
    assert!(run.script.survival.parked().is_some());
    assert!(!run
        .output
        .logs
        .iter()
        .any(|line| line.contains("3205,3200,0 outside")));

    // The player moves after the stop: the claim ends and one more click
    // goes out (the third and last of this episode).
    run.tick(13, at(3212), stranded());
    assert!(run
        .output
        .logged("moved from 3211,3200,0 to 3212,3200,0 after the stop"));
    assert_eq!(run.output.stop_field(), None);
    assert_eq!(run.queued_clicks(), [at(3212)]);
    run.send_clicks(13, true);
    for observed in 14..=16 {
        run.tick(observed, at(3212), stranded());
    }
    let reason = format!("{REASON}3212,3200,0 outside the danger area");
    assert_eq!(run.output.stop_field().as_deref(), Some(reason.as_str()));

    // The need ends: the stop ends and the quest resumes.
    run.tick(17, at(3212), immune());
    assert!(run.script.survival.parked().is_none(), "the stop ends");
    assert_eq!(run.output.stop_field(), None);
    assert!(run.output.logged("quester poison stop ended"));
    assert_eq!(counts.began.load(Ordering::Relaxed), 2, "the quest resumes");
}

/// A refused click and a player who never stands still spend the click
/// budget; the stop then names the failure and claims no safety.
#[test]
fn poison_stop_without_a_stand_still_claims_no_safety() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut run, _) = run(at(3220), &[THROWERS]);
    run.tick(1, at(3205), clear());
    run.tick(2, at(3205), stranded());
    assert_eq!(
        run.send_clicks(2, false),
        [at(3205)],
        "the client refuses it"
    );
    run.tick(3, at(3206), stranded());
    assert!(run.output.logged("the client refused the stand click"));
    let mut x = 3206;
    for observed in 4..40 {
        let clicks = run.send_clicks(observed, true);
        assert!(clicks.len() <= 1);
        x += 1;
        run.tick(observed, at(x), stranded());
        if run.script.survival.parked().is_some() {
            break;
        }
    }
    let field = run.output.stop_field().expect("the stop is reported");
    assert!(field.starts_with(REASON), "{field}");
    assert!(
        field.contains("but the player was not seen standing still (the player still moved"),
        "{field}"
    );
    assert!(!run.output.any_safe_claim());
}

/// The walk step begins at tick 1 outside the zone; tick 2 sees that tile
/// under the walk's danger area; tick 3 is inside; then the poison onset
/// starts the walk back. Returns the next observed tick.
fn begin_retreat(run: &mut Run) -> u64 {
    run.tick(1, at(3194), clear());
    run.tick(2, at(3194), clear());
    run.tick(3, at(3200), clear());
    let mut observed = 4;
    while run.walks().is_empty() && observed < 7 {
        run.tick(observed, at(3200), stranded());
        observed += 1;
    }
    assert_eq!(run.walks(), [at(3194)], "the walk out");
    observed
}

/// Retreat walk failures use the shared bounded walk recovery: two retries
/// from fresh observations, and the last failure stays in the stop reason.
#[test]
fn poison_stop_retreat_retries_and_keeps_the_last_failure() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut run, counts) = run(at(3202), &[THROWERS]);
    let mut observed = begin_retreat(&mut run);
    assert_eq!(run.walks(), [at(3194)], "the walk out");
    assert_eq!(counts.cancelled.load(Ordering::Relaxed), 1);
    for attempt in 2..=3usize {
        run.post_walk(observed, WalkEnd::NoPath, Some("no route to the safe tile"));
        run.tick(observed, at(3200), stranded());
        observed += 1;
        assert!(run.output.logged(&format!(
            "the walk out failed (no route to the safe tile); attempt {attempt}/3"
        )));
        assert!(run.script.survival.parked().is_none(), "retry, not a stop");
        run.tick(observed, at(3200), stranded());
        observed += 1;
        assert_eq!(run.walks().len(), attempt, "a fresh walk");
    }
    run.post_walk(observed, WalkEnd::NoPath, Some("still no route"));
    run.tick(observed, at(3200), stranded());
    let clicks = run.send_clicks(observed, true);
    assert_eq!(clicks, [at(3200)], "stand still after the third failure");
    for tick in observed + 1..=observed + 3 {
        run.tick(tick, at(3200), stranded());
    }
    assert_eq!(
        run.output.stop_field().as_deref(),
        Some(format!("{REASON}3200,3200,0 inside the danger area (the walk out did not arrive: still no route)").as_str())
    );
    assert_eq!(run.walks().len(), 3);
}

/// The walk out reports arrival, but the player stands still inside the
/// area: no safety is claimed and the runner walks out again.
#[test]
fn poison_stop_judges_the_tile_the_player_stands_on() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut run, _) = run(at(3202), &[THROWERS]);
    let observed = begin_retreat(&mut run);
    assert_eq!(run.walks(), [at(3194)]);
    run.post_walk(observed, WalkEnd::Arrived, None);
    run.tick(observed, at(3197), stranded());
    assert_eq!(run.send_clicks(observed, true), [at(3197)]);
    for tick in observed + 1..=observed + 3 {
        run.tick(tick, at(3197), stranded());
    }
    assert!(run
        .output
        .logged("the player stood still at 3197,3200,0 inside the danger area; planning again"));
    assert!(!run.output.any_safe_claim());
    assert!(run.script.survival.parked().is_none());
    assert_eq!(run.walks(), [at(3194), at(3194)], "a second walk out");
}

/// A safe tile seen under one danger area is not used under another: a
/// walk step that adds a zone over that tile, or adds any other zone.
#[test]
fn poison_stop_forgets_a_safe_tile_when_the_danger_area_changes() {
    if !api::test_support::game_data_available() {
        return;
    }
    // `pass` covers the old safe tile; `far` does not.
    let zones = [
        THROWERS,
        ("pass", (3192, 3195, 3192, 3207)),
        ("far", (3206, 3207, 3192, 3207)),
    ];
    for added in ["pass", "far"] {
        let (mut run, counts) = run(at(3202), &zones);
        // Tick 1 begins the walk step; tick 2 sees the tile under its area.
        run.tick(1, at(3194), clear());
        run.tick(2, at(3194), clear());
        assert_eq!(
            run.script.poison_stop.safe_for(run.script.danger_scope()),
            Some(at(3194)),
            "seen under the first area"
        );
        set_walk(&mut run.script, &counts, at(3202), &["throwers", added]);
        let mut observed = 3;
        while run.queued_clicks().is_empty() && observed < 6 {
            run.tick(observed, at(3200), stranded());
            observed += 1;
        }
        assert!(run.walks().is_empty(), "{added}: no walk to the old tile");
        assert_eq!(run.send_clicks(observed, true), [at(3200)], "{added}");
        for tick in observed..=observed + 3 {
            run.tick(tick, at(3200), stranded());
        }
        assert_eq!(
            run.output.stop_field().as_deref(),
            Some(format!("{REASON}3200,3200,0 inside the danger area (no safe tile seen in this danger area)").as_str()),
            "{added}"
        );
    }
}

/// Inside with no safe tile seen: the stop stands still in place and says
/// so; no walk goes out.
#[test]
fn poison_stop_without_a_safe_tile_stands_still_in_place() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut run, _) = run(at(3202), &[THROWERS]);
    run.tick(1, at(3200), clear());
    let mut observed = 2;
    while run.queued_clicks().is_empty() && observed < 5 {
        run.tick(observed, at(3200), stranded());
        observed += 1;
    }
    assert_eq!(run.send_clicks(observed, true), [at(3200)]);
    for tick in observed..=observed + 3 {
        run.tick(tick, at(3200), stranded());
    }
    assert_eq!(
        run.output.stop_field().as_deref(),
        Some(format!("{REASON}3200,3200,0 inside the danger area (no safe tile seen in this danger area)").as_str())
    );
    assert!(run.walks().is_empty());
}

/// Held food keeps the guarded walk's own eater in charge (no D8 stop), and
/// with no zone table the stop names an unknown area.
#[test]
fn poison_stop_needs_no_food_and_names_an_unknown_area() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut run, counts) = run(at(3220), &[THROWERS]);
    run.tick(1, at(3200), clear());
    let mut fed = stranded();
    fed.food.ordinary_count = 1;
    run.tick(2, at(3205), fed);
    assert!(
        run.script.survival.parked().is_none(),
        "the walk guard owns the bite"
    );
    assert!(run.script.step.is_some());
    assert!(!run.script.poison_stop.walking_out());
    assert!(run.queued_clicks().is_empty());

    run.lease = None;
    run.tick(3, at(3200), stranded());
    assert_eq!(counts.cancelled.load(Ordering::Relaxed), 1);
    assert_eq!(run.send_clicks(3, true), [at(3200)]);
    for tick in 4..=6 {
        run.tick(tick, at(3200), stranded());
    }
    assert_eq!(
        run.output.stop_field().as_deref(),
        Some(
            format!("{REASON}3200,3200,0 where the danger area is unknown (no zone table or tile)")
                .as_str()
        )
    );
}

/// The walk out and the parked stop poll without heap allocation: the
/// retreat's pending ticks, the stand-still's pending ticks, and the
/// stop's watch.
#[test]
fn poison_stop_polls_retreat_stand_still_and_stop_without_allocation() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut run, _) = run(at(3202), &[THROWERS]);
    let mut observed = begin_retreat(&mut run);
    assert_eq!(run.walks(), [at(3194)]);
    run.tick(observed, at(3199), stranded());
    observed += 1;
    for tick in observed..observed + 20 {
        let allocations = run.tick(tick, at(3199), stranded());
        assert_eq!(allocations, 0, "retreat poll at tick {tick}");
    }
    observed += 20;
    run.post_walk(observed, WalkEnd::Arrived, None);
    run.tick(observed, at(3194), stranded());
    assert_eq!(run.send_clicks(observed, true), [at(3194)]);
    observed += 1;
    // The settle window: no observation counts before two ticks.
    let allocations = run.tick(observed, at(3194), stranded());
    assert_eq!(allocations, 0, "stand-still poll");
    run.tick(observed + 1, at(3194), stranded());
    run.tick(observed + 2, at(3194), stranded());
    assert!(
        run.script.survival.parked().is_some(),
        "{:?}",
        run.output.logs
    );
    observed += 3;
    for tick in observed..observed + 50 {
        let allocations = run.tick(tick, at(3194), stranded());
        assert_eq!(allocations, 0, "parked stop poll at tick {tick}");
    }
}
