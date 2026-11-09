use super::*;
use crate::local_state::{SlotState, StatePort};
use crate::native::{ledger, HostEffect, InteractionReceipt, WalkReceipt};
use crate::quester::families::tests::{def, local_player, with_tick_scene_reach as with_tick};
use api::local_state::{
    AccountKey, ContentScope, DomainEdit, DomainIdentity, DomainLease, ErrorCode, LocalStatePort,
    StoreVersion, SubmitReceipt, WriteSettlement,
};
use api::prereq_knowledge::CatalogRef;
use api::prereq_steps::PrereqCrossing;
use api::selected::{ClientRevision, FamilyPreparation, RunKey};
use api::snapshot::{
    ChatLineView, GameSnapshot, ItemActionFamily, ItemContainer, ItemView, LocLayer, NpcView,
    SnapshotView,
};
use std::sync::LazyLock;
use std::time::Duration;

#[test]
fn repair_walk_preserves_explicit_cure_upkeep_without_food() {
    let stamp = EvidenceStamp {
        run: api::selected::RunKey {
            slot: 1,
            run: 2,
            session: 3,
        },
        tick: 4,
        sequence: 5,
    };
    let target = WorldTile {
        x: 10,
        z: 20,
        level: 0,
    };
    let mut request = reach::walk_request(target, 0, None, stamp);
    assert!(!request.cure_guard);
    request.protect = false;
    request.food_guard = false;
    request.cure_guard = true;
    request.allow.food = false;
    let replay = RepairWalk::from_request(&request).request(target, 0, stamp);
    assert!(replay.cure_guard);
    assert!(!replay.protect && !replay.food_guard && !replay.allow.food);
}

static CATALOG: LazyLock<Arc<PrereqCatalog>> = LazyLock::new(|| {
    FamilyPreparation::run(|worker| {
        api::game_data::for_revision(ClientRevision::R289)
            .unwrap()
            .prepare_quests(worker)
            .unwrap();
        api::game_data::for_revision(ClientRevision::R289)
            .unwrap()
            .prepare_prereq_steps(worker)
            .unwrap()
    })
    .unwrap()
    .join()
    .unwrap()
});

struct NoStore;
impl LocalStatePort for NoStore {
    fn lease(&self, identity: &DomainIdentity) -> Result<DomainLease, ErrorCode> {
        Err(ErrorCode::NotReady {
            domain: identity.domain(),
        })
    }
    fn submit(&self, edit: DomainEdit) -> SubmitReceipt {
        SubmitReceipt::Refused(ErrorCode::Busy {
            domain: edit.identity().domain(),
        })
    }
    fn observe(&self, _: api::identity::RequestId) -> Option<WriteSettlement> {
        None
    }
}
impl StatePort for NoStore {
    fn lease_version(&self, identity: &DomainIdentity) -> Result<StoreVersion, ErrorCode> {
        self.lease(identity).map(|lease| lease.version())
    }
}
fn state() -> SlotState {
    SlotState::new(
        Arc::new(NoStore),
        AccountKey::try_new("local-289", "prereq_test").unwrap(),
        ContentScope::try_new(ClientRevision::R289, "test-content").unwrap(),
    )
}
fn row(consent: ConsentSwitch) -> (u16, &'static PrereqStep) {
    CATALOG
        .iter()
        .find(|(_, row)| row.consent == consent)
        .unwrap()
}
fn gate(row: &PrereqStep, crossing: &PrereqCrossing, truth: Truth) -> ActionableGate {
    ActionableGate {
        gate: QuestGate::Window(row.establishes.clone()),
        truth,
        leg: 2,
        crossing: crossing.crossing,
    }
}
fn runner(row: &PrereqStep, crossing: &PrereqCrossing, truth: Truth) -> PrereqRun {
    let mut run = PrereqRun::begin(
        gate(row, crossing, truth),
        RunEntry::NeedsStep,
        PrereqAllow {
            dialogue: true,
            quest_start: true,
        },
    );
    run.catalog = Some(Arc::clone(&CATALOG));
    run
}
fn snapshot(at: WorldTile) -> GameSnapshot {
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_local_player(local_player(at));
    snapshot.seed_inventory(vec![], 28);
    snapshot.seed_chat_modal(-1, vec![]);
    snapshot.seed_chat_options(vec![], -1);
    snapshot.seed_main_modal(-1, vec![]);
    snapshot
}
fn page_snapshot(snapshot: &mut GameSnapshot, text: &str) {
    snapshot.seed_chat_modal(2100, vec![text.into()]);
    snapshot.seed_chat_options(vec![], 2105);
}
fn close_snapshot(snapshot: &mut GameSnapshot) {
    snapshot.seed_chat_modal(-1, vec![]);
    snapshot.seed_chat_options(vec![], -1);
}
fn full_pack(snapshot: &mut GameSnapshot) {
    snapshot.seed_inventory(
        (0..28)
            .map(|slot| ItemView {
                def: def(995, "Coins"),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot,
                count: 1,
                actions: vec![],
                component_id: 3214,
            })
            .collect(),
        28,
    );
}
fn crossing_snapshot(crossing: &PrereqCrossing) -> GameSnapshot {
    let mut snapshot = snapshot(crossing.crossing.takeoff);
    let (_, row) = CATALOG
        .iter()
        .find(|(_, row)| row.crossing(&crossing.crossing).is_some())
        .unwrap();
    crate::quester::families::tests::seed_prereq_crossing(&mut snapshot, row, &crossing.crossing);
    snapshot
}
fn npc_view(row: &PrereqStep, at: WorldTile, distance: i32) -> NpcView {
    let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let name = data.npc_name(row.target.npc).unwrap().display.clone();
    NpcView {
        index: 7,
        r#type: Some(usize::try_from(row.target.npc).expect("nonnegative catalog NPC ID")),
        name,
        actions: vec![Some("Talk-to".into())],
        tile: at,
        distance,
        animation: -1,
        animation_frame: 0,
        pose_animation: -1,
        orientation: 0,
        target_orientation: 0,
        overhead_text: None,
        spot_animation: -1,
        spot_animation_stamp: -1,
        health: 1,
        total_health: 1,
        face_entity: -1,
        target: None,
        moving: false,
        running: false,
        in_combat: false,
        level: 0,
        size: 1,
        network: at,
        x: 0,
        z: 0,
        yaw: 0,
    }
}
fn npc_snapshot(row: &PrereqStep) -> GameSnapshot {
    let at = row.target.anchor;
    let mut snapshot = snapshot(at);
    snapshot.seed_npcs(vec![npc_view(row, at, 0)]);
    snapshot
}
fn npc_snapshot_with_open_scene(row: &PrereqStep) -> GameSnapshot {
    let mut snapshot = npc_snapshot(row);
    snapshot.seed_scene(api::snapshot::SceneView {
        available: true,
        base_x: row.target.anchor.x - 8,
        base_z: row.target.anchor.z - 8,
        level: row.target.anchor.level,
        width: 17,
        height: 17,
        collision_flags: vec![0; 17 * 17],
    });
    snapshot
}

fn poll(
    run: &mut PrereqRun,
    snapshot: &GameSnapshot,
    ledger: &mut Option<Box<ledger::Ledger>>,
    tick: u64,
    state: &SlotState,
    budget: &mut PrereqBudget,
) -> Poll<Result<Done, PrereqRefusal>> {
    with_tick(snapshot, ledger, tick, |native| {
        native.cx.active_now = Duration::ZERO;
        run.poll(native.actions, &mut native.cx, Some(state), budget)
    })
}
fn accept_last(ledger: &mut Option<Box<ledger::Ledger>>, tick: u64, chat_since: i32) {
    let authority = ledger.as_ref().unwrap().outbox.last().unwrap().authority();
    ledger.as_mut().unwrap().complete_interaction(
        &authority,
        InteractionReceipt {
            request_id: authority.request_id().get(),
            evidence: EvidenceStamp {
                run: authority.run(),
                tick,
                sequence: tick,
            },
            accepted: true,
            chat_since,
        },
    );
}
fn last_interaction(ledger: &Option<Box<ledger::Ledger>>) -> &InteractReq {
    match &ledger.as_ref().unwrap().outbox.last().unwrap().effect {
        HostEffect::Interaction(request) => request,
        _ => panic!("expected an interaction"),
    }
}
fn arrive(ledger: &mut Option<Box<ledger::Ledger>>, tick: u64) {
    set_walk_result(ledger, tick, WalkEnd::Arrived, None);
}
fn arrive_at_goal(ledger: &mut Option<Box<ledger::Ledger>>, tick: u64, goal: WorldTile) {
    set_walk_result(ledger, tick, WalkEnd::Arrived, Some(goal));
}
fn set_walk_result(
    ledger: &mut Option<Box<ledger::Ledger>>,
    tick: u64,
    end: WalkEnd,
    goal: Option<WorldTile>,
) {
    let authority = ledger.as_ref().unwrap().outbox.last().unwrap().authority();
    assert!(matches!(
        ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        HostEffect::Walk(_)
    ));
    ledger.as_mut().unwrap().walk = Some(WalkReceipt {
        goal,
        request_id: authority.request_id().get(),
        evidence: EvidenceStamp {
            run: authority.run(),
            tick,
            sequence: tick,
        },
        end,
        blocked: None,
        detail: None,
        refusal: None,
        assessment: None,
        escape: None,
        prereq_allow: Some(PrereqAllow {
            dialogue: true,
            quest_start: true,
        }),
    });
}
fn message(snapshot: &mut GameSnapshot, sequence: i32, text: &str) {
    snapshot.seed_chat_lines(vec![ChatLineView {
        sequence,
        text: text.into(),
        type_: 0,
        username: None,
    }]);
}
fn start_probe(
    row: &PrereqStep,
    crossing: &PrereqCrossing,
    snapshot: &GameSnapshot,
    state: &SlotState,
) -> (PrereqRun, PrereqBudget, Option<Box<ledger::Ledger>>) {
    let mut run = runner(row, crossing, Truth::Unknown);
    let mut budget = PrereqBudget::default();
    let mut ledger = None;
    assert!(poll(&mut run, snapshot, &mut ledger, 1, state, &mut budget).is_pending());
    assert!(matches!(last_interaction(&ledger), InteractReq::Loc { .. }));
    accept_last(&mut ledger, 1, 10);
    (run, budget, ledger)
}
fn start_talk(
    row: &PrereqStep,
    snapshot: &GameSnapshot,
    state: &SlotState,
) -> (PrereqRun, PrereqBudget, Option<Box<ledger::Ledger>>) {
    let mut run = runner(row, &row.crossings[0], Truth::False);
    let mut budget = PrereqBudget::default();
    let mut ledger = None;
    if row.consent == ConsentSwitch::QuestStart {
        // This helper isolates one conversation. The child-row test below
        // exercises the real A2 -> A1 precondition owner instead.
        let (ordinal, holy) = self::row(ConsentSwitch::PrereqDialogue);
        let fact = with_tick(snapshot, &mut ledger, 1, |native| Fact {
            quest: holy.establishes.quest.clone(),
            signal: holy.establishes.signal.clone(),
            possible: holy.establishes.values,
            after: holy.after_for(holy.establishes.values).unwrap(),
            refused: None,
            source: FactSource::Page {
                row: ordinal,
                line: 0,
            },
            observed: native.cx.evidence(),
        });
        assert!(state.record_prereq_fact(fact));
    }
    assert!(poll(&mut run, snapshot, &mut ledger, 1, state, &mut budget).is_pending());
    arrive(&mut ledger, 1);
    assert!(poll(&mut run, snapshot, &mut ledger, 2, state, &mut budget).is_pending());
    assert!(matches!(last_interaction(&ledger), InteractReq::Npc { .. }));
    accept_last(&mut ledger, 2, 0);
    (run, budget, ledger)
}
fn next_page_send(
    run: &mut PrereqRun,
    snapshot: &GameSnapshot,
    ledger: &mut Option<Box<ledger::Ledger>>,
    tick: &mut u64,
    state: &SlotState,
    budget: &mut PrereqBudget,
) {
    let before = ledger.as_ref().unwrap().outbox.len();
    for _ in 0..=PAGE_TICKS + SEND_SLACK_TICKS {
        assert!(poll(run, snapshot, ledger, *tick, state, budget).is_pending());
        if ledger.as_ref().unwrap().outbox.len() > before {
            assert!(matches!(
                last_interaction(ledger),
                InteractReq::ContinueDialog { .. } | InteractReq::Answer { .. }
            ));
            accept_last(ledger, *tick, 0);
            *tick += 1;
            return;
        }
        *tick += 1;
    }
    panic!("the page did not produce its acknowledgement within the content bound");
}

#[test]
fn budget_visited_uses_a_length_and_no_option() {
    let mut budget = PrereqBudget::default();
    assert_eq!(std::mem::size_of_val(&budget.visited), 6);
    assert!(budget.row(0));
    assert!(budget.row(1));
    assert!(!budget.row(0), "an interleaved same-session repeat refuses");
    assert!(budget.row(2));
    assert!(!budget.row(3));
    assert_eq!(budget.visited_len, 3);
    budget.session_changed();
    assert!(budget.row(0), "a new session can revisit the same row");
    assert!(budget.row(1));
    assert!(!budget.row(0));
    assert_eq!(budget.visited_len, 3, "session revisits use no row slots");
}

#[test]
fn refusal_entry_with_denied_consent_returns_not_allowed_before_preconditions() {
    if !api::test_support::game_data_available() {
        return;
    }
    for consent in [ConsentSwitch::PrereqDialogue, ConsentSwitch::QuestStart] {
        let (ordinal, row) = row(consent);
        let crossing = &row.crossings[0];
        let state = state();
        let snapshot = crossing_snapshot(crossing);
        let mut ledger = None;
        let observed = with_tick(&snapshot, &mut ledger, 1, |native| native.cx.evidence());
        let refusal = &row.probe.refuse[0];
        let gate = gate(row, crossing, Truth::False);
        let fact = Fact {
            quest: row.establishes.quest.clone(),
            signal: row.establishes.signal.clone(),
            possible: refusal.proves,
            after: row.after_for(refusal.proves).unwrap(),
            refused: Some(row.establishes.values),
            source: FactSource::Refusal {
                row: ordinal,
                entry: 0,
            },
            observed,
        };
        assert!(state.record_prereq_fact(fact.clone()));
        let before = state.prereq_snapshot();
        let observation = Arc::new(RefusalObservation {
            crossing: crossing.crossing,
            leg: 2,
            gates: vec![gate.clone()].into(),
            matched: CatalogRef {
                row: ordinal,
                entry: 0,
            },
            fact,
        });
        let mut run = PrereqRun::begin(
            gate,
            RunEntry::GateRefused(observation),
            PrereqAllow::default(),
        );
        run.catalog = Some(Arc::clone(&CATALOG));
        let mut budget = PrereqBudget::default();
        assert!(matches!(
            poll(&mut run, &snapshot, &mut ledger, 2, &state, &mut budget),
            Poll::Ready(Err(PrereqRefusal::NotAllowed { .. }))
        ));
        assert!(ledger
            .as_ref()
            .is_none_or(|ledger| ledger.outbox.is_empty()));
        assert_eq!(state.prereq_snapshot(), before);
        assert_eq!(budget.visited_len, 0);
    }
}

#[test]
fn needs_step_entry_with_denied_consent_returns_not_allowed_before_probe() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let state = state();
    let snapshot = crossing_snapshot(&row.crossings[0]);
    let mut run = runner(row, &row.crossings[0], Truth::Unknown);
    run.allow = PrereqAllow::default();
    let mut ledger = None;
    let mut budget = PrereqBudget::default();
    assert!(matches!(
        poll(&mut run, &snapshot, &mut ledger, 1, &state, &mut budget),
        Poll::Ready(Err(PrereqRefusal::NotAllowed { .. }))
    ));
    assert!(ledger.is_none());
    assert_eq!(budget.probes, 0);
}

#[test]
fn nested_child_consent_is_checked_before_candidate_walk() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, parent) = row(ConsentSwitch::QuestStart);
    let state = state();
    let snapshot = snapshot(parent.crossings[0].crossing.to);
    let mut run = runner(parent, &parent.crossings[0], Truth::False);
    run.allow = PrereqAllow {
        dialogue: false,
        quest_start: true,
    };
    let mut ledger = None;
    let mut budget = PrereqBudget::default();
    assert!(matches!(
        poll(&mut run, &snapshot, &mut ledger, 1, &state, &mut budget),
        Poll::Ready(Err(PrereqRefusal::NotAllowed {
            switch: PrereqSwitch::Dialogue,
            ..
        }))
    ));
    assert!(ledger.is_none());
    assert_eq!(budget.probes, 0);
    assert_eq!(budget.visited_len, 1);
}

#[test]
fn a_wait_is_not_spent_by_polls_without_a_new_player_info() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let state = state();
    let mut snapshot = crossing_snapshot(&row.crossings[0]);
    let (mut run, mut budget, mut ledger) = start_probe(row, &row.crossings[0], &snapshot, &state);
    for index in 0..10 {
        // A scene-only publication changes a loc name, not the evidence tick.
        snapshot.seed_locs(vec![]);
        assert!(
            poll(&mut run, &snapshot, &mut ledger, 1, &state, &mut budget).is_pending(),
            "poll {index}"
        );
    }
    assert!(matches!(run.phase, Phase::Probe { .. }));
    assert!(state.prereq_snapshot().facts().is_empty());
}

#[test]
fn a_wait_is_spent_only_when_observed_ticks_pass_its_bound() {
    if !api::test_support::game_data_available() {
        return;
    }
    let snapshot = snapshot(WorldTile {
        x: 1,
        z: 2,
        level: 0,
    });
    let mut ledger = None;
    let wait = with_tick(&snapshot, &mut ledger, 10, |native| {
        TickWait::new(&native.cx, 3)
    });
    for tick in 10..=13 {
        assert!(!wait.spent(tick));
    }
    assert!(wait.spent(14));
    assert!(matches!(
        wait.failure(PrereqWaitName::W3, 14),
        PrereqFailure::WaitSpent {
            wait: PrereqWaitName::W3,
            since: 10,
            at: 14,
            bound: 3
        }
    ));
}

#[test]
fn a_stale_pass_message_does_not_settle_the_probe() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let crossing = &row.crossings[0];
    let state = state();
    let mut snapshot = crossing_snapshot(crossing);
    let (mut run, mut budget, mut ledger) = start_probe(row, crossing, &snapshot, &state);
    snapshot.seed_local_player(local_player(crossing.pass.landing));
    message(&mut snapshot, 10, &crossing.pass.message);
    assert!(poll(&mut run, &snapshot, &mut ledger, 2, &state, &mut budget).is_pending());
    assert!(state.prereq_snapshot().facts().is_empty());
}

#[test]
fn pass_message_and_landing_settle_the_probe() {
    if !api::test_support::game_data_available() {
        return;
    }
    for consent in [ConsentSwitch::PrereqDialogue, ConsentSwitch::QuestStart] {
        let (_, row) = row(consent);
        for crossing in row.crossings.iter() {
            let state = state();
            let mut snapshot = crossing_snapshot(crossing);
            let (mut run, mut budget, mut ledger) = start_probe(row, crossing, &snapshot, &state);
            snapshot.seed_local_player(local_player(crossing.pass.landing));
            message(&mut snapshot, 11, &crossing.pass.message);
            let result = poll(&mut run, &snapshot, &mut ledger, 2, &state, &mut budget);
            assert!(matches!(result, Poll::Ready(Ok(Done { crossed: true }))));
            assert_eq!(
                state.prereq_snapshot().facts()[0].possible,
                crossing.pass.proves
            );
            assert_eq!(budget.talks, 0);
        }
    }
}

