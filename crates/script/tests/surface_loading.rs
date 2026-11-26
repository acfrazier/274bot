use std::sync::Arc;

use script::slot::{
    RetirementTicket, RunState, SlotScript, StartOutcome, StartPoll, StopAuthorization,
};
use script::surface::test_support::{StubSurfaceCard, StubSurfaceFactory};
use script::surface::{no_wilderness, SurfaceReady, SurfaceStart};

fn start() -> SurfaceStart {
    SurfaceStart {
        card: Arc::new(StubSurfaceCard),
        settings_bag: None,
        game_data: None,
        named_banks: Arc::default(),
        wilderness_rules: no_wilderness(),
        loadouts: Arc::default(),
    }
}

fn bound_slot(incarnation: u64) -> (SlotScript, Arc<StubSurfaceFactory>) {
    let mut slot = SlotScript::new();
    slot.bind_incarnation(incarnation);
    let factory = Arc::new(StubSurfaceFactory::new());
    slot.bind_surface_factory(factory.clone());
    (slot, factory)
}

#[test]
fn loading_is_published_before_the_factory_runs_and_the_token_is_slot_owned() {
    let (mut slot, factory) = bound_slot(71);
    let identity = slot
        .next_surface_start_identity()
        .expect("dry next identity is available");
    assert_eq!(identity.slot_incarnation, 71);
    assert_eq!(identity.run_key, slot.runtime_generation() + 1);
    assert_eq!(identity.start_serial, 1);

    let token = slot
        .admit_surface_loading(start(), identity)
        .expect("slot admits Loading before VM work");
    let loading = slot.surface_execution_observation();
    assert_eq!(loading.state, RunState::Starting);
    assert_eq!(loading.start_serial, identity.start_serial);
    assert!(loading.execution.is_some());
    assert!(
        factory.last_run().is_none(),
        "admission does not compile or Load"
    );
    assert_eq!(slot.poll_start(), StartPoll::Pending);

    slot.load_admitted_surface(token)
        .expect("owner loads after admission guards are released");
    assert!(factory.last_run().is_some());
    assert_eq!(slot.poll_start(), StartPoll::Pending);
}

#[test]
fn a_stale_dry_identity_cannot_admit_a_different_execution() {
    let (mut slot, factory) = bound_slot(72);
    let mut stale = slot
        .next_surface_start_identity()
        .expect("dry next identity is available");
    stale.run_key += 1;

    assert!(slot.admit_surface_loading(start(), stale).is_err());
    assert_eq!(slot.runtime_generation(), 0);
    assert_eq!(slot.surface_execution_observation().state, RunState::Idle);
    assert!(factory.last_run().is_none());
}

#[test]
fn expected_stop_ticket_matches_only_its_retained_idle_frontier() {
    let (mut slot, factory) = bound_slot(73);
    slot.start_surface(start()).expect("ordinary surface Start");
    let run = factory.last_run().expect("surface run");
    run.set_readiness(SurfaceReady::Ready);
    assert_eq!(slot.poll_start(), StartPoll::Settled(StartOutcome::Ready));

    let before = slot
        .surface_execution_observation()
        .execution
        .expect("running execution");
    let authorization = StopAuthorization {
        plan_id: 19,
        step_ordinal: 4,
        stop_ordinal: 2,
    };
    let ticket: RetirementTicket = slot
        .stop_expected_execution(&before, authorization)
        .expect("slot issues one authorized ticket");
    assert!(
        slot.stop_expected_execution(&before, authorization)
            .is_err(),
        "the same execution cannot receive a second ticket"
    );

    slot.observe_lifecycle();
    let idle = slot.surface_execution_observation();
    assert_eq!(idle.state, RunState::Idle);
    assert!(ticket.matches_idle(&idle));
    assert_eq!(idle.retirement_ticket, Some(ticket));
    assert!(
        idle.extension_end.is_none(),
        "the stub has no scenario dock"
    );

    slot.start_surface(start())
        .expect("a later Start is allowed");
    assert!(!ticket.matches_idle(&slot.surface_execution_observation()));
}

#[test]
fn dry_identity_reads_do_not_advance_generation_or_start_serial() {
    let (slot, _) = bound_slot(74);
    let first = slot.next_surface_start_identity().expect("first identity");
    let second = slot
        .next_surface_start_identity()
        .expect("same dry identity");
    assert_eq!(first, second);
    assert_eq!(slot.runtime_generation(), 0);
}
