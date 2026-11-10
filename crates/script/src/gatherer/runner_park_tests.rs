use super::tests::BaitFixture;
use super::*;
use crate::acquire::{BankSeen, BlockReason, Looked, NeedKind, Presence};
use crate::quester::families::tests::{def, with_tick_bank};
use api::bank_memory::{BankMemory, Origin};
use api::snapshot::{ItemActionFamily, ItemContainer};

const BAIT: i32 = 313;

fn row(id: i32, count: i32, container: ItemContainer) -> ItemView {
    ItemView {
        def: def(
            id,
            if id == BAIT {
                "Fishing bait"
            } else {
                "Bronze axe"
            },
        ),
        count,
        container,
        action_family: ItemActionFamily::Held,
        slot: 1,
        actions: vec![Some("Wield".into())],
        component_id: 3214,
    }
}

fn need(item: i32, minimum: i32, presence: Presence) -> Need {
    Need {
        item,
        name: Arc::from(if item == BAIT {
            "Fishing bait"
        } else {
            "Bronze axe"
        }),
        target: minimum,
        minimum,
        presence,
        kind: NeedKind::Required,
        sources: SourceSet::PACK | SourceSet::WORN | SourceSet::BANK,
        alternatives: Arc::from([]),
    }
}

fn parked(fixture: &BaitFixture, need: Need, reason: BlockReason) -> Gatherer {
    let mut gatherer = fixture.gatherer(GatherRetained::default());
    let mut ledger = None;
    with_tick_bank(
        &fixture.snapshot,
        Some(&fixture.session),
        &mut ledger,
        1,
        |tick| {
            let request =
                gatherer.acquire_request(Arc::from([need]), None, CloseBank::Close, &tick.cx);
            let block = AcquireBlock {
                item: Arc::from("missing supply"),
                need: 0,
                reason,
                looked: Looked::default(),
                message: Arc::from("missing supply; waiting: put the supply in the pack"),
            };
            gatherer.park_supply(request, block, 1);
        },
    );
    gatherer.needs_validate = false;
    gatherer
}

fn reasons() -> [BlockReason; 7] {
    [
        BlockReason::SupplyMissing,
        BlockReason::TooFewCoins {
            needed: 10,
            held: 0,
            banked: 0,
        },
        BlockReason::VendorUnreachable,
        BlockReason::VendorPickUndecided,
        BlockReason::NoVendor,
        BlockReason::NoStock {
            restock_ticks: 400,
            refused_at_tick: 4600,
            aligned_to: None,
            revisit_due: 5000,
        },
        BlockReason::NoSpace { slots_needed: 1 },
    ]
}

#[test]
fn every_supply_park_holds_for_1000_ticks_without_a_plan_or_allocation() {
    if !api::test_support::game_data_available() {
        return;
    }
    for reason in reasons() {
        let mut fixture = BaitFixture::new();
        fixture
            .snapshot
            .seed_inventory(fixture.snapshot.inventory().to_vec(), 1);
        let mut gatherer = parked(&fixture, need(BAIT, 5, Presence::Held), reason);
        let mut ledger = None;
        with_tick_bank(
            &fixture.snapshot,
            Some(&fixture.session),
            &mut ledger,
            1,
            |tick| {
                assert!(matches!(gatherer.tick(tick).unwrap(), ScriptFlow::Continue));
            },
        );
        let block = gatherer.scratch.parked.as_ref().unwrap().block.clone();
        let code = gatherer.scratch.parked.as_ref().unwrap().code;
        let calls = crate::acquire::plan::calls_for_test();
        for edge in 2..=1001 {
            with_tick_bank(
                &fixture.snapshot,
                Some(&fixture.session),
                &mut ledger,
                edge,
                |tick| {
                    let allocations = allocation_counter::measure(|| {
                        assert!(matches!(gatherer.tick(tick).unwrap(), ScriptFlow::Continue));
                    });
                    assert_eq!(allocations.count_total, 0, "{code} at {edge}");
                },
            );
        }
        let park = gatherer.scratch.parked.as_ref().unwrap();
        assert_eq!(park.block, block);
        assert_eq!(park.code, code);
        assert_eq!(park.since_tick, 1);
        assert_eq!(crate::acquire::plan::calls_for_test(), calls);
        assert!(gatherer.failure.is_none());
        assert_eq!(gatherer.phase(), ("blocked", NativePhase::Blocked));
        assert!(gatherer.waits_for_evidence());
        assert!(ledger.is_none());
    }
}