#[test]
fn probe_walks_to_each_pack_takeoff_and_clicks_the_blocked_barrier_anchor() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    assert_eq!(row.crossings.len(), 2, "the pack contains both A1 takeoffs");
    for crossing in row.crossings.iter() {
        let identity = crossing.crossing;
        assert_ne!(identity.takeoff, identity.at);
        let state = state();
        let mut snapshot = crossing_snapshot(crossing);
        let mut loc = snapshot.locs()[0].clone();
        loc.layer = LocLayer::Ground;
        loc.shape = 10;
        loc.angle = 1;
        loc.block_walk = true;
        loc.actions = vec![Some("Pass-through".into())];
        let mut scene = api::snapshot::SceneView {
            available: true,
            base_x: identity.at.x - 2,
            base_z: identity.at.z - 2,
            level: identity.at.level,
            width: 5,
            height: 5,
            collision_flags: vec![0; 25],
        };
        scene.collision_flags[12] = client::dash3d::CollisionFlag::SQ_BLOCKED;
        assert_eq!(
            api::query::loc_approach::can_operate_from(&loc, &scene, identity.takeoff),
            Some(true)
        );
        snapshot.seed_locs(vec![loc]);
        snapshot.seed_scene(scene);
        snapshot.seed_local_player(local_player(WorldTile {
            x: identity.takeoff.x - 1,
            ..identity.takeoff
        }));
        let mut run = runner(row, crossing, Truth::Unknown);
        let mut budget = PrereqBudget::default();
        let mut ledger = None;
        assert!(poll(&mut run, &snapshot, &mut ledger, 1, &state, &mut budget).is_pending());
        let HostEffect::Walk(request) = &ledger.as_ref().unwrap().outbox.last().unwrap().effect
        else {
            panic!("a probe must approach its exact directed takeoff");
        };
        assert_eq!(request.target, identity.takeoff);
        assert_ne!(request.target, identity.at, "the loc anchor is blocked");
        arrive(&mut ledger, 1);
        snapshot.seed_local_player(local_player(identity.takeoff));
        assert!(poll(&mut run, &snapshot, &mut ledger, 2, &state, &mut budget).is_pending());
        assert_eq!(
            last_interaction(&ledger),
            &InteractReq::Loc {
                x: identity.at.x,
                z: identity.at.z,
                level: identity.at.level,
                action: "Pass-through".into(),
                id: Some(identity.loc_id),
            }
        );
        accept_last(&mut ledger, 2, 10);
        snapshot.seed_local_player(local_player(crossing.pass.landing));
        message(&mut snapshot, 11, &crossing.pass.message);
        assert!(matches!(
            poll(&mut run, &snapshot, &mut ledger, 3, &state, &mut budget),
            Poll::Ready(Ok(Done { crossed: true }))
        ));
    }
}

#[test]
fn pass_message_without_landing_keeps_waiting_until_the_bound() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let crossing = &row.crossings[0];
    let state = state();
    let mut snapshot = crossing_snapshot(crossing);
    let (mut run, mut budget, mut ledger) = start_probe(row, crossing, &snapshot, &state);
    message(&mut snapshot, 11, &crossing.pass.message);
    for tick in 2..=4 {
        assert!(poll(&mut run, &snapshot, &mut ledger, tick, &state, &mut budget).is_pending());
    }
    assert!(matches!(
        poll(&mut run, &snapshot, &mut ledger, 5, &state, &mut budget),
        Poll::Ready(Err(PrereqRefusal::StepFailed {
            why: PrereqFailure::ProbeUnacknowledged,
            ..
        }))
    ));
    assert!(state.prereq_snapshot().facts().is_empty());
}

#[test]
fn refusal_page_newer_than_the_watermark_settles_the_probe() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let crossing = &row.crossings[0];
    let state = state();
    let mut snapshot = crossing_snapshot(crossing);
    let (mut run, mut budget, mut ledger) = start_probe(row, crossing, &snapshot, &state);
    page_snapshot(&mut snapshot, "STOP!");
    assert!(poll(&mut run, &snapshot, &mut ledger, 2, &state, &mut budget).is_pending());
    assert!(matches!(run.phase, Phase::ProbeDrain(_)));
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::ContinueDialog { .. }
    ));
    assert_eq!(budget.talks, 0);
}

#[test]
fn cant_reach_ends_the_probe_at_once() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let crossing = &row.crossings[0];
    let state = state();
    let mut snapshot = crossing_snapshot(crossing);
    let (mut run, mut budget, mut ledger) = start_probe(row, crossing, &snapshot, &state);
    message(&mut snapshot, 11, "I can't reach that!");
    assert!(matches!(
        poll(&mut run, &snapshot, &mut ledger, 2, &state, &mut budget),
        Poll::Ready(Err(PrereqRefusal::StepFailed {
            why: PrereqFailure::ProbeUnacknowledged,
            ..
        }))
    ));
    assert!(state.prereq_snapshot().facts().is_empty());
}

#[test]
fn probe_unacknowledged_after_the_bound() {
    if !api::test_support::game_data_available() {
        return;
    }
    for consent in [ConsentSwitch::PrereqDialogue, ConsentSwitch::QuestStart] {
        let (_, row) = row(consent);
        let crossing = row
            .crossings
            .iter()
            .max_by_key(|crossing| crossing.delay.ticks)
            .unwrap();
        let state = state();
        let snapshot = crossing_snapshot(crossing);
        let (mut run, mut budget, mut ledger) = start_probe(row, crossing, &snapshot, &state);
        let bound = u64::from(OP_TICKS + crossing.delay.ticks + SEND_SLACK_TICKS);
        assert_eq!(
            bound,
            if consent == ConsentSwitch::PrereqDialogue {
                3
            } else {
                5
            }
        );
        assert!(poll(
            &mut run,
            &snapshot,
            &mut ledger,
            1 + bound,
            &state,
            &mut budget
        )
        .is_pending());
        assert!(matches!(
            poll(
                &mut run,
                &snapshot,
                &mut ledger,
                2 + bound,
                &state,
                &mut budget
            ),
            Poll::Ready(Err(PrereqRefusal::StepFailed {
                why: PrereqFailure::ProbeUnacknowledged,
                ..
            }))
        ));
    }
}

#[test]
fn outward_message_on_an_inward_probe_fails_closed_and_writes_the_stage_fact() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::QuestStart);
    let inward = row
        .crossings
        .iter()
        .find(|crossing| crossing.delay.ticks > 0)
        .unwrap();
    let outward = row
        .crossings
        .iter()
        .find(|crossing| crossing.pass.message != inward.pass.message)
        .unwrap();
    let state = state();
    let mut snapshot = crossing_snapshot(inward);
    let (mut run, mut budget, mut ledger) = start_probe(row, inward, &snapshot, &state);
    snapshot.seed_local_player(local_player(outward.pass.landing));
    message(&mut snapshot, 11, &outward.pass.message);
    assert!(matches!(
        poll(&mut run, &snapshot, &mut ledger, 2, &state, &mut budget),
        Poll::Ready(Err(PrereqRefusal::StepFailed {
            why: PrereqFailure::ProbeDirection { .. },
            ..
        }))
    ));
    assert_eq!(
        state.prereq_snapshot().facts()[0].possible,
        outward.pass.proves
    );
}

#[test]
fn stage_61_full_pack_unknown_bank_crosses_on_the_probe() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let crossing = &row.crossings[0];
    let state = state();
    let mut snapshot = crossing_snapshot(crossing);
    full_pack(&mut snapshot);
    let (mut run, mut budget, mut ledger) = start_probe(row, crossing, &snapshot, &state);
    message(&mut snapshot, 11, &crossing.pass.message);
    snapshot.seed_local_player(local_player(crossing.pass.landing));
    assert!(matches!(
        poll(&mut run, &snapshot, &mut ledger, 2, &state, &mut budget),
        Poll::Ready(Ok(Done { crossed: true }))
    ));
    assert_eq!(budget.talks, 0);
}

#[test]
fn reclaim_opener_with_full_pack_closes_the_chat_and_gives_no_dagger() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let branch = row
        .talk
        .branches
        .iter()
        .find(|branch| branch.outcome == BranchOutcome::Reclaimed)
        .unwrap();
    let state = state();
    let mut snapshot = npc_snapshot(row);
    full_pack(&mut snapshot);
    let (mut run, mut budget, mut ledger) = start_talk(row, &snapshot, &state);
    page_snapshot(&mut snapshot, &branch.opener);
    assert!(poll(&mut run, &snapshot, &mut ledger, 3, &state, &mut budget).is_pending());
    assert!(matches!(last_interaction(&ledger), InteractReq::CloseModal));
    assert!(state.prereq_snapshot().facts().is_empty());
    accept_last(&mut ledger, 3, 0);
    close_snapshot(&mut snapshot);
    assert!(poll(&mut run, &snapshot, &mut ledger, 4, &state, &mut budget).is_pending());
    assert!(matches!(
        poll(&mut run, &snapshot, &mut ledger, 5, &state, &mut budget),
        Poll::Ready(Err(PrereqRefusal::PreconditionMissing {
            missing: PrereqMissing::FreeSlots { required: 1 },
            ..
        }))
    ));
}

#[test]
fn satisfied_opener_closes_before_the_blessing_page() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let branch = row
        .talk
        .branches
        .iter()
        .find(|branch| branch.outcome == BranchOutcome::Satisfied)
        .unwrap();
    let state = state();
    let mut snapshot = npc_snapshot(row);
    let (mut run, mut budget, mut ledger) = start_talk(row, &snapshot, &state);
    page_snapshot(&mut snapshot, "Greetings again adventurer");
    assert!(poll(&mut run, &snapshot, &mut ledger, 3, &state, &mut budget).is_pending());
    assert!(matches!(last_interaction(&ledger), InteractReq::CloseModal));
    assert_eq!(state.prereq_snapshot().facts()[0].possible, branch.proves);
    accept_last(&mut ledger, 3, 0);
    close_snapshot(&mut snapshot);
    assert!(poll(&mut run, &snapshot, &mut ledger, 4, &state, &mut budget).is_pending());
    assert!(matches!(
        poll(&mut run, &snapshot, &mut ledger, 5, &state, &mut budget),
        Poll::Ready(Ok(Done { crossed: false }))
    ));
}

#[test]
fn an_unmatched_opener_without_an_authored_wildcard_closes_and_writes_nothing() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let state = state();
    let mut snapshot = npc_snapshot(row);
    let (mut run, mut budget, mut ledger) = start_talk(row, &snapshot, &state);
    // The two frozen Drezel rows intentionally prove their authored wildcard
    // opener. Exercise the generic no-matching-branch exit with no wildcard.
    let rows = CATALOG
        .iter()
        .map(|(_, authored)| {
            let mut authored = authored.clone();
            if authored.id == row.id {
                authored.talk.branches = authored
                    .talk
                    .branches
                    .iter()
                    .filter(|branch| branch.opener.as_ref() != "*")
                    .cloned()
                    .collect();
            }
            authored
        })
        .collect();
    run.catalog = Some(Arc::new(PrereqCatalog::from_rows(rows).unwrap()));
    page_snapshot(
        &mut snapshot,
        "This page belongs to an unrecognized branch.",
    );
    assert!(poll(&mut run, &snapshot, &mut ledger, 3, &state, &mut budget).is_pending());
    assert!(matches!(last_interaction(&ledger), InteractReq::CloseModal));
    accept_last(&mut ledger, 3, 0);
    close_snapshot(&mut snapshot);
    assert!(poll(&mut run, &snapshot, &mut ledger, 4, &state, &mut budget).is_pending());
    assert!(matches!(
        poll(&mut run, &snapshot, &mut ledger, 5, &state, &mut budget),
        Poll::Ready(Err(PrereqRefusal::StepUnwitnessed { .. }))
    ));
    assert!(state.prereq_snapshot().facts().is_empty());
}

fn conversation_completes(consent: ConsentSwitch) {
    let (_, row) = row(consent);
    let outcome = if consent == ConsentSwitch::PrereqDialogue {
        BranchOutcome::Advice
    } else {
        BranchOutcome::Start
    };
    let branch = row
        .talk
        .branches
        .iter()
        .find(|branch| branch.outcome == outcome)
        .unwrap();
    let state = state();
    let mut snapshot = npc_snapshot(row);
    if consent == ConsentSwitch::PrereqDialogue {
        full_pack(&mut snapshot);
    }
    let (mut run, mut budget, mut ledger) = start_talk(row, &snapshot, &state);
    let initial_facts = state.prereq_snapshot().facts().len();
    let mut tick = 3;
    page_snapshot(&mut snapshot, &branch.opener);
    next_page_send(
        &mut run,
        &snapshot,
        &mut ledger,
        &mut tick,
        &state,
        &mut budget,
    );
    assert_eq!(state.prereq_snapshot().facts().len(), initial_facts);
    let effect = branch.effect_page.as_deref().unwrap();
    page_snapshot(&mut snapshot, effect);
    next_page_send(
        &mut run,
        &snapshot,
        &mut ledger,
        &mut tick,
        &state,
        &mut budget,
    );
    assert_eq!(
        state.prereq_snapshot().facts().len(),
        initial_facts,
        "showing the effect page is not its acknowledgement"
    );
    let terminal = branch.terminal.as_deref().unwrap();
    if terminal != effect {
        page_snapshot(&mut snapshot, terminal);
        next_page_send(
            &mut run,
            &snapshot,
            &mut ledger,
            &mut tick,
            &state,
            &mut budget,
        );
        assert!(state
            .prereq_snapshot()
            .facts()
            .iter()
            .any(|fact| fact.quest == row.establishes.quest && fact.possible == branch.proves));
    }
    close_snapshot(&mut snapshot);
    let mut completed = false;
    for _ in 0..=PAGE_TICKS + SEND_SLACK_TICKS + DIALOG_GAP_TICKS as u16 {
        match poll(&mut run, &snapshot, &mut ledger, tick, &state, &mut budget) {
            Poll::Ready(Ok(Done { crossed: false })) => {
                completed = true;
                break;
            }
            Poll::Pending => tick += 1,
            _ => panic!("the witnessed conversation did not complete"),
        }
    }
    assert!(completed);
    assert!(state
        .prereq_snapshot()
        .facts()
        .iter()
        .any(|fact| fact.quest == row.establishes.quest && fact.possible == branch.proves));
    assert_eq!(budget.talks, 1);
    assert!(!with_tick(
        &snapshot,
        &mut ledger,
        tick,
        |native| chat_open(&native.cx)
    ));
}

#[test]
fn the_a1_walk_through_completes_with_the_active_clock_frozen() {
    if !api::test_support::game_data_available() {
        return;
    }
    conversation_completes(ConsentSwitch::PrereqDialogue);
}

#[test]
fn a2_effect_page_writes_the_fact_and_the_terminal_page_retires_the_chat() {
    if !api::test_support::game_data_available() {
        return;
    }
    conversation_completes(ConsentSwitch::QuestStart);
}

#[test]
fn reclaim_retalks_once_with_the_same_row_and_budget() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let reclaimed = row
        .talk
        .branches
        .iter()
        .find(|branch| branch.outcome == BranchOutcome::Reclaimed)
        .unwrap();
    for second_outcome in [BranchOutcome::Advice, BranchOutcome::Reclaimed] {
        let second = row
            .talk
            .branches
            .iter()
            .find(|branch| branch.outcome == second_outcome)
            .unwrap();
        let state = state();
        let mut snapshot = npc_snapshot(row);
        let (mut run, mut budget, mut ledger) = start_talk(row, &snapshot, &state);
        let mut tick = 3;
        for (index, branch) in [reclaimed, second].into_iter().enumerate() {
            page_snapshot(&mut snapshot, &branch.opener);
            next_page_send(
                &mut run,
                &snapshot,
                &mut ledger,
                &mut tick,
                &state,
                &mut budget,
            );
            let effect = branch.effect_page.as_deref().unwrap();
            page_snapshot(&mut snapshot, effect);
            next_page_send(
                &mut run,
                &snapshot,
                &mut ledger,
                &mut tick,
                &state,
                &mut budget,
            );
            if branch.terminal.as_deref().unwrap() != effect {
                page_snapshot(&mut snapshot, branch.terminal.as_deref().unwrap());
                next_page_send(
                    &mut run,
                    &snapshot,
                    &mut ledger,
                    &mut tick,
                    &state,
                    &mut budget,
                );
            }
            close_snapshot(&mut snapshot);
            let before = ledger.as_ref().unwrap().outbox.last().unwrap().request_id;
            let mut completed = false;
            for _ in 0..=PAGE_TICKS + SEND_SLACK_TICKS + DIALOG_GAP_TICKS as u16 {
                match poll(&mut run, &snapshot, &mut ledger, tick, &state, &mut budget) {
                    Poll::Pending
                        if index == 0
                            && ledger.as_ref().unwrap().outbox.last().unwrap().request_id
                                != before =>
                    {
                        assert!(matches!(last_interaction(&ledger), InteractReq::Npc { .. }));
                        accept_last(&mut ledger, tick, 0);
                        tick += 1;
                        completed = true;
                        break;
                    }
                    Poll::Pending => tick += 1,
                    Poll::Ready(Ok(Done { crossed: false }))
                        if index == 1 && second_outcome == BranchOutcome::Advice =>
                    {
                        completed = true;
                        break;
                    }
                    Poll::Ready(Err(PrereqRefusal::StepFailed {
                        why: PrereqFailure::ReclaimLoop,
                        ..
                    })) if index == 1 && second_outcome == BranchOutcome::Reclaimed => {
                        completed = true;
                        break;
                    }
                    _ => panic!("reclaim must permit exactly one owned retalk"),
                }
            }
            assert!(
                completed,
                "conversation must settle within its authored tick bounds"
            );
        }
        assert_eq!(budget.visited_len, 1);
        assert_eq!(budget.talks, 2);
        assert_eq!(budget.probes, 0);
        assert_eq!(budget.re_issues, 0);
        assert_eq!(state.prereq_snapshot().facts()[0].possible, second.proves);
        assert!(!ledger.as_ref().unwrap().test_owner_live());
    }
}

#[test]
fn a2_child_probe_selects_reachable_crossing_and_parent_talk_share_the_same_budget() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (holy_ordinal, holy) = row(ConsentSwitch::PrereqDialogue);
    let (mort_ordinal, mort) = row(ConsentSwitch::QuestStart);
    let north = &holy.crossings[0];
    let south = &holy.crossings[1];
    let state = state();
    let mut snapshot = snapshot(north.crossing.to);
    let mut run = runner(mort, &mort.crossings[0], Truth::False);
    let mut budget = PrereqBudget::default();
    let mut ledger = None;
    assert!(poll(&mut run, &snapshot, &mut ledger, 1, &state, &mut budget).is_pending());
    let HostEffect::Walk(request) = &ledger.as_ref().unwrap().outbox.last().unwrap().effect else {
        panic!("the nested probe must request reachability over all authored stands");
    };
    assert_eq!(request.target, north.crossing.takeoff);
    assert_eq!(
        request.candidate_goals.as_ref().unwrap().as_slice(),
        &[north.crossing.takeoff, south.crossing.takeoff]
    );
    assert_eq!(budget.probes, 0);
    assert_eq!(
        &budget.visited[..usize::from(budget.visited_len)],
        &[mort_ordinal, holy_ordinal]
    );
    assert_eq!(budget.visited_len, 2);
    arrive_at_goal(&mut ledger, 1, south.crossing.takeoff);
    snapshot.seed_local_player(local_player(south.crossing.takeoff));
    crate::quester::families::tests::seed_prereq_crossing(&mut snapshot, holy, &south.crossing);
    let probe_tick = (2..=5)
        .find(|tick| {
            assert!(
                poll(&mut run, &snapshot, &mut ledger, *tick, &state, &mut budget).is_pending()
            );
            matches!(
                ledger.as_ref().unwrap().outbox.last().unwrap().effect,
                HostEffect::Interaction(_)
            )
        })
        .expect("the selected stand must start its authored probe");
    let Phase::Child { run: child, .. } = &run.phase else {
        panic!("the parent must retain its running prerequisite child");
    };
    assert_eq!(child.gate.crossing, south.crossing);
    assert!(matches!(last_interaction(&ledger), InteractReq::Loc { .. }));
    assert_eq!(budget.probes, 1);
    accept_last(&mut ledger, probe_tick, 10);
    snapshot.seed_local_player(local_player(south.pass.landing));
    message(&mut snapshot, 11, &south.pass.message);
    let parent_walk_tick = probe_tick + 1;
    assert!(poll(
        &mut run,
        &snapshot,
        &mut ledger,
        parent_walk_tick,
        &state,
        &mut budget
    )
    .is_pending());
    assert!(matches!(
        ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        HostEffect::Walk(_)
    ));
    arrive(&mut ledger, parent_walk_tick);
    snapshot = npc_snapshot(mort);
    assert!(poll(
        &mut run,
        &snapshot,
        &mut ledger,
        parent_walk_tick + 1,
        &state,
        &mut budget
    )
    .is_pending());
    assert!(matches!(last_interaction(&ledger), InteractReq::Npc { .. }));
    assert_eq!(budget.visited_len, 2);
    assert_eq!(budget.probes, 1);
    assert_eq!(budget.talks, 1);
    assert_eq!(budget.re_issues, 0);
    assert_eq!(
        state.prereq_snapshot().facts()[0].possible,
        holy.establishes.values
    );
    drop(run);
    assert!(!ledger.as_ref().unwrap().test_owner_live());
}

