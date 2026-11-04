use super::super::select::PlacementClass;
use super::*;
use crate::native::WalkStallReason;
use crate::quester::families::tests::{def, with_tick, with_tick_scene_reach};
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
            zones: None,
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

fn prerequisite_gate() -> (
    api::prereq_steps::ActionableGate,
    api::prereq_steps::CrossingId,
    Arc<str>,
) {
    let data =
        api::game_data::for_revision(api::selected::ClientRevision::R289).expect("R289 data");
    let catalog = api::selected::FamilyPreparation::run(move |worker| {
        data.prepare_prereq_steps(worker)
            .expect("R289 prerequisite catalog")
    })
    .expect("family preparation worker")
    .join()
    .expect("prerequisite preparation");
    let (_, row) = catalog
        .iter()
        .find(|(_, row)| row.crossings.iter().any(|entry| entry.crossing.option > 0))
        .expect("R289 has a prerequisite crossing");
    let entry = row
        .crossings
        .iter()
        .find(|entry| entry.crossing.option > 0)
        .expect("selected row has a supported operation");
    let crossing = entry.crossing;
    // The dialogue fixture needs its actual barrier promise, not any journal interval.
    let gate = api::selected::QuestGate::Window(row.establishes.clone());
    (
        api::prereq_steps::ActionableGate {
            gate,
            truth: api::selected::Truth::Unknown,
            leg: 0,
            crossing,
        },
        crossing,
        Arc::clone(&entry.pass.message),
    )
}

