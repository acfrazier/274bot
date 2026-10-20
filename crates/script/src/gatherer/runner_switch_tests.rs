use super::super::select::PlacementClass;
use super::*;
use crate::native::WalkStallReason;
use crate::quester::families::tests::{def, with_tick};
use api::gather_methods::{known_rows, TargetClass};
use api::named_banks::{NamedBank, NamedBankFacts};
use api::selected::{EntityId, FamilyPreparation};
use api::snapshot::{
    GameSnapshot, ItemActionFamily, ItemContainer, ItemView, NpcView, StatView, WorldStateView,
    WorldTile,
};

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

fn catherby_fishing_fixture() -> (Gatherer, GameSnapshot) {
    let (config, snapshot) = FamilyPreparation::run(|families| {
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let mut settings = crate::native::SettingsBag::new();
        for (key, value) in [
            ("skill", serde_json::json!("Fishing")),
            ("fishingMethod", serde_json::json!("fishing.saltfish.op1")),
            ("location", serde_json::json!("Site")),
            ("site", serde_json::json!("fishing.catherby")),
            ("radius", serde_json::json!(32)),
            ("disposition", serde_json::json!("Bank")),
        ] {
            settings.insert(key.into(), value);
        }
        let mut cx = crate::native::PrepareContext {
            pin: selected.selected_pin().unwrap(),
            selected,
            banks: Arc::default(),
            families,
        };
        let config = super::super::card::prepare(&mut cx, 1, Arc::new(settings)).unwrap();
        let here = WorldTile {
            x: 2848,
            z: 3432,
            level: 0,
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(crate::quester::families::tests::local_player(here));
        snapshot.seed_world(WorldStateView {
            map_base_x: 2792,
            map_base_z: 3384,
            members: true,
            ..WorldStateView::default()
        });
        snapshot.seed_stats(vec![StatView {
            index: 10,
            name: "fishing".into(),
            effective: 76,
            base: 76,
            xp: 1336443,
            used: true,
        }]);
        snapshot.seed_inventory(
            vec![ItemView {
                def: def(303, "Small fishing net"),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: 0,
                count: 1,
                actions: Vec::new(),
                component_id: 3214,
            }],
            28,
        );
        snapshot.seed_equipment(vec![]);
        snapshot.seed_locs(vec![]);
        snapshot.seed_npcs(vec![]);
        (config, snapshot)
    })
    .unwrap()
    .join()
    .unwrap();
    let anchor = WorldTile {
        x: 2836,
        z: 3431,
        level: 0,
    };
    let mut gatherer = Gatherer::new(
        RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        Arc::clone(&config),
        Arc::clone(config.get::<Arc<Prepared>>().unwrap()),
        GatherRetained {
            anchor: Some(anchor),
            ..GatherRetained::default()
        },
    );
    gatherer.area = Some(WorkArea {
        mode: super::super::area::AreaMode::Site,
        anchor,
        radius: 32,
    });
    gatherer.needs_validate = false;
    (gatherer, snapshot)
}

fn catherby_return_plan(
    gatherer: &Gatherer,
    snapshot: &GameSnapshot,
    here: WorldTile,
) -> TargetPlan {
    let area = gatherer
        .area
        .expect("the fixture has a content-derived work area");
    let skill_stat = with_tick(snapshot, &mut None, 1, |tick| {
        gatherer.skill_stat(tick.cx.snapshot().stats())
    });
    let ResourceReturn::Ready(plan) = resource_return_target(
        &gatherer.prepared.catalog,
        &gatherer.prepared.methods,
        gatherer.settings(),
        area,
        ReturnObservation {
            here,
            now: 1,
            skill_stat,
            avoided: &gatherer.avoid,
            collision: None,
        },
    ) else {
        panic!("the fixture has a return stand in each work-area quadrant");
    };
    plan.plan
}

fn catherby_partial_return_fixture() -> (Gatherer, GameSnapshot, TargetPlan, TargetPlan, WorldTile)
{
    let (mut gatherer, mut snapshot) = catherby_fishing_fixture();
    let observed_here = snapshot
        .local_player()
        .expect("the fixture has an observed player")
        .player
        .network;
    let observed_plan = catherby_return_plan(&gatherer, &snapshot, observed_here);
    let old_here = WorldTile {
        x: observed_plan.tile.x
            - i32::from(gatherer.area.expect("the fixture has a work area").radius),
        z: observed_plan.tile.z,
        level: observed_plan.tile.level,
    };
    let initial = catherby_return_plan(&gatherer, &snapshot, old_here);
    let current = WorldTile {
        x: initial.tile.x,
        z: initial.tile.z + i32::from(RESOURCE_APPROACH_RADIUS) + 1,
        level: initial.tile.level,
    };
    assert!(
        old_here
            .x
            .abs_diff(initial.tile.x)
            .max(old_here.z.abs_diff(initial.tile.z))
            > u32::from(RESOURCE_APPROACH_RADIUS) + 1,
        "the expired return has made partial progress toward its content stand"
    );
    // Fishing observation stands are selected from content, not clamped toward
    // the player (select.rs:1060-1077). A fresh plan can correctly keep its goal.
    let replan = catherby_return_plan(&gatherer, &snapshot, current);
    gatherer.trip = TripStep::Return;
    gatherer.target = Some(initial.clone());
    let mut player = snapshot
        .local_player()
        .expect("the fixture has a player")
        .clone();
    player.player.network = current;
    player.player.actor.moving = false;
    snapshot.seed_local_player(player);
    (gatherer, snapshot, initial, replan, current)
}

fn stalled_return_receipt(
    evidence: api::quest_progress::EvidenceStamp,
    at: WorldTile,
    aiming: WorldTile,
    why: WalkStallReason,
    tries: u32,
) -> WalkReceipt {
    WalkReceipt {
        goal: None,
        request_id: 7,
        evidence,
        end: WalkEnd::Stalled(Arc::new(crate::native::WalkStall {
            at,
            aiming,
            why,
            tries,
        })),
        blocked: None,
        detail: None,
        refusal: None,
        assessment: None,
        escape: None,
    }
}
fn assert_return_recovery_submitted(gatherer: &Gatherer) {
    assert!(gatherer.failure.is_none());
    assert!(matches!(gatherer.active, Active::Walk(_)));
    assert_eq!(
        gatherer.last_event.as_ref(),
        "return recovery action submitted after transient Stalled"
    );
    assert_eq!(gatherer.scratch.return_recovery_actions, 1);
}
fn catherby_npc(index: usize, type_id: usize, tile: WorldTile) -> NpcView {
    NpcView {
        index,
        r#type: Some(type_id),
        name: None,
        actions: vec![],
        tile,
        distance: 0,
        animation: -1,
        animation_frame: 0,
        pose_animation: -1,
        orientation: 0,
        target_orientation: 0,
        overhead_text: None,
        spot_animation: -1,
        spot_animation_stamp: -1,
        health: 0,
        total_health: 0,
        face_entity: -1,
        target: None,
        moving: false,
        running: false,
        in_combat: false,
        level: 0,
        size: 1,
        network: tile,
        x: 0,
        z: 0,
        yaw: 0,
    }
}

fn credit_catherby_yield(gatherer: &mut Gatherer, snapshot: &GameSnapshot, now: u64) {
    let mut ledger = None;
    with_tick(snapshot, &mut ledger, now, |tick| {
        gatherer.observed_progress = Some((0, 1336403, tick.cx.snapshot().here().unwrap().value));
        gatherer.observe_progress(tick)
    });
}

#[test]
fn catherby_wait_deadline_is_not_proof_that_temporary_spots_are_unavailable() {
    let (mut gatherer, mut snapshot) = catherby_fishing_fixture();
    let west = WorldTile {
        x: 2836,
        z: 3431,
        level: 0,
    };
    let other = WorldTile {
        x: 2838,
        z: 3431,
        level: 0,
    };
    // Both authored NPCs can be temporarily changed to whirlpools for 60 ticks.
    // A retained wait can expire before the observed replacement does.
    snapshot.seed_npcs(vec![
        catherby_npc(2170, 404, west),
        catherby_npc(2171, 404, other),
    ]);
    credit_catherby_yield(&mut gatherer, &snapshot, 9720);
    gatherer.wait_until = Some(9753);
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 9753, |tick| {
        gatherer.begin_idle(tick, false)
    });
    assert!(
        gatherer.failure.is_none(),
        "an elapsed wait is not content proof of permanent absence"
    );
    snapshot.seed_npcs(vec![catherby_npc(2171, 320, other)]);
    with_tick(&snapshot, &mut ledger, 9760, |tick| {
        gatherer.begin_idle(tick, false)
    });
    let target = gatherer
        .target
        .as_ref()
        .expect("the restored fishing NPC must be approached");
    assert_eq!(target.entity, EntityId::Npc(320));
    assert_eq!(target.npc_index, 2171);
    assert_eq!(target.tile, other);
    assert!(matches!(gatherer.active, Active::Gather(_)));
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        crate::native::HostEffect::Interaction(crate::shim::InteractReq::Npc {
            index: Some(2171), action, ..
        }) if action == "Net"
    ));
    assert!(gatherer.wait_until.is_none());
}