#[test]
fn nested_candidate_no_path_remains_a_typed_prerequisite_refusal() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, holy) = row(ConsentSwitch::PrereqDialogue);
    let (_, mort) = row(ConsentSwitch::QuestStart);
    let state = state();
    let before = state.prereq_snapshot();
    let snapshot = snapshot(holy.crossings[0].crossing.to);
    let mut run = runner(mort, &mort.crossings[0], Truth::False);
    let mut budget = PrereqBudget::default();
    let mut ledger = None;
    assert!(poll(&mut run, &snapshot, &mut ledger, 1, &state, &mut budget).is_pending());
    let HostEffect::Walk(request) = &ledger.as_ref().unwrap().outbox.last().unwrap().effect else {
        panic!("the nested probe must submit its candidate stands");
    };
    assert_eq!(
        request.candidate_goals.as_ref().unwrap().as_slice(),
        &[
            holy.crossings[0].crossing.takeoff,
            holy.crossings[1].crossing.takeoff
        ]
    );
    set_walk_result(&mut ledger, 1, WalkEnd::NoPath, None);
    assert!(matches!(
        poll(&mut run, &snapshot, &mut ledger, 2, &state, &mut budget),
        Poll::Ready(Err(PrereqRefusal::NoRouteToStep {
            end: WalkEnd::NoPath,
            ..
        }))
    ));
    assert_eq!(state.prereq_snapshot(), before);
    assert_eq!(budget.probes, 0);
    assert_eq!(budget.talks, 0);
    assert_eq!(
        ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .filter(|effect| matches!(effect.effect, HostEffect::Walk(_)))
            .count(),
        1
    );
    assert!(!ledger.as_ref().unwrap().test_owner_live());
}

#[test]
fn page_wait_counts_from_the_ack_send_tick() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let state = state();
    let mut snapshot = npc_snapshot(row);
    let branch = row
        .talk
        .branches
        .iter()
        .find(|branch| branch.outcome == BranchOutcome::Advice)
        .unwrap();
    let (mut run, mut budget, mut ledger) = start_talk(row, &snapshot, &state);
    page_snapshot(&mut snapshot, &branch.opener);
    let mut tick = 3;
    next_page_send(
        &mut run,
        &snapshot,
        &mut ledger,
        &mut tick,
        &state,
        &mut budget,
    );
    let send_tick = tick - 1;
    for tick in send_tick + 1..=send_tick + 3 {
        assert!(poll(&mut run, &snapshot, &mut ledger, tick, &state, &mut budget).is_pending());
    }
    assert!(
        matches!(poll(&mut run, &snapshot, &mut ledger, send_tick + 4, &state, &mut budget),
        Poll::Ready(Err(PrereqRefusal::StepFailed { why: PrereqFailure::WaitSpent { wait: PrereqWaitName::W3, since, bound: 3, .. }, .. })) if since == send_tick)
    );
}

#[test]
fn terminal_ack_close_then_gap_ticks() {
    if !api::test_support::game_data_available() {
        return;
    }
    let state = state();
    let mut snapshot = snapshot(WorldTile {
        x: 1,
        z: 2,
        level: 0,
    });
    page_snapshot(&mut snapshot, "A terminal page.");
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |native| {
        native
            .actions
            .begin::<ClosePage>((), &mut native.cx)
            .unwrap()
    });
    accept_last(&mut ledger, 1, 0);
    for _ in 0..10 {
        assert!(with_tick(&snapshot, &mut ledger, 1, |native| native
            .actions
            .poll(&handle, &mut native.cx))
        .is_pending());
    }
    close_snapshot(&mut snapshot);
    assert!(with_tick(&snapshot, &mut ledger, 2, |native| native
        .actions
        .poll(&handle, &mut native.cx))
    .is_pending());
    assert!(matches!(
        with_tick(&snapshot, &mut ledger, 3, |native| native
            .actions
            .poll(&handle, &mut native.cx)),
        Poll::Ready(Ok(Ok(())))
    ));
    assert!(state.prereq_snapshot().facts().is_empty());
}

fn cancel_at_effect(after_effect: bool) {
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let branch = row
        .talk
        .branches
        .iter()
        .find(|branch| branch.outcome == BranchOutcome::Advice)
        .unwrap();
    let state = state();
    let mut snapshot = npc_snapshot(row);
    let (mut run, mut budget, mut ledger) = start_talk(row, &snapshot, &state);
    let mut tick = 3;
    page_snapshot(&mut snapshot, &branch.opener);
    next_page_send(
        &mut run,
        &snapshot,
        &mut ledger,
        &mut tick,
        &state,
        &mut budget,
    );
    if after_effect {
        page_snapshot(&mut snapshot, branch.effect_page.as_deref().unwrap());
        next_page_send(
            &mut run,
            &snapshot,
            &mut ledger,
            &mut tick,
            &state,
            &mut budget,
        );
        assert_eq!(branch.terminal.as_deref(), branch.effect_page.as_deref());
        // The advice effect is its terminal page. Only its acknowledged close
        // proves the effect; showing the identical page again proves nothing.
        close_snapshot(&mut snapshot);
        assert!(poll(&mut run, &snapshot, &mut ledger, tick, &state, &mut budget).is_pending());
        assert_eq!(state.prereq_snapshot().facts()[0].possible, branch.proves);
    } else {
        assert!(state.prereq_snapshot().facts().is_empty());
    }
    drop(run);
    let ledger = ledger.as_ref().unwrap();
    assert!(!ledger.test_owner_live());
    assert!(
        ledger.outbox.iter().all(|action| !action.live()),
        "cancellation revokes queued clicks without closing or acknowledging the page"
    );
    assert_eq!(
        state.prereq_snapshot().facts().len(),
        usize::from(after_effect)
    );
}

#[test]
fn cancel_before_the_effect_page_writes_nothing() {
    if !api::test_support::game_data_available() {
        return;
    }
    cancel_at_effect(false);
}

#[test]
fn cancel_after_the_effect_page_keeps_the_fact() {
    if !api::test_support::game_data_available() {
        return;
    }
    cancel_at_effect(true);
}

#[test]
fn cancel_during_probe_revokes_the_live_handle() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let state = state();
    let snapshot = crossing_snapshot(&row.crossings[0]);
    let (run, _, ledger) = start_probe(row, &row.crossings[0], &snapshot, &state);
    assert!(ledger.as_ref().unwrap().test_owner_live());
    drop(run);
    assert!(!ledger.as_ref().unwrap().test_owner_live());
    assert!(ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .all(|action| !action.live()));
    assert!(state.prereq_snapshot().facts().is_empty());
}

#[test]
fn combat_during_talk_ends_step_failed_without_retry() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let state = state();
    let mut snapshot = npc_snapshot(row);
    let (mut run, mut budget, mut ledger) = start_talk(row, &snapshot, &state);
    let mut player = local_player(row.target.anchor);
    player.player.actor.in_combat = true;
    snapshot.seed_local_player(player);
    // A health-bar window alone is not an attack after the Mordred cutover.
    crate::quester::families::tests::seed_dialogue_attacker(
        &mut snapshot,
        api::snapshot::ActorKind::Npc,
    );
    assert!(matches!(
        poll(&mut run, &snapshot, &mut ledger, 3, &state, &mut budget),
        Poll::Ready(Err(PrereqRefusal::StepFailed {
            why: PrereqFailure::CombatInterrupted,
            ..
        }))
    ));
    drop(run);
    assert_eq!(budget.talks, 1);
    assert!(state.prereq_snapshot().facts().is_empty());
    assert!(!ledger.as_ref().unwrap().test_owner_live());
}

#[test]
fn prerequisite_state_layout_and_ten_repeated_walks_stay_bounded() {
    if !api::test_support::game_data_available() {
        return;
    }
    use std::mem::size_of;
    println!(
        "prereq ownership layouts: budget={} run={} walk={}",
        size_of::<PrereqBudget>(),
        size_of::<PrereqRun>(),
        size_of::<super::super::walk::PrereqWalk>()
    );
    assert!(size_of::<SessionMemory>() <= 24);
    assert!(size_of::<Fact>() <= 160);
    assert!(size_of::<RefusalObservation>() <= 256);
    assert!(size_of::<TickWait>() <= 80);
    assert!(size_of::<PrereqBudget>() <= 12);
    assert!(size_of::<PrereqRun>() <= 1024);
    assert!(size_of::<super::super::walk::PrereqWalk>() <= 1536);
    // Per-bot slot state that had only a printed size. Measured 184 B on the
    // 64-bit CI targets; pinned at the reading plus 8 B.
    assert!(size_of::<SlotState>() <= 192);
    let state = state();
    let snapshot = snapshot(WorldTile {
        x: 1,
        z: 2,
        level: 0,
    });
    let mut ledger = None;
    let observed = with_tick(&snapshot, &mut ledger, 1, |native| native.cx.evidence());
    let facts = [ConsentSwitch::PrereqDialogue, ConsentSwitch::QuestStart].map(|consent| {
        let (_, row) = row(consent);
        let crossing = &row.crossings[0];
        Fact {
            quest: row.establishes.quest.clone(),
            signal: row.establishes.signal.clone(),
            possible: crossing.pass.proves,
            after: row.after_for(crossing.pass.proves).unwrap(),
            refused: None,
            source: FactSource::Crossing {
                loc_id: crossing.crossing.loc_id,
                option: crossing.crossing.option,
                passed: true,
            },
            observed,
        }
    });
    let empty = allocation_counter::measure(|| {
        std::hint::black_box(SessionMemory::default());
    });
    assert_eq!(empty.count_total, 0);
    let learned = allocation_counter::measure(|| {
        for fact in &facts {
            assert!(state.record_prereq_fact(fact.clone()));
        }
    });
    let warmed_bytes = state.with_prereq_memory(SessionMemory::retained_bytes);
    assert_eq!(
        warmed_bytes,
        size_of::<SessionMemory>() + 2 * size_of::<Fact>()
    );
    assert_eq!(learned.bytes_current as usize, 2 * size_of::<Fact>());
    let repeated = allocation_counter::measure(|| {
        for tick in 2..12 {
            for fact in &facts {
                let mut fact = fact.clone();
                fact.observed.tick = tick;
                fact.observed.sequence = tick;
                state.record_prereq_fact(fact);
            }
        }
    });
    assert_eq!(repeated.count_total, 0);
    assert_eq!(
        state.with_prereq_memory(SessionMemory::retained_bytes),
        warmed_bytes
    );
    // Once both facts are learned, later ordinary walks must retire their
    // enclosing owners without retaining another request, ledger or fact slice.
    let completed_walks = allocation_counter::measure(|| {
        for tick in 20..30 {
            let (mut walk, mut walk_ledger) = walk_wrapper(&snapshot);
            wrapper_terminal(&mut walk_ledger, tick, WalkEnd::Arrived);
            assert!(matches!(
                wrapper_poll(&mut walk, &snapshot, &mut walk_ledger, tick, &state),
                Poll::Ready(Ok(WalkReceipt {
                    end: WalkEnd::Arrived,
                    ..
                }))
            ));
            assert!(!walk_ledger.as_ref().unwrap().test_owner_live());
            assert_eq!(
                state.with_prereq_memory(SessionMemory::retained_bytes),
                warmed_bytes
            );
        }
    });
    assert_eq!(completed_walks.bytes_current, 0);
    let mut evidence_copy = None;
    let copied = allocation_counter::measure(|| evidence_copy = Some(state.prereq_snapshot()));
    assert_eq!(copied.bytes_current as usize, 2 * size_of::<Fact>());
    assert_eq!(evidence_copy.as_ref().unwrap().facts().len(), 2);
    // Inclusive measurement: the real runner, native owner/ledger, observations,
    // snapshot fixture, and learned state all remain inside the measured call.
    let peak =
        allocation_counter::measure(|| conversation_completes(ConsentSwitch::PrereqDialogue));
    println!("prereq layouts slot={} memory={} fact={} observation={} wait={} budget={} run={} walk={} interrupted-receipt={}; empty={empty:?}; learned={learned:?}; ten-fact-updates={repeated:?}; ten-completed-walks={completed_walks:?}; route-evidence-copy={copied:?}; inclusive-resolution={peak:?}",
        size_of::<SlotState>(), size_of::<SessionMemory>(), size_of::<Fact>(),
        size_of::<RefusalObservation>(), size_of::<TickWait>(), size_of::<PrereqBudget>(),
        size_of::<PrereqRun>(), size_of::<super::super::walk::PrereqWalk>(),
        size_of::<crate::native::WalkReceipt>());
}

#[test]
fn branch_and_effect_text_can_span_rendered_chat_widgets_without_allocating() {
    let texts = vec![
        "Okay, I will keep it equipped".to_owned(),
        "whenever I fight werewolves.".to_owned(),
    ];
    let fragment = "Okay, I will keep it equipped whenever I fight werewolves.";
    assert!(chat_page_matches(&texts, fragment));
    assert!(chat_page_matches(&texts, "EQUIPPED whenever"));
    assert!(!chat_page_matches(&texts, "werewolves. another page"));
    assert!(!chat_page_matches(&texts, "equippedwhenever"));
    assert!(!chat_page_matches(&texts, ""));
    let allocations = allocation_counter::measure(|| {
        for _ in 0..100 {
            assert!(chat_page_matches(&texts, fragment));
        }
    });
    assert_eq!(allocations.count_total, 0);
}

#[test]
fn refused_entry_selects_the_matched_catalogue_row_not_the_first_route_gate() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, holy) = row(ConsentSwitch::PrereqDialogue);
    let (ordinal, mort) = row(ConsentSwitch::QuestStart);
    let crossing = &mort.crossings[0];
    let earlier = gate(holy, &holy.crossings[0], Truth::True);
    let matched = gate(mort, crossing, Truth::False);
    let gates: Arc<[ActionableGate]> = vec![earlier, matched.clone()].into();
    let snapshot = crossing_snapshot(crossing);
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 10, |native| {
        let possible = mort.probe.refuse[0].proves;
        let fact = Fact {
            quest: mort.establishes.quest.clone(),
            signal: mort.establishes.signal.clone(),
            possible,
            after: mort.after_for(possible).unwrap(),
            refused: Some(mort.establishes.values),
            source: FactSource::Refusal {
                row: ordinal,
                entry: 0,
            },
            observed: native.cx.evidence(),
        };
        let observation = RefusalObservation {
            crossing: crossing.crossing,
            leg: matched.leg,
            gates: Arc::clone(&gates),
            matched: CatalogRef {
                row: ordinal,
                entry: 0,
            },
            fact,
        };
        let entry = RunEntry::GateRefused(Arc::new(observation.clone()));
        assert_eq!(
            PrereqRun::first_supported(&gates, &entry, &native.cx),
            Some(matched)
        );
        let wrong_crossing = RefusalObservation {
            crossing: holy.crossings[0].crossing,
            ..observation
        };
        assert!(PrereqRun::first_supported(
            &gates,
            &RunEntry::GateRefused(Arc::new(wrong_crossing)),
            &native.cx
        )
        .is_none());
    });
}

fn walk_wrapper(
    snapshot: &GameSnapshot,
) -> (super::super::walk::PrereqWalk, Option<Box<ledger::Ledger>>) {
    let mut ledger = None;
    let walk = with_tick(snapshot, &mut ledger, 1, |native| {
        let mut request = reach::walk_request(
            WorldTile {
                x: 3450,
                z: 3450,
                level: 0,
            },
            0,
            None,
            native.cx.evidence(),
        );
        request.protect = true;
        request.food_guard = true;
        request.options.allow_teleports = crate::native::WalkBit::Forbid;
        request.options.allow_wilderness = crate::native::WalkBit::Forbid;
        request.options.allow_danger_zones = crate::native::WalkBit::Forbid;
        request.allow.food = false;
        request.allow.escape = false;
        request.avoid = vec![Arc::from("wilderness")].into_boxed_slice();
        request.cross = vec![Arc::from("morytania")].into_boxed_slice();
        super::super::walk::PrereqWalk::begin(request, native.actions, &mut native.cx).unwrap()
    });
    (walk, ledger)
}

fn wrapper_poll(
    walk: &mut super::super::walk::PrereqWalk,
    snapshot: &GameSnapshot,
    ledger: &mut Option<Box<ledger::Ledger>>,
    tick: u64,
    state: &SlotState,
) -> Poll<Result<WalkReceipt, ActionError>> {
    with_tick(snapshot, ledger, tick, |native| {
        native.cx.active_now = Duration::ZERO;
        walk.poll(native.actions, &mut native.cx, Some(state))
    })
}

fn wrapper_terminal(ledger: &mut Option<Box<ledger::Ledger>>, tick: u64, end: WalkEnd) {
    arrive(ledger, tick);
    ledger.as_mut().unwrap().walk.as_mut().unwrap().end = end;
}

fn resolve_probe_in_walk(
    walk: &mut super::super::walk::PrereqWalk,
    row: &PrereqStep,
    crossing: &PrereqCrossing,
    state: &SlotState,
    ledger: &mut Option<Box<ledger::Ledger>>,
    tick: u64,
) -> GameSnapshot {
    let mut snapshot = crossing_snapshot(crossing);
    wrapper_terminal(
        ledger,
        tick,
        WalkEnd::NeedsStep(vec![gate(row, crossing, Truth::Unknown)].into()),
    );
    assert!(wrapper_poll(walk, &snapshot, ledger, tick, state).is_pending());
    assert!(matches!(last_interaction(ledger), InteractReq::Loc { .. }));
    accept_last(ledger, tick, 100);
    snapshot.seed_local_player(local_player(crossing.pass.landing));
    message(&mut snapshot, 101, &crossing.pass.message);
    assert!(wrapper_poll(walk, &snapshot, ledger, tick + 1, state).is_pending());
    assert!(matches!(
        ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        HostEffect::Walk(_)
    ));
    snapshot
}