#[test]
fn prerequisite_handoff_reissues_original_walk_once_and_cancel_revokes_it() {
    let (mut gatherer, mut snapshot) = fixture();
    let (gate, crossing, pass_message) = prerequisite_gate();
    let target = WorldTile {
        x: crossing.at.x + 80,
        z: crossing.at.z + 80,
        level: crossing.at.level,
    };
    let mut player = snapshot
        .local_player()
        .expect("fixture has a local player")
        .clone();
    player.player.network = crossing.takeoff;
    player.player.actor.tile = crossing.takeoff;
    player.player.actor.moving = false;
    snapshot.seed_local_player(player);
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let catalog = FamilyPreparation::run(move |worker| data.prepare_prereq_steps(worker).unwrap())
        .unwrap()
        .join()
        .unwrap();
    let (_, row) = catalog
        .iter()
        .find(|(_, row)| row.crossing(&crossing).is_some())
        .unwrap();
    crate::quester::families::tests::seed_prereq_crossing(&mut snapshot, row, &crossing);

    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 1, |tick| {
        assert!(gatherer.trip_walk(target, 2, ArrivalKind::Reach, None, tick));
    });
    let (outer_id, outer_run) = {
        let action = &ledger.as_ref().unwrap().outbox[0];
        (action.request_id.get(), action.run())
    };
    ledger.as_mut().unwrap().walk = Some(WalkReceipt {
        goal: None,
        request_id: outer_id,
        evidence: api::quest_progress::EvidenceStamp {
            run: outer_run,
            tick: 2,
            sequence: 2,
        },
        end: WalkEnd::NeedsStep(Arc::from([gate])),
        blocked: None,
        detail: None,
        refusal: None,
        assessment: None,
        escape: None,
        prereq_allow: Some(crate::native::PrereqAllow {
            dialogue: true,
            quest_start: true,
        }),
    });
    with_tick_scene_reach(&snapshot, &mut ledger, 2, |tick| {
        gatherer.poll_active(tick);
    });
    assert!(matches!(gatherer.active, Active::Walk(_)));
    let (probe_authority, probe_id, probe_run) = {
        let probe = ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .find(|action| {
                matches!(
                    &action.effect,
                    crate::native::HostEffect::Interaction(
                        crate::interact::InteractReq::Loc { id: Some(id), .. }
                    ) if *id == crossing.loc_id
                )
            })
            .expect("prerequisite probe was submitted");
        (probe.authority(), probe.request_id.get(), probe.run())
    };
    ledger.as_mut().unwrap().complete_interaction(
        &probe_authority,
        crate::native::InteractionReceipt {
            request_id: probe_id,
            evidence: api::quest_progress::EvidenceStamp {
                run: probe_run,
                tick: 2,
                sequence: 2,
            },
            accepted: true,
            chat_since: 0,
        },
    );
    let mut player = snapshot
        .local_player()
        .expect("fixture has a local player")
        .clone();
    player.player.network = crossing.to;
    player.player.actor.tile = crossing.to;
    snapshot.seed_local_player(player);
    snapshot.seed_chat_lines(vec![api::snapshot::ChatLineView {
        sequence: 1,
        type_: 0,
        username: None,
        text: pass_message.to_string(),
    }]);
    with_tick(&snapshot, &mut ledger, 3, |tick| {
        gatherer.poll_active(tick);
    });

    let walk_ids: Vec<_> = ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .filter_map(|action| match &action.effect {
            crate::native::HostEffect::Walk(request) => {
                Some((action.request_id.get(), request.clone()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(walk_ids.len(), 1, "only the live reissue remains queued");
    let (reissued_id, reissued) = &walk_ids[0];
    assert!(
        *reissued_id > outer_id,
        "the outer request was replaced once"
    );
    assert_eq!(reissued.target, target);
    assert_eq!(reissued.radius, 2);
    assert_eq!(
        reissued.options.allow_teleports,
        WalkOptions::default().allow_teleports
    );
    assert!(reissued.cross.is_empty());
    assert!(reissued.avoid.is_empty());
    assert!(!reissued.protect);
    assert!(reissued.food_guard && reissued.allow.food);
    assert!(!reissued.cure_guard);
    assert_eq!(reissued.required_after.tick, 3);
    assert!(gatherer.target.is_none());

    with_tick(&snapshot, &mut ledger, 4, |tick| {
        gatherer.poll_active(tick);
    });
    assert_eq!(
        ledger.as_ref().unwrap().outbox.len(),
        1,
        "an unchanged pending wrapper keeps only its one live reissue"
    );
    gatherer.cancel_active();
    let reissued_action = ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .find(|action| action.request_id.get() == *reissued_id)
        .expect("re-issued walk remains recorded");
    assert!(
        !reissued_action.live(),
        "cancellation revokes the owned walk"
    );
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
            zones: None,
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
            walk_danger: Some(&crate::native::AllowWalkDanger),
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
        prereq_allow: None,
    }
}
fn assert_return_recovery_submitted(gatherer: &Gatherer) {
    assert!(gatherer.failure.is_none());
    assert!(matches!(gatherer.active, Active::Walk(_)));
    assert_eq!(
        gatherer.last_event.as_ref(),
        "return recovery action submitted after transient Stalled"
    );
    assert_eq!(gatherer.scratch.return_stall_recovery.actions(), 1);
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
        crate::native::HostEffect::Interaction(crate::interact::InteractReq::Npc {
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
        crate::native::HostEffect::Interaction(crate::interact::InteractReq::Npc {
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
            zones: None,
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
type CallerLedger = Option<Box<crate::native::ledger::Ledger>>;

struct CallerNoStore;

impl api::local_state::LocalStatePort for CallerNoStore {
    fn lease(
        &self,
        identity: &api::local_state::DomainIdentity,
    ) -> Result<api::local_state::DomainLease, api::local_state::ErrorCode> {
        Err(api::local_state::ErrorCode::NotReady {
            domain: identity.domain(),
        })
    }

    fn submit(
        &self,
        edit: api::local_state::DomainEdit,
    ) -> api::local_state::SubmitReceipt {
        api::local_state::SubmitReceipt::Refused(api::local_state::ErrorCode::Busy {
            domain: edit.identity().domain(),
        })
    }

    fn observe(
        &self,
        _: api::identity::RequestId,
    ) -> Option<api::local_state::WriteSettlement> {
        None
    }
}

impl crate::local_state::StatePort for CallerNoStore {
    fn lease_version(
        &self,
        identity: &api::local_state::DomainIdentity,
    ) -> Result<api::local_state::StoreVersion, api::local_state::ErrorCode> {
        api::local_state::LocalStatePort::lease(self, identity).map(|lease| lease.version())
    }
}

static CALLER_STATE: std::sync::LazyLock<crate::local_state::SlotState> =
    std::sync::LazyLock::new(|| {
        crate::local_state::SlotState::new(
            Arc::new(CallerNoStore),
            api::local_state::AccountKey::try_new("local-289", "bank_callers").unwrap(),
            api::local_state::ContentScope::try_new(
                api::selected::ClientRevision::R289,
                "test-content",
            )
            .unwrap(),
        )
    });

fn prepare_kitchen_journal() {
    static PREPARED: std::sync::LazyLock<()> = std::sync::LazyLock::new(|| {
        let _ = FamilyPreparation::run(|worker| {
            let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
            data.prepare_quests(worker).unwrap();
            data.prepare_prereq_steps(worker).unwrap()
        })
        .unwrap()
        .join()
        .unwrap();
    });
    let _ = &*PREPARED;
}

fn kitchen_journal_gate() -> api::prereq_steps::ActionableGate {
    use api::prereq_steps::{CrossingId, CrossingKind};
    use api::selected::{FactKey, InclusiveRange, QuestGate, StageWindow};
    api::prereq_steps::ActionableGate {
        gate: QuestGate::Window(StageWindow {
            quest: FactKey::new("blackarmgang"),
            signal: FactKey::new("phoenixgang"),
            values: InclusiveRange {
                min: Some(10),
                max: Some(10),
            },
        }),
        truth: api::selected::Truth::Unknown,
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

fn with_journal_tick<R>(
    snapshot: &GameSnapshot,
    ledger: &mut CallerLedger,
    tick: u64,
    f: impl FnOnce(&mut crate::native::NativeTick<'_>) -> R,
) -> R {
    crate::quester::families::tests::with_tick_scene_reach(snapshot, ledger, tick, |native| {
        native.state = Some(&CALLER_STATE);
        native.cx.active_now = std::time::Duration::ZERO;
        f(native)
    })
}

fn caller_bank_pick(
    ledger: &CallerLedger,
) -> Option<(
    crate::native_bank::BankPickRequest,
    crate::native::ledger::HostAuthority,
)> {
    ledger.as_ref()?.outbox.iter().rev().find_map(|action| {
        match &action.effect {
            crate::native::HostEffect::BankPick(request) => {
                Some((request.clone(), action.authority()))
            }
            _ => None,
        }
    })
}

fn caller_reissued_bank_pick(
    ledger: &CallerLedger,
    original_id: u64,
) -> Option<(
    crate::native_bank::BankPickRequest,
    crate::native::ledger::HostAuthority,
)> {
    ledger.as_ref()?.outbox.iter().rev().find_map(|action| {
        if !action.live() || action.request_id.get() == original_id {
            return None;
        }
        match &action.effect {
            crate::native::HostEffect::BankPick(request) => {
                Some((request.clone(), action.authority()))
            }
            _ => None,
        }
    })
}

fn publish_caller_bank_pick(
    ledger: &mut CallerLedger,
    authority: &crate::native::ledger::HostAuthority,
    tick: u64,
    outcome: crate::native_bank::BankPickOutcome,
) {
    ledger
        .as_mut()
        .expect("bank pick request created ledger")
        .complete_bank_pick(
            authority,
            crate::native_bank::BankPickReceipt {
                request_id: authority.request_id().get(),
                evidence: api::quest_progress::EvidenceStamp {
                    run: authority.run(),
                    tick,
                    sequence: tick,
                },
                outcome,
            },
        );
}

fn assert_bank_pick_reissued_unchanged(
    original: &crate::native_bank::BankPickRequest,
    reissued: &crate::native_bank::BankPickRequest,
) {
    assert!(Arc::ptr_eq(&original.facts, &reissued.facts));
    assert_eq!(original.facts.as_ref(), reissued.facts.as_ref());
    assert_eq!(original.from, reissued.from);
    assert_eq!(original.preferences, reissued.preferences);
    assert_eq!(original.options, reissued.options);
    assert_eq!(original.eligible, reissued.eligible);
    assert_eq!(original.explicit_bank, reissued.explicit_bank);
    assert_eq!(original.allow_air_fallback, reissued.allow_air_fallback);
}

fn accept_caller_interaction(ledger: &mut CallerLedger, tick: u64) {
    let authority = ledger
        .as_ref()
        .expect("interaction created ledger")
        .outbox
        .last()
        .expect("interaction is queued")
        .authority();
    ledger
        .as_mut()
        .expect("interaction created ledger")
        .complete_interaction(
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
}

fn assert_no_gatherer_bank_work(ledger: &CallerLedger, original_id: u64) {
    for action in &ledger.as_ref().expect("bank pick created ledger").outbox {
        match &action.effect {
            crate::native::HostEffect::BankPick(_) => {
                assert_eq!(action.request_id.get(), original_id, "no early bank reissue");
            }
            crate::native::HostEffect::Walk(_) => panic!("bank walk began before selection"),
            crate::native::HostEffect::Interaction(crate::interact::InteractReq::OpenStand {
                ..
            }) => panic!("bank access began before selection"),
            _ => {}
        }
    }
}

fn seed_kitchen_journal_page(snapshot: &mut GameSnapshot) {
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
}

fn assert_original_allow_gatherer(gatherer: &Gatherer, allow: crate::native::PrereqAllow) {
    match &gatherer.active {
        Active::Select(select) => assert_eq!(select.original_allow(), Some(allow)),
        _ => panic!("gatherer no longer owns the bank selection"),
    }
}

#[test]
fn gatherer_reissues_original_bank_pick_only_after_real_journal_close() {
    prepare_kitchen_journal();
    let (mut gatherer, mut snapshot) = bank_fixture(Some("Varrock East"));
    snapshot.seed_quest_statuses(
        vec![api::snapshot::QuestStatusView {
            name: "Shield of Arrav".into(),
            component_id: 42,
            colour: 0x00f800,
        }],
        true,
    );

    let mut ledger = None;
    gatherer.trip = TripStep::Select;
    with_journal_tick(&snapshot, &mut ledger, 1, |tick| gatherer.begin_trip(tick));
    with_journal_tick(&snapshot, &mut ledger, 2, |tick| gatherer.poll_active(tick));
    let (original_request, original_authority) =
        caller_bank_pick(&ledger).expect("gatherer submits its bank pick");
    let original_id = original_authority.request_id().get();
    assert_eq!(
        original_request.from,
        WorldTile {
            x: 3200,
            z: 3200,
            level: 0
        }
    );
    assert_eq!(
        original_request.preferences,
        api::named_banks::BankPreferences::default()
    );
    assert_eq!(original_request.options, WalkOptions::default());
    assert_eq!(original_request.explicit_bank, Some(0));
    assert!(!original_request.allow_air_fallback);

    let gate = kitchen_journal_gate();
    let allow = crate::native::PrereqAllow {
        dialogue: true,
        quest_start: false,
    };
    publish_caller_bank_pick(
        &mut ledger,
        &original_authority,
        3,
        crate::native_bank::BankPickOutcome::NeedsStep {
            gates: Arc::from([gate.clone()]),
            allow,
        },
    );
    with_journal_tick(&snapshot, &mut ledger, 3, |tick| gatherer.poll_active(tick));
    assert!(matches!(gatherer.active, Active::Select(_)));
    assert_eq!(gatherer.trip, TripStep::Select);
    assert!(gatherer.scratch.selected_bank.is_none());
    assert_original_allow_gatherer(&gatherer, allow);
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        crate::native::HostEffect::Interaction(crate::interact::InteractReq::IfButton {
            component_id: 42
        })
    ));
    assert_no_gatherer_bank_work(&ledger, original_id);

    accept_caller_interaction(&mut ledger, 3);
    seed_kitchen_journal_page(&mut snapshot);
    with_journal_tick(&snapshot, &mut ledger, 4, |tick| gatherer.poll_active(tick));
    assert!(matches!(
        &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        crate::native::HostEffect::Interaction(crate::interact::InteractReq::CloseModal)
    ));
    assert!(gatherer.scratch.selected_bank.is_none());
    assert_no_gatherer_bank_work(&ledger, original_id);

    accept_caller_interaction(&mut ledger, 4);
    snapshot.seed_main_modal(-1, vec![]);
    with_journal_tick(&snapshot, &mut ledger, 5, |tick| gatherer.poll_active(tick));
    assert_eq!(
        CALLER_STATE
            .prereq_snapshot()
            .observe(
                &gate.gate,
                api::quest_progress::EvidenceStamp {
                    run: original_authority.run(),
                    tick: 4,
                    sequence: 4,
                },
            )
            .map(|fact| fact.truth),
        Some(api::selected::Truth::True),
        "the journal fact is available only after the modal close was observed"
    );
    assert!(gatherer.scratch.selected_bank.is_none());
    assert_no_gatherer_bank_work(&ledger, original_id);

    let mut reissued = caller_reissued_bank_pick(&ledger, original_id);
    for tick in 6..=9 {
        if reissued.is_some() {
            break;
        }
        with_journal_tick(&snapshot, &mut ledger, tick, |native| {
            gatherer.poll_active(native)
        });
        reissued = caller_reissued_bank_pick(&ledger, original_id);
    }
    let (reissued_request, reissued_authority) =
        reissued.expect("a fresh request follows acquired journal proof and observed close");
    assert_ne!(reissued_authority.request_id(), original_authority.request_id());
    assert_bank_pick_reissued_unchanged(&original_request, &reissued_request);
    assert_original_allow_gatherer(&gatherer, allow);
    assert!(!original_authority.live());

    let bank = gatherer.prepared.banks.banks()[0].to_owned();
    let bank_tile = bank.tile;
    publish_caller_bank_pick(
        &mut ledger,
        &reissued_authority,
        10,
        crate::native_bank::BankPickOutcome::Selected(crate::native_bank::SelectedBank {
            bank_index: 0,
            access_tile: bank_tile,
            kind: PickKind::Reachable,
            access: Some(Arc::new(crate::bank::BankStandAccess {
                bank,
                stand_tile: bank_tile,
                kind: crate::bank::AccessKind::Booth,
                stand_op: 1,
                name: None,
                choose: None,
            })),
        }),
    );
    with_journal_tick(&snapshot, &mut ledger, 11, |tick| gatherer.poll_active(tick));
    assert!(gatherer.failure.is_none());
    assert_eq!(gatherer.trip, TripStep::Access);
    assert_eq!(
        gatherer
            .scratch
            .selected_bank
            .as_ref()
            .map(|selected| selected.access_tile),
        Some(bank_tile)
    );

    snapshot.seed_locs(vec![api::snapshot::LocView {
        id: 2213,
        name: Some("Bank booth".into()),
        actions: vec![Some("Use-quickly".into())],
        tile: bank_tile,
        distance: 0,
        typecode: 0,
        info: 0,
        description: None,
        layer: api::snapshot::LocLayer::GroundDecoration,
        shape: 0,
        angle: 0,
        width: 1,
        length: 1,
        footprint_width: 1,
        footprint_length: 1,
        block_walk: false,
        block_range: false,
        active: true,
        animation: -1,
        map_function: -1,
        map_scene: -1,
        force_approach: 0,
    }]);
    with_journal_tick(&snapshot, &mut ledger, 12, |tick| gatherer.begin_trip(tick));
    with_journal_tick(&snapshot, &mut ledger, 13, |tick| gatherer.poll_active(tick));
    let walk_target = ledger
        .as_ref()
        .unwrap()
        .outbox
        .iter()
        .find_map(|action| match &action.effect {
            crate::native::HostEffect::Walk(request) => Some(request.target),
            _ => None,
        })
        .expect("selected bank continues through the gatherer's access walk");
    assert_eq!(walk_target, bank_tile);

    let mut player = snapshot.local_player().unwrap().clone();
    player.player.network = bank_tile;
    player.player.actor.tile = bank_tile;
    snapshot.seed_local_player(player);
    crate::quester::families::tests::post_walk_receipt(
        &mut ledger,
        14,
        WalkEnd::Arrived,
        None,
    );
    with_journal_tick(&snapshot, &mut ledger, 14, |tick| gatherer.poll_active(tick));
    assert_eq!(gatherer.trip, TripStep::Open);
    with_journal_tick(&snapshot, &mut ledger, 15, |tick| gatherer.begin_trip(tick));
    with_journal_tick(&snapshot, &mut ledger, 16, |tick| gatherer.poll_active(tick));
    assert!(ledger.as_ref().unwrap().outbox.iter().any(|action| {
        matches!(
            &action.effect,
            crate::native::HostEffect::Interaction(
                crate::interact::InteractReq::OpenStand { .. }
            )
        )
    }));
}

#[test]
fn gatherer_preserves_detailed_bank_no_candidate_without_wrapping_it() {
    let (mut gatherer, snapshot) = bank_fixture(Some("Varrock East"));
    let mut ledger = None;
    gatherer.trip = TripStep::Select;
    with_tick(&snapshot, &mut ledger, 1, |tick| gatherer.begin_trip(tick));
    with_tick(&snapshot, &mut ledger, 2, |tick| gatherer.poll_active(tick));
    let (_, authority) = caller_bank_pick(&ledger).expect("bank pick was emitted");
    let detail: Arc<str> = Arc::from("no routable bank candidate after NoPath");
    publish_caller_bank_pick(
        &mut ledger,
        &authority,
        3,
        crate::native_bank::BankPickOutcome::NoCandidate {
            detail: Arc::clone(&detail),
        },
    );
    with_tick(&snapshot, &mut ledger, 3, |tick| gatherer.poll_active(tick));
    let failure = gatherer.failure.expect("NoCandidate is terminal for the bank trip");
    assert_eq!(failure.code.as_ref(), "bank-unavailable");
    assert_eq!(failure.message.as_ref(), detail.as_ref());
    assert!(!failure.message.contains("bank selection failed"));
    assert!(!failure.message.contains("Failed("));
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
            PrereqWalk::begin(
                WalkRequest {
                    target: target.tile,
                    candidate_goals: None,
                    loc_id: match target.entity {
                        EntityId::Loc(id) => Some(id),
                        _ => None,
                    },
                    radius: 1,
                    arrival: nav_planner::contract::ArrivalKind::Reach,
                    options: crate::native::WalkOptions::default(),
                    required_after: tick.cx.evidence(),
                    evidence: None,
                    cross: Vec::new().into_boxed_slice(),
                    avoid: Box::new([]),
                    protect: false,
                    food_guard: false,
                    cure_guard: false,
                    allow: Default::default(),
                },
                tick.actions,
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
                prereq_allow: None,
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
                prereq_allow: None,
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
                prereq_allow: None,
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
    assert_eq!(gatherer.scratch.return_stall_recovery.actions(), 0);
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
                crate::native::HostEffect::Interaction(crate::interact::InteractReq::InvButton {
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
        crate::native::HostEffect::Interaction(crate::interact::InteractReq::SetNoteMode {
            on: false
        })
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
        crate::native::HostEffect::Interaction(crate::interact::InteractReq::WithdrawX {
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

#[test]
fn power_gatherer_rejects_bank_edit_at_lower_mine() {
    let (power_config, bank_config) = FamilyPreparation::run(|families| {
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let mut cx = crate::native::PrepareContext {
            zones: None,
            pin: selected.selected_pin().unwrap(),
            selected,
            banks: Arc::default(),
            families,
        };
        let mut power_settings = crate::native::SettingsBag::new();
        for (key, value) in [
            ("skill", serde_json::json!("Mining")),
            ("miningResources", serde_json::json!(["copper"])),
            ("location", serde_json::json!("Site")),
            (
                "site",
                serde_json::json!("mining.desert_mining_camp.underground"),
            ),
            ("radius", serde_json::json!(20)),
            ("disposition", serde_json::json!("Power")),
        ] {
            power_settings.insert(key.into(), value);
        }
        let power_config =
            super::super::card::prepare(&mut cx, 1, Arc::new(power_settings)).unwrap();
        let mut bank_settings = crate::native::SettingsBag::new();
        for (key, value) in [
            ("skill", serde_json::json!("Mining")),
            ("miningResources", serde_json::json!(["copper"])),
            ("location", serde_json::json!("Site")),
            (
                "site",
                serde_json::json!("mining.desert_mining_camp.underground"),
            ),
            ("radius", serde_json::json!(20)),
            ("disposition", serde_json::json!("Bank")),
        ] {
            bank_settings.insert(key.into(), value);
        }
        let bank_config = super::super::card::prepare(&mut cx, 2, Arc::new(bank_settings)).unwrap();
        (power_config, bank_config)
    })
    .unwrap()
    .join()
    .unwrap();
    let bank_start_error = match super::super::card::create(
        RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        Arc::clone(&bank_config),
        &mut crate::native::RetainedMemory::default(),
    ) {
        Err(crate::native::StartError::Config(error)) => error,
        Err(error) => panic!("unexpected Bank-mode Start error: {error}"),
        Ok(_) => panic!("Bank-mode Start unexpectedly accepted the lower mine"),
    };
    let mut gatherer = Gatherer::new(
        RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        Arc::clone(&power_config),
        Arc::clone(power_config.get::<Arc<Prepared>>().unwrap()),
        GatherRetained::default(),
    );

    let edit_error = match gatherer.configure(Arc::clone(&bank_config)) {
        Err(error) => error,
        Ok(_) => panic!("the running Power Gatherer accepted Bank mode at the lower mine"),
    };

    assert_eq!(edit_error, bank_start_error);
    assert_eq!(gatherer.settings().disposition, "Power");
    assert!(gatherer.pending.is_none());
}

fn antipoison_dose() -> ItemView {
    let dose_id = crate::combat::guard::shared_tables()
        .unwrap()
        .potion(crate::combat::tables::PotionKind::Antipoison)
        .unwrap()
        .doses[0]
        .unwrap()
        .id;
    ItemView {
        def: def(dose_id, "Antipoison"),
        container: ItemContainer::Inventory,
        action_family: ItemActionFamily::Held,
        slot: 0,
        count: 1,
        actions: Vec::new(),
        component_id: 0,
    }
}

fn held_lobster(slot: i32) -> ItemView {
    let lobster_id = api::game_data::for_revision(api::selected::ClientRevision::R289)
        .unwrap()
        .item_by_alias("lobster")
        .unwrap()
        .id;
    ItemView {
        def: def(lobster_id, "Lobster"),
        container: ItemContainer::Inventory,
        action_family: ItemActionFamily::Held,
        slot,
        count: 1,
        actions: vec![Some("Eat".into())],
        component_id: 0,
    }
}

fn cure_condition(
    tick: u32,
    poison: api::condition::PoisonState,
    hp: u8,
) -> api::condition::BotCondition {
    api::condition::BotCondition {
        tick,
        session: api::condition::SessionMark::Live,
        hp,
        hp_max: 99,
        hp_known: true,
        poison,
        poison_window: None,
        food: api::condition::FoodSummary::default(),
        cures: api::condition::CureDoses::default(),
    }
}

const ONSET_AT_LINE: api::condition::PoisonState = api::condition::PoisonState::Onset {
    since: 20,
    q: Some(11),
    hit_since_line: false,
};

/// The last interaction the runner emitted, as (`Held` action, request id).
fn last_held(ledger: &Option<Box<crate::native::ledger::Ledger>>) -> Option<(String, u64)> {
    let action = ledger.as_ref()?.outbox.last()?;
    match &action.effect {
        crate::native::HostEffect::Interaction(crate::interact::InteractReq::Held {
            action: name,
            ..
        }) => Some((name.clone(), action.authority().request_id().get())),
        _ => None,
    }
}

/// Start gathering at frame 20, then poison the bot at the line so the
/// runner cancels the gather step and drinks in tick 2.
fn poisoned_while_gathering(
    inventory: Vec<ItemView>,
) -> (
    Gatherer,
    api::snapshot::GameSnapshot,
    Option<Box<crate::native::ledger::Ledger>>,
) {
    use api::condition::PoisonState;

    let (mut gatherer, mut snapshot) = fixture();
    snapshot.seed_inventory(inventory, 28);
    snapshot.seed_tick(20);
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.cx.condition = Some(cure_condition(20, PoisonState::Clear, 60));
        tick.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
        gatherer.tick(tick).unwrap();
    });
    assert!(matches!(gatherer.active, Active::Gather(_)));
    with_tick(&snapshot, &mut ledger, 2, |tick| {
        tick.cx.condition = Some(cure_condition(20, ONSET_AT_LINE, 5));
        tick.cx.poison_unknown_pulse = 11;
        tick.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
        gatherer.tick(tick).unwrap();
    });
    assert!(matches!(gatherer.active, Active::Tend(_)));
    assert_eq!(
        gatherer.survival.report().decision,
        crate::survival::Decision::Drinking
    );
    assert_eq!(
        last_held(&ledger).map(|(action, _)| action).as_deref(),
        Some("Drink")
    );
    (gatherer, snapshot, ledger)
}

/// Tick a poisoned or cured Gatherer at `observed` with the cure's condition.
fn tick_with(
    gatherer: &mut Gatherer,
    snapshot: &GameSnapshot,
    ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
    observed: u64,
    poison: api::condition::PoisonState,
) -> ScriptFlow {
    with_tick(snapshot, ledger, observed, |tick| {
        tick.cx.condition = Some(cure_condition(u32::try_from(observed).unwrap(), poison, 5));
        tick.cx.poison_unknown_pulse = 11;
        tick.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
        gatherer.tick(tick).unwrap()
    })
}

const CURED: api::condition::PoisonState = api::condition::PoisonState::Immune { until_lower: 80 };

#[test]
fn gatherer_interrupts_gather_step_for_drink_at_line() {
    let (mut gatherer, mut snapshot, mut ledger) =
        poisoned_while_gathering(vec![antipoison_dose()]);
    let drink = last_held(&ledger);
    snapshot.seed_inventory(vec![], 28);
    // The drink sent at observed tick 2 delays the player through the next
    // two server ticks (`consume_effects.rs2:124-126`, `PlayerOps.ts:376-378`),
    // so the runner sends no gather click the engine would drop, and resumes
    // in tick 5 when the drink lock ends.
    for observed in 3..=4 {
        tick_with(&mut gatherer, &snapshot, &mut ledger, observed, CURED);
        assert!(matches!(gatherer.active, Active::None), "tick {observed}");
        assert_eq!(last_held(&ledger), drink, "tick {observed}");
    }
    tick_with(&mut gatherer, &snapshot, &mut ledger, 5, CURED);
    assert!(
        matches!(gatherer.active, Active::Gather(_)),
        "gathering resumes once the cure settles and the drink lock ends"
    );
    assert_eq!(
        gatherer.survival.report().decision,
        crate::survival::Decision::None
    );
    assert!(!gatherer.survival_deferred);
    assert!(!gatherer.needs_validate);
}

/// A cure and a bite are both due (design A6.4): the drink goes first, and the
/// bite waits for the drink's lock instead of landing in the drink's
/// `p_delay(1)`, where the engine drops it (`OpHeldHandler.ts:16-19`).
#[test]
fn gatherer_boundary_drinks_before_bite() {
    let (mut gatherer, mut snapshot, mut ledger) =
        poisoned_while_gathering(vec![antipoison_dose(), held_lobster(1)]);
    let drink = last_held(&ledger);
    snapshot.seed_inventory(vec![held_lobster(1)], 28);
    // The bite reads the frame's Hitpoints row: 5 of 99, below the Gatherer's
    // default eat floor of half its base.
    let mut stats = snapshot.stats().to_vec();
    stats.push(StatView {
        index: 3,
        name: "hitpoints".into(),
        effective: 5,
        base: 99,
        xp: 0,
        used: true,
    });
    snapshot.seed_stats(stats);
    for observed in 3..=4 {
        tick_with(&mut gatherer, &snapshot, &mut ledger, observed, CURED);
        assert_eq!(
            last_held(&ledger),
            drink,
            "no bite inside the drink lock at tick {observed}"
        );
        assert!(!matches!(gatherer.active, Active::Gather(_)));
        assert_eq!(
            gatherer.survival.report().decision,
            crate::survival::Decision::Drinking
        );
    }
    tick_with(&mut gatherer, &snapshot, &mut ledger, 5, CURED);
    assert_eq!(
        last_held(&ledger).map(|(action, _)| action).as_deref(),
        Some("Eat"),
        "the bite follows when the drink lock ends"
    );
    assert!(matches!(gatherer.active, Active::Tend(_)));
}

/// The engine drops a held click while the player is delayed, for example in
/// the tick after a mining success's `p_delay(0)` (`mining.rs2:139`,
/// `OpHeldHandler.ts:16-19`). The drink then shows no dose drop by its
/// three-tick bound. The Gatherer clicks again in that tick and, once cured,
/// resumes gathering in the same run: an unsettled survival click is a
/// deferral, not `oneop-failed`.
#[test]
fn gatherer_resends_a_dropped_drink_and_resumes() {
    let (mut gatherer, mut snapshot, mut ledger) =
        poisoned_while_gathering(vec![antipoison_dose()]);
    let first = last_held(&ledger).expect("first drink");
    // The host sent the click; the delayed engine dropped it, so the dose
    // stays held and the line stays an onset until the drink's bound.
    for observed in 3..=5 {
        let flow = tick_with(
            &mut gatherer,
            &snapshot,
            &mut ledger,
            observed,
            ONSET_AT_LINE,
        );
        assert!(matches!(flow, ScriptFlow::Continue), "tick {observed}");
        assert!(gatherer.failure.is_none(), "tick {observed}");
    }
    let second = last_held(&ledger).expect("second drink");
    assert_eq!(second.0, "Drink");
    assert_ne!(second.1, first.1, "the drink is clicked again in tick 5");
    assert!(matches!(gatherer.active, Active::Tend(_)));

    // The second drink lands: dose drop and immunity at the next tick. The
    // run then gathers again once the new drink lock ends (tick 8).
    snapshot.seed_inventory(vec![], 28);
    for observed in 6..=8 {
        let flow = tick_with(&mut gatherer, &snapshot, &mut ledger, observed, CURED);
        assert!(matches!(flow, ScriptFlow::Continue), "tick {observed}");
    }
    assert!(gatherer.failure.is_none());
    assert!(
        matches!(gatherer.active, Active::Gather(_)),
        "the interrupted gathering resumes after the resent drink"
    );
}

/// Seed the frame's Hitpoints row at 5 of 99, below the Gatherer's default
/// eat floor of half its base.
fn seed_low_hp(snapshot: &mut GameSnapshot) {
    let mut stats = snapshot.stats().to_vec();
    stats.retain(|stat| stat.index != 3);
    stats.push(StatView {
        index: 3,
        name: "hitpoints".into(),
        effective: 5,
        base: 99,
        xp: 0,
        used: true,
    });
    snapshot.seed_stats(stats);
}

/// Each distinct held click the runner sent, in order, as (action, request id).
fn note_held(
    ledger: &Option<Box<crate::native::ledger::Ledger>>,
    seen: &mut Vec<(String, u64)>,
) -> usize {
    if let Some(held) = last_held(ledger) {
        if !seen.iter().any(|(_, id)| *id == held.1) {
            seen.push(held);
        }
    }
    seen.len()
}

/// The drink's dose drop arrives one tick later than usual (the engine ran
/// the click in the second server tick after it was sent). The drink's
/// `p_delay(1)` (`consume_effects.rs2:124-126`) starts in the tick that ran
/// it (`PlayerOps.ts:376-379`), so the bite waits for the lock counted from
/// the dose drop, not from the send: no bite in tick 5, the bite in tick 6.
#[test]
fn gatherer_late_drink_effect_moves_the_drink_lock() {
    let (mut gatherer, mut snapshot, mut ledger) =
        poisoned_while_gathering(vec![antipoison_dose(), held_lobster(1)]);
    let drink = last_held(&ledger);
    seed_low_hp(&mut snapshot);
    tick_with(&mut gatherer, &snapshot, &mut ledger, 3, ONSET_AT_LINE);
    assert_eq!(last_held(&ledger), drink, "tick 3: the dose is still held");
    snapshot.seed_inventory(vec![held_lobster(1)], 28);
    for observed in 4..=5 {
        tick_with(&mut gatherer, &snapshot, &mut ledger, observed, CURED);
        assert_eq!(
            last_held(&ledger),
            drink,
            "no bite inside the lock of a drink run in tick 4: tick {observed}"
        );
        assert!(!matches!(gatherer.active, Active::Gather(_)));
    }
    tick_with(&mut gatherer, &snapshot, &mut ledger, 6, CURED);
    assert_eq!(
        last_held(&ledger).map(|(action, _)| action).as_deref(),
        Some("Eat"),
        "the bite follows the lock of the late drink"
    );
}

/// The dose drop arrives at the drink op's three-tick bound (tick 5). The
/// op settles on it, and the work waits for that drink's lock (ticks 5 and 6)
/// instead of sending a gather click the delayed engine drops
/// (`OpHeldHandler.ts:16-19`).
#[test]
fn gatherer_drink_effect_at_its_bound_holds_work_through_the_lock() {
    let (mut gatherer, mut snapshot, mut ledger) =
        poisoned_while_gathering(vec![antipoison_dose()]);
    let drink = last_held(&ledger);
    for observed in 3..=4 {
        tick_with(
            &mut gatherer,
            &snapshot,
            &mut ledger,
            observed,
            ONSET_AT_LINE,
        );
        assert!(
            matches!(gatherer.active, Active::Tend(_)),
            "tick {observed}"
        );
    }
    snapshot.seed_inventory(vec![], 28);
    for observed in 5..=6 {
        tick_with(&mut gatherer, &snapshot, &mut ledger, observed, CURED);
        assert!(
            !matches!(gatherer.active, Active::Gather(_)),
            "no work inside the lock of a drink run in tick 5: tick {observed}"
        );
        assert_eq!(last_held(&ledger), drink, "tick {observed}");
    }
    tick_with(&mut gatherer, &snapshot, &mut ledger, 7, CURED);
    assert!(
        matches!(gatherer.active, Active::Gather(_)),
        "gathering resumes when the late drink's lock ends"
    );
    assert!(gatherer.failure.is_none());
}

/// Two bites in a row, and the first bite's count drop arrives one tick later
/// than usual. The next-food clock starts in the server tick that ran the
/// first bite (`consume.rs2:122-127`: `%eat_delay = map_clock + $eat_delay`),
/// so the second bite waits for the clock counted from the count drop. A bite
/// sent inside it is ignored by the content without a reply.
#[test]
fn gatherer_late_bite_effect_moves_the_next_food_clock() {
    let (mut gatherer, mut snapshot) = fixture();
    snapshot.seed_inventory(vec![held_lobster(0), held_lobster(1)], 28);
    snapshot.seed_tick(20);
    let mut ledger = None;
    let mut held = Vec::new();
    tick_with_hp(&mut gatherer, &snapshot, &mut ledger, 1, 60);
    assert!(matches!(gatherer.active, Active::Gather(_)));
    seed_low_hp(&mut snapshot);
    tick_with_hp(&mut gatherer, &snapshot, &mut ledger, 2, 5);
    assert_eq!(note_held(&ledger, &mut held), 1);
    assert_eq!(held[0].0, "Eat", "the first bite is sent in tick 2");
    tick_with_hp(&mut gatherer, &snapshot, &mut ledger, 3, 5);
    assert_eq!(
        note_held(&ledger, &mut held),
        1,
        "tick 3: no count drop yet"
    );
    snapshot.seed_inventory(vec![held_lobster(1)], 28);
    let delay = crate::combat::guard::shared_tables()
        .unwrap()
        .food(held_lobster(1).def.id)
        .unwrap()
        .eat_delay_arg
        .expect("lobster uses the shared next-food clock");
    // The first bite ran in tick 4: the content accepts the next bite in
    // tick `4 + delay + 1`, so it is sent after frame `4 + delay`.
    let ready = 4 + u64::try_from(delay).unwrap();
    for observed in 4..ready {
        tick_with_hp(&mut gatherer, &snapshot, &mut ledger, observed, 5);
        assert_eq!(
            note_held(&ledger, &mut held),
            1,
            "no second bite inside the next-food clock: tick {observed}"
        );
        assert!(!matches!(gatherer.active, Active::Gather(_)));
    }
    tick_with_hp(&mut gatherer, &snapshot, &mut ledger, ready, 5);
    assert_eq!(note_held(&ledger, &mut held), 2, "tick {ready}");
    assert_eq!(held[1].0, "Eat", "the second bite follows the clock");
}

/// Natural regeneration raises Hitpoints in the frame before the first bite's
/// count drop arrives (`stat.rs2:58-60` restores one point on its own timer).
/// That rise is not the bite: the next-food clock starts in the server tick
/// that ran the click (`consume.rs2:122-127`), which only the count drop
/// shows. The clock must be counted from the count drop's tick, so the second
/// bite waits exactly as in the late-bite case without the rise.
#[test]
fn gatherer_natural_regeneration_before_a_late_bite_does_not_start_the_food_clock() {
    let (mut gatherer, mut snapshot) = fixture();
    snapshot.seed_inventory(vec![held_lobster(0), held_lobster(1)], 28);
    snapshot.seed_tick(20);
    let mut ledger = None;
    let mut held = Vec::new();
    tick_with_hp(&mut gatherer, &snapshot, &mut ledger, 1, 60);
    assert!(matches!(gatherer.active, Active::Gather(_)));
    seed_low_hp(&mut snapshot);
    tick_with_hp(&mut gatherer, &snapshot, &mut ledger, 2, 5);
    assert_eq!(note_held(&ledger, &mut held), 1);
    assert_eq!(held[0].0, "Eat", "the first bite is sent in tick 2");
    // Tick 3: Hitpoints 5 -> 6 from natural regeneration; the count is intact.
    let mut stats = snapshot.stats().to_vec();
    for stat in &mut stats {
        if stat.index == 3 {
            stat.effective = 6;
        }
    }
    snapshot.seed_stats(stats);
    tick_with_hp(&mut gatherer, &snapshot, &mut ledger, 3, 6);
    assert_eq!(
        note_held(&ledger, &mut held),
        1,
        "tick 3: a hitpoints rise alone is not a bite"
    );
    snapshot.seed_inventory(vec![held_lobster(1)], 28);
    let delay = crate::combat::guard::shared_tables()
        .unwrap()
        .food(held_lobster(1).def.id)
        .unwrap()
        .eat_delay_arg
        .expect("lobster uses the shared next-food clock");
    // The first bite ran in tick 4 (the count drop): the content accepts the
    // next bite in tick `4 + delay + 1`, so it is sent after frame `4 + delay`.
    let ready = 4 + u64::try_from(delay).unwrap();
    for observed in 4..ready {
        tick_with_hp(&mut gatherer, &snapshot, &mut ledger, observed, 6);
        assert_eq!(
            note_held(&ledger, &mut held),
            1,
            "no second bite inside the next-food clock: tick {observed}"
        );
        assert!(!matches!(gatherer.active, Active::Gather(_)));
    }
    tick_with_hp(&mut gatherer, &snapshot, &mut ledger, ready, 6);
    assert_eq!(note_held(&ledger, &mut held), 2, "tick {ready}");
    assert_eq!(held[1].0, "Eat", "the second bite follows the clock");
}

/// The same rise settles nothing: the bite's op keeps waiting for the count
/// drop (`OneOp` settles a bite on the count alone).
#[test]
fn gatherer_natural_regeneration_does_not_settle_a_bite() {
    let (mut gatherer, mut snapshot) = fixture();
    snapshot.seed_inventory(vec![held_lobster(0), held_lobster(1)], 28);
    snapshot.seed_tick(20);
    let mut ledger = None;
    tick_with_hp(&mut gatherer, &snapshot, &mut ledger, 1, 60);
    seed_low_hp(&mut snapshot);
    tick_with_hp(&mut gatherer, &snapshot, &mut ledger, 2, 5);
    assert!(
        matches!(gatherer.active, Active::Tend(_)),
        "the bite is out"
    );
    let mut stats = snapshot.stats().to_vec();
    for stat in &mut stats {
        if stat.index == 3 {
            stat.effective = 6;
        }
    }
    snapshot.seed_stats(stats);
    tick_with_hp(&mut gatherer, &snapshot, &mut ledger, 3, 6);
    assert!(
        matches!(gatherer.active, Active::Tend(_)),
        "a hitpoints rise alone does not settle the bite"
    );
    snapshot.seed_inventory(vec![held_lobster(1)], 28);
    tick_with_hp(&mut gatherer, &snapshot, &mut ledger, 4, 6);
    assert!(
        !matches!(gatherer.active, Active::Tend(_)),
        "the count drop settles the bite"
    );
}

/// Tick an unpoisoned Gatherer at `observed` with the condition's hitpoints.
fn tick_with_hp(
    gatherer: &mut Gatherer,
    snapshot: &GameSnapshot,
    ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
    observed: u64,
    hp: u8,
) -> ScriptFlow {
    with_tick(snapshot, ledger, observed, |tick| {
        tick.cx.condition = Some(cure_condition(
            u32::try_from(observed).unwrap(),
            api::condition::PoisonState::Clear,
            hp,
        ));
        tick.cx.cure_poison = crate::survival::CurePoison::WhenNeeded;
        gatherer.tick(tick).unwrap()
    })
}

#[test]
fn gatherer_walk_requests_food_guard() {
    let (mut gatherer, mut snapshot) = fixture();
    let here = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    snapshot.seed_local_player(crate::quester::families::tests::local_player(here));
    let target = WorldTile {
        x: 3210,
        z: 3200,
        level: 0,
    };
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 1, |tick| {
        assert!(gatherer.trip_walk(target, 0, ArrivalKind::Area, None, tick));
    });
    let Some(crate::native::HostEffect::Walk(request)) = ledger
        .as_ref()
        .and_then(|ledger| ledger.outbox.last())
        .map(|action| &action.effect)
    else {
        panic!("the trip must begin a walk");
    };
    assert!(request.food_guard);
    assert!(request.allow.food);
}