#[test]
fn catherby_big_fish_replacement_does_not_prove_permanent_absence() {
    let (mut gatherer, mut snapshot) = catherby_fishing_fixture();
    let west = WorldTile {
        x: 2836,
        z: 3431,
        level: 0,
    };
    let other = WorldTile {
        x: 2838,
        z: 3431,
        level: 0,
    };
    // macro_event_big_fish.rs2:6 temporarily uses type 390 for five ticks.
    // It is not a permanent removal of either content-authored saltfish.
    snapshot.seed_npcs(vec![
        catherby_npc(2170, 390, west),
        catherby_npc(2171, 390, other),
    ]);
    credit_catherby_yield(&mut gatherer, &snapshot, 9720);
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 9723, |tick| {
        gatherer.begin_idle(tick, false)
    });
    assert!(
        gatherer.failure.is_none(),
        "temporary NPC replacements must not block fishing"
    );
    snapshot.seed_npcs(vec![catherby_npc(2171, 320, other)]);
    with_tick(&snapshot, &mut ledger, 9728, |tick| {
        gatherer.begin_idle(tick, false)
    });
    let target = gatherer
        .target
        .as_ref()
        .expect("the restored fishing NPC must be approached");
    assert_eq!(target.entity, EntityId::Npc(320));
    assert_eq!(target.npc_index, 2171);
    assert!(matches!(gatherer.active, Active::Gather(_)));
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        crate::native::HostEffect::Interaction(crate::shim::InteractReq::Npc {
            index: Some(2171), action, ..
        }) if action == "Net"
    ));
}