fn resolve_journal_in_walk(
    walk: &mut super::super::walk::PrereqWalk,
    state: &SlotState,
    ledger: &mut Option<Box<ledger::Ledger>>,
    tick: u64,
) -> GameSnapshot {
    let gate = kitchen_journal_gate();
    let mut snapshot = snapshot(gate.crossing.takeoff);
    snapshot.seed_quest_statuses(
        vec![api::snapshot::QuestStatusView {
            name: "Shield of Arrav".into(),
            component_id: 42,
            colour: 0x00f800,
        }],
        true,
    );
    wrapper_terminal(ledger, tick, WalkEnd::NeedsStep(vec![gate].into()));
    assert!(wrapper_poll(walk, &snapshot, ledger, tick, state).is_pending());
    assert!(matches!(
        last_interaction(ledger),
        InteractReq::IfButton { component_id: 42 }
    ));
    accept_last(ledger, tick, 0);
    snapshot.seed_main_modal(
        crate::quest_journal::ROOT_289,
        vec![
            crate::quest_journal::test_widget(
                crate::quest_journal::TITLE_289,
                "@dre@Shield of Arrav",
            ),
            crate::quest_journal::test_widget(
                8145,
                "@str@I searched the Phoenix Gang Headquarters and found",
            ),
            crate::quest_journal::test_widget(8146, "@str@half of the Shield of Arrav."),
            crate::quest_journal::test_widget(8147, "@red@QUEST COMPLETE!"),
        ],
    );
    assert!(wrapper_poll(walk, &snapshot, ledger, tick + 1, state).is_pending());
    assert!(matches!(last_interaction(ledger), InteractReq::CloseModal));
    accept_last(ledger, tick + 1, 0);
    snapshot.seed_main_modal(-1, vec![]);
    assert!(wrapper_poll(walk, &snapshot, ledger, tick + 2, state).is_pending());
    assert!(matches!(
        ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        HostEffect::Walk(_)
    ));
    snapshot
}

#[test]
fn reissued_walks_keep_the_original_effective_prerequisite_grants() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, holy) = row(ConsentSwitch::PrereqDialogue);
    let (_, mort) = row(ConsentSwitch::QuestStart);
    let crossing = &holy.crossings[0];
    let original = PrereqAllow {
        dialogue: true,
        quest_start: false,
    };
    for reissued in [
        None,
        Some(PrereqAllow::default()),
        Some(PrereqAllow {
            dialogue: true,
            quest_start: true,
        }),
    ] {
        let state = state();
        let mut snapshot = crossing_snapshot(crossing);
        let (mut walk, mut ledger) = walk_wrapper(&snapshot);
        wrapper_terminal(
            &mut ledger,
            2,
            WalkEnd::NeedsStep(vec![gate(holy, crossing, Truth::Unknown)].into()),
        );
        ledger.as_mut().unwrap().walk.as_mut().unwrap().prereq_allow = Some(original);
        assert!(wrapper_poll(&mut walk, &snapshot, &mut ledger, 2, &state).is_pending());
        assert!(matches!(last_interaction(&ledger), InteractReq::Loc { .. }));
        accept_last(&mut ledger, 2, 100);
        snapshot.seed_local_player(local_player(crossing.pass.landing));
        message(&mut snapshot, 101, &crossing.pass.message);
        assert!(wrapper_poll(&mut walk, &snapshot, &mut ledger, 3, &state).is_pending());

        wrapper_terminal(
            &mut ledger,
            4,
            WalkEnd::NeedsStep(vec![gate(mort, &mort.crossings[0], Truth::False)].into()),
        );
        ledger.as_mut().unwrap().walk.as_mut().unwrap().prereq_allow = reissued;
        let emitted = ledger.as_ref().unwrap().outbox.len();
        let Poll::Ready(Ok(receipt)) = wrapper_poll(&mut walk, &snapshot, &mut ledger, 4, &state)
        else {
            panic!("a reissued walk must not grant the originally forbidden quest start");
        };
        assert_eq!(receipt.prereq_allow, Some(original));
        let Some(crate::native::WalkRefusal::Prerequisite(refusal)) = receipt.refusal else {
            panic!("the owner must receive the typed consent refusal");
        };
        assert!(matches!(
            refusal.as_ref(),
            PrereqRefusal::NotAllowed {
                switch: crate::native::PrereqSwitch::QuestStart,
                ..
            }
        ));
        assert_eq!(ledger.as_ref().unwrap().outbox.len(), emitted);
        assert!(!ledger.as_ref().unwrap().test_owner_live());
    }
}

#[test]
fn cancel_during_prerequisite_handover_revokes_the_replacement_owner() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let snapshot = crossing_snapshot(&row.crossings[0]);
    let state = state();
    let (mut walk, mut ledger) = walk_wrapper(&snapshot);
    let first = ledger.as_ref().unwrap().outbox.last().unwrap().authority();
    let revoker = walk.revoker().expect("logical cancellation capability");
    wrapper_terminal(
        &mut ledger,
        2,
        WalkEnd::NeedsStep(vec![gate(row, &row.crossings[0], Truth::Unknown)].into()),
    );
    assert!(wrapper_poll(&mut walk, &snapshot, &mut ledger, 2, &state).is_pending());
    assert!(
        !first.live(),
        "the original owner retires before the probe starts"
    );
    let probe = ledger.as_ref().unwrap().outbox.last().unwrap().authority();
    assert!(probe.live());
    revoker.revoke();
    assert!(
        !probe.live(),
        "the original logical revoker follows the replacement owner"
    );
    assert!(matches!(
        wrapper_poll(&mut walk, &snapshot, &mut ledger, 3, &state),
        Poll::Ready(Ok(WalkReceipt {
            end: WalkEnd::Cancelled,
            ..
        }))
    ));
    assert!(!ledger.as_ref().unwrap().test_owner_live());
    assert!(matches!(
        wrapper_poll(&mut walk, &snapshot, &mut ledger, 4, &state),
        Poll::Ready(Err(ActionError::Stale))
    ));
    assert!(state.prereq_snapshot().facts().is_empty());
}

#[test]
fn same_gate_twice_ends_step_repeated_and_preserves_the_original_walk() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let crossing = &row.crossings[0];
    let state = state();
    let (mut walk, mut ledger) = walk_wrapper(&crossing_snapshot(crossing));
    let snapshot = resolve_probe_in_walk(&mut walk, row, crossing, &state, &mut ledger, 2);
    let HostEffect::Walk(request) = &ledger.as_ref().unwrap().outbox.last().unwrap().effect else {
        panic!("the witnessed probe must reissue the original walk");
    };
    assert_eq!(
        request.target,
        WorldTile {
            x: 3450,
            z: 3450,
            level: 0
        }
    );
    assert!(request.protect && request.food_guard);
    assert!(request.allow.food && !request.allow.escape);
    assert_eq!(&*request.avoid[0], "wilderness");
    assert_eq!(&*request.cross[0], "morytania");
    assert_eq!(request.required_after.tick, 3);
    wrapper_terminal(
        &mut ledger,
        4,
        WalkEnd::NeedsStep(vec![gate(row, crossing, Truth::Unknown)].into()),
    );
    let Poll::Ready(Ok(receipt)) = wrapper_poll(&mut walk, &snapshot, &mut ledger, 4, &state)
    else {
        panic!("a repeated row must terminate instead of probing again");
    };
    assert_eq!(receipt.end, WalkEnd::Blocked);
    let Some(crate::native::WalkRefusal::Prerequisite(refusal)) = receipt.refusal else {
        panic!("the owner must receive the typed prerequisite refusal");
    };
    assert!(matches!(
        refusal.as_ref(),
        PrereqRefusal::StepRepeated { .. }
    ));
    assert_eq!(state.prereq_snapshot().facts().len(), 1);
    assert!(!ledger.as_ref().unwrap().test_owner_live());
}

#[test]
fn a_journal_and_two_distinct_rows_each_reissue_before_the_budget_refuses_a_fourth() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, holy) = row(ConsentSwitch::PrereqDialogue);
    let (_, mort) = row(ConsentSwitch::QuestStart);
    let state = state();
    let (mut walk, mut ledger) = walk_wrapper(&crossing_snapshot(&holy.crossings[0]));
    let original_walk = ledger.as_ref().unwrap().outbox.last().unwrap().request_id;
    resolve_journal_in_walk(&mut walk, &state, &mut ledger, 2);
    let journal_walk = ledger.as_ref().unwrap().outbox.last().unwrap().request_id;
    resolve_probe_in_walk(&mut walk, holy, &holy.crossings[0], &state, &mut ledger, 5);
    let holy_walk = ledger.as_ref().unwrap().outbox.last().unwrap().request_id;
    let snapshot =
        resolve_probe_in_walk(&mut walk, mort, &mort.crossings[0], &state, &mut ledger, 7);
    assert_eq!(state.prereq_snapshot().facts().len(), 3);
    let mort_walk = ledger.as_ref().unwrap().outbox.last().unwrap().request_id;
    // The foreground outbox drops retired owners, so retain each walk's
    // identity before its replacement instead of treating it as a history.
    let walk_ids = [original_walk, journal_walk, holy_walk, mort_walk];
    assert!(
        walk_ids.windows(2).all(|ids| ids[0] < ids[1]),
        "the original walk and each row's fresh reissue: {walk_ids:?}"
    );

    let mut fourth = gate(holy, &holy.crossings[0], Truth::Unknown);
    let QuestGate::Window(window) = &mut fourth.gate else {
        panic!("the Priest fixture is a stage window");
    };
    window.values = api::selected::InclusiveRange {
        min: Some(5),
        max: Some(61),
    };
    assert!(CATALOG.for_journal(&fourth.gate).is_some());
    let emitted = ledger.as_ref().unwrap().outbox.len();
    wrapper_terminal(&mut ledger, 9, WalkEnd::NeedsStep(vec![fourth].into()));
    let Poll::Ready(Ok(receipt)) = wrapper_poll(&mut walk, &snapshot, &mut ledger, 9, &state)
    else {
        panic!("a fourth prerequisite must not begin after the distinct-row budget");
    };
    assert_eq!(receipt.end, WalkEnd::Blocked);
    let Some(crate::native::WalkRefusal::Prerequisite(refusal)) = receipt.refusal else {
        panic!("the exhausted budget returns its typed refusal");
    };
    assert!(matches!(
        refusal.as_ref(),
        PrereqRefusal::StepRepeated { .. }
    ));
    assert_eq!(ledger.as_ref().unwrap().outbox.len(), emitted);
    assert!(!ledger.as_ref().unwrap().test_owner_live());
}

#[test]
fn exhausted_re_issues_keep_the_recorded_refusal() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (ordinal, holy) = row(ConsentSwitch::PrereqDialogue);
    let (_, mort) = row(ConsentSwitch::QuestStart);
    let state = state();
    let (mut walk, mut ledger) = walk_wrapper(&crossing_snapshot(&holy.crossings[0]));
    resolve_journal_in_walk(&mut walk, &state, &mut ledger, 2);
    resolve_probe_in_walk(&mut walk, holy, &holy.crossings[0], &state, &mut ledger, 5);
    let snapshot =
        resolve_probe_in_walk(&mut walk, mort, &mort.crossings[0], &state, &mut ledger, 7);
    let crossing = &holy.crossings[0];
    let gates: Arc<[ActionableGate]> = vec![gate(holy, crossing, Truth::False)].into();
    let possible = holy.probe.refuse[0].proves;
    let fact = with_tick(&snapshot, &mut ledger, 9, |native| Fact {
        quest: holy.establishes.quest.clone(),
        signal: holy.establishes.signal.clone(),
        possible,
        after: holy.after_for(possible).unwrap(),
        refused: Some(holy.establishes.values),
        source: FactSource::Refusal {
            row: ordinal,
            entry: 0,
        },
        observed: native.cx.evidence(),
    });
    assert!(
        state.record_prereq_fact(fact.clone()),
        "the host records before ownership decisions"
    );
    let observation = Arc::new(RefusalObservation {
        crossing: crossing.crossing,
        leg: gates[0].leg,
        gates,
        matched: CatalogRef {
            row: ordinal,
            entry: 0,
        },
        fact: fact.clone(),
    });
    wrapper_terminal(&mut ledger, 9, WalkEnd::GateRefused(observation));
    let Poll::Ready(Ok(receipt)) = wrapper_poll(&mut walk, &snapshot, &mut ledger, 9, &state)
    else {
        panic!("the distinct-row reissue budget must terminate the logical walk");
    };
    assert_eq!(receipt.end, WalkEnd::Blocked);
    let Some(crate::native::WalkRefusal::Prerequisite(refusal)) = receipt.refusal else {
        panic!("the exhausted budget must preserve the refusal observation");
    };
    assert!(matches!(
        refusal.as_ref(),
        PrereqRefusal::RefusedAfterStep { .. }
    ));
    assert!(state.prereq_snapshot().facts().contains(&fact));
    assert!(!ledger.as_ref().unwrap().test_owner_live());
}

#[test]
fn reach_handover_captures_inline_walk_grants_and_cancels_the_probe_owner() {
    if !api::test_support::game_data_available() {
        return;
    }
    use crate::quester::prereq::reach::PrereqReach;

    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let crossing = &row.crossings[0];
    let state = state();
    let mut snapshot = crossing_snapshot(crossing);
    let mut door = snapshot.locs()[0].clone();
    door.name = Some("Door".into());
    door.actions = vec![Some("Open".into())];
    snapshot.seed_locs(vec![door]);
    let args = reach::ReachArgs {
        kind: reach::ReachKind::Loc {
            id: Some(crossing.crossing.loc_id),
            name: None,
        },
        op: Arc::from("Open"),
        anchor: Some(crossing.crossing.at),
        radius: 2,
        wait_if_missing: false,
        target_tile: Some(crossing.crossing.at),
        reachable_only: false,
    };
    let mut ledger = None;
    let mut action = with_tick(&snapshot, &mut ledger, 1, |native| {
        PrereqReach::begin(args, native.actions, &mut native.cx).unwrap()
    });
    let original = ledger.as_ref().unwrap().outbox.last().unwrap().authority();
    accept_last(&mut ledger, 1, 0);
    message(&mut snapshot, 1, "I can't reach that!");
    assert!(with_tick(&snapshot, &mut ledger, 2, |native| action.poll(
        native.actions,
        &mut native.cx,
        Some(&state)
    ))
    .is_pending());
    assert!(matches!(
        ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        HostEffect::Walk(_)
    ));
    assert!(original.live(), "Reach owns its inline recovery walk");
    wrapper_terminal(
        &mut ledger,
        3,
        WalkEnd::NeedsStep(vec![gate(row, crossing, Truth::Unknown)].into()),
    );
    assert!(with_tick(&snapshot, &mut ledger, 3, |native| action.poll(
        native.actions,
        &mut native.cx,
        Some(&state)
    ))
    .is_pending());
    assert!(
        !original.live(),
        "the terminal Reach retires before its prerequisite owner"
    );
    assert!(matches!(last_interaction(&ledger), InteractReq::Loc { .. }));
    let probe = ledger.as_ref().unwrap().outbox.last().unwrap().authority();
    assert!(
        probe.live(),
        "the terminal's grants authorize the owned probe"
    );
    action.cancel();
    assert!(!probe.live());
    assert!(matches!(
        with_tick(&snapshot, &mut ledger, 4, |native| action.poll(
            native.actions,
            &mut native.cx,
            Some(&state)
        )),
        Poll::Ready(Err(ActionError::Cancelled))
    ));
    assert!(!ledger.as_ref().unwrap().test_owner_live());
    assert!(state.prereq_snapshot().facts().is_empty());
}

#[test]
fn approach_walk_inherits_protect_and_allow() {
    if !api::test_support::game_data_available() {
        return;
    }
    use crate::native::WalkBit;
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let crossing = &row.crossings[0];
    for truth in [Truth::False, Truth::Unknown] {
        let state = state();
        let mut snapshot = crossing_snapshot(crossing);
        snapshot.seed_local_player(local_player(WorldTile {
            x: crossing.crossing.at.x - 1,
            ..crossing.crossing.at
        }));
        let (mut walk, mut ledger) = walk_wrapper(&snapshot);
        wrapper_terminal(
            &mut ledger,
            2,
            WalkEnd::NeedsStep(vec![gate(row, crossing, truth)].into()),
        );
        assert!(wrapper_poll(&mut walk, &snapshot, &mut ledger, 2, &state).is_pending());
        let HostEffect::Walk(request) = &ledger.as_ref().unwrap().outbox.last().unwrap().effect
        else {
            panic!("resolution must own a guarded approach walk");
        };
        assert_eq!(
            request.target,
            if truth == Truth::False {
                row.target.anchor
            } else {
                crossing.crossing.takeoff
            }
        );
        assert!(request.protect && request.food_guard);
        assert!(request.allow.food && !request.allow.escape);
        assert_eq!(request.options.allow_teleports, WalkBit::Forbid);
        assert_eq!(request.options.allow_wilderness, WalkBit::Forbid);
        assert_eq!(request.options.allow_danger_zones, WalkBit::Forbid);
        assert_eq!(&*request.avoid[0], "wilderness");
        assert_eq!(&*request.cross[0], "morytania");
        assert_eq!(request.required_after.tick, 2);
        walk.cancel();
        assert!(!ledger.as_ref().unwrap().test_owner_live());
    }
}

#[test]
fn protection_warning_reaches_the_owner_during_approach() {
    if !api::test_support::game_data_available() {
        return;
    }
    use crate::native::{WalkEvent, WalkEventKind};
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let state = state();
    let snapshot = crossing_snapshot(&row.crossings[0]);
    let (mut walk, mut ledger) = walk_wrapper(&snapshot);
    wrapper_terminal(
        &mut ledger,
        2,
        WalkEnd::NeedsStep(vec![gate(row, &row.crossings[0], Truth::False)].into()),
    );
    assert!(wrapper_poll(&mut walk, &snapshot, &mut ledger, 2, &state).is_pending());
    let authority = ledger.as_ref().unwrap().outbox.last().unwrap().authority();
    ledger.as_mut().unwrap().walk_events.push(WalkEvent {
        request_id: authority.request_id().get(),
        evidence: EvidenceStamp {
            run: authority.run(),
            tick: 3,
            sequence: 3,
        },
        kind: WalkEventKind::Unprotectable {
            protect: crate::combat::GuardProtect::Missiles,
        },
        detail: Arc::from("Prayer 40 needed for Protect from Missiles"),
    });
    with_tick(&snapshot, &mut ledger, 3, |native| {
        let handle = walk
            .walk_handle()
            .expect("the logical owner exposes its current approach");
        let event = native
            .actions
            .take_walk_event(handle, &mut native.cx)
            .unwrap();
        assert!(matches!(event.kind, WalkEventKind::Unprotectable { .. }));
        assert!(native
            .actions
            .take_walk_event(handle, &mut native.cx)
            .is_none());
    });
    walk.cancel();
    assert!(!ledger.as_ref().unwrap().test_owner_live());
}

#[test]
fn exhausted_transition_budget_defers_repair_admission_without_allocating() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let state = state();
    let snapshot = snapshot(WorldTile {
        x: 1,
        z: 2,
        level: 0,
    });
    let mut request = reach::walk_request(
        row.target.anchor,
        1,
        None,
        EvidenceStamp {
            run: api::selected::RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 1,
        },
    );
    request.cross = vec![Arc::from("morytania")].into_boxed_slice();
    request.avoid = vec![Arc::from("wilderness")].into_boxed_slice();
    for phase in [
        Phase::Approach(None),
        Phase::ProbeApproach(None),
        Phase::TalkStart,
        Phase::ProbeDrain(ChatDrive::unstarted(true)),
    ] {
        let mut run = runner(row, &row.crossings[0], Truth::False);
        run.phase = phase;
        let mut budget = PrereqBudget::default();
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 2, |native| {
            while native.cx.charge_transition() {}
            let allocated = allocation_counter::measure(|| {
                for _ in 0..10 {
                    assert!(run
                        .poll_with_walk(
                            native.actions,
                            &mut native.cx,
                            Some(&state),
                            &mut budget,
                            RepairWalk::from_request(&request)
                        )
                        .is_pending());
                }
            });
            assert_eq!(
                allocated.count_total, 0,
                "budget-only repair polls allocated: {allocated:?}"
            );
        });
        assert!(!ledger
            .as_ref()
            .is_some_and(|ledger| ledger.test_owner_live()));
    }
}

