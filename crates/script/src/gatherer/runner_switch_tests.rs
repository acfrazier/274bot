use super::super::select::PlacementClass;
use super::*;
use crate::quester::families::tests::{def, with_tick};
use api::gather_methods::{known_rows, TargetClass};
use api::named_banks::{NamedBank, NamedBankFacts};
use api::selected::{EntityId, FamilyPreparation};
use api::snapshot::{GameSnapshot, ItemActionFamily, ItemContainer, ItemView, WorldTile};

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
    let (config, snapshot) = FamilyPreparation::run(move |families| {
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
        let config = super::super::card::prepare(&mut cx, 1, Arc::new(settings)).unwrap();
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(crate::quester::families::tests::local_player(WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        }));
        (config, snapshot)
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
            ledger
                .outbox
                .iter()
                .find_map(|action| match &action.effect {
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
                        quest_owned: false,
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
                        options: crate::native::WalkOptions::default(),
                        required_after: tick.cx.evidence(),
                        evidence: None,
                        cross: Vec::new().into_boxed_slice(),
                        protect: false,
                        food_guard: false,
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

fn alternative_tree(gatherer: &Gatherer, approached: &TargetPlan) -> WorldTile {
    let method = &gatherer.prepared.catalog.methods()[usize::from(approached.method_index)];
    gatherer
        .prepared
        .catalog
        .spots(method, &gatherer.area.unwrap().region())
        .unwrap()
        .filter(|spot| spot.entity == approached.entity && spot.origin != approached.tile)
        .filter(|spot| {
            spot.origin
                .x
                .abs_diff(approached.tile.x)
                .max(spot.origin.z.abs_diff(approached.tile.z))
                > 2
        })
        .min_by_key(|spot| {
            spot.origin.x.abs_diff(approached.tile.x) + spot.origin.z.abs_diff(approached.tile.z)
        })
        .expect("the content-derived work area includes another tree")
        .origin
}

fn assert_approached_tree_identity(depleted: bool) {
    let (mut gatherer, mut snapshot) = fixture();
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 1, |tick| {
        gatherer.tick(tick).unwrap()
    });
    let approached = gatherer.target.clone().unwrap();
    let method = &gatherer.prepared.catalog.methods()[usize::from(approached.method_index)];
    let other = alternative_tree(&gatherer, &approached);
    let mut approached_loc = snapshot.locs()[0].clone();
    let mut other_loc = approached_loc.clone();
    other_loc.tile = other;
    if depleted {
        approached_loc.id = known_rows(&method.targets)
            .iter()
            .find_map(|target| match (target.class, target.entity) {
                (TargetClass::Depleted, EntityId::Loc(id)) => Some(id),
                _ => None,
            })
            .expect("the method supplies a depleted form");
    }
    gatherer.cancel_active();
    with_tick(&snapshot, &mut ledger, 2, |tick| {
        gatherer.start_target(
            SelectedTarget {
                plan: approached.clone(),
                class: PlacementClass::Unloaded,
            },
            tick,
        );
    });
    assert!(matches!(gatherer.active, Active::Walk(_)));

    // The live server tile has reached the selected footprint while the
    // rendered actor still makes the other tree nearest to fresh selection.
    let server = WorldTile {
        x: approached.tile.x + 1,
        ..approached.tile
    };
    let mut player = snapshot.local_player().unwrap().clone();
    player.player.network = server;
    player.player.actor.tile = other;
    snapshot.seed_local_player(player);
    snapshot.seed_locs(vec![approached_loc, other_loc]);
    ledger.as_mut().unwrap().outbox.clear();
    with_tick(&snapshot, &mut ledger, 3, |tick| {
        if !depleted {
            let EntityId::Loc(id) = approached.entity else {
                panic!("tree loc")
            };
            assert!(tick
                .cx
                .snapshot()
                .walk_loc_arrived(server, approached.tile, 1, id));
        }
        gatherer.tick(tick).unwrap()
    });
    if depleted {
        assert_eq!(gatherer.target.as_ref().unwrap().tile, other);
        assert!(matches!(gatherer.active, Active::Walk(_)));
    } else {
        assert_eq!(
            gatherer.target.as_ref().unwrap().tile,
            approached.tile,
            "arrival must not discard the live approached tree for the newly nearest tree"
        );
        assert!(matches!(gatherer.active, Active::Gather(_)));
        let outbox = &ledger.as_ref().unwrap().outbox;
        assert_eq!(outbox.len(), 1, "exactly one gather interaction");
        assert!(matches!(
            outbox[0].effect,
            crate::native::HostEffect::Interaction(_)
        ));
    }
}

#[test]
fn arrived_live_tree_keeps_identity_when_nearest_tree_changes() {
    assert_approached_tree_identity(false);
}

#[test]
fn arrived_depleted_tree_reselects_the_live_alternative() {
    assert_approached_tree_identity(true);
}

#[test]
fn arrived_live_tree_is_not_clicked_after_leaving_its_footprint() {
    let (mut gatherer, mut snapshot) = fixture();
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 1, |tick| {
        gatherer.tick(tick).unwrap()
    });
    let approached = gatherer.target.clone().unwrap();
    gatherer.cancel_active();
    // Model the idle handoff after an arrival receipt, with newer server
    // position evidence that no longer permits the selected Loc operation.
    gatherer.target = Some(approached.clone());
    let server = WorldTile {
        x: approached.tile.x - 3,
        ..approached.tile
    };
    let mut player = snapshot.local_player().unwrap().clone();
    player.player.network = server;
    snapshot.seed_local_player(player);
    ledger.as_mut().unwrap().outbox.clear();
    with_tick(&snapshot, &mut ledger, 2, |tick| {
        let EntityId::Loc(id) = approached.entity else {
            panic!("tree loc")
        };
        assert!(!tick
            .cx
            .snapshot()
            .walk_loc_arrived(server, approached.tile, 1, id));
        gatherer.begin_idle(tick, true);
    });
    assert!(matches!(gatherer.active, Active::Walk(_)));
    assert!(ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .all(|action| { !matches!(action.effect, crate::native::HostEffect::Interaction(_)) }));
}

fn assert_route_ended_tree_reselection(observation: TargetObservation, loaded: bool) {
    let (mut gatherer, mut snapshot) = fixture();
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 1, |tick| {
        gatherer.tick(tick).unwrap()
    });
    let approached = gatherer.target.clone().unwrap();
    let other = alternative_tree(&gatherer, &approached);
    let mut approached_loc = snapshot.locs()[0].clone();
    let mut other_loc = approached_loc.clone();
    other_loc.tile = other;
    if observation == TargetObservation::Depleted {
        let method = &gatherer.prepared.catalog.methods()[usize::from(approached.method_index)];
        approached_loc.id = known_rows(&method.targets)
            .iter()
            .find_map(|target| match (target.class, target.entity) {
                (TargetClass::Depleted, EntityId::Loc(id)) => Some(id),
                _ => None,
            })
            .expect("the method supplies a depleted form");
    }
    snapshot.seed_locs(if observation == TargetObservation::Gone {
        vec![other_loc]
    } else {
        vec![approached_loc, other_loc]
    });
    if !loaded {
        snapshot.seed_world(api::snapshot::WorldStateView {
            map_base_x: approached.tile.x + 104,
            map_base_z: approached.tile.z,
            ..Default::default()
        });
    }
    let mut player = snapshot.local_player().unwrap().clone();
    player.player.network.x = approached.tile.x - 3;
    player.player.actor.tile = other;
    snapshot.seed_local_player(player);
    gatherer.cancel_active();
    gatherer.target = Some(approached);
    ledger.as_mut().unwrap().outbox.clear();
    with_tick(&snapshot, &mut ledger, 2, |tick| {
        gatherer.handle_walk(
            WalkReceipt {
                request_id: 7,
                evidence: tick.cx.evidence(),
                end: WalkEnd::RouteEnded,
                blocked: None,
                detail: None,
                refusal: None,
                assessment: None,
                escape: None,
            },
            tick,
        );
        if observation == TargetObservation::Active || !loaded {
            assert_eq!(
                gatherer.failure.as_ref().unwrap().code.as_ref(),
                "walk-failed",
                "a still-present or unobserved tree must retain the routing failure"
            );
        } else {
            assert!(
                gatherer.failure.is_none(),
                "normal target disappearance must not turn into a routing failure"
            );
            gatherer.begin_idle(tick, true);
        }
    });
    if observation != TargetObservation::Active && loaded {
        assert_eq!(gatherer.target.as_ref().unwrap().tile, other);
        assert!(matches!(gatherer.active, Active::Walk(_)));
    }
    assert!(ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .all(|action| { !matches!(action.effect, crate::native::HostEffect::Interaction(_)) }));
}

