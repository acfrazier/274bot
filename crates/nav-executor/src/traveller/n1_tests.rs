use super::*;

#[test]
fn adult_spirit_tree_with_one_packed_destination_does_not_decline() {
    let mut c = scene_client();
    plant_loc(&mut c, 1293, "Spirit tree", "Talk-to", 1, 1);
    let mut snapshot = snap_at(&mut c, 1, 1);
    let mut edge = door_edge();
    edge.kind = TransportKind::SpiritTree;
    edge.loc_id = 1293;
    edge.at = WorldTile {
        x: 3201,
        z: 3201,
        level: 0,
    };
    edge.to = WorldTile {
        x: 2542,
        z: 3169,
        level: 0,
    };
    let edges = [edge.clone()];
    let route = web_route(edge);
    let mut options = TravelOptions {
        edges: Some(&edges),
        ..TravelOptions::default()
    };
    let mut traveller = Traveller::new();
    let mut driver = FollowRec {
        route: Some((1, 1)),
        ..FollowRec::default()
    };
    assert!(traveller
        .follow(&mut driver, &snapshot, route.clone(), &mut options)
        .is_none());
    plant_choice_dialog(&mut c, &["No thanks, old tree.", "Where can I go?"]);
    bump_rebuild(&mut c, &mut snapshot);
    assert!(traveller
        .follow(&mut driver, &snapshot, route, &mut options)
        .is_none());
    assert_eq!(
        driver.if_button_components,
        [102],
        "the only packed destination does not change the adult gate order"
    );
}

#[test]
fn npc_fare_is_rechecked_after_the_approach() {
    for kind in [TransportKind::Npc, TransportKind::Boat] {
        let mut c = scene_client();
        plant_driver_npc(&mut c, 7, 1, 1);
        plant_inv_stack(&mut c, 995, 30);
        let mut snapshot = snap_at(&mut c, 1, 2);
        let mut edge = cart_edge();
        edge.kind = kind;
        edge.consumed_req = vec![(995, 30)];
        let route = web_route(edge);
        let mut options = TravelOptions::default();
        let mut traveller = Traveller::new();
        let mut driver = FollowRec {
            route: Some((1, 2)),
            ..FollowRec::default()
        };
        assert!(traveller
            .follow(&mut driver, &snapshot, route.clone(), &mut options)
            .is_none());
        assert_eq!(driver.npc_ops, 1);
        plant_inv_stack(&mut c, 995, 29);
        plant_choice_dialog(&mut c, &["Yes please.", "No, thank you."]);
        bump_rebuild(&mut c, &mut snapshot);
        assert!(
            matches!(traveller.follow(&mut driver, &snapshot, route, &mut options), Some(TravelOutcome::Blocked { detail, .. }) if detail.contains("30") && detail.contains("29"))
        );
        assert_eq!(
            driver.if_buttons, 0,
            "an unaffordable fare must not answer Yes"
        );
    }
}

pub(super) fn spell_supply(c: &mut Client, edge: &TransportEdge) {
    for &(id, count) in &edge.consumed_req {
        plant_inv_stack(c, id, count);
    }
    c.set_iface_mut(
        301,
        IfTypeMut {
            link_obj_type: Some(edge.consumed_req.iter().map(|&(id, _)| id + 1).collect()),
            link_obj_number: Some(edge.consumed_req.iter().map(|&(_, count)| count).collect()),
            ..Default::default()
        },
    );
    c.stat_base_level[6] = 50;
    c.stat_effective_level[6] = 50;
    c.bump_gens(ServerProt::UPDATE_STAT);
}

#[test]
fn spell_teleport_refuses_missing_runes_or_live_level_before_pressing() {
    for (level, law_count) in [(24, 1), (25, 0)] {
        let mut c = scene_client();
        let edge = varrock_spell_edge();
        spell_supply(&mut c, &edge);
        c.stat_effective_level[6] = level;
        c.set_iface_mut(
            301,
            IfTypeMut {
                link_obj_type: Some(vec![555, 557, 564]),
                link_obj_number: Some(vec![1, 3, law_count]),
                ..Default::default()
            },
        );
        let snapshot = snap_at(&mut c, 0, 0);
        let mut traveller = Traveller::new();
        let mut driver = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        assert!(matches!(
            traveller.follow(
                &mut driver,
                &snapshot,
                web_route(edge),
                &mut TravelOptions::default()
            ),
            Some(TravelOutcome::Blocked { .. })
        ));
        assert_eq!(driver.if_buttons, 0);
    }
}

