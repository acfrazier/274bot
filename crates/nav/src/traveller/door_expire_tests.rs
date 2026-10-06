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
        // The first closed read Opens at once; a leaf that has not answered
        // that Open is re-Opened only after a 2-tick closed window.
        plant_player(&mut c, x - 1, z);
        for (poll, ops) in [(1, 1), (2, 1), (3, 2)] {
            bump_rebuild(&mut c, &mut snapshot);
            snapshot.seed_locs(vec![leaf(&loc, &edge, false)]);
            assert!(traveller
                .follow(&mut driver, &snapshot, route.clone(), &mut options)
                .is_none());
            assert_eq!(
                driver.loc_ops, ops,
                "closed poll {poll}: Open now, then after the closed window, not after 60 ticks"
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
        // Seen open, then closed again: a real toggle re-Opens at once.
        bump_rebuild(&mut c, &mut snapshot);
        snapshot.seed_locs(vec![leaf(&loc, &edge, false)]);
        assert!(traveller
            .follow(&mut driver, &snapshot, route.clone(), &mut options)
            .is_none());
        assert_eq!(driver.loc_ops, 3, "a toggled leaf is re-Opened at once");
        plant_player(&mut c, x, z);
        bump_rebuild(&mut c, &mut snapshot);
        snapshot.seed_locs(vec![leaf(&loc, &edge, false)]);
        assert!(matches!(
            traveller.follow(&mut driver, &snapshot, route, &mut options),
            Some(TravelOutcome::Arrived { at }) if at == edge.to
        ));
        assert_eq!(
            driver.loc_ops, 3,
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

#[test]
fn approach_completion_walks_a_leaf_opened_meanwhile_instead_of_closing_it() {
    let mut c = scene_client();
    plant_loc(&mut c, 1519, "Large door", "Open", 17, 19);
    let mut snapshot = snap_at(&mut c, 13, 19);
    let edge = castle_door(1519, 17, 19);
    let loc = snapshot.locs()[0].clone();
    snapshot.seed_locs(vec![leaf(&loc, &edge, false)]);
    let route = web_route(edge.clone());
    let mut traveller = Traveller::new();
    let mut driver = FollowRec {
        route: Some((17, 19)),
        ..FollowRec::default()
    };
    let mut options = TravelOptions {
        close_enough: 0,
        ..TravelOptions::default()
    };
    assert!(traveller
        .follow(&mut driver, &snapshot, route.clone(), &mut options)
        .is_none());
    assert_eq!(driver.loc_ops, 0);
    assert_eq!(driver.walked, [(16, 19)], "approach the closed door");
    // Another player opens the leaf while we finish the approach walk.
    plant_player(&mut c, 16, 19);
    bump_rebuild(&mut c, &mut snapshot);
    snapshot.seed_locs(vec![leaf(&loc, &edge, true)]);
    assert!(traveller
        .follow(&mut driver, &snapshot, route.clone(), &mut options)
        .is_none());
    assert_eq!(
        driver.loc_ops, 0,
        "approach completion sent OP_LOC1 to the open leaf (its Close op)"
    );
    bump_rebuild(&mut c, &mut snapshot);
    snapshot.seed_locs(vec![leaf(&loc, &edge, true)]);
    assert!(traveller
        .follow(&mut driver, &snapshot, route.clone(), &mut options)
        .is_none());
    assert_eq!(driver.loc_ops, 0);
    assert_eq!(
        driver.walked,
        [(16, 19), (17, 19)],
        "walk through the open leaf"
    );
}

#[test]
fn open_leaf_door_does_not_step_into_a_closed_wall_from_the_door_tile() {
    // Same shape as cheap_door_hop_does_not_step_into_a_closed_wall_from_the_door_tile,
    // but the edge carries a packed open leaf, so door recovery owns it.
    let mut c = scene_client();
    plant_door_at(&mut c, false, 6, 6);
    c.collision[0].add_wall(6, 6, 0, 2, false);
    let mut snap = snap_at(&mut c, 5, 6);
    let mut edge = door_edge();
    edge.at = WorldTile {
        x: 3206,
        z: 3206,
        level: 0,
    };
    edge.to = WorldTile {
        x: 3207,
        z: 3206,
        level: 0,
    };
    edge.dir = Some(DoorDir::E);
    edge.open_loc_id = Some(1531);
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(edge.clone()),
        }],
        dest: edge.to,
        ticks: 1.0,
    };
    let mut options = TravelOptions {
        budget_ticks_per_hop: 60,
        close_enough: 0,
        ..TravelOptions::default()
    };
    let mut rec = FollowRec {
        route: Some((5, 6)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.loc_ops, 1);
    // The server walked us onto `at` before the queued Open fired.
    plant_player(&mut c, 6, 6);
    rec.route = Some((6, 6));
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert!(rec.sink.steps.is_empty(), "step into the closed wall");
    assert_eq!(rec.loc_ops, 1, "the queued Open still has its window");
    // The Open fires: the leaf reads open, so the kept probe steps through.
    plant_door_at(&mut c, true, 6, 6);
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(
        rec.sink.steps,
        vec![client::io::ClientProt::MOVE_GAMECLICK.id, 5, 0, 3207, 3206]
    );
    assert_eq!(rec.loc_ops, 1);
}

#[test]
fn never_opening_door_gets_a_handful_of_paced_opens_within_the_default_budget() {
    let mut c = scene_client();
    plant_loc(&mut c, 1519, "Large door", "Open", 17, 19);
    let mut snapshot = snap_at(&mut c, 16, 19);
    let edge = castle_door(1519, 17, 19);
    let loc = snapshot.locs()[0].clone();
    snapshot.seed_locs(vec![leaf(&loc, &edge, false)]);
    let route = web_route(edge.clone());
    let mut traveller = Traveller::new();
    let mut driver = FollowRec {
        route: Some((16, 19)),
        ..FollowRec::default()
    };
    let mut options = TravelOptions {
        close_enough: 0,
        ..TravelOptions::default()
    };
    let mut polls = 0;
    let mut open_polls = Vec::new();
    let out = loop {
        let before = driver.loc_ops;
        if let Some(o) = traveller.follow(&mut driver, &snapshot, route.clone(), &mut options) {
            break o;
        }
        if driver.loc_ops != before {
            open_polls.push(polls);
        }
        polls += 1;
        assert!(polls < 500);
        bump_rebuild(&mut c, &mut snapshot);
        snapshot.seed_locs(vec![leaf(&loc, &edge, false)]);
    };
    assert_eq!(
        open_polls,
        [0, 2, 6, 14, 30],
        "re-Open after a closed window that doubles per unanswered Open"
    );
    assert!(
        matches!(out, TravelOutcome::Stalled { tries: 5, .. }),
        "the receipt counts the real Opens: {out:?}"
    );
}

#[test]
fn door_recovery_reopens_and_walks_a_leaf_matched_by_footprint_not_origin() {
    // A two-tile leaf whose origin is 4 tiles from `at` but whose footprint
    // reaches within 3: the arm's `find_transport_loc` matches it, and door
    // recovery must re-Open and walk the same leaf through `find_door_loc`,
    // not an origin-distance scan that loses it.
    let mut c = scene_client();
    plant_loc(&mut c, 1519, "Large door", "Open", 13, 19);
    let mut snapshot = snap_at(&mut c, 16, 19);
    let edge = castle_door(1519, 17, 19);
    let mut closed = snapshot.locs()[0].clone();
    closed.tile = WorldTile {
        x: 3213,
        z: 3219,
        level: 0,
    };
    closed.footprint_width = 2;
    closed.footprint_length = 1;
    let mut open = closed.clone();
    open.id = 1520;
    open.actions = vec![Some("Close".to_owned())];
    snapshot.seed_locs(vec![closed.clone()]);
    let route = web_route(edge.clone());
    let mut traveller = Traveller::new();
    let mut driver = FollowRec {
        route: Some((16, 19)),
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
        driver.loc_ops, 1,
        "the arm Opens the footprint-matched leaf"
    );
    traveller
        .follow
        .as_mut()
        .unwrap()
        .transport
        .as_mut()
        .unwrap()
        .troll = true;
    for ops in [1, 2] {
        bump_rebuild(&mut c, &mut snapshot);
        snapshot.seed_locs(vec![closed.clone()]);
        assert!(traveller
            .follow(&mut driver, &snapshot, route.clone(), &mut options)
            .is_none());
        assert_eq!(driver.loc_ops, ops, "door recovery re-Opens the same leaf");
    }
    bump_rebuild(&mut c, &mut snapshot);
    snapshot.seed_locs(vec![open]);
    assert!(traveller
        .follow(&mut driver, &snapshot, route.clone(), &mut options)
        .is_none());
    assert_eq!(driver.loc_ops, 2);
    assert_eq!(driver.walked, [(17, 19)], "walk through its open leaf");
    plant_player(&mut c, 17, 19);
    bump_rebuild(&mut c, &mut snapshot);
    snapshot.seed_locs(vec![closed]);
    assert!(matches!(
        traveller.follow(&mut driver, &snapshot, route, &mut options),
        Some(TravelOutcome::Arrived { at }) if at == edge.to
    ));
}