#[test]
fn route_ended_gone_tree_reselects_without_blocking() {
    assert_route_ended_tree_reselection(TargetObservation::Gone, true);
}

#[test]
fn route_ended_depleted_tree_reselects_without_blocking() {
    assert_route_ended_tree_reselection(TargetObservation::Depleted, true);
}

#[test]
fn route_ended_still_present_tree_keeps_the_routing_failure() {
    assert_route_ended_tree_reselection(TargetObservation::Active, true);
}

#[test]
fn route_ended_unloaded_tree_keeps_the_routing_failure() {
    assert_route_ended_tree_reselection(TargetObservation::Gone, false);
}

#[test]
fn unloaded_loc_approach_preserves_reach_and_loc_identity() {
    let (mut gatherer, snapshot) = fixture();
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 1, |tick| {
        gatherer.tick(tick).unwrap()
    });
    let plan = gatherer.target.clone().unwrap();
    let EntityId::Loc(id) = plan.entity else {
        panic!("the fixture selects a content-derived tree");
    };
    gatherer.cancel_active();
    ledger.as_mut().unwrap().outbox.clear();
    with_tick(&snapshot, &mut ledger, 2, |tick| {
        gatherer.start_target(
            SelectedTarget {
                plan: plan.clone(),
                class: PlacementClass::Unloaded,
            },
            tick,
        );
    });
    let crate::native::HostEffect::Walk(request) = &ledger.as_ref().unwrap().outbox[0].effect
    else {
        panic!("an unloaded loc needs a navigation approach");
    };
    assert_eq!(request.arrival, ArrivalKind::Reach);
    assert_eq!(request.loc_id, Some(id));
    assert_eq!(request.target, plan.tile);
}