#[test]
fn catherby_exhausted_survey_renews_the_9753_approach_budget() {
    let (mut gatherer, mut snapshot) = catherby_fishing_fixture();
    let here = WorldTile {
        x: 2838,
        z: 3432,
        level: 0,
    };
    snapshot.seed_local_player(crate::quester::families::tests::local_player(here));
    credit_catherby_yield(&mut gatherer, &snapshot, 9720);
    gatherer.avoid(
        WorldTile {
            x: 2836,
            z: 3431,
            level: 0,
        },
        9753,
    );
    let mut ledger = None;
    let mut walks = 0;
    // Hold the player outside complete visibility so every approach is
    // unproductive. Eight attempts per authored placement exhaust the survey
    // before the old avoid expires at the user's absolute deadline.
    for now in 9723..=9764 {
        with_tick(&snapshot, &mut ledger, now, |tick| {
            gatherer.begin_idle(tick, false)
        });
        if matches!(gatherer.active, Active::Walk(_)) {
            walks += 1;
        }
        if now == 9752 {
            assert_eq!(
                walks, 16,
                "both Catherby placements have an eight-walk budget"
            );
            assert_eq!(gatherer.wait_until, Some(9753));
        }
        if now == 9753 {
            assert!(
                gatherer.wait_until.is_none(),
                "expiry must renew the survey"
            );
        }
        assert!(
            gatherer.failure.is_none(),
            "tick {now}: {:?}",
            gatherer.failure
        );
        gatherer.active = Active::None;
        gatherer.target = None;
    }
    assert!(
        walks > 16,
        "renewal must admit another pass after both budgets exhaust"
    );
}

#[test]
fn catherby_absent_survey_renews_empty_coverage_and_approaches() {
    let (mut gatherer, mut snapshot) = catherby_fishing_fixture();
    let west = WorldTile {
        x: 2838,
        z: 3432,
        level: 0,
    };
    snapshot.seed_local_player(crate::quester::families::tests::local_player(west));
    credit_catherby_yield(&mut gatherer, &snapshot, 9720);
    let mut ledger = None;
    for now in 9723..=9729 {
        with_tick(&snapshot, &mut ledger, now, |tick| {
            gatherer.begin_idle(tick, false)
        });
        assert!(matches!(gatherer.active, Active::Walk(_)));
        gatherer.active = Active::None;
        gatherer.target = None;
    }
    // Complete visibility records empty coverage, but temporary absence must
    // renew it so another unobserved pass can start.
    snapshot.seed_local_player(crate::quester::families::tests::local_player(WorldTile {
        x: 2848,
        ..west
    }));
    with_tick(&snapshot, &mut ledger, 9730, |tick| {
        gatherer.begin_idle(tick, false)
    });
    assert!(gatherer.failure.is_none());
    assert!(matches!(gatherer.active, Active::None));
    snapshot.seed_local_player(crate::quester::families::tests::local_player(west));
    with_tick(&snapshot, &mut ledger, 9731, |tick| {
        gatherer.begin_idle(tick, false)
    });
    assert!(
        matches!(gatherer.active, Active::Walk(_)),
        "absence must clear coverage and admit another observation walk"
    );
}