#[test]
fn a_talk_row_outlives_the_generic_deadline_and_keeps_its_typed_expiry() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let state = state();
    let snapshot = npc_snapshot(row);
    let (mut run, mut budget, mut ledger) = start_talk(row, &snapshot, &state);
    let before = state.prereq_snapshot();
    let sent = ledger.as_ref().unwrap().outbox.len();
    // The accepted Talk at tick 2 reaches its content trigger at observed tick 3.
    assert!(poll(&mut run, &snapshot, &mut ledger, 3, &state, &mut budget).is_pending());
    let deadline = 3 + crate::quester::families::dialogue::DIALOGUE_OPEN_TICKS;
    assert_eq!(deadline, 7);
    assert!(poll(
        &mut run,
        &snapshot,
        &mut ledger,
        deadline,
        &state,
        &mut budget,
    )
    .is_pending());
    assert_eq!(state.prereq_snapshot(), before);
    assert_eq!(budget.talks, 1);
    assert_eq!(ledger.as_ref().unwrap().outbox.len(), sent);
    assert!(ledger.as_ref().unwrap().test_owner_live());
    // The row owns the inclusive deadline at tick 9, not the generic tick 7.
    assert_eq!(2 + talk_bound(row), 9);
    assert!(poll(&mut run, &snapshot, &mut ledger, 9, &state, &mut budget).is_pending());
    let result = poll(&mut run, &snapshot, &mut ledger, 10, &state, &mut budget);
    assert!(matches!(
        result,
        Poll::Ready(Err(PrereqRefusal::StepFailed {
            why: PrereqFailure::WaitSpent {
                wait: PrereqWaitName::W5,
                since: 2,
                bound: 7,
                ..
            },
            ..
        }))
    ));
    assert_eq!(state.prereq_snapshot(), before);
    assert_eq!(budget.talks, 1);
    assert_eq!(ledger.as_ref().unwrap().outbox.len(), sent);
    assert!(!ledger.as_ref().unwrap().test_owner_live());
}

const CANT_REACH_TEXT: &str = "I can't reach that!";

fn talk_bound(row: &PrereqStep) -> u64 {
    u64::from(row.target.leash + NPC_STEP_TICKS + OP_TICKS + SEND_SLACK_TICKS)
}

fn talk_drive(run: &PrereqRun) -> &ChatDrive {
    match &run.phase {
        Phase::Talk(drive) => drive,
        _ => panic!("the runner is not in its Talk phase"),
    }
}

fn last_request_id(ledger: &Option<Box<ledger::Ledger>>) -> u64 {
    ledger
        .as_ref()
        .unwrap()
        .outbox
        .last()
        .unwrap()
        .authority()
        .request_id()
        .get()
}

#[test]
fn a_cancelled_talk_ends_its_wait_at_once_and_re_approaches_the_moved_npc() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let state = state();
    let mut snapshot = npc_snapshot_with_open_scene(row);
    let (mut run, mut budget, mut ledger) = start_talk(row, &snapshot, &state);
    let anchor = row.target.anchor;

    // The engine clears the Talk on the next cycle and writes the message.
    message(&mut snapshot, 1, CANT_REACH_TEXT);
    let sent = last_request_id(&ledger);
    assert!(
        poll(&mut run, &snapshot, &mut ledger, 3, &state, &mut budget).is_pending(),
        "a cancel is not a spent wait"
    );
    assert!(
        3 - 2 < talk_bound(row),
        "the wait ended after one observed tick, far inside its bound"
    );
    let drive = talk_drive(&run);
    assert_eq!(drive.reach_failures, 1);
    assert!(drive.pending.is_none(), "the cancelled wait is closed");
    assert!(drive.handle.is_none(), "the cancelled Talk's owner is gone");
    assert_eq!(last_request_id(&ledger), sent);

    // Nothing is sent on the cancel's own tick. Drezel wandered five tiles
    // away: the next observed tick walks to where he is now.
    let far = WorldTile {
        x: anchor.x + 5,
        ..anchor
    };
    snapshot.seed_npcs(vec![npc_view(row, far, 5)]);
    assert!(poll(&mut run, &snapshot, &mut ledger, 3, &state, &mut budget).is_pending());
    assert_eq!(last_request_id(&ledger), sent);
    assert!(poll(&mut run, &snapshot, &mut ledger, 4, &state, &mut budget).is_pending());
    let HostEffect::Walk(request) = &ledger.as_ref().unwrap().outbox.last().unwrap().effect else {
        panic!("the retry approaches the NPC before it talks");
    };
    assert_eq!(request.target, far, "the walk targets the observed tile");

    // He walks back next to the player: the Talk is sent again and the page
    // that follows it is acknowledged inside the ordinary bound.
    arrive(&mut ledger, 5);
    let near = WorldTile {
        x: anchor.x + 1,
        ..anchor
    };
    snapshot.seed_npcs(vec![npc_view(row, near, 1)]);
    assert!(poll(&mut run, &snapshot, &mut ledger, 6, &state, &mut budget).is_pending());
    assert!(matches!(last_interaction(&ledger), InteractReq::Npc { .. }));
    accept_last(&mut ledger, 6, 1);
    let branch = row
        .talk
        .branches
        .iter()
        .find(|branch| branch.outcome == BranchOutcome::Advice)
        .unwrap();
    page_snapshot(&mut snapshot, &branch.opener);
    let mut tick = 7;
    next_page_send(
        &mut run,
        &snapshot,
        &mut ledger,
        &mut tick,
        &state,
        &mut budget,
    );
    let drive = talk_drive(&run);
    assert_eq!(drive.reach_failures, 1, "the earlier cancel stays counted");
    assert_eq!(budget.talks, 1, "a retried Talk is one Talk of the budget");
}

#[test]
fn repeated_cancels_end_in_a_terminal_failure_that_keeps_the_last_detail() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let state = state();
    let mut snapshot = npc_snapshot(row);
    let (mut run, mut budget, mut ledger) = start_talk(row, &snapshot, &state);
    let anchor = row.target.anchor;
    let mut tick = 3;
    let mut sent_at = 2;
    for cancel in 1..=i32::from(TALK_REACH_RETRIES) {
        message(&mut snapshot, cancel, CANT_REACH_TEXT);
        assert!(
            poll(&mut run, &snapshot, &mut ledger, tick, &state, &mut budget).is_pending(),
            "cancel {cancel} is retried"
        );
        assert_eq!(talk_drive(&run).reach_failures, cancel as u8);
        // The next observed tick sends the Talk again, and the host accepts it.
        sent_at = tick + 1;
        let sent = last_request_id(&ledger);
        assert!(poll(
            &mut run,
            &snapshot,
            &mut ledger,
            sent_at,
            &state,
            &mut budget
        )
        .is_pending());
        assert_ne!(last_request_id(&ledger), sent, "the Talk was sent again");
        assert!(matches!(last_interaction(&ledger), InteractReq::Npc { .. }));
        accept_last(&mut ledger, sent_at, cancel);
        tick = sent_at + 2;
    }
    // The cancel after the last retry is terminal. The detail is that
    // cancel's: the player's and the NPC's tiles, and the wait it ended.
    let last = i32::from(TALK_REACH_RETRIES) + 1;
    message(&mut snapshot, last, CANT_REACH_TEXT);
    let near = WorldTile {
        x: anchor.x + 1,
        ..anchor
    };
    snapshot.seed_npcs(vec![npc_view(row, near, 1)]);
    let result = poll(&mut run, &snapshot, &mut ledger, tick, &state, &mut budget);
    let Poll::Ready(Err(PrereqRefusal::StepFailed { why, .. })) = result else {
        panic!("the budget is spent: expected a terminal failure");
    };
    let PrereqFailure::TalkUnreachable {
        attempts,
        since,
        at,
        from,
        npc,
        cause,
    } = why
    else {
        panic!("expected TalkUnreachable, saw {why:?}");
    };
    assert_eq!(attempts, TALK_REACH_RETRIES + 1);
    assert_eq!(at, tick);
    assert_eq!(since, sent_at, "the wait it ended began at its receipt");
    assert_eq!(from, Some(anchor));
    assert_eq!(npc, Some(near));
    assert_eq!(cause, TalkReachCause::Cancelled);
    assert_eq!(budget.talks, 1);
    assert!(!ledger.as_ref().unwrap().test_owner_live());
}

#[test]
fn an_arrived_re_approach_with_a_missing_target_spends_one_attempt_before_retargeting() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let state = state();
    let mut snapshot = npc_snapshot_with_open_scene(row);
    let (mut run, mut budget, mut ledger) = start_talk(row, &snapshot, &state);
    let before = state.prereq_snapshot();
    message(&mut snapshot, 1, CANT_REACH_TEXT);
    assert!(poll(&mut run, &snapshot, &mut ledger, 3, &state, &mut budget).is_pending());
    let moved = WorldTile {
        x: row.target.anchor.x + 5,
        ..row.target.anchor
    };
    snapshot.seed_npcs(vec![npc_view(row, moved, 5)]);
    assert!(poll(&mut run, &snapshot, &mut ledger, 4, &state, &mut budget).is_pending());
    let sent = ledger.as_ref().unwrap().outbox.len();
    snapshot.seed_npcs(Vec::new());
    arrive(&mut ledger, 5);
    for _ in 0..3 {
        assert!(poll(&mut run, &snapshot, &mut ledger, 5, &state, &mut budget).is_pending());
        assert_eq!(talk_drive(&run).reach_failures, 2);
        assert!(talk_drive(&run).handle.is_none());
        assert_eq!(talk_drive(&run).retry_after, Some(5));
        assert_eq!(ledger.as_ref().unwrap().outbox.len(), sent);
    }
    assert_eq!(state.prereq_snapshot(), before);
    assert_eq!(budget.talks, 1);
}

/// R3-2: after a cancel, the replacement dialogue's walk reaches the NPC's
/// earlier tile while the NPC moved on. That failed re-approach is one
/// failed attempt of the budget, not the end of the conversation: the next
/// observed tick approaches again, and a Talk from the new tile gets its page.
#[test]
fn a_failed_re_approach_is_retried_inside_the_budget_and_the_talk_still_passes() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let state = state();
    let mut snapshot = npc_snapshot_with_open_scene(row);
    let (mut run, mut budget, mut ledger) = start_talk(row, &snapshot, &state);
    let anchor = row.target.anchor;

    message(&mut snapshot, 1, CANT_REACH_TEXT);
    assert!(poll(&mut run, &snapshot, &mut ledger, 3, &state, &mut budget).is_pending());
    assert_eq!(talk_drive(&run).reach_failures, 1);

    // Drezel is five tiles away: the next tick walks to that tile.
    let first = WorldTile {
        x: anchor.x + 5,
        ..anchor
    };
    snapshot.seed_npcs(vec![npc_view(row, first, 5)]);
    assert!(poll(&mut run, &snapshot, &mut ledger, 4, &state, &mut budget).is_pending());
    let HostEffect::Walk(request) = &ledger.as_ref().unwrap().outbox.last().unwrap().effect else {
        panic!("the retry approaches the NPC");
    };
    assert_eq!(request.target, first);

    // The walk arrives at that tile, but he wandered on: the approach fails.
    let second = WorldTile {
        x: anchor.x - 4,
        ..anchor
    };
    snapshot.seed_npcs(vec![npc_view(row, second, 4)]);
    arrive(&mut ledger, 5);
    assert!(
        poll(&mut run, &snapshot, &mut ledger, 5, &state, &mut budget).is_pending(),
        "a failed re-approach is not terminal while the budget lasts"
    );
    let drive = talk_drive(&run);
    assert_eq!(
        drive.reach_failures, 2,
        "the failed approach used one attempt"
    );
    assert!(drive.handle.is_none(), "the failed attempt's owner is gone");

    // The next tick approaches his new tile.
    assert!(poll(&mut run, &snapshot, &mut ledger, 6, &state, &mut budget).is_pending());
    let HostEffect::Walk(request) = &ledger.as_ref().unwrap().outbox.last().unwrap().effect else {
        panic!("the next attempt approaches the NPC again");
    };
    assert_eq!(
        request.target, second,
        "the walk targets the newly observed tile"
    );

    // He stands next to the player now: the Talk goes out and its page passes.
    arrive(&mut ledger, 7);
    let near = WorldTile {
        x: anchor.x + 1,
        ..anchor
    };
    snapshot.seed_npcs(vec![npc_view(row, near, 1)]);
    assert!(poll(&mut run, &snapshot, &mut ledger, 8, &state, &mut budget).is_pending());
    assert!(matches!(last_interaction(&ledger), InteractReq::Npc { .. }));
    accept_last(&mut ledger, 8, 1);
    let branch = row
        .talk
        .branches
        .iter()
        .find(|branch| branch.outcome == BranchOutcome::Advice)
        .unwrap();
    page_snapshot(&mut snapshot, &branch.opener);
    let mut tick = 9;
    next_page_send(
        &mut run,
        &snapshot,
        &mut ledger,
        &mut tick,
        &state,
        &mut budget,
    );
    assert_eq!(talk_drive(&run).reach_failures, 2);
    assert_eq!(budget.talks, 1);
}

/// R3-2: failed re-approaches use the same budget as cancels. When the
/// budget is spent, the step fails with the last failure's detail: the
/// approach, its walk's end, and both tiles.
#[test]
fn failed_re_approaches_end_in_a_terminal_failure_that_keeps_the_approach_detail() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let state = state();
    let mut snapshot = npc_snapshot_with_open_scene(row);
    let (mut run, mut budget, mut ledger) = start_talk(row, &snapshot, &state);
    let anchor = row.target.anchor;

    message(&mut snapshot, 1, CANT_REACH_TEXT);
    assert!(poll(&mut run, &snapshot, &mut ledger, 3, &state, &mut budget).is_pending());
    let mut tick = 4;
    let mut result = Poll::Pending;
    let mut approach_started = 0;
    let mut away = 5;
    for attempt in 2..=TALK_REACH_RETRIES + 1 {
        // Each attempt walks to where he was, and he is elsewhere on arrival.
        let seen = WorldTile {
            x: anchor.x + away,
            ..anchor
        };
        snapshot.seed_npcs(vec![npc_view(row, seen, away.abs())]);
        approach_started = tick;
        assert!(poll(&mut run, &snapshot, &mut ledger, tick, &state, &mut budget).is_pending());
        assert!(matches!(
            ledger.as_ref().unwrap().outbox.last().unwrap().effect,
            HostEffect::Walk(_)
        ));
        away = -away;
        snapshot.seed_npcs(vec![npc_view(
            row,
            WorldTile {
                x: anchor.x + away,
                ..anchor
            },
            away.abs(),
        )]);
        arrive(&mut ledger, tick + 1);
        result = poll(
            &mut run,
            &snapshot,
            &mut ledger,
            tick + 1,
            &state,
            &mut budget,
        );
        if attempt <= TALK_REACH_RETRIES {
            assert!(result.is_pending(), "attempt {attempt} is retried");
            assert_eq!(talk_drive(&run).reach_failures, attempt);
        }
        tick += 2;
    }
    let Poll::Ready(Err(PrereqRefusal::StepFailed { why, .. })) = result else {
        panic!("the budget is spent: expected a terminal failure");
    };
    let PrereqFailure::TalkUnreachable {
        attempts,
        since,
        at,
        from,
        npc,
        cause,
    } = why
    else {
        panic!("expected TalkUnreachable, saw {why:?}");
    };
    assert_eq!(attempts, TALK_REACH_RETRIES + 1);
    assert_eq!(
        cause,
        TalkReachCause::Approach {
            walk: Some(WalkEnd::Arrived)
        },
        "the last failure's detail is kept: the approach and its walk's end"
    );
    assert_eq!(since, approach_started, "the failed approach began then");
    assert_eq!(at, tick - 1);
    assert_eq!(from, Some(anchor));
    assert_eq!(
        npc,
        Some(WorldTile {
            x: anchor.x + away,
            ..anchor
        })
    );
    assert_eq!(budget.talks, 1);
    assert!(!ledger.as_ref().unwrap().test_owner_live());
}

fn modal_wait(ledger: &Option<Box<ledger::Ledger>>) -> Option<u64> {
    ledger
        .as_ref()
        .unwrap()
        .modal_wait()
        .map(std::num::NonZeroU64::get)
}

/// The script-owned auto-run hold of a prerequisite Talk: the dialogue holds a
/// modal-wait lease while it waits for the result of its own send, holds
/// none while it approaches or drives an open page, and takes a new lease for
/// each send. A cancel drops the attempt's owner and its lease at once.
#[test]
fn a_talk_holds_the_auto_run_wait_only_while_its_own_send_awaits_a_page() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let state = state();
    let mut snapshot = npc_snapshot_with_open_scene(row);
    let (mut run, mut budget, mut ledger) = start_talk(row, &snapshot, &state);
    let anchor = row.target.anchor;
    let talk = modal_wait(&ledger).expect("the sent Talk waits for its page");
    assert!(poll(&mut run, &snapshot, &mut ledger, 3, &state, &mut budget).is_pending());
    assert_eq!(
        modal_wait(&ledger),
        Some(talk),
        "no page yet: the wait holds"
    );

    message(&mut snapshot, 1, CANT_REACH_TEXT);
    assert!(poll(&mut run, &snapshot, &mut ledger, 4, &state, &mut budget).is_pending());
    assert_eq!(
        modal_wait(&ledger),
        None,
        "the cancel ends the wait at once"
    );

    let far = WorldTile {
        x: anchor.x + 5,
        ..anchor
    };
    snapshot.seed_npcs(vec![npc_view(row, far, 5)]);
    assert!(poll(&mut run, &snapshot, &mut ledger, 5, &state, &mut budget).is_pending());
    assert!(matches!(
        ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        HostEffect::Walk(_)
    ));
    assert_eq!(modal_wait(&ledger), None, "an approach expects no page");

    arrive(&mut ledger, 6);
    let near = WorldTile {
        x: anchor.x + 1,
        ..anchor
    };
    snapshot.seed_npcs(vec![npc_view(row, near, 1)]);
    assert!(poll(&mut run, &snapshot, &mut ledger, 7, &state, &mut budget).is_pending());
    assert!(matches!(last_interaction(&ledger), InteractReq::Npc { .. }));
    let retry = modal_wait(&ledger).expect("the new Talk waits for its page");
    assert_ne!(retry, talk, "each wait has its own key");
    accept_last(&mut ledger, 7, 1);

    let branch = row
        .talk
        .branches
        .iter()
        .find(|branch| branch.outcome == BranchOutcome::Advice)
        .unwrap();
    page_snapshot(&mut snapshot, &branch.opener);
    let mut tick = 8;
    next_page_send(
        &mut run,
        &snapshot,
        &mut ledger,
        &mut tick,
        &state,
        &mut budget,
    );
    let advance = modal_wait(&ledger).expect("the Continue waits for its result");
    assert_ne!(advance, retry, "a new send takes a new lease");

    let handle = match &mut run.phase {
        Phase::Talk(drive) => drive.handle.take(),
        _ => panic!("the runner is not in its Talk phase"),
    };
    drop(handle);
    assert_eq!(
        modal_wait(&ledger),
        None,
        "a dropped dialogue ends its wait"
    );
}

#[test]
fn a_cancel_message_older_than_the_send_does_not_end_the_wait() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let state = state();
    let mut snapshot = npc_snapshot(row);
    let mut ledger = None;
    let mut run = runner(row, &row.crossings[0], Truth::False);
    let mut budget = PrereqBudget::default();
    // An earlier interaction's refusal is already in the chat when the Talk
    // is sent; the dispatch records that chat position.
    message(&mut snapshot, 4, CANT_REACH_TEXT);
    assert!(poll(&mut run, &snapshot, &mut ledger, 1, &state, &mut budget).is_pending());
    arrive(&mut ledger, 1);
    assert!(poll(&mut run, &snapshot, &mut ledger, 2, &state, &mut budget).is_pending());
    assert!(matches!(last_interaction(&ledger), InteractReq::Npc { .. }));
    accept_last(&mut ledger, 2, 4);
    for tick in 3..=2 + talk_bound(row) {
        assert!(
            poll(&mut run, &snapshot, &mut ledger, tick, &state, &mut budget).is_pending(),
            "tick {tick}"
        );
        assert_eq!(talk_drive(&run).reach_failures, 0);
    }
}

