use super::*;
use crate::quester::families::tests::with_tick;
use api::gather_methods::{known_rows, TargetClass};
use api::selected::{EntityId, FamilyPreparation};
use api::snapshot::GameSnapshot;

fn fixture() -> (Gatherer, GameSnapshot) {
    let (config, mut snapshot) = FamilyPreparation::run(|families| {
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let mut cx = crate::native::PrepareContext {
            pin: selected.selected_pin().unwrap(),
            selected,
            banks: Arc::default(),
            families,
        };
        super::super::test_full_pack_fixture(&mut cx).unwrap()
    })
    .unwrap()
    .join()
    .unwrap();
    snapshot.seed_inventory(vec![], 28);
    let gatherer = Gatherer::new(
        RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        Arc::clone(&config),
        Arc::clone(config.get::<Arc<Prepared>>().unwrap()),
        GatherRetained::default(),
    );
    (gatherer, snapshot)
}

fn assert_target_handoff(depleted: bool) {
    let (mut gatherer, mut snapshot) = fixture();
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 1, |tick| {
        gatherer.tick(tick).unwrap()
    });
    assert!(matches!(gatherer.active, Active::Gather(_)));
    let next = gatherer.target.clone().unwrap();
    let mut previous = next.clone();
    previous.tile.x += 3;
    let mut previous_loc = snapshot.locs()[0].clone();
    previous_loc.tile = previous.tile;
    let next_loc = snapshot.locs()[0].clone();
    snapshot.seed_locs(vec![previous_loc.clone(), next_loc.clone()]);
    gatherer.cancel_active();
    with_tick(&snapshot, &mut ledger, 2, |tick| {
        gatherer.active = Active::Gather(
            tick.actions
                .begin::<GatherRun>(
                    GatherRunArgs {
                        target: previous.clone(),
                        catalog: Arc::clone(&gatherer.prepared.catalog),
                        stall_ticks: DEFAULT_STALL_TICKS,
                    },
                    &mut tick.cx,
                )
                .unwrap(),
        );
        gatherer.target = Some(previous);
    });
    let method = &gatherer.prepared.catalog.methods()[usize::from(next.method_index)];
    previous_loc.id = known_rows(&method.targets)
        .iter()
        .find_map(|target| match (target.class, target.entity) {
            (TargetClass::Depleted, EntityId::Loc(id)) => Some(id),
            _ => None,
        })
        .expect("normal tree has a content-derived depleted form");
    snapshot.seed_locs(if depleted {
        vec![previous_loc, next_loc]
    } else {
        vec![next_loc]
    });
    ledger.as_mut().unwrap().outbox.clear();

    with_tick(&snapshot, &mut ledger, 3, |tick| {
        gatherer.tick(tick).unwrap()
    });

    assert!(
        matches!(gatherer.active, Active::Gather(_)),
        "depletion must hand off without an idle tick"
    );
    assert_eq!(gatherer.target.as_ref().unwrap().tile, next.tile);
    assert_eq!(
        ledger.as_ref().unwrap().outbox.len(),
        1,
        "one gather click for the newly selected target"
    );
    assert!(
        !gatherer.needs_validate,
        "the handoff completes normal validation in the same tick"
    );
    ledger.as_mut().unwrap().outbox.clear();
    with_tick(&snapshot, &mut ledger, 3, |tick| {
        gatherer.tick(tick).unwrap()
    });
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "same-tick reentry cannot click the target again"
    );
    with_tick(&snapshot, &mut ledger, 4, |tick| {
        gatherer.tick(tick).unwrap()
    });
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "an active target is not clicked again"
    );
}

#[test]
fn depleted_tree_selects_and_clicks_next_live_target_in_the_observation_tick() {
    assert_target_handoff(true);
}

#[test]
fn missing_tree_selects_and_clicks_next_live_target_in_the_observation_tick() {
    assert_target_handoff(false);
}

#[test]
fn resource_approach_arrival_selects_and_clicks_without_a_settle_tick() {
    let (mut gatherer, snapshot) = fixture();
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 1, |tick| {
        gatherer.tick(tick).unwrap()
    });
    let target = gatherer.target.clone().unwrap();
    gatherer.cancel_active();
    with_tick(&snapshot, &mut ledger, 2, |tick| {
        gatherer.active = Active::Walk(
            tick.actions
                .begin::<Walk>(
                    WalkRequest {
                        target: target.tile,
                        loc_id: match target.entity {
                            EntityId::Loc(id) => Some(id),
                            _ => None,
                        },
                        radius: 1,
                        arrival: nav::arrival::ArrivalKind::Reach,
                        options: FindOptions {
                            allow_teleports: false,
                            allow_wilderness: false,
                            allow_bank_fetch: false,
                        },
                        required_after: tick.cx.evidence(),
                        evidence: None,
                        cross: Vec::new().into_boxed_slice(),
                        protect: false,
                        allow: Default::default(),
                    },
                    &mut tick.cx,
                )
                .unwrap(),
        );
        gatherer.target = Some(target.clone());
    });
    ledger.as_mut().unwrap().outbox.clear();
    with_tick(&snapshot, &mut ledger, 3, |tick| {
        gatherer.tick(tick).unwrap()
    });
    assert!(matches!(gatherer.active, Active::Gather(_)));
    assert_eq!(gatherer.target.as_ref().unwrap().tile, target.tile);
    assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
}