#[test]
fn every_park_survives_relog_and_hint_generation_without_a_plan() {
    if !api::test_support::game_data_available() {
        return;
    }
    for reason in reasons() {
        let mut fixture = BaitFixture::new();
        fixture
            .snapshot
            .seed_inventory(fixture.snapshot.inventory().to_vec(), 1);
        let mut gatherer = parked(&fixture, need(BAIT, 5, Presence::Held), reason);
        let block = gatherer.scratch.parked.as_ref().unwrap().block.clone();
        fixture.session.relogged();
        assert_eq!(fixture.session.origin(), Origin::Hint);
        gatherer.run.session = 0;
        let calls = crate::acquire::plan::calls_for_test();
        let mut ledger = None;
        for edge in 2..=102 {
            with_tick_bank(
                &fixture.snapshot,
                Some(&fixture.session),
                &mut ledger,
                edge,
                |tick| {
                    assert!(matches!(gatherer.tick(tick).unwrap(), ScriptFlow::Continue));
                },
            );
        }
        let park = gatherer.scratch.parked.as_ref().unwrap();
        assert_eq!(park.block, block);
        assert_eq!(park.since_tick, 1);
        assert_eq!(crate::acquire::plan::calls_for_test(), calls);
        assert!(gatherer.active_matches_none());
        assert!(ledger.is_none());
    }
}

#[test]
fn relog_rephrases_the_parked_bank_clause_as_a_saved_hint() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut fixture = BaitFixture::new();
    fixture
        .snapshot
        .seed_inventory(fixture.snapshot.inventory().to_vec(), 1);
    let now = std::time::SystemTime::now();
    let observed_at_unix = now
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| time.as_secs());
    let looked = Looked {
        bank: Some(BankSeen {
            origin: Origin::Session,
            observed_at_unix,
            rows: 0,
            count: 0,
        }),
        ..Looked::default()
    };
    let block = AcquireBlock {
        item: Arc::from("Fishing bait"),
        need: 0,
        reason: BlockReason::SupplyMissing,
        message: crate::acquire::plan::block_message(
            "Fishing bait",
            &BlockReason::SupplyMissing,
            &looked,
            "nobody sells it",
            now,
            1,
        ),
        looked,
    };
    let mut gatherer = fixture.gatherer(GatherRetained::default());
    let mut ledger = None;
    with_tick_bank(
        &fixture.snapshot,
        Some(&fixture.session),
        &mut ledger,
        1,
        |tick| {
            let request = gatherer.acquire_request(
                Arc::from([need(BAIT, 5, Presence::Held)]),
                None,
                CloseBank::Close,
                &tick.cx,
            );
            gatherer.park_supply(request, block, 1);
        },
    );
    gatherer.needs_validate = false;
    assert!(gatherer
        .scratch
        .parked
        .as_ref()
        .unwrap()
        .block
        .message
        .contains("less than a minute ago this session"));
    assert_eq!(fixture.session.origin(), Origin::Session);

    fixture.session.relogged();
    assert_eq!(fixture.session.origin(), Origin::Hint);
    gatherer.run.session = 0;
    let calls = crate::acquire::plan::calls_for_test();
    let mut ledger = None;
    with_tick_bank(
        &fixture.snapshot,
        Some(&fixture.session),
        &mut ledger,
        2,
        |tick| {
            assert!(matches!(gatherer.tick(tick).unwrap(), ScriptFlow::Continue));
        },
    );

    let park = gatherer.scratch.parked.as_ref().unwrap();
    assert_eq!(park.block.reason, BlockReason::SupplyMissing);
    assert_eq!(park.block.looked.bank.unwrap().origin, Origin::Hint);
    assert!(
        park.block.message.contains("from a saved hint"),
        "{}",
        park.block.message
    );
    assert!(
        !park.block.message.contains("this session"),
        "{}",
        park.block.message
    );
    assert_eq!(
        crate::acquire::plan::calls_for_test(),
        calls,
        "a relog rephrases the retained reason without planning"
    );
}