#[test]
fn an_ordinary_talk_still_passes_inside_its_content_bound_and_fails_past_it() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    assert_eq!(talk_bound(row), 7, "the derived W5 bound is unchanged");
    let advice = row
        .talk
        .branches
        .iter()
        .find(|branch| branch.outcome == BranchOutcome::Advice)
        .unwrap();

    // The page arrives as late as the bound allows: no failure.
    let state = state();
    let mut snapshot = npc_snapshot(row);
    full_pack(&mut snapshot);
    let (mut run, mut budget, mut ledger) = start_talk(row, &snapshot, &state);
    for tick in 3..2 + talk_bound(row) {
        assert!(poll(&mut run, &snapshot, &mut ledger, tick, &state, &mut budget).is_pending());
    }
    page_snapshot(&mut snapshot, &advice.opener);
    let late = 2 + talk_bound(row);
    assert!(poll(&mut run, &snapshot, &mut ledger, late, &state, &mut budget).is_pending());
    assert_eq!(talk_drive(&run).reach_failures, 0);

    // No page and no cancel message by the next tick: the wait is spent.
    let state = self::state();
    let snapshot = npc_snapshot(row);
    let (mut run, mut budget, mut ledger) = start_talk(row, &snapshot, &state);
    for tick in 3..=2 + talk_bound(row) {
        assert!(poll(&mut run, &snapshot, &mut ledger, tick, &state, &mut budget).is_pending());
    }
    let result = poll(
        &mut run,
        &snapshot,
        &mut ledger,
        3 + talk_bound(row),
        &state,
        &mut budget,
    );
    assert!(matches!(
        result,
        Poll::Ready(Err(PrereqRefusal::StepFailed {
            why: PrereqFailure::WaitSpent {
                wait: PrereqWaitName::W5,
                since: 2,
                bound: 7,
                ..
            },
            ..
        }))
    ));
}

#[test]
fn a_walk_deadline_returns_no_route_to_step_without_a_fact_or_retry() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, row) = row(ConsentSwitch::PrereqDialogue);
    let state = state();
    let snapshot = snapshot(WorldTile {
        x: 1,
        z: 2,
        level: 0,
    });
    let mut run = runner(row, &row.crossings[0], Truth::False);
    let mut budget = PrereqBudget::default();
    let mut ledger = None;
    assert!(poll(&mut run, &snapshot, &mut ledger, 1, &state, &mut budget).is_pending());
    assert_eq!(
        ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .filter(|effect| matches!(effect.effect, HostEffect::Walk(_)))
            .count(),
        1,
        "the deadline starts with one owned walk"
    );
    let before = state.prereq_snapshot();
    let result = with_tick(&snapshot, &mut ledger, 1, |native| {
        native.cx.active_now = crate::native::walk::WALK_DEADLINE + Duration::from_millis(1);
        run.poll(native.actions, &mut native.cx, Some(&state), &mut budget)
    });
    assert!(matches!(
        result,
        Poll::Ready(Err(PrereqRefusal::NoRouteToStep {
            end: WalkEnd::Failed,
            ..
        }))
    ));
    assert_eq!(state.prereq_snapshot(), before);
    assert_eq!(budget.talks, 0);
    assert_eq!(
        ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .filter(|effect| matches!(effect.effect, HostEffect::Walk(_)))
            .count(),
        0,
    );
    assert!(!ledger.as_ref().unwrap().test_owner_live());
}

#[test]
fn the_runner_module_reads_no_active_clock() {
    for source in [include_str!("run.rs"), include_str!("walk.rs")] {
        assert!(!source.contains("active_now"));
    }
}

fn kitchen_journal_gate() -> ActionableGate {
    use api::prereq_steps::{CrossingId, CrossingKind};
    use api::selected::{FactKey, StageWindow};
    ActionableGate {
        gate: QuestGate::Window(StageWindow {
            quest: FactKey::new("blackarmgang"),
            signal: FactKey::new("phoenixgang"),
            values: InclusiveRange {
                min: Some(10),
                max: Some(10),
            },
        }),
        truth: Truth::Unknown,
        leg: 0,
        crossing: CrossingId {
            loc_id: 2628,
            option: 1,
            at: WorldTile {
                x: 2788,
                z: 3190,
                level: 0,
            },
            takeoff: WorldTile {
                x: 2788,
                z: 3190,
                level: 0,
            },
            to: WorldTile {
                x: 2788,
                z: 3189,
                level: 0,
            },
            kind: CrossingKind::Door,
        },
    }
}

fn kitchen_journal_observation(lines: &[&str], expected: Option<Truth>) {
    journal_observation(
        kitchen_journal_gate(),
        "Shield of Arrav",
        "Shield of Arrav",
        0x00f800,
        lines,
        expected,
    );
}

fn journal_observation(
    gate: ActionableGate,
    title: &str,
    row_title: &str,
    colour: i32,
    lines: &[&str],
    expected: Option<Truth>,
) {
    use api::snapshot::QuestStatusView;
    let state = state();
    let _ = &*CATALOG;
    let mut snapshot = snapshot(gate.crossing.takeoff);
    snapshot.seed_quest_statuses(
        vec![QuestStatusView {
            name: row_title.into(),
            component_id: 42,
            colour,
        }],
        true,
    );
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 1, |native| {
        assert!(
            PrereqRun::first_supported(
                std::slice::from_ref(&gate),
                &RunEntry::NeedsStep,
                &native.cx
            )
            .is_some(),
            "an unknown gate needs an authoritative journal evidence source"
        );
    });
    let mut run = PrereqRun::begin(
        gate.clone(),
        RunEntry::NeedsStep,
        PrereqAllow {
            dialogue: false,
            quest_start: false,
        },
    );
    let mut budget = PrereqBudget::default();
    assert!(poll(&mut run, &snapshot, &mut ledger, 1, &state, &mut budget).is_pending());
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::IfButton { component_id: 42 }
    ));
    accept_last(&mut ledger, 1, 0);
    // A causal journal frame is required. Neither a green row nor the send
    // receipt is membership evidence.
    assert!(state.prereq_snapshot().facts().is_empty());
    assert!(poll(&mut run, &snapshot, &mut ledger, 1, &state, &mut budget).is_pending());
    assert_eq!(
        ledger.as_ref().unwrap().outbox.len(),
        1,
        "polling never repeats the page request"
    );
    let mut widgets = vec![crate::quest_journal::test_widget(
        crate::quest_journal::TITLE_289,
        &format!("@dre@{title}"),
    )];
    widgets.extend(
        lines
            .iter()
            .enumerate()
            .map(|(index, line)| crate::quest_journal::test_widget(8145 + index as i32, line)),
    );
    snapshot.seed_main_modal(crate::quest_journal::ROOT_289, widgets);
    assert!(poll(&mut run, &snapshot, &mut ledger, 2, &state, &mut budget).is_pending());
    assert!(matches!(last_interaction(&ledger), InteractReq::CloseModal));
    accept_last(&mut ledger, 2, 0);
    assert!(
        state.prereq_snapshot().facts().is_empty(),
        "wait for the observed close"
    );
    snapshot.seed_main_modal(-1, vec![]);
    let result = poll(&mut run, &snapshot, &mut ledger, 3, &state, &mut budget);
    match expected {
        Some(Truth::True) => assert!(matches!(result, Poll::Ready(Ok(Done { crossed: false })))),
        Some(Truth::False) => assert!(matches!(
            result,
            Poll::Ready(Err(PrereqRefusal::PreconditionMissing { .. }))
        )),
        _ => assert!(matches!(
            result,
            Poll::Ready(Err(PrereqRefusal::StepUnwitnessed { .. }))
        )),
    }
    if expected.is_none() {
        assert!(state.prereq_snapshot().facts().is_empty());
    }
    with_tick(&snapshot, &mut ledger, 3, |native| {
        let acquired = EvidenceStamp {
            tick: 2,
            sequence: 2,
            ..native.cx.evidence()
        };
        assert_eq!(
            state
                .prereq_snapshot()
                .observe(&gate.gate, acquired)
                .map(|fact| fact.truth),
            expected
        );
        if expected == Some(Truth::False) {
            assert_eq!(
                state
                    .prereq_snapshot()
                    .observe(&gate.gate, native.cx.evidence())
                    .unwrap()
                    .truth,
                Truth::Unknown,
                "a negative rule does not overclaim future Phoenix progress"
            );
        }
    });
    assert_eq!(
        ledger.as_ref().unwrap().outbox.len(),
        2,
        "one page request and one close, never a door or NPC probe"
    );
}

#[test]
fn kitchen_journal_phoenix_completion_establishes_membership() {
    if !api::test_support::game_data_available() {
        return;
    }
    // blackarmgang_journal.rs2:16,96-108,203-204. The row colour is shared with Black Arm.
    kitchen_journal_observation(
        &[
            "@str@I searched the Phoenix Gang Headquarters and found",
            "@str@half of the Shield of Arrav.",
            "@red@QUEST COMPLETE!",
        ],
        Some(Truth::True),
    );
}

#[test]
fn kitchen_journal_joined_phoenix_page_without_completion_leaves_no_fact() {
    if !api::test_support::game_data_available() {
        return;
    }
    // phoenixgang = 9, blackarmgang_journal.rs2:16,60-63,203-204.
    // Two needles match, but this page never says QUEST COMPLETE.
    kitchen_journal_observation(
        &[
            "@dbl@I searched the @dre@Phoenix Gang Headquarters@dbl@ and found",
            "what seemed to be half of the @dre@Shield of Arrav@dbl@. A",
            "@dre@Fellow Adventurer@dbl@ also found a @dre@Shield Half@dbl@ and gave it to",
            "me. I took them both to the @dre@Curator@dbl@ in @dre@Varrock Museum",
        ],
        None,
    );
}

fn kitchen_journal_timeout_run() -> (
    PrereqRun,
    GameSnapshot,
    Option<Box<ledger::Ledger>>,
    SlotState,
    PrereqBudget,
) {
    let _ = &*CATALOG;
    let gate = kitchen_journal_gate();
    let mut snapshot = snapshot(gate.crossing.takeoff);
    snapshot.seed_quest_statuses(
        vec![api::snapshot::QuestStatusView {
            name: "Shield of Arrav".into(),
            component_id: 42,
            colour: 0x00f800,
        }],
        true,
    );
    (
        PrereqRun::begin(gate, RunEntry::NeedsStep, PrereqAllow::default()),
        snapshot,
        None,
        state(),
        PrereqBudget::default(),
    )
}

#[test]
fn kitchen_journal_timeout_retries_once_on_a_later_observed_tick_and_recovers() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut run, mut snapshot, mut ledger, state, mut budget) = kitchen_journal_timeout_run();
    assert_eq!(run.journal_attempt(), None);
    assert!(poll(&mut run, &snapshot, &mut ledger, 1, &state, &mut budget).is_pending());
    assert_eq!(
        run.journal_attempt(),
        Some(JournalAttempt {
            attempts_started: 1,
            owner_active: true,
            retry_after: None,
        })
    );
    accept_last(&mut ledger, 1, 0);
    let first_request = ledger.as_ref().unwrap().outbox[0].authority().request_id();
    // Inject the lost page by leaving the modal closed for the reader's five ticks.
    for tick in 2..=6 {
        assert!(poll(&mut run, &snapshot, &mut ledger, tick, &state, &mut budget).is_pending());
        assert!(state.prereq_snapshot().facts().is_empty());
    }
    let timed_out = Some(JournalAttempt {
        attempts_started: 1,
        owner_active: false,
        retry_after: Some(6),
    });
    assert_eq!(run.journal_attempt(), timed_out);
    for _ in 0..3 {
        assert!(poll(&mut run, &snapshot, &mut ledger, 6, &state, &mut budget).is_pending());
        assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
        assert_eq!(run.journal_attempt(), timed_out);
    }
    assert!(poll(&mut run, &snapshot, &mut ledger, 7, &state, &mut budget).is_pending());
    assert_eq!(
        run.journal_attempt(),
        Some(JournalAttempt {
            attempts_started: 2,
            owner_active: true,
            retry_after: Some(6),
        })
    );
    // Beginning the retry retires the first owner's queued actions.
    assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
    assert_ne!(
        ledger.as_ref().unwrap().outbox[0].authority().request_id(),
        first_request
    );
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::IfButton { component_id: 42 }
    ));
    assert_eq!(
        budget.visited_len, 1,
        "a retry does not spend another catalog row"
    );
    accept_last(&mut ledger, 7, 0);
    snapshot.seed_main_modal(
        crate::quest_journal::ROOT_289,
        vec![
            crate::quest_journal::test_widget(
                crate::quest_journal::TITLE_289,
                "@dre@Shield of Arrav",
            ),
            crate::quest_journal::test_widget(
                8145,
                "@str@I searched the Phoenix Gang Headquarters and found",
            ),
            crate::quest_journal::test_widget(8146, "@str@half of the Shield of Arrav."),
            crate::quest_journal::test_widget(8147, "@red@QUEST COMPLETE!"),
        ],
    );
    assert!(poll(&mut run, &snapshot, &mut ledger, 8, &state, &mut budget).is_pending());
    assert!(
        state.prereq_snapshot().facts().is_empty(),
        "an observed close still gates proof"
    );
    assert!(matches!(last_interaction(&ledger), InteractReq::CloseModal));
    accept_last(&mut ledger, 8, 0);
    snapshot.seed_main_modal(-1, vec![]);
    assert!(matches!(
        poll(&mut run, &snapshot, &mut ledger, 9, &state, &mut budget),
        Poll::Ready(Ok(Done { crossed: false }))
    ));
    assert_eq!(run.journal_attempt(), None);
    assert_eq!(state.prereq_snapshot().facts().len(), 1);
    assert_eq!(ledger.as_ref().unwrap().outbox.len(), 2);
    #[cfg(feature = "test-hooks")]
    {
        let attempts = run.journal_attempts().collect::<Vec<_>>();
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0].0, 1);
        assert_eq!(
            attempts[0].3,
            Some("the native action failed: journal modal timeout")
        );
        assert_eq!(attempts[1].0, 2);
        assert_eq!(attempts[1].3, None);
        assert!(attempts.iter().all(|attempt| attempt.2.is_some()));
    }
}

#[test]
fn kitchen_journal_timeout_stops_after_two_reads_with_the_last_timeout_detail() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut run, snapshot, mut ledger, state, mut budget) = kitchen_journal_timeout_run();
    assert!(poll(&mut run, &snapshot, &mut ledger, 1, &state, &mut budget).is_pending());
    accept_last(&mut ledger, 1, 0);
    let first_request = ledger.as_ref().unwrap().outbox[0].authority().request_id();
    for tick in 2..=7 {
        assert!(poll(&mut run, &snapshot, &mut ledger, tick, &state, &mut budget).is_pending());
        assert!(state.prereq_snapshot().facts().is_empty());
    }
    assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
    assert_ne!(
        ledger.as_ref().unwrap().outbox[0].authority().request_id(),
        first_request
    );
    accept_last(&mut ledger, 7, 0);
    for tick in 8..12 {
        assert!(poll(&mut run, &snapshot, &mut ledger, tick, &state, &mut budget).is_pending());
    }
    assert!(matches!(
        poll(&mut run, &snapshot, &mut ledger, 12, &state, &mut budget),
        Poll::Ready(Err(PrereqRefusal::StepFailed {
            why: PrereqFailure::Action(detail), ..
        })) if detail.as_ref() == "the native action failed: journal modal timeout"
    ));
    assert!(state.prereq_snapshot().facts().is_empty());
    assert_eq!(
        ledger.as_ref().unwrap().outbox.len(),
        1,
        "no third read or gate probe"
    );
    assert!(!ledger.as_ref().unwrap().test_owner_live());
    #[cfg(feature = "test-hooks")]
    {
        let attempts = run.journal_attempts().collect::<Vec<_>>();
        assert_eq!(attempts.len(), 2);
        assert_eq!(
            attempts[0].3,
            Some("the native action failed: journal modal timeout")
        );
        assert_eq!(
            attempts[1].3,
            Some("the native action failed: journal modal timeout")
        );
        assert!(attempts.iter().all(|attempt| attempt.2.is_some()));
    }
}

#[test]
fn kitchen_journal_timeout_retry_keeps_a_different_last_failure_detail() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut run, snapshot, mut ledger, state, mut budget) = kitchen_journal_timeout_run();
    assert!(poll(&mut run, &snapshot, &mut ledger, 1, &state, &mut budget).is_pending());
    accept_last(&mut ledger, 1, 0);
    for tick in 2..=7 {
        assert!(poll(&mut run, &snapshot, &mut ledger, tick, &state, &mut budget).is_pending());
    }
    let authority = ledger.as_ref().unwrap().outbox.last().unwrap().authority();
    ledger.as_mut().unwrap().complete_interaction(
        &authority,
        InteractionReceipt {
            request_id: authority.request_id().get(),
            evidence: EvidenceStamp {
                run: authority.run(),
                tick: 7,
                sequence: 7,
            },
            accepted: false,
            chat_since: 0,
        },
    );
    assert!(matches!(
        poll(&mut run, &snapshot, &mut ledger, 8, &state, &mut budget),
        Poll::Ready(Err(PrereqRefusal::StepFailed {
            why: PrereqFailure::Action(detail), ..
        })) if detail.as_ref() == "the native action failed: journal click refused"
    ));
    #[cfg(feature = "test-hooks")]
    {
        let attempts = run.journal_attempts().collect::<Vec<_>>();
        assert_eq!(attempts.len(), 2);
        assert_eq!(
            attempts[0].3,
            Some("the native action failed: journal modal timeout")
        );
        assert_eq!(
            attempts[1].3,
            Some("the native action failed: journal click refused")
        );
    }
    assert!(state.prereq_snapshot().facts().is_empty());
    assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
}

#[test]
fn kitchen_journal_black_arm_completion_refuses_phoenix_membership() {
    if !api::test_support::game_data_available() {
        return;
    }
    // blackarmgang_journal.rs2:114,186-197. Phoenix is below joined here.
    kitchen_journal_observation(
        &[
            "@str@allowed me to join the Black Arm Gang.",
            "@str@I searched the Black Arm Gang Headquarters and found",
            "@str@half of the Shield of Arrav.",
            "@red@QUEST COMPLETE!",
        ],
        Some(Truth::False),
    );
}

#[test]
fn kitchen_journal_green_row_without_distinctive_page_never_proves_membership() {
    if !api::test_support::game_data_available() {
        return;
    }
    kitchen_journal_observation(&["@red@QUEST COMPLETE!"], None);
}

#[test]
fn kitchen_journal_conflicting_gang_pages_leave_no_fact() {
    if !api::test_support::game_data_available() {
        return;
    }
    kitchen_journal_observation(
        &[
            "@str@I searched the Phoenix Gang Headquarters and found",
            "@str@I searched the Black Arm Gang Headquarters and found",
            "@str@allowed me to join the Black Arm Gang.",
            "@str@half of the Shield of Arrav.",
            "@red@QUEST COMPLETE!",
        ],
        None,
    );
}

#[test]
fn kitchen_journal_hero_password_establishes_pre_completion_eligibility() {
    if !api::test_support::game_data_available() {
        return;
    }
    use api::selected::{FactKey, StageWindow};
    let mut gate = kitchen_journal_gate();
    gate.gate = QuestGate::Window(StageWindow {
        quest: FactKey::new("hero"),
        signal: FactKey::new("heroquest"),
        values: InclusiveRange {
            min: Some(3),
            max: Some(15),
        },
    });
    journal_observation(
        gate,
        "Heroes' Quest",
        "Hero's Quest",
        0xffff00,
        &[
            "@str@I spoke to Straven about the Master Thieves Armband.",
            "@str@Then I told Alfonse the password 'Gherkin'.",
        ],
        Some(Truth::True),
    );
}

