use super::*;

fn npc_route(kind: TransportKind) -> Route {
    let edge = TransportEdge {
        kind,
        player_delta: None,
        ..cart_edge()
    };
    Route {
        dest: edge.to,
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
        ticks: 1.0,
    }
}

#[test]
fn moving_npc_reach_failure_reapproaches_and_arrives_for_all_npc_hops() {
    for kind in [
        TransportKind::Boat,
        TransportKind::Npc,
        TransportKind::Glider,
    ] {
        let mut c = scene_client();
        plant_driver_npc(&mut c, 7, 2, 1);
        let mut snap = snap_at(&mut c, 1, 1);
        let mut rec = FollowRec {
            route: Some((1, 1)),
            ..Default::default()
        };
        let mut traveller = Traveller::new();
        let route = npc_route(kind);
        let mut options = TravelOptions::default();
        assert!(traveller
            .follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none());
        assert_eq!(rec.npc_ops, 1);
        c.npc[0].as_mut().unwrap().entity.route_x[0] = 5;
        c.add_chat(0, "I can't reach that!", "");
        bump_rebuild(&mut c, &mut snap);
        assert!(
            traveller
                .follow(&mut rec, &snap, route.clone(), &mut options)
                .is_none(),
            "{kind:?}: a real reach failure must recover"
        );
        assert_eq!(rec.npc_ops, 1, "never talk from a stale stand");
        let &(x, z) = rec
            .walked
            .last()
            .expect("approach the moved network target");
        assert_eq!((x, z), (4, 1));
        plant_player(&mut c, x, z);
        rec.route = Some((x, z));
        bump_rebuild(&mut c, &mut snap);
        assert!(traveller
            .follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none());
        assert_eq!(rec.npc_ops, 2);
        // The old failure is behind the retry's watermark.
        bump_rebuild(&mut c, &mut snap);
        assert!(traveller
            .follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none());
        plant_player(&mut c, 100, 0);
        bump_rebuild(&mut c, &mut snap);
        assert!(matches!(
            traveller.follow(&mut rec, &snap, route, &mut options),
            Some(TravelOutcome::Arrived { .. })
        ));
    }
}

#[test]
fn npc_recovery_exhaustion_reports_three_attempts() {
    let mut c = scene_client();
    plant_driver_npc(&mut c, 7, 2, 1);
    let mut snap = snap_at(&mut c, 1, 1);
    let mut rec = FollowRec {
        route: Some((1, 1)),
        ..Default::default()
    };
    let mut traveller = Traveller::new();
    let route = npc_route(TransportKind::Boat);
    let mut options = TravelOptions::default();
    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    for attempt in 1..=3 {
        c.add_chat(0, "I can't reach that!", "");
        bump_rebuild(&mut c, &mut snap);
        let result = traveller.follow(&mut rec, &snap, route.clone(), &mut options);
        if attempt == 3 {
            match result {
                Some(TravelOutcome::Blocked { detail, .. }) => assert!(
                    detail.contains("3 attempts") && detail.contains("reach"),
                    "{detail}"
                ),
                other => panic!("expected honest retry exhaustion, got {other:?}"),
            }
        } else {
            assert!(result.is_none(), "attempt {attempt}: {result:?}");
            if let Some(&(x, z)) = rec.walked.last() {
                plant_player(&mut c, x, z);
                rec.route = Some((x, z));
            }
            bump_rebuild(&mut c, &mut snap);
            assert!(traveller
                .follow(&mut rec, &snap, route.clone(), &mut options)
                .is_none());
            assert_eq!(rec.npc_ops, attempt + 1);
        }
    }
    assert_eq!(rec.npc_ops, 3);
}

#[test]
fn npc_wall_invalid_adjacency_uses_a_reachable_operable_stand() {
    let mut c = scene_client();
    plant_driver_npc(&mut c, 7, 2, 2);
    c.collision[0].flags[1][2] |= client::dash3d::CollisionFlag::W_E;
    c.collision[0].flags[2][2] |= client::dash3d::CollisionFlag::W_W;
    let mut snap = snap_at(&mut c, 1, 2);
    let mut rec = FollowRec {
        route: Some((1, 2)),
        ..Default::default()
    };
    let mut traveller = Traveller::new();
    let route = npc_route(TransportKind::Npc);
    let mut options = TravelOptions::default();
    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.npc_ops, 0, "shared edge wall forbids talking");
    let &(x, z) = rec.walked.last().expect("route to another cardinal side");
    assert_ne!((x, z), (1, 2));
    assert_eq!((x - 2).abs() + (z - 2).abs(), 1);
    plant_player(&mut c, x, z);
    rec.route = Some((x, z));
    bump_rebuild(&mut c, &mut snap);
    assert!(traveller
        .follow(&mut rec, &snap, route, &mut options)
        .is_none());
    assert_eq!(rec.npc_ops, 1);
}