#[test]
fn catherby_no_spots_blocks_at_800_ticks_despite_walking_progress() {
    let (mut gatherer, mut snapshot) = catherby_fishing_fixture();
    credit_catherby_yield(&mut gatherer, &snapshot, 100);
    snapshot.seed_local_player(crate::quester::families::tests::local_player(WorldTile {
        x: 2849,
        z: 3432,
        level: 0,
    }));
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 899, |tick| {
        gatherer.observe_progress(tick);
        gatherer.begin_idle(tick, false);
    });
    assert_eq!(gatherer.last_gameplay_tick, 899);
    assert!(
        gatherer.failure.is_none(),
        "799 ticks without yield still renews"
    );
    with_tick(&snapshot, &mut ledger, 900, |tick| {
        gatherer.begin_idle(tick, false)
    });
    let failure = gatherer
        .failure
        .as_ref()
        .expect("800 ticks without yield must block");
    assert_eq!(failure.code.as_ref(), "resource-unavailable");
    assert!(failure
        .message
        .contains("no fishing spot observed for 800 ticks"));
    assert!(failure.message.contains("method=fishing.saltfish.op1"));
    assert!(failure.message.contains("site=fishing.catherby"));
    assert_eq!(gatherer.phase(), ("blocked", NativePhase::Blocked));
}

#[test]
fn catherby_unobserved_spots_cannot_start_another_walk_after_800_ticks() {
    let (mut gatherer, mut snapshot) = catherby_fishing_fixture();
    snapshot.seed_local_player(crate::quester::families::tests::local_player(WorldTile {
        x: 2838,
        z: 3432,
        level: 0,
    }));
    credit_catherby_yield(&mut gatherer, &snapshot, 100);
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 899, |tick| {
        gatherer.begin_idle(tick, false)
    });
    assert!(matches!(gatherer.active, Active::Walk(_)));
    gatherer.active = Active::None;
    gatherer.target = None;
    with_tick(&snapshot, &mut ledger, 900, |tick| {
        gatherer.begin_idle(tick, false)
    });
    assert!(
        gatherer.failure.is_some(),
        "observation walks cannot renew the yield bound"
    );
    assert!(matches!(gatherer.active, Active::None));
}

#[test]
fn catherby_hazard_wait_is_bounded_by_yield_not_walking_progress() {
    let (mut gatherer, mut snapshot) = catherby_fishing_fixture();
    credit_catherby_yield(&mut gatherer, &snapshot, 100);
    snapshot.seed_local_player(crate::quester::families::tests::local_player(WorldTile {
        x: 2849,
        z: 3432,
        level: 0,
    }));
    snapshot.seed_npcs(vec![catherby_npc(
        2170,
        404,
        WorldTile {
            x: 2836,
            z: 3431,
            level: 0,
        },
    )]);
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 899, |tick| {
        gatherer.observe_progress(tick);
        gatherer.begin_idle(tick, false);
    });
    assert_eq!(
        gatherer.wait_until,
        Some(900),
        "a fresh wait cannot exceed the no-yield bound"
    );
    assert!(gatherer.failure.is_none());
    with_tick(&snapshot, &mut ledger, 900, |tick| {
        gatherer.begin_idle(tick, false)
    });
    assert!(gatherer.failure.is_some());
}

#[test]
fn catherby_idle_and_observation_walks_report_surveying_not_working() {
    let (mut gatherer, mut snapshot) = catherby_fishing_fixture();
    credit_catherby_yield(&mut gatherer, &snapshot, 100);
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 101, |tick| {
        gatherer.begin_idle(tick, false)
    });
    assert_eq!(gatherer.phase(), ("surveying", NativePhase::Waiting));
    snapshot.seed_local_player(crate::quester::families::tests::local_player(WorldTile {
        x: 2838,
        z: 3432,
        level: 0,
    }));
    with_tick(&snapshot, &mut ledger, 102, |tick| {
        gatherer.begin_idle(tick, false)
    });
    assert!(matches!(gatherer.active, Active::Walk(_)));
    assert_eq!(gatherer.phase(), ("surveying", NativePhase::Waiting));
}

