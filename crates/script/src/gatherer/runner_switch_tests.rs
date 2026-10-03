use super::*;
use crate::quester::families::tests::with_tick;
use api::gather_methods::{known_rows, TargetClass};
use api::selected::{EntityId, FamilyPreparation};
use api::snapshot::GameSnapshot;
use api::named_banks::{NamedBank, NamedBankFacts};
use api::snapshot::WorldTile;

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

fn bank_fixture(bank_choice: Option<&'static str>) -> (Gatherer, GameSnapshot) {
    let bank_tile = WorldTile {
        x: 3245,
        z: 3423,
        level: 0,
    };
    let (config, mut snapshot) = FamilyPreparation::run(|families| {
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let banks = Arc::new(NamedBankFacts::from_banks(vec![NamedBank::new(
            "Varrock East",
            bank_tile,
        )]));
        let mut settings = crate::native::SettingsBag::new();
        settings.insert("disposition".into(), serde_json::json!("Bank"));
        if let Some(bank) = bank_choice {
            settings.insert("bank".into(), serde_json::json!(bank));
        }
        let mut cx = crate::native::PrepareContext {
            pin: selected.selected_pin().unwrap(),
            selected,
            banks,
            families,
        };
        let config = super::super::card::prepare(&mut cx, 1, Arc::new(settings))?;
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(crate::quester::families::tests::local_player(
            WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
        ));
        Ok((config, snapshot))
    })
    .unwrap()
    .join()
    .unwrap();
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

fn gatherer_bank_pick(bank_choice: Option<&'static str>) -> crate::native_bank::BankPickRequest {
    let (mut gatherer, snapshot) = bank_fixture(bank_choice);
    let mut ledger = None;
    gatherer.trip = TripStep::Select;
    with_tick(&snapshot, &mut ledger, 1, |tick| gatherer.begin_trip(tick));
    with_tick(&snapshot, &mut ledger, 2, |tick| gatherer.poll_active(tick));
    ledger
        .as_ref()
        .and_then(|ledger| {
            ledger.outbox.iter().find_map(|action| match &action.effect {
                crate::native::HostEffect::BankPick(request) => Some(request.clone()),
                _ => None,
            })
        })
        .expect("the bank trip must submit a selection request")
}

#[test]
fn gatherer_nearest_bank_setting_emits_no_explicit_bank() {
    assert!(gatherer_bank_pick(None).explicit_bank.is_none());
}

#[test]
fn gatherer_named_bank_setting_remains_an_explicit_pick() {
    assert_eq!(
        gatherer_bank_pick(Some("Varrock East")).explicit_bank,
        Some(0)
    );
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