#[test]
fn unloaded_npc_observation_stand_uses_area_without_loc_identity() {
    let (mut gatherer, snapshot) = fixture();
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 1, |tick| {
        gatherer.tick(tick).unwrap()
    });
    let mut plan = gatherer.target.clone().unwrap();
    plan.entity = EntityId::Npc(309);
    plan.npc_index = -1;
    gatherer.cancel_active();
    ledger.as_mut().unwrap().outbox.clear();
    with_tick(&snapshot, &mut ledger, 2, |tick| {
        gatherer.start_target(
            SelectedTarget {
                plan,
                class: PlacementClass::Unloaded,
            },
            tick,
        );
    });
    let crate::native::HostEffect::Walk(request) = &ledger.as_ref().unwrap().outbox[0].effect
    else {
        panic!("an unloaded NPC needs an observation walk");
    };
    assert_eq!(request.arrival, ArrivalKind::Area);
    assert_eq!(request.loc_id, None);
}

fn worn(id: i32) -> ItemView {
    ItemView {
        def: def(id, "Worn"),
        container: ItemContainer::Equipment,
        action_family: ItemActionFamily::Held,
        slot: 0,
        count: 1,
        actions: Vec::new(),
        component_id: -1,
    }
}

/// Starts the fixture's first target re-pointed at a method content refuses in
/// monkey form. `worn` is the observed worn set, or `None` when unobserved.
fn start_monkey_form_target(worn: Option<Vec<ItemView>>) -> Gatherer {
    let (mut gatherer, snapshot) = fixture();
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 1, |tick| {
        gatherer.tick(tick).unwrap()
    });
    let mut plan = gatherer
        .target
        .clone()
        .expect("the fixture selects a target");
    let catalog = Arc::clone(&gatherer.prepared.catalog);
    plan.method_index = catalog
        .methods()
        .iter()
        .position(|method| {
            catalog
                .forbidden_states(method)
                .is_ok_and(|states| states.contains(&GatherForbiddenState::MonkeyForm))
        })
        .expect("289 content has a monkey-form method") as u16;
    // The fixture frame already carries worn items; an unobserved set needs a
    // frame that never posted its worn table.
    let snapshot = match worn {
        Some(rows) => {
            let mut observed = snapshot;
            observed.seed_equipment(rows);
            observed
        }
        None => {
            let mut bare = GameSnapshot::new();
            bare.seed_ingame(2);
            bare
        }
    };
    gatherer.cancel_active();
    gatherer.target = None;
    gatherer.needs_validate = false;
    ledger.as_mut().unwrap().outbox.clear();
    with_tick(&snapshot, &mut ledger, 2, |tick| {
        gatherer.start_target(
            SelectedTarget {
                plan,
                class: PlacementClass::Unloaded,
            },
            tick,
        );
    });
    gatherer
}

#[test]
fn monkey_form_begin_refuses_a_worn_greegree() {
    let gatherer = start_monkey_form_target(Some(vec![worn(4024)]));
    assert_eq!(
        gatherer
            .failure
            .as_ref()
            .map(|failure| failure.code.as_ref()),
        Some("forbidden-state:monkey-form")
    );
    assert!(gatherer.target.is_none());
}

#[test]
fn monkey_form_begin_allows_a_worn_amulet_of_glory() {
    let gatherer = start_monkey_form_target(Some(vec![worn(1704)]));
    assert!(gatherer.failure.is_none());
    assert!(gatherer.target.is_some());
}

#[test]
fn monkey_form_begin_allows_an_empty_worn_set() {
    let gatherer = start_monkey_form_target(Some(Vec::new()));
    assert!(gatherer.failure.is_none());
    assert!(gatherer.target.is_some());
}

#[test]
fn monkey_form_begin_waits_for_an_observed_worn_set() {
    let gatherer = start_monkey_form_target(None);
    assert!(gatherer.failure.is_none());
    assert!(gatherer.needs_validate);
    assert!(gatherer.target.is_none());
}