type KitchenSelectAction = crate::quester::prereq::reach::PrereqSelect;
type KitchenSelectArgs = crate::bank::SelectArgs;
type KitchenSelectLedger = Option<Box<ledger::Ledger>>;
type KitchenPickAuthority = crate::native::ledger::HostAuthority;

fn kitchen_select_args() -> KitchenSelectArgs {
    KitchenSelectArgs {
        facts: Arc::new(api::named_banks::NamedBankFacts::from_banks(vec![
            api::named_banks::NamedBank::new(
                "First test bank",
                WorldTile {
                    x: 2784,
                    z: 3190,
                    level: 0,
                },
            ),
            api::named_banks::NamedBank::new(
                "Selected test bank",
                WorldTile {
                    x: 2790,
                    z: 3190,
                    level: 0,
                },
            ),
        ])),
        from: kitchen_journal_gate().crossing.takeoff,
        preferences: api::named_banks::BankPreferences {
            use_mage_bank: true,
            use_zanaris_bank: false,
        },
        options: crate::native::WalkOptions {
            allow_teleports: crate::native::WalkBit::Forbid,
            allow_wilderness: crate::native::WalkBit::Allow,
            ..crate::native::WalkOptions::default()
        },
        explicit: Some(Arc::from("Selected test bank")),
        allow_air_fallback: false,
    }
}

fn kitchen_select_fixture() -> (
    KitchenSelectAction,
    KitchenSelectArgs,
    GameSnapshot,
    KitchenSelectLedger,
    SlotState,
) {
    let _ = &*CATALOG;
    let gate = kitchen_journal_gate();
    let mut snapshot = snapshot(gate.crossing.takeoff);
    snapshot.seed_quest_statuses(
        vec![api::snapshot::QuestStatusView {
            name: "Shield of Arrav".into(),
            component_id: 42,
            colour: 0x00f800,
        }],
        true,
    );
    let args = kitchen_select_args();
    let state = state();
    let mut ledger = None;
    let action = with_tick(&snapshot, &mut ledger, 1, |native| {
        let mut action =
            KitchenSelectAction::begin(args.clone(), native.actions, &mut native.cx).unwrap();
        assert!(action
            .poll(native.actions, &mut native.cx, Some(&state))
            .is_pending());
        action
    });
    (action, args, snapshot, ledger, state)
}

fn with_kitchen_session<R>(
    snapshot: &GameSnapshot,
    ledger: &mut KitchenSelectLedger,
    tick: u64,
    session: u64,
    f: impl FnOnce(&mut crate::native::NativeTick<'_>) -> R,
) -> R {
    crate::quester::families::tests::with_tick_snapshots(
        snapshot,
        snapshot,
        ledger,
        tick,
        |native, snapshot| {
            let evidence = EvidenceStamp {
                run: RunKey {
                    slot: 1,
                    run: 1,
                    session,
                },
                tick,
                sequence: tick,
            };
            native.cx.evidence = evidence;
            native.cx.snapshot = SnapshotView::new(Some(snapshot), evidence);
            f(native)
        },
    )
}

fn kitchen_select_poll_in_session(
    action: &mut KitchenSelectAction,
    snapshot: &GameSnapshot,
    ledger: &mut KitchenSelectLedger,
    tick: u64,
    state: &SlotState,
    session: u64,
) -> Poll<Result<crate::bank::SelectedBank, crate::native::ActionError>> {
    with_kitchen_session(snapshot, ledger, tick, session, |native| {
        native.cx.active_now = Duration::ZERO;
        action.poll(native.actions, &mut native.cx, Some(state))
    })
}

fn kitchen_select_poll(
    action: &mut KitchenSelectAction,
    snapshot: &GameSnapshot,
    ledger: &mut KitchenSelectLedger,
    tick: u64,
    state: &SlotState,
) -> Poll<Result<crate::bank::SelectedBank, crate::native::ActionError>> {
    kitchen_select_poll_in_session(action, snapshot, ledger, tick, state, 1)
}

fn last_live_kitchen_bank_pick(
    ledger: &KitchenSelectLedger,
) -> Option<(crate::bank::BankPickRequest, KitchenPickAuthority)> {
    ledger.as_ref()?.outbox.iter().rev().find_map(|action| {
        if !action.live() {
            return None;
        }
        match &action.effect {
            HostEffect::BankPick(request) => Some((request.clone(), action.authority())),
            _ => None,
        }
    })
}

fn last_live_kitchen_journal_authority(
    ledger: &KitchenSelectLedger,
) -> Option<KitchenPickAuthority> {
    ledger.as_ref()?.outbox.iter().rev().find_map(|action| {
        if !action.live() {
            return None;
        }
        matches!(
            &action.effect,
            HostEffect::Interaction(InteractReq::IfButton { component_id: 42 })
        )
        .then(|| action.authority())
    })
}

fn assert_kitchen_bank_pick_args(request: &crate::bank::BankPickRequest, args: &KitchenSelectArgs) {
    assert!(Arc::ptr_eq(&request.facts, &args.facts));
    assert_eq!(request.facts.as_ref(), args.facts.as_ref());
    assert_eq!(request.from, args.from);
    assert_eq!(request.preferences, args.preferences);
    assert_eq!(request.options, args.options);
    assert_eq!(request.eligible.as_ref(), &[0, 1]);
    assert_eq!(request.explicit_bank, Some(1));
    assert_eq!(request.allow_air_fallback, args.allow_air_fallback);
}

fn publish_kitchen_bank_pick(
    ledger: &mut KitchenSelectLedger,
    authority: &KitchenPickAuthority,
    request_id: u64,
    tick: u64,
    outcome: crate::bank::BankPickOutcome,
) {
    ledger.as_mut().unwrap().complete_bank_pick(
        authority,
        crate::bank::BankPickReceipt {
            request_id,
            evidence: EvidenceStamp {
                run: authority.run(),
                tick,
                sequence: tick,
            },
            outcome,
        },
    );
}

fn start_kitchen_journal(
    action: &mut KitchenSelectAction,
    snapshot: &GameSnapshot,
    ledger: &mut KitchenSelectLedger,
    state: &SlotState,
    authority: &KitchenPickAuthority,
    gate: ActionableGate,
    allow: PrereqAllow,
) -> Poll<Result<crate::bank::SelectedBank, crate::native::ActionError>> {
    start_kitchen_journal_at(
        action,
        snapshot,
        ledger,
        state,
        authority,
        crate::bank::BankPickOutcome::NeedsStep {
            gates: Arc::from([gate]),
            allow,
        },
        2,
    )
}

fn start_kitchen_journal_at(
    action: &mut KitchenSelectAction,
    snapshot: &GameSnapshot,
    ledger: &mut KitchenSelectLedger,
    state: &SlotState,
    authority: &KitchenPickAuthority,
    outcome: crate::bank::BankPickOutcome,
    tick: u64,
) -> Poll<Result<crate::bank::SelectedBank, crate::native::ActionError>> {
    let allow = match &outcome {
        crate::bank::BankPickOutcome::NeedsStep { allow, .. } => *allow,
        _ => panic!("the journal fixture starts from a prerequisite diagnosis"),
    };
    publish_kitchen_bank_pick(
        ledger,
        authority,
        authority.request_id().get(),
        tick,
        outcome,
    );
    with_kitchen_session(snapshot, ledger, tick, authority.run().session, |native| {
        let observed_allow = action
            .inspect_in(&mut native.cx, |select, cx| select.prereq_allow(cx))
            .flatten();
        assert_eq!(observed_allow, Some(allow));
        native.cx.active_now = Duration::ZERO;
        action.poll(native.actions, &mut native.cx, Some(state))
    })
}

fn seed_kitchen_journal_page(snapshot: &mut GameSnapshot, lines: &[&str]) {
    let mut widgets = vec![crate::quest_journal::test_widget(
        crate::quest_journal::TITLE_289,
        "@dre@Shield of Arrav",
    )];
    widgets.extend(
        lines
            .iter()
            .enumerate()
            .map(|(index, line)| crate::quest_journal::test_widget(8145 + index as i32, line)),
    );
    snapshot.seed_main_modal(crate::quest_journal::ROOT_289, widgets);
}

fn assert_no_kitchen_bank_reissue(ledger: &KitchenSelectLedger, original_request_id: u64) {
    let outbox = &ledger.as_ref().unwrap().outbox;
    assert!(outbox.iter().all(|action| {
        !matches!(&action.effect, HostEffect::BankPick(_))
            || action.authority().request_id().get() == original_request_id
    }));
}

#[test]
fn bank_select_reissues_only_after_the_real_journal_owner_proves_the_gate() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut action, args, mut snapshot, mut ledger, state) = kitchen_select_fixture();
    let gate = kitchen_journal_gate();
    let (first_request, first_authority) =
        last_live_kitchen_bank_pick(&ledger).expect("Select emitted its bank-pick request");
    assert_kitchen_bank_pick_args(&first_request, &args);

    let allow = PrereqAllow {
        dialogue: true,
        quest_start: false,
    };
    assert!(start_kitchen_journal(
        &mut action,
        &snapshot,
        &mut ledger,
        &state,
        &first_authority,
        gate.clone(),
        allow,
    )
    .is_pending());
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::IfButton { component_id: 42 }
    ));
    accept_last(&mut ledger, 2, 0);

    seed_kitchen_journal_page(
        &mut snapshot,
        &[
            "@str@I searched the Phoenix Gang Headquarters and found",
            "@str@half of the Shield of Arrav.",
            "@red@QUEST COMPLETE!",
        ],
    );
    assert!(kitchen_select_poll(&mut action, &snapshot, &mut ledger, 3, &state).is_pending());
    assert!(matches!(last_interaction(&ledger), InteractReq::CloseModal));
    accept_last(&mut ledger, 3, 0);
    snapshot.seed_main_modal(-1, vec![]);
    assert!(kitchen_select_poll(&mut action, &snapshot, &mut ledger, 4, &state).is_pending());
    assert_eq!(state.prereq_snapshot().facts().len(), 1);
    assert_eq!(
        state
            .prereq_snapshot()
            .observe(
                &gate.gate,
                EvidenceStamp {
                    run: first_authority.run(),
                    tick: 3,
                    sequence: 3,
                },
            )
            .map(|fact| fact.truth),
        Some(Truth::True),
        "the bank owner may reissue only after acquired page proof and observed close"
    );

    let mut reissued = last_live_kitchen_bank_pick(&ledger)
        .filter(|(_, authority)| authority.request_id() != first_authority.request_id());
    for tick in 5..=8 {
        if reissued.is_some() {
            break;
        }
        assert!(
            kitchen_select_poll(&mut action, &snapshot, &mut ledger, tick, &state).is_pending()
        );
        reissued = last_live_kitchen_bank_pick(&ledger)
            .filter(|(_, authority)| authority.request_id() != first_authority.request_id());
    }
    let (reissued_request, reissued_authority) =
        reissued.expect("a fresh bank-pick request follows the journal proof");
    assert_ne!(
        reissued_authority.request_id(),
        first_authority.request_id()
    );
    assert_kitchen_bank_pick_args(&reissued_request, &args);
    assert_eq!(action.original_allow(), Some(allow));
    assert!(
        !first_authority.live(),
        "the first bank owner has been retired"
    );

    let stale_selected = crate::bank::SelectedBank {
        bank_index: 0,
        access_tile: args.facts.banks()[0].tile,
        kind: crate::bank::PickKind::Reachable,
        access: None,
    };
    publish_kitchen_bank_pick(
        &mut ledger,
        &first_authority,
        first_authority.request_id().get(),
        9,
        crate::bank::BankPickOutcome::Selected(stale_selected.clone()),
    );
    publish_kitchen_bank_pick(
        &mut ledger,
        &reissued_authority,
        first_authority.request_id().get(),
        9,
        crate::bank::BankPickOutcome::Selected(stale_selected),
    );
    assert!(kitchen_select_poll(&mut action, &snapshot, &mut ledger, 9, &state).is_pending());

    let selected = crate::bank::SelectedBank {
        bank_index: 1,
        access_tile: args.facts.banks()[1].tile,
        kind: crate::bank::PickKind::Reachable,
        access: None,
    };
    publish_kitchen_bank_pick(
        &mut ledger,
        &reissued_authority,
        reissued_authority.request_id().get(),
        10,
        crate::bank::BankPickOutcome::Selected(selected.clone()),
    );
    assert!(matches!(
        kitchen_select_poll(&mut action, &snapshot, &mut ledger, 11, &state),
        Poll::Ready(Ok(found)) if found == selected
    ));
}

#[test]
fn bank_select_session_change_revalidates_same_gate_reads_and_completes() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut action, args, mut snapshot, mut ledger, state) = kitchen_select_fixture();
    let gate = kitchen_journal_gate();
    let (first_request, first_authority) =
        last_live_kitchen_bank_pick(&ledger).expect("Select emitted its bank-pick request");
    assert_kitchen_bank_pick_args(&first_request, &args);
    let allow = PrereqAllow {
        dialogue: true,
        quest_start: false,
    };
    assert!(start_kitchen_journal(
        &mut action,
        &snapshot,
        &mut ledger,
        &state,
        &first_authority,
        gate.clone(),
        allow,
    )
    .is_pending());
    let old_reader = last_live_kitchen_journal_authority(&ledger)
        .expect("the first session owns a real journal read");
    assert_eq!(old_reader.run().session, 1);

    let same_session = with_tick(&snapshot, &mut ledger, 3, |native| {
        action.resume_after_session(
            native.actions,
            crate::native::request::NativeKind::BankPlan,
            &mut native.cx,
        )
    });
    assert!(matches!(
        same_session,
        Err(crate::native::ActionError::Stale)
    ));
    assert!(
        old_reader.live(),
        "same-session misuse leaves the old reader intact"
    );

    let resumed = with_kitchen_session(&snapshot, &mut ledger, 4, 2, |native| {
        action.resume_after_session(
            native.actions,
            crate::native::request::NativeKind::BankPlan,
            &mut native.cx,
        )
    });
    assert!(resumed.is_ok(), "{resumed:?}");
    assert!(!old_reader.live(), "the old-session reader is retired");

    let mut reissued = None;
    for tick in 5..=9 {
        let result =
            kitchen_select_poll_in_session(&mut action, &snapshot, &mut ledger, tick, &state, 2);
        assert!(
            result.is_pending(),
            "session-2 selection at tick {tick}: {result:?}"
        );
        reissued = last_live_kitchen_bank_pick(&ledger)
            .filter(|(_, authority)| authority.request_id() != first_authority.request_id());
        if reissued.is_some() {
            break;
        }
    }
    let (reissued_request, reissued_authority) =
        reissued.expect("the kept operation revalidates selection in session 2");
    assert_eq!(reissued_authority.run().session, 2);
    assert_kitchen_bank_pick_args(&reissued_request, &args);
    assert_eq!(action.original_allow(), Some(allow));
    assert!(
        !first_authority.live(),
        "the original bank owner is retired"
    );

    let changed_allow = PrereqAllow {
        dialogue: false,
        quest_start: true,
    };
    assert!(start_kitchen_journal_at(
        &mut action,
        &snapshot,
        &mut ledger,
        &state,
        &reissued_authority,
        crate::bank::BankPickOutcome::NeedsStep {
            gates: Arc::from([gate.clone()]),
            allow: changed_allow,
        },
        10,
    )
    .is_pending());
    assert_eq!(
        action.original_allow(),
        Some(allow),
        "the first effective grants remain authoritative after revalidation"
    );
    let new_reader = last_live_kitchen_journal_authority(&ledger)
        .expect("the reissued selection starts a genuine new-session journal read");
    assert_eq!(new_reader.run().session, 2);
    assert!(!old_reader.live());
    assert!(!reissued_authority.live());
    accept_last(&mut ledger, 10, 0);

    seed_kitchen_journal_page(
        &mut snapshot,
        &[
            "@str@I searched the Phoenix Gang Headquarters and found",
            "@str@half of the Shield of Arrav.",
            "@red@QUEST COMPLETE!",
        ],
    );
    assert!(
        kitchen_select_poll_in_session(&mut action, &snapshot, &mut ledger, 11, &state, 2)
            .is_pending()
    );
    assert!(matches!(last_interaction(&ledger), InteractReq::CloseModal));
    accept_last(&mut ledger, 11, 0);
    snapshot.seed_main_modal(-1, vec![]);
    assert!(
        kitchen_select_poll_in_session(&mut action, &snapshot, &mut ledger, 12, &state, 2)
            .is_pending()
    );
    assert_eq!(
        state
            .prereq_snapshot()
            .observe(
                &gate.gate,
                EvidenceStamp {
                    run: reissued_authority.run(),
                    tick: 11,
                    sequence: 11,
                },
            )
            .map(|fact| fact.truth),
        Some(Truth::True),
        "the new-session page proof completes the prerequisite"
    );

    let mut completed_selection = None;
    for tick in 13..=17 {
        let result =
            kitchen_select_poll_in_session(&mut action, &snapshot, &mut ledger, tick, &state, 2);
        assert!(
            result.is_pending(),
            "completed selection at tick {tick}: {result:?}"
        );
        completed_selection = last_live_kitchen_bank_pick(&ledger)
            .filter(|(_, authority)| authority.request_id() != reissued_authority.request_id());
        if completed_selection.is_some() {
            break;
        }
    }
    let (request, authority) =
        completed_selection.expect("the journal completion reissues the original bank pick");
    assert_eq!(authority.run().session, 2);
    assert_kitchen_bank_pick_args(&request, &args);
    assert_eq!(action.original_allow(), Some(allow));

    let selected = crate::bank::SelectedBank {
        bank_index: 1,
        access_tile: args.facts.banks()[1].tile,
        kind: crate::bank::PickKind::Reachable,
        access: None,
    };
    publish_kitchen_bank_pick(
        &mut ledger,
        &authority,
        authority.request_id().get(),
        18,
        crate::bank::BankPickOutcome::Selected(selected.clone()),
    );
    assert!(matches!(
        kitchen_select_poll_in_session(&mut action, &snapshot, &mut ledger, 18, &state, 2),
        Poll::Ready(Ok(found)) if found == selected
    ));
}