#[test]
fn npc_selection_skips_nearest_spawn_when_its_ring_is_unreachable() {
    let mut c = scene_client();
    plant_driver_npc(&mut c, 7, 4, 4);
    for x in 3..=5 {
        for z in 3..=5 {
            c.collision[0].flags[x][z] |= client::dash3d::CollisionFlag::SQ_BLOCKED;
        }
    }
    let mut other = ClientNpc::at(8, 4);
    other.r#type = Some(7);
    other.entity.x = 8 * 128 + 64;
    other.entity.z = 4 * 128 + 64;
    c.npc.push(Some(Box::new(other)));
    c.npc_ids.push(1);
    c.npc_count = 2;
    let snap = snap_at(&mut c, 2, 4);
    let mut rec = FollowRec {
        route: Some((2, 4)),
        ..Default::default()
    };
    let mut traveller = Traveller::new();
    let mut options = TravelOptions::default();
    assert!(traveller
        .follow(
            &mut rec,
            &snap,
            npc_route(TransportKind::Boat),
            &mut options
        )
        .is_none());
    let &(x, z) = rec
        .walked
        .last()
        .expect("reachable same-type instance selected");
    assert_eq!((x - 8).abs() + (z - 4).abs(), 1);
}

#[test]
fn npc_retargets_before_the_old_approach_has_settled() {
    let mut c = scene_client();
    plant_driver_npc(&mut c, 7, 6, 2);
    let mut snap = snap_at(&mut c, 0, 2);
    let mut rec = FollowRec {
        route: Some((0, 2)),
        ..Default::default()
    };
    let mut traveller = Traveller::new();
    let route = npc_route(TransportKind::Npc);
    let mut options = TravelOptions::default();
    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    c.npc[0].as_mut().unwrap().entity.route_z[0] = 6;
    bump_rebuild(&mut c, &mut snap);
    assert!(traveller
        .follow(&mut rec, &snap, route, &mut options)
        .is_none());
    assert_eq!(rec.npc_ops, 0);
    let &(x, z) = rec.walked.last().unwrap();
    assert_eq!(
        (x - 6).abs() + (z - 6).abs(),
        1,
        "retarget immediately, not after arriving at the old tile"
    );
}

#[test]
fn tracked_npc_vanished_expiry_names_the_missing_target() {
    let mut c = scene_client();
    plant_driver_npc(&mut c, 7, 6, 2);
    let mut snap = snap_at(&mut c, 0, 2);
    let mut rec = FollowRec {
        route: Some((0, 2)),
        ..Default::default()
    };
    let mut traveller = Traveller::new();
    let route = npc_route(TransportKind::Npc);
    let mut options = TravelOptions {
        budget_ticks_per_hop: 3,
        ..Default::default()
    };
    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    c.npc[0].as_mut().unwrap().r#type = Some(8);
    let mut outcome = None;
    for _ in 0..4 {
        bump_rebuild(&mut c, &mut snap);
        outcome = traveller.follow(&mut rec, &snap, route.clone(), &mut options);
        if outcome.is_some() {
            break;
        }
    }
    match outcome {
        Some(TravelOutcome::Blocked { detail, .. }) => assert!(
            detail.contains("tracked target missing") && detail.contains("7"),
            "{detail}"
        ),
        other => panic!("expected missing-target terminal detail, got {other:?}"),
    }
}

#[test]
fn npc_reach_chat_during_fare_dialog_never_recancels_the_fare() {
    let mut c = scene_client();
    plant_driver_npc(&mut c, 7, 2, 1);
    let mut snap = snap_at(&mut c, 1, 1);
    let mut rec = FollowRec {
        route: Some((1, 1)),
        ..Default::default()
    };
    let mut traveller = Traveller::new();
    let route = npc_route(TransportKind::Boat);
    let mut options = TravelOptions::default();
    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    plant_continue_dialog(&mut c);
    c.add_chat(0, "I can't reach that!", "");
    bump_rebuild(&mut c, &mut snap);
    assert!(traveller
        .follow(&mut rec, &snap, route, &mut options)
        .is_none());
    assert_eq!(rec.npc_ops, 1);
    assert!(rec.walked.is_empty(), "no approach click during fare");
    assert_eq!(rec.pause_buttons, 1);
}