#[test]
fn catherby_first_tick_starts_the_no_yield_window_on_the_session_clock() {
    let (mut gatherer, snapshot) = catherby_fishing_fixture();
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 10000, |tick| {
        gatherer.tick(tick).unwrap();
    });
    assert_eq!(gatherer.scratch.last_yield_tick, 10000);
    assert!(gatherer.failure.is_none());
    with_tick(&snapshot, &mut ledger, 10799, |tick| {
        gatherer.tick(tick).unwrap();
    });
    assert!(gatherer.failure.is_none());
    with_tick(&snapshot, &mut ledger, 10800, |tick| {
        gatherer.tick(tick).unwrap();
    });
    assert!(gatherer.failure.is_some());
}

#[test]
fn catherby_product_and_gather_completion_each_renew_the_no_yield_window() {
    let (mut gatherer, mut snapshot) = catherby_fishing_fixture();
    credit_catherby_yield(&mut gatherer, &snapshot, 100);
    assert_eq!(gatherer.scratch.last_yield_tick, 100);
    snapshot.seed_inventory(
        vec![
            ItemView {
                def: def(303, "Small fishing net"),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: 0,
                count: 1,
                actions: Vec::new(),
                component_id: 3214,
            },
            ItemView {
                def: def(gatherer.prepared.products[0], "Fish"),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: 1,
                count: 1,
                actions: Vec::new(),
                component_id: 3214,
            },
        ],
        28,
    );
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 899, |tick| {
        gatherer.observe_progress(tick)
    });
    assert_eq!(
        gatherer.scratch.last_yield_tick, 899,
        "a product delta without XP renews"
    );
    assert_eq!(
        gatherer.last_gameplay_tick, 100,
        "the product delta did not move the player"
    );
    for (now, gained, xp) in [(1698, 1, 0), (2497, 0, 40)] {
        with_tick(&snapshot, &mut ledger, now, |tick| {
            gatherer.handle_gather(
                GatherResult {
                    end: GatherEnd::Depleted,
                    gained,
                    xp,
                },
                tick,
            );
        });
        assert_eq!(gatherer.scratch.last_yield_tick, now);
        gatherer.needs_validate = false;
        with_tick(&snapshot, &mut ledger, now + 799, |tick| {
            gatherer.begin_idle(tick, false)
        });
        assert!(gatherer.failure.is_none());
    }
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
                        candidate_goals: None,
                        loc_id: match target.entity {
                            EntityId::Loc(id) => Some(id),
                            _ => None,
                        },
                        radius: 1,
                        arrival: nav::contract::ArrivalKind::Reach,
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
                observation_goals: None,
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
                goal: None,
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
                observation_goals: None,
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
    let chosen = WorldTile {
        x: plan.tile.x + 1,
        ..plan.tile
    };
    let goals = crate::native::WalkGoals::new(vec![plan.tile, chosen]).unwrap();
    gatherer.cancel_active();
    ledger.as_mut().unwrap().outbox.clear();
    with_tick(&snapshot, &mut ledger, 2, |tick| {
        gatherer.start_target(
            SelectedTarget {
                plan,
                class: PlacementClass::Unloaded,
                observation_goals: Some(goals),
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
    assert_eq!(request.radius, 0);
    assert_eq!(
        request.candidate_goals.as_ref().unwrap().as_slice().len(),
        2
    );
    with_tick(&snapshot, &mut ledger, 3, |tick| {
        gatherer.handle_walk(
            WalkReceipt {
                goal: Some(chosen),
                request_id: 7,
                evidence: tick.cx.evidence(),
                end: WalkEnd::Arrived,
                blocked: None,
                detail: None,
                assessment: None,
                refusal: None,
                escape: None,
            },
            tick,
        );
    });
    assert_eq!(gatherer.target.as_ref().unwrap().tile, chosen);
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
                observation_goals: None,
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
#[test]
fn progressed_return_stall_replans_once_from_the_observed_position() {
    let (mut gatherer, snapshot, initial, replan, current) = catherby_partial_return_fixture();
    let mut ledger = None;
    let mut submitted_at = None;
    with_tick(&snapshot, &mut ledger, 700, |tick| {
        submitted_at = Some(tick.cx.evidence());
        gatherer.handle_walk(
            stalled_return_receipt(
                tick.cx.evidence(),
                current,
                initial.tile,
                WalkStallReason::Expired,
                3,
            ),
            tick,
        );
    });
    assert_return_recovery_submitted(&gatherer);
    let ledger = ledger.as_ref().expect("the recovery walk was admitted");
    let request = ledger
        .outbox
        .iter()
        .find_map(|action| match &action.effect {
            crate::native::HostEffect::Walk(request) => Some(request),
            _ => None,
        })
        .expect("one recovery walk was submitted");
    assert_eq!(request.target, replan.tile);
    assert_eq!(request.arrival, ArrivalKind::Area);
    assert_eq!(request.required_after, submitted_at.unwrap());
}

#[test]
fn second_transient_return_stall_is_terminal_without_a_third_walk() {
    let (mut gatherer, snapshot, initial, _, current) = catherby_partial_return_fixture();
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 710, |tick| {
        gatherer.handle_walk(
            stalled_return_receipt(
                tick.cx.evidence(),
                current,
                initial.tile,
                WalkStallReason::Expired,
                3,
            ),
            tick,
        );
    });
    assert_return_recovery_submitted(&gatherer);
    gatherer.cancel_active();
    ledger.as_mut().unwrap().outbox.clear();
    with_tick(&snapshot, &mut ledger, 711, |tick| {
        gatherer.handle_walk(
            stalled_return_receipt(
                tick.cx.evidence(),
                current,
                initial.tile,
                WalkStallReason::Expired,
                4,
            ),
            tick,
        );
    });
    assert_eq!(
        gatherer
            .failure
            .as_ref()
            .map(|failure| failure.code.as_ref()),
        Some("return-failed")
    );
    assert!(gatherer.failure.as_ref().is_some_and(|failure| {
        failure.message.contains("Expired") && failure.message.contains("tries: 4")
    }));
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "a second stalled route cannot be retried"
    );
}

#[test]
fn refused_recovery_route_is_terminal_and_preserves_host_reason() {
    let (mut gatherer, snapshot, initial, _, current) = catherby_partial_return_fixture();
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 720, |tick| {
        gatherer.handle_walk(
            stalled_return_receipt(
                tick.cx.evidence(),
                current,
                initial.tile,
                WalkStallReason::Dropped,
                2,
            ),
            tick,
        );
    });
    assert_return_recovery_submitted(&gatherer);
    gatherer.cancel_active();
    ledger.as_mut().unwrap().outbox.clear();
    let detail: Arc<str> = Arc::from("native follow failed: route permanently refused");
    with_tick(&snapshot, &mut ledger, 721, |tick| {
        gatherer.handle_walk(
            WalkReceipt {
                goal: None,
                request_id: 8,
                evidence: tick.cx.evidence(),
                end: WalkEnd::Refused,
                blocked: None,
                detail: Some(Arc::clone(&detail)),
                refusal: None,
                assessment: None,
                escape: None,
            },
            tick,
        );
    });
    let failure = gatherer
        .failure
        .as_ref()
        .expect("a permanent refusal is terminal");
    assert_eq!(failure.code.as_ref(), "return-failed");
    assert!(failure.message.contains(detail.as_ref()));
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "refusal cannot submit another recovery walk"
    );
}