#[test]
fn jewellery_teleport_refuses_a_depleted_charge_before_rubbing() {
    let mut c = scene_client();
    plant_inv_item(&mut c, 1704); // uncharged glory, not the planned charged variant
    let snapshot = snap_at(&mut c, 0, 0);
    let mut traveller = Traveller::new();
    let mut driver = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    assert!(
        matches!(traveller.follow(&mut driver, &snapshot, web_route(glory_edge()), &mut TravelOptions::default()), Some(TravelOutcome::Blocked { detail, .. }) if detail.contains("1712"))
    );
    assert_eq!(driver.held_ops, 0);
}

#[test]
fn disconnected_transport_that_made_progress_is_expired_not_dropped() {
    for moved in [false, true] {
        let mut c = scene_client();
        plant_inv_item(&mut c, 2552);
        let mut snapshot = snap_at(&mut c, 0, 0);
        let route = web_route(ring_edge());
        let mut traveller = Traveller::new();
        let mut driver = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        let mut options = TravelOptions::default();
        assert!(traveller
            .follow(&mut driver, &snapshot, route.clone(), &mut options)
            .is_none());
        if moved {
            plant_player(&mut c, 1, 0);
            bump_rebuild(&mut c, &mut snapshot);
        }
        c.ingame = false;
        bump_rebuild(&mut c, &mut snapshot);
        let expected = if moved {
            HopFailure::Expired
        } else {
            HopFailure::Dropped
        };
        assert!(
            matches!(traveller.follow(&mut driver, &snapshot, route, &mut options), Some(TravelOutcome::Stalled { why, .. }) if why == expected)
        );
    }
}

#[test]
fn slammed_door_does_not_receive_a_second_full_wait_budget() {
    for start_x in [0, 5] {
        let mut c = scene_client();
        plant_door(&mut c, false, 1);
        let mut snapshot = snap_at(&mut c, start_x, 0);
        let route = web_route(TransportEdge {
            open_loc_id: Some(1531),
            ..door_edge()
        });
        let mut traveller = Traveller::new();
        let mut driver = FollowRec {
            route: Some((start_x, 0)),
            ..FollowRec::default()
        };
        let mut options = TravelOptions {
            budget_ticks_per_hop: 3,
            close_enough: 1,
            ..TravelOptions::default()
        };
        assert!(traveller
            .follow(&mut driver, &snapshot, route.clone(), &mut options)
            .is_none());
        let mut terminal = None;
        for poll in 1..=5 {
            bump_rebuild(&mut c, &mut snapshot);
            if let Some(outcome) =
                traveller.follow(&mut driver, &snapshot, route.clone(), &mut options)
            {
                terminal = Some((poll, outcome));
                break;
            }
        }
        assert!(matches!(terminal, Some((_, TravelOutcome::Stalled { .. }))), "crossing/approach escalation from {start_x} must retain the spent wait budget: {terminal:?}");
    }
}

#[test]
fn unbound_jewellery_control_is_named_instead_of_blaming_the_scene() {
    let mut c = scene_client();
    plant_inv_item_unbound_tab(&mut c, 2552);
    let mut snapshot = snap_at(&mut c, 0, 0);
    let route = web_route(ring_edge());
    let mut traveller = Traveller::new();
    let mut driver = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut options = TravelOptions {
        budget_ticks_per_hop: 1,
        ..TravelOptions::default()
    };
    assert!(traveller
        .follow(&mut driver, &snapshot, route.clone(), &mut options)
        .is_none());
    bump_rebuild(&mut c, &mut snapshot);
    let outcome = traveller.follow(&mut driver, &snapshot, route, &mut options);
    assert!(
        matches!(outcome, Some(TravelOutcome::Blocked { detail, .. }) if detail.contains("2552") && detail.contains("inventory control") && !detail.contains("loaded scene"))
    );
    assert_eq!(driver.held_ops, 0);
}