#[test]
fn combined_session_bank_coverage_wakes_once_but_hint_coverage_does_not() {
    if !api::test_support::game_data_available() {
        return;
    }
    for origin in [Origin::Session, Origin::Hint] {
        let mut fixture = BaitFixture::new();
        let mut pack = fixture.snapshot.inventory().to_vec();
        pack.push(row(BAIT, 3, ItemContainer::Inventory));
        fixture.snapshot.seed_inventory(pack, 28);
        let mut gatherer = parked(
            &fixture,
            need(BAIT, 5, Presence::Held),
            BlockReason::SupplyMissing,
        );
        let memory = BankMemory::seeded(&[(BAIT, 2)], origin);
        let calls = crate::acquire::plan::calls_for_test();
        let mut ledger = None;
        let holding = with_tick_bank(&fixture.snapshot, Some(&memory), &mut ledger, 2, |tick| {
            gatherer.poll_park(tick)
        });
        assert_eq!(holding, origin != Origin::Session);
        assert_eq!(
            crate::acquire::plan::calls_for_test() - calls,
            usize::from(origin == Origin::Session)
        );
        assert_eq!(gatherer.scratch.parked.is_some(), origin != Origin::Session);
        assert!(ledger.is_none(), "the wake itself does not dispatch a walk");
    }
}

#[test]
fn held_exact_or_lower_wieldable_tool_wakes_to_wear_without_a_bank_or_purchase() {
    if !api::test_support::game_data_available() {
        return;
    }
    for reason in [
        BlockReason::SupplyMissing,
        BlockReason::TooFewCoins {
            needed: 5,
            held: 0,
            banked: 0,
        },
    ] {
        for (id, worn) in [(1353, false), (1351, false), (1351, true)] {
            let mut fixture = BaitFixture::new();
            let mut wanted = need(1353, 1, Presence::Wielded);
            wanted.alternatives = Arc::from([1353, 1351]);
            let mut gatherer = parked(&fixture, wanted, reason.clone());
            if worn {
                fixture
                    .snapshot
                    .seed_equipment(vec![row(id, 1, ItemContainer::Equipment)]);
            } else {
                fixture
                    .snapshot
                    .seed_inventory(vec![row(id, 1, ItemContainer::Inventory)], 28);
            }
            let mut ledger = None;
            let calls = crate::acquire::plan::calls_for_test();
            with_tick_bank(
                &fixture.snapshot,
                Some(&fixture.session),
                &mut ledger,
                2,
                |tick| {
                    assert!(!gatherer.poll_park(tick));
                    assert!(gatherer.scratch.parked.is_none());
                    if !worn {
                        gatherer.poll_active(tick);
                    }
                },
            );
            assert_eq!(crate::acquire::plan::calls_for_test() - calls, 1);
            if worn {
                assert!(gatherer.active_matches_none());
                assert!(ledger.is_none());
            } else {
                assert!(matches!(gatherer.active, Active::Acquire(_)));
                let actions = &ledger.as_ref().expect("core wear action").outbox;
                assert!(matches!(actions.as_slice(), [action] if matches!(
                    &action.effect,
                    crate::native::HostEffect::Interaction(crate::interact::InteractReq::Held { target_item_id: Some(sent), action, .. })
                        if *sent == id && action == "Wield"
                )));
            }
        }
    }
}

#[test]
fn session_banked_lower_tier_wakes_the_original_tool_need() {
    if !api::test_support::game_data_available() {
        return;
    }
    let fixture = BaitFixture::new();
    let mut wanted = need(1353, 1, Presence::Wielded);
    wanted.alternatives = Arc::from([1353, 1351]);
    let mut gatherer = parked(&fixture, wanted, BlockReason::SupplyMissing);
    let memory = BankMemory::seeded(&[(1351, 1)], Origin::Session);
    let mut ledger = None;
    with_tick_bank(&fixture.snapshot, Some(&memory), &mut ledger, 2, |tick| {
        assert!(!gatherer.poll_park(tick));
    });
    assert!(gatherer.scratch.parked.is_none());
    assert!(matches!(gatherer.active, Active::Acquire(_)));
}