#[test]
fn end_blocked_return_stall_is_terminal_not_a_transient_replan() {
    let (mut gatherer, snapshot, initial, _, current) = catherby_partial_return_fixture();
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 730, |tick| {
        gatherer.handle_walk(
            stalled_return_receipt(
                tick.cx.evidence(),
                current,
                initial.tile,
                WalkStallReason::EndBlocked,
                1,
            ),
            tick,
        );
    });
    assert_eq!(
        gatherer
            .failure
            .as_ref()
            .map(|failure| failure.code.as_ref()),
        Some("return-failed")
    );
    assert_eq!(gatherer.scratch.return_recovery_actions, 0);
    if let Some(ledger) = ledger {
        assert!(ledger.outbox.is_empty());
    }
}

#[test]
fn typed_bank_deposit_failure_keeps_the_content_reason() {
    let (mut gatherer, _) = fixture();
    gatherer.deposit_failure(ActionError::BankDeposit(
        crate::bank::ops::DepositFailure::FullStack,
    ));
    let failure = gatherer
        .failure
        .expect("the typed deposit failure is terminal");
    assert_eq!(failure.code.as_ref(), "bank-deposit-failed");
    assert_eq!(
        failure.message.as_ref(),
        "bank deposit refused: full stack in bank"
    );
}

