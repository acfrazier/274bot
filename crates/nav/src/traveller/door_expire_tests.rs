use super::*;

fn castle_door(closed_id: i32, x: i32, z: i32) -> TransportEdge {
    TransportEdge {
        at: WorldTile {
            x: 3200 + x,
            z: 3200 + z,
            level: 0,
        },
        to: WorldTile {
            x: 3200 + x,
            z: 3200 + z,
            level: 0,
        },
        loc_id: closed_id,
        open_loc_id: Some(closed_id + 1),
        dir: Some(DoorDir::E),
        ..door_edge()
    }
}

fn leaf(loc: &api::snapshot::LocView, edge: &TransportEdge, open: bool) -> api::snapshot::LocView {
    let mut leaf = loc.clone();
    leaf.id = if open {
        edge.open_loc_id.unwrap()
    } else {
        edge.loc_id
    };
    leaf.tile = edge.at;
    if open {
        leaf.tile.x += 1; // The engine swings the open leaf away from its closed tile.
    }
    leaf.actions = vec![Some(if open { "Close" } else { "Open" }.to_owned())];
    leaf
}

#[test]
fn castle_door_closed_after_open_walk_reopens_promptly_and_retries_before_expiry() {
    for (closed_id, x, z) in [(1519, 17, 19), (1516, 13, 20)] {
        let mut c = scene_client();
        plant_loc(&mut c, closed_id, "Large door", "Open", x, z);
        let mut snapshot = snap_at(&mut c, x - 2, z + 1);
        let edge = castle_door(closed_id, x, z);
        let loc = snapshot.locs()[0].clone();
        snapshot.seed_locs(vec![leaf(&loc, &edge, true)]);
        let route = web_route(edge.clone());
        let mut traveller = Traveller::new();
        let mut driver = FollowRec {
            route: Some((x, z)),
            ..FollowRec::default()
        };
        let mut options = TravelOptions {
            close_enough: 0,
            ..TravelOptions::default()
        };
        assert!(traveller
            .follow(&mut driver, &snapshot, route.clone(), &mut options)
            .is_none());
        assert_eq!(
            driver.loc_ops, 0,
            "the shifted open leaf must never be Closed"
        );
        assert_eq!(driver.walked, [(x, z)]);

        // Exact trace shape: the hop armed while open, the player approached,
        // then another player closed the leaf before we crossed the wall.
        plant_player(&mut c, x - 1, z);
        for attempt in 1..=2 {
            bump_rebuild(&mut c, &mut snapshot);
            snapshot.seed_locs(vec![leaf(&loc, &edge, false)]);
            assert!(traveller
                .follow(&mut driver, &snapshot, route.clone(), &mut options)
                .is_none());
            assert_eq!(
                driver.loc_ops, attempt,
                "closed door must Open now, not after 60 ticks"
            );
        }
        bump_rebuild(&mut c, &mut snapshot);
        snapshot.seed_locs(vec![leaf(&loc, &edge, true)]);
        assert!(traveller
            .follow(&mut driver, &snapshot, route.clone(), &mut options)
            .is_none());
        assert_eq!(
            driver.loc_ops, 2,
            "an open double-door leaf is not an Open target"
        );
        assert_eq!(driver.walked, [(x, z), (x, z)]);
        plant_player(&mut c, x, z);
        bump_rebuild(&mut c, &mut snapshot);
        snapshot.seed_locs(vec![leaf(&loc, &edge, false)]);
        assert!(matches!(
            traveller.follow(&mut driver, &snapshot, route, &mut options),
            Some(TravelOutcome::Arrived { at }) if at == edge.to
        ));
        assert_eq!(
            driver.loc_ops, 2,
            "a closer behind us must not reopen the door"
        );
    }
}

#[test]
fn castle_door_toggled_forever_exhausts_one_budget_with_real_attempt_count() {
    let mut c = scene_client();
    plant_loc(&mut c, 1519, "Large door", "Open", 17, 19);
    let mut snapshot = snap_at(&mut c, 15, 20);
    let edge = castle_door(1519, 17, 19);
    let loc = snapshot.locs()[0].clone();
    snapshot.seed_locs(vec![leaf(&loc, &edge, true)]);
    let route = web_route(edge.clone());
    let mut traveller = Traveller::new();
    let mut driver = FollowRec {
        route: Some((17, 19)),
        ..FollowRec::default()
    };
    let mut options = TravelOptions {
        close_enough: 0,
        budget_ticks_per_hop: 3,
        ..TravelOptions::default()
    };
    assert!(traveller
        .follow(&mut driver, &snapshot, route.clone(), &mut options)
        .is_none());
    plant_player(&mut c, 16, 19);
    let mut terminal = None;
    for poll in 1..=4 {
        bump_rebuild(&mut c, &mut snapshot);
        snapshot.seed_locs(vec![leaf(&loc, &edge, poll % 2 == 0)]);
        if let Some(outcome) = traveller.follow(&mut driver, &snapshot, route.clone(), &mut options)
        {
            terminal = Some(outcome);
            break;
        }
    }
    assert!(
        matches!(terminal, Some(TravelOutcome::Stalled { why: HopFailure::Expired, tries, .. }) if tries == driver.loc_ops as u32 && tries > 1),
        "toggling must retry inside one bounded budget: {terminal:?}, ops={}",
        driver.loc_ops
    );
}

#[test]
fn castle_double_door_family_matches_its_shifted_open_leaf_not_its_sibling() {
    let mut c = scene_client();
    plant_loc(&mut c, 1519, "Large door", "Open", 17, 19);
    let mut snapshot = snap_at(&mut c, 16, 19);
    let edge = castle_door(1519, 17, 19);
    let loc = snapshot.locs()[0].clone();
    let mut sibling = leaf(&loc, &edge, false);
    sibling.id = 1516;
    snapshot.seed_locs(vec![sibling.clone(), leaf(&loc, &edge, true)]);
    assert!(super::super::edge_loc_open(&snapshot, &edge));
    assert_eq!(
        super::super::find_door_loc(&snapshot, &edge).unwrap().id,
        1520
    );
    snapshot.seed_locs(vec![sibling, leaf(&loc, &edge, false)]);
    assert!(!super::super::edge_loc_open(&snapshot, &edge));
}