#[test]
fn resume_and_settings_each_plan_once_but_hold_release_does_not() {
    if !api::test_support::game_data_available() {
        return;
    }
    let fixture = BaitFixture::new();
    let mut gatherer = parked(
        &fixture,
        need(BAIT, 1, Presence::Held),
        BlockReason::SupplyMissing,
    );
    let mut ledger = None;
    gatherer.interrupt(Interrupt::Pause);
    assert_eq!(gatherer.phase(), ("paused", NativePhase::Waiting));
    gatherer.interrupt(Interrupt::Hold(false));
    let calls = crate::acquire::plan::calls_for_test();
    with_tick_bank(
        &fixture.snapshot,
        Some(&fixture.session),
        &mut ledger,
        2,
        |tick| {
            assert!(gatherer.poll_park(tick));
        },
    );
    assert_eq!(crate::acquire::plan::calls_for_test(), calls);
    gatherer.interrupt(Interrupt::Resume);
    assert!(!gatherer.waits_for_evidence());
    with_tick_bank(
        &fixture.snapshot,
        Some(&fixture.session),
        &mut ledger,
        3,
        |tick| {
            assert!(gatherer.poll_park(tick));
        },
    );
    assert_eq!(crate::acquire::plan::calls_for_test() - calls, 1);
    assert_eq!(gatherer.scratch.parked.as_ref().unwrap().since_tick, 1);
    assert!(gatherer.waits_for_evidence());
    gatherer.configure(Arc::clone(&fixture.config)).unwrap();
    with_tick_bank(
        &fixture.snapshot,
        Some(&fixture.session),
        &mut ledger,
        4,
        |tick| {
            assert!(gatherer.poll_park(tick));
        },
    );
    assert_eq!(crate::acquire::plan::calls_for_test() - calls, 2);
    assert_eq!(gatherer.scratch.parked.as_ref().unwrap().since_tick, 1);
    assert!(ledger.is_none());
    gatherer.user_input();
    assert!(gatherer.scratch.parked.is_none());
    assert_eq!(
        gatherer.failure.as_ref().unwrap().code.as_ref(),
        "manual-movement"
    );
}

#[test]
fn resume_with_hint_admits_one_verifying_trip_and_clears_evidence_idle() {
    if !api::test_support::game_data_available() {
        return;
    }
    let fixture = BaitFixture::new();
    let mut gatherer = parked(
        &fixture,
        need(BAIT, 1, Presence::Held),
        BlockReason::SupplyMissing,
    );
    gatherer.interrupt(Interrupt::Resume);
    let calls = crate::acquire::plan::calls_for_test();
    let mut ledger = None;
    with_tick_bank(
        &fixture.snapshot,
        Some(&fixture.hint),
        &mut ledger,
        2,
        |tick| {
            assert!(!gatherer.poll_park(tick));
        },
    );
    assert_eq!(crate::acquire::plan::calls_for_test() - calls, 1);
    assert!(gatherer.scratch.parked.is_none());
    assert!(!gatherer.waits_for_evidence());
    assert!(matches!(gatherer.active, Active::Acquire(_)));
}

#[test]
fn no_stock_due_and_no_space_use_the_same_single_replan_gate() {
    if !api::test_support::game_data_available() {
        return;
    }
    for (reason, edge, free_slots) in [
        (
            BlockReason::NoStock {
                restock_ticks: 400,
                refused_at_tick: 20,
                aligned_to: None,
                revisit_due: 420,
            },
            420,
            0,
        ),
        (
            BlockReason::NoStock {
                restock_ticks: 400,
                refused_at_tick: 420,
                aligned_to: Some(25),
                revisit_due: 425,
            },
            425,
            0,
        ),
        (BlockReason::NoSpace { slots_needed: 1 }, 10, 1),
    ] {
        let mut fixture = BaitFixture::new();
        fixture
            .snapshot
            .seed_inventory(fixture.snapshot.inventory().to_vec(), 1);
        let mut gatherer = parked(&fixture, need(BAIT, 1, Presence::Held), reason);
        let calls = crate::acquire::plan::calls_for_test();
        let mut ledger = None;
        with_tick_bank(
            &fixture.snapshot,
            Some(&fixture.session),
            &mut ledger,
            edge - 1,
            |tick| {
                assert!(gatherer.poll_park(tick));
            },
        );
        assert_eq!(crate::acquire::plan::calls_for_test(), calls);
        if free_slots > 0 {
            fixture.snapshot.seed_inventory(Vec::new(), 28);
        }
        with_tick_bank(
            &fixture.snapshot,
            Some(&fixture.session),
            &mut ledger,
            edge,
            |tick| {
                // Bank-only test requests re-park as SupplyMissing after the wake.
                assert!(gatherer.poll_park(tick));
            },
        );
        assert_eq!(crate::acquire::plan::calls_for_test() - calls, 1);
        assert_eq!(
            gatherer.scratch.parked.as_ref().unwrap().code,
            "supply-missing"
        );
        assert_eq!(gatherer.scratch.parked.as_ref().unwrap().since_tick, edge);
    }
}