fn unbankable_bank_fixture(fish_count: i32) -> (Gatherer, GameSnapshot) {
    let (mut gatherer, mut snapshot) = catherby_fishing_fixture();
    gatherer.tool = ToolState {
        id: 303,
        worn: false,
    };
    gatherer.trip = TripStep::Deposit;
    let selected = &gatherer.prepared.selected;
    let item = |alias: &str, slot, count, container| {
        let fact = selected
            .item_by_alias(alias)
            .expect("selected content item");
        let mut definition = def(fact.id, fact.name.as_deref().unwrap());
        definition.stackable = fact.stackable;
        ItemView {
            def: definition,
            container,
            action_family: ItemActionFamily::Held,
            slot,
            count,
            actions: if container == ItemContainer::BankSide {
                vec![Some("Deposit-All".into())]
            } else {
                Vec::new()
            },
            component_id: 3214,
        }
    };
    let mut pack = vec![
        item("net", 0, 1, ItemContainer::Inventory),
        item("macro_cube", 1, 28, ItemContainer::Inventory),
    ];
    for slot in 2..28 {
        pack.push(item(
            if slot < 2 + fish_count {
                "raw_shrimp"
            } else {
                "net"
            },
            slot,
            1,
            ItemContainer::Inventory,
        ));
    }
    let side = pack
        .iter()
        .map(|row| ItemView {
            container: ItemContainer::BankSide,
            actions: vec![Some("Deposit-All".into())],
            ..row.clone()
        })
        .collect();
    snapshot.seed_inventory(pack, 28);
    snapshot.seed_bank_observation(10, 1, Some(Vec::new()), side);
    (gatherer, snapshot)
}

#[test]
fn unbankable_items_are_kept_and_the_bank_trip_continues_after_freeing_space() {
    for fish_count in [26, 1] {
        let (mut gatherer, mut snapshot) = unbankable_bank_fixture(fish_count);
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 1, |tick| gatherer.begin_trip(tick));
        with_tick(&snapshot, &mut ledger, 2, |tick| gatherer.poll_active(tick));
        let action = ledger.as_mut().unwrap().outbox.remove(0);
        assert!(
            matches!(
                action.effect,
                crate::native::HostEffect::Interaction(crate::shim::InteractReq::InvButton {
                    id: 317,
                    ..
                })
            ),
            "only the bankable fish may be sent"
        );
        ledger.as_mut().unwrap().complete_interaction(
            &action.authority(),
            crate::native::InteractionReceipt {
                request_id: action.request_id.get(),
                evidence: api::quest_progress::EvidenceStamp {
                    run: action.run(),
                    tick: 2,
                    sequence: 2,
                },
                accepted: true,
                chat_since: 0,
            },
        );
        let kept: Vec<_> = snapshot
            .inventory()
            .iter()
            .filter(|row| row.def.id != 317)
            .cloned()
            .collect();
        snapshot.seed_inventory(kept, 28);
        snapshot.seed_bank_observation(10, 1, Some(Vec::new()), Vec::new());
        with_tick(&snapshot, &mut ledger, 3, |tick| gatherer.poll_active(tick));
        assert!(gatherer.failure.is_none(), "{:?}", gatherer.failure);
        assert_eq!(gatherer.trip, TripStep::Withdraw);
        assert_eq!(gatherer.retained.deposited, fish_count as u32);
        assert_eq!(
            crate::bank::ops::count_id(snapshot.inventory(), 3062),
            28,
            "the Strange box stack is kept"
        );
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }
}

#[test]
fn unbankable_items_stop_only_when_the_pack_remains_full_after_deposit() {
    for start_full in [true, false] {
        let (mut gatherer, mut snapshot) = unbankable_bank_fixture(0);
        if !start_full {
            let rows = snapshot.inventory()[..2].to_vec();
            snapshot.seed_inventory(rows, 28);
        }
        let mut ledger = None;
        with_tick(&snapshot, &mut ledger, 1, |tick| gatherer.begin_trip(tick));
        with_tick(&snapshot, &mut ledger, 2, |tick| gatherer.poll_active(tick));
        if start_full {
            let failure = gatherer
                .failure
                .expect("a full pack cannot start another gather trip");
            assert_eq!(failure.code.as_ref(), "bank-deposit-failed");
            assert_eq!(
                failure.message.as_ref(),
                crate::bank::ops::DepositFailure::Unbankable.message()
            );
        } else {
            assert!(gatherer.failure.is_none(), "{:?}", gatherer.failure);
            assert_eq!(gatherer.trip, TripStep::Withdraw);
        }
        assert_eq!(crate::bank::ops::count_id(snapshot.inventory(), 3062), 28);
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }
}