#[test]
fn failed_npc_instance_is_replaced_by_a_reachable_same_type() {
    let mut c = scene_client();
    plant_driver_npc(&mut c, 7, 4, 4);
    let mut other = ClientNpc::at(7, 4);
    other.r#type = Some(7);
    other.entity.x = 7 * 128 + 64;
    other.entity.z = 4 * 128 + 64;
    c.npc.push(Some(Box::new(other)));
    c.npc_ids.push(1);
    c.npc_count = 2;
    let mut snap = snap_at(&mut c, 3, 4);
    let mut rec = FollowRec {
        route: Some((3, 4)),
        ..Default::default()
    };
    let targets = std::cell::RefCell::new(Vec::new());
    let mut options = TravelOptions {
        on_event: Some(Box::new(|event| {
            if let TravelEvent::TransportAttempt { target, .. } = event {
                targets.borrow_mut().push(target);
            }
        })),
        ..Default::default()
    };
    let mut traveller = Traveller::new();
    let route = npc_route(TransportKind::Boat);
    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    c.add_chat(0, "I can't reach that!", "");
    bump_rebuild(&mut c, &mut snap);
    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    let &(x, z) = rec.walked.last().expect("approach alternative instance");
    assert_eq!((x - 7).abs() + (z - 4).abs(), 1);
    plant_player(&mut c, x, z);
    rec.route = Some((x, z));
    bump_rebuild(&mut c, &mut snap);
    assert!(traveller
        .follow(&mut rec, &snap, route, &mut options)
        .is_none());
    assert_eq!(
        targets.borrow().as_slice(),
        &[
            WorldTile {
                x: 3204,
                z: 3204,
                level: 0
            },
            WorldTile {
                x: 3207,
                z: 3204,
                level: 0
            },
        ]
    );
}

#[test]
fn npc_retry_and_dialogue_share_the_original_leg_budget() {
    let mut c = scene_client();
    plant_driver_npc(&mut c, 7, 4, 4);
    let mut snap = snap_at(&mut c, 3, 4);
    let mut rec = FollowRec {
        route: Some((3, 4)),
        ..Default::default()
    };
    let mut traveller = Traveller::new();
    let route = npc_route(TransportKind::Boat);
    let mut options = TravelOptions {
        budget_ticks_per_hop: 5,
        ..Default::default()
    };
    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    // Duplicate polls do not burn the single delivered-tick clock.
    for _ in 0..10 {
        assert!(traveller
            .follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none());
    }
    for _ in 0..3 {
        bump_rebuild(&mut c, &mut snap);
        assert!(traveller
            .follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none());
    }
    c.add_chat(0, "I can't reach that!", "");
    bump_rebuild(&mut c, &mut snap);
    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    let &(x, z) = rec.walked.last().unwrap();
    plant_player(&mut c, x, z);
    rec.route = Some((x, z));
    bump_rebuild(&mut c, &mut snap);
    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.npc_ops, 2);
    plant_continue_dialog(&mut c);
    bump_rebuild(&mut c, &mut snap);
    assert!(
        matches!(
            traveller.follow(&mut rec, &snap, route, &mut options),
            Some(TravelOutcome::Stalled { tries: 2, .. })
        ),
        "retry and fare must not reset the leg clock"
    );
}

#[test]
fn sailor_crandor_variant_answers_the_normal_fare_not_crandor() {
    let mut c = scene_client();
    plant_npc_ops(&mut c, 378, 4, 4, "Seaman Thresnor", &["Talk-to"]);
    let mut snap = snap_at(&mut c, 3, 4);
    let mut rec = FollowRec {
        route: Some((3, 4)),
        ..Default::default()
    };
    let mut traveller = Traveller::new();
    let edge = TransportEdge {
        kind: TransportKind::Boat,
        player_delta: None,
        loc_id: 378,
        ..cart_edge()
    };
    let route = Route {
        dest: edge.to,
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
        ticks: 9.0,
    };
    let mut options = TravelOptions::default();
    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    plant_choice_dialog(
        &mut c,
        &["Can you take me to Crandor?", "Yes please.", "No thankyou."],
    );
    bump_rebuild(&mut c, &mut snap);
    assert!(traveller
        .follow(&mut rec, &snap, route, &mut options)
        .is_none());
    assert_eq!(
        rec.if_button_components,
        vec![102],
        "normal ride is second, not first"
    );
    assert_eq!(rec.npc_ops, 1);
}