#[test]
fn bank_select_same_row_session_reissues_stop_at_global_cap() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut action, args, snapshot, mut ledger, state) = kitchen_select_fixture();
    let gate = kitchen_journal_gate();
    let (first_request, first_authority) =
        last_live_kitchen_bank_pick(&ledger).expect("Select emitted its bank-pick request");
    assert_kitchen_bank_pick_args(&first_request, &args);
    let allow = PrereqAllow {
        dialogue: true,
        quest_start: false,
    };
    assert!(start_kitchen_journal(
        &mut action,
        &snapshot,
        &mut ledger,
        &state,
        &first_authority,
        gate.clone(),
        allow,
    )
    .is_pending());
    let mut bank_request_ids = vec![first_authority.request_id()];
    let mut journal_request_ids = vec![last_live_kitchen_journal_authority(&ledger)
        .expect("first session journal reader")
        .request_id()];
    let mut previous_bank_request = first_authority.request_id();
    let mut tick = 3;

    for session in 2..=4 {
        let old_reader = last_live_kitchen_journal_authority(&ledger)
            .expect("each prior session owns a real journal read");
        assert_eq!(old_reader.run().session, session - 1);
        let resumed = with_kitchen_session(&snapshot, &mut ledger, tick, session, |native| {
            action.resume_after_session(
                native.actions,
                crate::native::request::NativeKind::BankPlan,
                &mut native.cx,
            )
        });
        assert!(resumed.is_ok(), "{resumed:?}");
        assert!(
            !old_reader.live(),
            "session change retires the old journal owner"
        );
        tick += 1;

        let mut reissued = None;
        for _ in 0..5 {
            let result = kitchen_select_poll_in_session(
                &mut action,
                &snapshot,
                &mut ledger,
                tick,
                &state,
                session,
            );
            assert!(
                result.is_pending(),
                "session-{session} selection: {result:?}"
            );
            reissued = last_live_kitchen_bank_pick(&ledger)
                .filter(|(_, authority)| authority.request_id() != previous_bank_request);
            tick += 1;
            if reissued.is_some() {
                break;
            }
        }
        let (request, authority) =
            reissued.expect("each allowed logical reissue sends a fresh bank selection");
        assert_eq!(authority.run().session, session);
        assert_kitchen_bank_pick_args(&request, &args);
        bank_request_ids.push(authority.request_id());
        previous_bank_request = authority.request_id();

        publish_kitchen_bank_pick(
            &mut ledger,
            &authority,
            authority.request_id().get(),
            tick,
            crate::bank::BankPickOutcome::NeedsStep {
                gates: Arc::from([gate.clone()]),
                allow,
            },
        );
        let result = kitchen_select_poll_in_session(
            &mut action,
            &snapshot,
            &mut ledger,
            tick,
            &state,
            session,
        );
        if session < 4 {
            assert!(
                result.is_pending(),
                "the same row is actionable in session {session}: {result:?}"
            );
            let reader = last_live_kitchen_journal_authority(&ledger)
                .expect("same prerequisite starts a new-session journal read");
            assert_eq!(reader.run().session, session);
            journal_request_ids.push(reader.request_id());
        } else {
            assert!(matches!(
                result,
                Poll::Ready(Err(crate::native::ActionError::Prerequisite(refusal)))
                    if matches!(refusal.as_ref(), PrereqRefusal::StepRepeated { .. })
            ));
            assert_eq!(
                journal_request_ids.len(),
                3,
                "the logical reissue cap stops before a fourth session reader"
            );
        }
        tick += 1;
    }

    assert_eq!(bank_request_ids.len(), 4);
    assert!(bank_request_ids.windows(2).all(|ids| ids[0] != ids[1]));
    assert_eq!(journal_request_ids.len(), 3);
    assert_eq!(action.original_allow(), Some(allow));
}

#[test]
fn false_journal_page_does_not_reissue_or_select_a_bank() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut action, _, mut snapshot, mut ledger, state) = kitchen_select_fixture();
    let gate = kitchen_journal_gate();
    let (_, first_authority) =
        last_live_kitchen_bank_pick(&ledger).expect("Select emitted its bank-pick request");
    let allow = PrereqAllow {
        dialogue: false,
        quest_start: false,
    };
    assert!(start_kitchen_journal(
        &mut action,
        &snapshot,
        &mut ledger,
        &state,
        &first_authority,
        gate.clone(),
        allow,
    )
    .is_pending());
    accept_last(&mut ledger, 2, 0);

    seed_kitchen_journal_page(
        &mut snapshot,
        &[
            "@str@allowed me to join the Black Arm Gang.",
            "@str@I searched the Black Arm Gang Headquarters and found",
            "@str@half of the Shield of Arrav.",
            "@red@QUEST COMPLETE!",
        ],
    );
    assert!(kitchen_select_poll(&mut action, &snapshot, &mut ledger, 3, &state).is_pending());
    assert!(matches!(last_interaction(&ledger), InteractReq::CloseModal));
    accept_last(&mut ledger, 3, 0);
    snapshot.seed_main_modal(-1, vec![]);
    let result = kitchen_select_poll(&mut action, &snapshot, &mut ledger, 4, &state);
    assert!(matches!(
        &result,
        Poll::Ready(Err(crate::native::ActionError::Prerequisite(refusal)))
            if matches!(refusal.as_ref(), PrereqRefusal::PreconditionMissing { .. })
    ));
    assert_eq!(
        state
            .prereq_snapshot()
            .observe(
                &gate.gate,
                EvidenceStamp {
                    run: first_authority.run(),
                    tick: 3,
                    sequence: 3,
                },
            )
            .map(|fact| fact.truth),
        Some(Truth::False)
    );
    assert_no_kitchen_bank_reissue(&ledger, first_authority.request_id().get());
}

#[test]
fn bank_select_journal_timeout_stops_after_two_reads_without_reissue() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut action, _, snapshot, mut ledger, state) = kitchen_select_fixture();
    let gate = kitchen_journal_gate();
    let (_, first_authority) =
        last_live_kitchen_bank_pick(&ledger).expect("Select emitted its bank-pick request");
    let allow = PrereqAllow {
        dialogue: true,
        quest_start: false,
    };
    assert!(start_kitchen_journal(
        &mut action,
        &snapshot,
        &mut ledger,
        &state,
        &first_authority,
        gate,
        allow,
    )
    .is_pending());
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::IfButton { component_id: 42 }
    ));
    let first_read_id = ledger
        .as_ref()
        .unwrap()
        .outbox
        .last()
        .unwrap()
        .authority()
        .request_id();
    accept_last(&mut ledger, 2, 0);
    for tick in 3..=7 {
        assert!(
            kitchen_select_poll(&mut action, &snapshot, &mut ledger, tick, &state).is_pending(),
            "first journal read at tick {tick}"
        );
    }

    assert!(kitchen_select_poll(&mut action, &snapshot, &mut ledger, 8, &state).is_pending());
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::IfButton { component_id: 42 }
    ));
    let second_read_id = ledger
        .as_ref()
        .unwrap()
        .outbox
        .last()
        .unwrap()
        .authority()
        .request_id();
    assert_ne!(second_read_id, first_read_id);
    accept_last(&mut ledger, 8, 0);
    for tick in 9..=12 {
        assert!(
            kitchen_select_poll(&mut action, &snapshot, &mut ledger, tick, &state).is_pending(),
            "second journal read at tick {tick}"
        );
    }

    let result = kitchen_select_poll(&mut action, &snapshot, &mut ledger, 13, &state);
    assert!(matches!(
        &result,
        Poll::Ready(Err(crate::native::ActionError::Prerequisite(refusal)))
            if matches!(
                refusal.as_ref(),
                PrereqRefusal::StepFailed {
                    why: PrereqFailure::Action(detail),
                    ..
                } if detail.as_ref() == "the native action failed: journal modal timeout"
            )
    ));
    let outbox = &ledger.as_ref().unwrap().outbox;
    let page_reads: Vec<_> = outbox
        .iter()
        .filter(|action| {
            matches!(
                &action.effect,
                HostEffect::Interaction(InteractReq::IfButton { component_id: 42 })
            )
        })
        .map(|action| action.authority().request_id())
        .collect();
    assert!(page_reads.len() <= 2);
    assert!(page_reads
        .iter()
        .all(|request_id| *request_id == first_read_id || *request_id == second_read_id));
    assert_no_kitchen_bank_reissue(&ledger, first_authority.request_id().get());
    assert!(state.prereq_snapshot().facts().is_empty());
}

#[test]
fn bank_select_cancel_during_second_journal_read_does_not_retry_or_reissue() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut action, _, snapshot, mut ledger, state) = kitchen_select_fixture();
    let gate = kitchen_journal_gate();
    let (_, first_authority) =
        last_live_kitchen_bank_pick(&ledger).expect("Select emitted its bank-pick request");
    let allow = PrereqAllow {
        dialogue: true,
        quest_start: false,
    };
    assert!(start_kitchen_journal(
        &mut action,
        &snapshot,
        &mut ledger,
        &state,
        &first_authority,
        gate,
        allow,
    )
    .is_pending());
    let first_read_id = last_live_kitchen_journal_authority(&ledger)
        .expect("first journal owner")
        .request_id();
    accept_last(&mut ledger, 2, 0);
    for tick in 3..=7 {
        assert!(
            kitchen_select_poll(&mut action, &snapshot, &mut ledger, tick, &state).is_pending()
        );
    }

    assert!(kitchen_select_poll(&mut action, &snapshot, &mut ledger, 8, &state).is_pending());
    let second_reader = last_live_kitchen_journal_authority(&ledger)
        .expect("timeout retry starts the bounded second journal reader");
    assert_ne!(second_reader.request_id(), first_read_id);
    assert!(matches!(
        last_interaction(&ledger),
        InteractReq::IfButton { component_id: 42 }
    ));

    action.cancel();
    assert!(
        !second_reader.live(),
        "cancellation during the second read revokes its native owner"
    );
    for tick in 9..=20 {
        let result = kitchen_select_poll(&mut action, &snapshot, &mut ledger, tick, &state);
        if tick == 9 {
            assert!(matches!(
                result,
                Poll::Ready(Err(crate::native::ActionError::Cancelled))
            ));
        } else {
            assert!(matches!(
                result,
                Poll::Ready(Err(crate::native::ActionError::Stale))
            ));
        }
    }

    let outbox = &ledger.as_ref().unwrap().outbox;
    let journal_sends: Vec<_> = outbox
        .iter()
        .filter(|action| {
            matches!(
                &action.effect,
                HostEffect::Interaction(InteractReq::IfButton { component_id: 42 })
            )
        })
        .map(|action| action.authority().request_id())
        .collect();
    assert!(journal_sends.len() <= 2);
    assert!(journal_sends.iter().all(|request_id| {
        *request_id == first_read_id || *request_id == second_reader.request_id()
    }));
    assert_no_kitchen_bank_reissue(&ledger, first_authority.request_id().get());
    assert!(state.prereq_snapshot().facts().is_empty());
}

// The quest-tab text the Priest in Peril journal shows at each stage, copied
// from content/scripts/quests/quest_priestperil/scripts/priestperil_journal.rs2
// (the line of each branch is named beside it). `|` is the content's line break.
const PRIEST_HEAD: &str = "@str@I spoke to King Roald who asked me to investigate why his|@str@friend Priest Drezel has stopped communicating with him.|";
const PRIEST_LOCKED: &str = "@str@I headed to the temple where Drezel lives, but it was all|@str@locked shut. I spoke through the locked door to Drezel.|";
const PRIEST_DOG: &str = "@str@He told me that there was an annoying dog below the|@str@temple, and asked me to kill it, which I did easily.|";
const PRIEST_FURIOUS: &str = "@str@When I told Roald what I had done, he was furious. The|@str@person who told me to kill the dog wasn't Drezel at all!|";
const PRIEST_CELL: &str = "@str@I returned to the temple and found the real Drezel locked|@str@in a makeshift cell upstairs, guarded by a vampire.|";
const PRIEST_WATER: &str = "@str@I used a key from the monument to open the cell door and|@str@used Holy Water to trap the vampire in his coffin.|";
const PRIEST_WATER_AND: &str = "@str@I used a key from the monument to open the cell door and|@str@and used Holy Water to trap the vampire in his coffin.|";
const PRIEST_CONTAMINATED: &str = "@str@I followed Drezel downstairs only to find that the Salve|@str@had been contaminated and now needed purifying.|";

fn priestperil_journal(stage: i32) -> Vec<&'static str> {
    match stage {
        // priestperil_journal.rs2:6-7
        0 => vec!["@dbl@I can start this quest by speaking to @dre@King Roald@dbl@ in @dre@Varrock Palace||@dbl@I must be able to defeat a @dre@level 30 enemy"],
        // :9-11
        1 => vec![
            "@str@I spoke to King Roald who asked me to investigate why his|@str@friend Priest Drezel has stopped communicating with him.||",
            "@dre@Drezel@dbl@ lives in a @dre@temple@dbl@ to the @dre@east@dbl@ of Varrock Palace. I should head there and @dre@investigate@dbl@ what's happened to him",
        ],
        // :13-16
        2 => vec![
            PRIEST_HEAD,
            PRIEST_LOCKED,
            "@dbl@He told me that there was an annoying @dre@dog@dbl@ below the temple, and has asked me to @dre@kill it@dbl@ for him",
        ],
        // :18-22
        3 => vec![
            PRIEST_HEAD,
            PRIEST_LOCKED,
            "@str@He told me that there was an annoying dog below the|@str@temple, and asked me to kill it, which I did easily.||",
            "@dbl@I should tell @dre@King Roald@dbl@ everything's fine with @dre@Drezel@dbl@ now I have killed that @dre@dog@dbl@ for him, and claim my @dre@reward",
        ],
        // :24-29
        4 => vec![
            PRIEST_HEAD,
            PRIEST_LOCKED,
            PRIEST_DOG,
            "@str@When I told Roald what I had done, he was furious. The|@str@person who told me to kill the dog wasn't Drezel at all!||",
            "@dbl@I must return to the @dre@temple@dbl@ and find out what happened to the real @dre@Drezel@dbl@, or the King will have me executed!",
        ],
        // :31-37
        5 => vec![
            PRIEST_HEAD,
            PRIEST_LOCKED,
            PRIEST_DOG,
            PRIEST_FURIOUS,
            "@str@I returned to the temple and found the real Drezel locked|@str@in a makeshift cell upstairs, guarded by a vampire.||",
            "@dbl@I need to find the @dre@key@dbl@ to his cell and free him!|",
        ],
        // :48-55
        6 => vec![
            PRIEST_HEAD,
            PRIEST_LOCKED,
            PRIEST_DOG,
            PRIEST_FURIOUS,
            PRIEST_CELL,
            "@str@I used a key from the monument to open the cell door|",
            "@dbl@but I still have to do something about that @dre@vampire||",
        ],
        // :65-72
        7 => vec![
            PRIEST_HEAD,
            PRIEST_LOCKED,
            PRIEST_DOG,
            PRIEST_FURIOUS,
            PRIEST_CELL,
            PRIEST_WATER,
            "@dbl@I should speak to @dre@Drezel@dbl@ again",
        ],
        // :74-81
        8 => vec![
            PRIEST_HEAD,
            PRIEST_LOCKED,
            PRIEST_DOG,
            PRIEST_FURIOUS,
            PRIEST_CELL,
            PRIEST_WATER_AND,
            "@dbl@I should head downstairs to the @dre@monument@dbl@ like @dre@Drezel@dbl@ asked me to, and assess what @dre@damage@dbl@ has been done",
        ],
        // :83-91, stages 10 to 59 (^priestperil_begin_bring_essence to below
        // ^priestperil_end_bring_essence)
        10..=59 => vec![
            PRIEST_HEAD,
            PRIEST_LOCKED,
            PRIEST_DOG,
            PRIEST_FURIOUS,
            PRIEST_CELL,
            PRIEST_WATER_AND,
            PRIEST_CONTAMINATED,
            "@dbl@I need to bring Drezel @dre@fifty rune essences@dbl@ to undo the damage done by the Zamorakians and @dre@purify the Salve",
        ],
        // :93-104, the final else: stage 9 (unused) and stages 60 and 61
        _ => vec![
            PRIEST_HEAD,
            PRIEST_LOCKED,
            PRIEST_DOG,
            PRIEST_FURIOUS,
            PRIEST_CELL,
            PRIEST_WATER_AND,
            PRIEST_CONTAMINATED,
            "@str@I brought Drezel fifty rune essences and the contaminants|@str@were dissolved from the Salve, and Drezel rewarded me|",
            "@str@for all of my help with an ancient holy weapon to fight|@str@with.||",
            "@red@QUEST COMPLETE!",
        ],
    }
}

fn priestperil_window(min: i32, max: i32) -> api::selected::StageWindow {
    api::selected::StageWindow {
        quest: api::selected::FactKey::new("priestperil"),
        signal: api::selected::FactKey::new("priestperil"),
        values: InclusiveRange {
            min: Some(min),
            max: Some(max),
        },
    }
}

/// The rules of the Priest journal row whose needles all occur in the page.
fn priestperil_matching_rules(
    rules: &[api::prereq_steps::JournalRule],
    stage: i32,
) -> Vec<InclusiveRange> {
    let lines: Vec<Arc<str>> = priestperil_journal(stage)
        .into_iter()
        .map(Arc::from)
        .collect();
    let text = crate::quester::progress::normalize_journal(&lines);
    rules
        .iter()
        .filter(|rule| rule.all.iter().all(|needle| text.contains(needle.as_ref())))
        .map(|rule| rule.proves)
        .collect()
}

fn priestperil_range(min: i32, max: i32) -> InclusiveRange {
    InclusiveRange {
        min: Some(min),
        max: Some(max),
    }
}

#[test]
fn priestperil_journal_each_stage_matches_only_its_own_rule() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, journal) = CATALOG
        .for_journal(&QuestGate::Window(priestperil_window(5, 61)))
        .expect("the Priest journal row establishes the mausoleum ladders");
    assert_eq!(journal.id.as_ref(), "priestperil:mausoleum-progress");
    for stage in 0..=61 {
        let expected = match stage {
            // Stages 0 to 4 are before the mausoleum ladders: no rule claims them.
            0..=4 => vec![],
            5..=8 => vec![priestperil_range(stage, stage)],
            10..=59 => vec![priestperil_range(10, 59)],
            // Stage 9 is never written; the content shows the completed page.
            _ => vec![priestperil_range(9, 61)],
        };
        let matched = priestperil_matching_rules(&journal.rules, stage);
        assert_eq!(matched, expected, "stage {stage}");
        for proves in &matched {
            assert_eq!(
                proves.test(&priestperil_range(stage, stage)),
                Truth::True,
                "stage {stage} is inside the range its rule proves"
            );
        }
    }
}

#[test]
fn the_old_priestperil_journal_row_cannot_witness_an_in_progress_stage() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (_, journal) = CATALOG
        .for_journal(&QuestGate::Window(priestperil_window(5, 61)))
        .expect("the Priest journal row establishes the mausoleum ladders");
    // The old row: the completed-quest rule alone.
    let old = &journal.rules[..1];
    assert_eq!(
        old[0].all.iter().map(AsRef::as_ref).collect::<Vec<&str>>(),
        [
            "i brought drezel fifty rune essences and the contaminants",
            "for all of my help with an ancient holy weapon to fight",
            "quest complete!",
        ]
    );
    for stage in [5, 6, 7, 8, 10, 35, 59] {
        assert!(
            priestperil_matching_rules(old, stage).is_empty(),
            "the old row witnesses nothing at stage {stage}"
        );
    }
    assert_eq!(
        priestperil_matching_rules(old, 60),
        [priestperil_range(9, 61)]
    );
}

fn priestperil_journal_gate(min: i32) -> ActionableGate {
    use api::prereq_steps::{CrossingId, CrossingKind};
    // The mausoleum crypt gate, loc 3444 (gates.rs2:1-3 opens it from stage 5).
    let tile = |z| WorldTile {
        x: 3405,
        z,
        level: 0,
    };
    ActionableGate {
        gate: QuestGate::Window(priestperil_window(min, 61)),
        truth: Truth::Unknown,
        leg: 0,
        crossing: CrossingId {
            loc_id: 3444,
            option: 1,
            at: tile(9895),
            takeoff: tile(9894),
            to: tile(9895),
            kind: CrossingKind::Door,
        },
    }
}

#[test]
fn priestperil_in_progress_journal_is_read_end_to_end() {
    if !api::test_support::game_data_available() {
        return;
    }
    // Stage 10, the third essence batch that blocked live run 2.
    journal_observation(
        priestperil_journal_gate(5),
        "Priest in Peril",
        "Priest in Peril",
        0x00f800,
        &priestperil_journal(10),
        Some(Truth::True),
    );
    // Stage 5 satisfies a stage-5 ladder but not a stage-8 ladder.
    journal_observation(
        priestperil_journal_gate(5),
        "Priest in Peril",
        "Priest in Peril",
        0x00f800,
        &priestperil_journal(5),
        Some(Truth::True),
    );
    journal_observation(
        priestperil_journal_gate(8),
        "Priest in Peril",
        "Priest in Peril",
        0x00f800,
        &priestperil_journal(5),
        Some(Truth::False),
    );
}