#[test]
fn tool_fetch_trip_keeps_unbankable_items_and_continues_to_withdraw_the_tool() {
    let (mut gatherer, mut snapshot) = unbankable_bank_fixture(0);
    let tool = ItemView {
        container: ItemContainer::Bank,
        actions: vec![Some("Withdraw-1".into())],
        ..snapshot.inventory()[0].clone()
    };
    snapshot.seed_inventory(vec![snapshot.inventory()[1].clone()], 28);
    snapshot.seed_bank_observation(10, 1, Some(vec![tool.clone()]), Vec::new());
    gatherer.trip = TripStep::Idle;
    gatherer.tool.id = -1;
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 1, |tick| {
        let verdict = SupplyPlan::plan(
            &gatherer.prepared,
            tick.cx.snapshot().stats().unwrap().value,
            tick.cx.snapshot().stock(),
        );
        assert!(
            gatherer.admit_supply(verdict),
            "the missing tool needs a trip"
        );
    });
    assert_eq!(gatherer.trip, TripStep::Select);
    // Arrival at the open bank follows the same Deposit boundary as disposal.
    gatherer.trip = TripStep::Deposit;
    with_tick(&snapshot, &mut ledger, 2, |tick| gatherer.begin_trip(tick));
    with_tick(&snapshot, &mut ledger, 3, |tick| gatherer.poll_active(tick));
    assert!(gatherer.failure.is_none(), "{:?}", gatherer.failure);
    assert_eq!(gatherer.trip, TripStep::Withdraw);
    with_tick(&snapshot, &mut ledger, 4, |tick| gatherer.begin_trip(tick));
    with_tick(&snapshot, &mut ledger, 5, |tick| gatherer.poll_active(tick));
    let mode = ledger.as_mut().unwrap().outbox.remove(0);
    assert!(matches!(
        mode.effect,
        crate::native::HostEffect::Interaction(crate::shim::InteractReq::SetNoteMode { on: false })
    ));
    ledger.as_mut().unwrap().complete_interaction(
        &mode.authority(),
        crate::native::InteractionReceipt {
            request_id: mode.request_id.get(),
            evidence: api::quest_progress::EvidenceStamp {
                run: mode.run(),
                tick: 5,
                sequence: 5,
            },
            accepted: true,
            chat_since: 0,
        },
    );
    with_tick(&snapshot, &mut ledger, 6, |tick| gatherer.poll_active(tick));
    let outbox = &ledger.as_ref().unwrap().outbox;
    assert_eq!(outbox.len(), 1, "only the required tool is withdrawn");
    assert!(matches!(
        outbox[0].effect,
        crate::native::HostEffect::Interaction(crate::shim::InteractReq::WithdrawX {
            bank_item_id: 303,
            lands_as_id: 303,
            count: 1,
            ..
        })
    ));
    snapshot.seed_inventory(
        vec![
            snapshot.inventory()[0].clone(),
            ItemView {
                container: ItemContainer::Inventory,
                slot: 0,
                ..tool
            },
        ],
        28,
    );
    with_tick(&snapshot, &mut ledger, 7, |tick| gatherer.poll_active(tick));
    assert!(gatherer.scratch.withdrew_supplies);
    assert!(gatherer.failure.is_none(), "{:?}", gatherer.failure);
    assert_eq!(gatherer.trip, TripStep::Close);
    assert_eq!(crate::bank::ops::count_id(snapshot.inventory(), 303), 1);
    assert_eq!(crate::bank::ops::count_id(snapshot.inventory(), 3062), 28);
}

#[test]
fn unbankable_items_report_item_reason_when_pack_fills_without_withdrawals() {
    let (mut gatherer, snapshot) = unbankable_bank_fixture(0);
    gatherer.trip = TripStep::Validate;
    gatherer.needs_validate = false;
    gatherer.scratch.kept_unbankable = true;
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 1, |tick| gatherer.begin_trip(tick));
    let failure = gatherer
        .failure
        .expect("the full pack has an unbankable item");
    assert_eq!(failure.code.as_ref(), "bank-deposit-failed");
    assert_eq!(
        failure.message.as_ref(),
        crate::bank::ops::DepositFailure::Unbankable.message()
    );
    assert!(matches!(gatherer.active, Active::None));
}

#[test]
fn unbankable_items_report_supply_reason_when_withdrawals_fill_the_pack() {
    let (mut gatherer, snapshot) = unbankable_bank_fixture(0);
    gatherer.trip = TripStep::Validate;
    gatherer.needs_validate = false;
    gatherer.scratch.kept_unbankable = true;
    gatherer.scratch.withdrew_supplies = true;
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 1, |tick| gatherer.begin_trip(tick));
    let failure = gatherer
        .failure
        .expect("the supply-filled pack cannot return to gathering");
    assert_eq!(failure.code.as_ref(), "inventory-blocked");
    assert_eq!(
        failure.message.as_ref(),
        "supply withdrawals filled the inventory; no space remains for gathering"
    );
    assert!(matches!(gatherer.active, Active::None));
}
