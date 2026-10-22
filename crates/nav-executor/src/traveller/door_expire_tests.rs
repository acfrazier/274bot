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

#[test]
fn shared_self_closing_door_retries_lost_operations_after_closed_loc_returns() {
    let mut clients: Vec<_> = (0..3)
        .map(|_| {
            let mut client = scene_client();
            plant_loc(&mut client, 2025, "Door", "Open", 6, 6);
            client
        })
        .collect();
    let mut snapshots: Vec<_> = clients
        .iter_mut()
        .map(|client| snap_at(client, 5, 6))
        .collect();
    let closed_loc = snapshots[0].locs()[0].clone();
    let mut inviswall = closed_loc.clone();
    inviswall.id = 83;
    inviswall.actions.clear();
    let mut displaced_leaf = closed_loc.clone();
    displaced_leaf.id = 1535;
    displaced_leaf.tile.z -= 1;
    displaced_leaf.actions.clear();

    let edge = TransportEdge {
        at: WorldTile {
            x: 3206,
            z: 3206,
            level: 0,
        },
        to: WorldTile {
            x: 3206,
            z: 3206,
            level: 0,
        },
        loc_id: 2025,
        open_loc_id: None,
        dir: Some(DoorDir::E),
        ..door_edge()
    };
    let route = web_route(edge.clone());
    let mut travellers: Vec<_> = (0..3).map(|_| Traveller::new()).collect();
    let mut drivers: Vec<_> = (0..3)
        .map(|_| FollowRec {
            route: Some((5, 6)),
            ..FollowRec::default()
        })
        .collect();
    let mut options: Vec<_> = (0..3)
        .map(|_| TravelOptions {
            close_enough: 0,
            ..TravelOptions::default()
        })
        .collect();
    let mut terminal = [false; 3];
    let mut landing_at = [None; 3];
    let mut unavailable_until = 0;
    let mut accepted = Vec::new();

    for tick in 0..=12 {
        let closed = tick >= unavailable_until;
        let walks_before: Vec<_> = drivers.iter().map(|driver| driver.walked.len()).collect();
        for i in 0..3 {
            if landing_at[i].is_some_and(|landing| tick >= landing) {
                plant_player(&mut clients[i], 6, 6);
            }
            if tick > 0 {
                bump_rebuild(&mut clients[i], &mut snapshots[i]);
            }
            snapshots[i].seed_locs(if closed {
                vec![closed_loc.clone()]
            } else {
                vec![inviswall.clone(), displaced_leaf.clone()]
            });
        }

        let mut requests = Vec::new();
        for i in 0..3 {
            if terminal[i] {
                continue;
            }
            let before = drivers[i].loc_ops;
            let outcome = travellers[i].follow(
                &mut drivers[i],
                &snapshots[i],
                route.clone(),
                &mut options[i],
            );
            if drivers[i].loc_ops > before {
                requests.push(i);
            }
            if let Some(outcome) = outcome {
                assert!(
                    matches!(outcome, TravelOutcome::Arrived { at } if at == edge.to),
                    "unexpected outcome: {outcome:?}"
                );
                terminal[i] = true;
            }
        }

        if !closed {
            assert!(
                requests.is_empty(),
                "never operate the invisible wall or inactive loc 1535"
            );
            assert!(
                drivers
                    .iter()
                    .zip(&walks_before)
                    .all(|(driver, before)| driver.walked.len() == *before),
                "the displaced inactive leaf is not a walk-through open leaf"
            );
        } else if let Some(&winner) = requests.first() {
            // All snapshots saw closed loc 2025 before queued operations ran.
            // The script accepts one operation, then swaps it for loc 83 and
            // inactive loc 1535 for three ticks, teleporting only its owner.
            landing_at[winner] = Some(tick + 1);
            unavailable_until = tick + 4;
            accepted.push((tick, winner));
        }

        if terminal == [true; 3] {
            break;
        }
    }

    assert_eq!(accepted, vec![(0, 0), (4, 1), (8, 2)]);
    assert_eq!(
        drivers
            .iter()
            .map(|driver| driver.loc_ops)
            .collect::<Vec<_>>(),
        vec![1, 2, 3],
        "the landed winner does not re-operate the door; each loser retries the closed loc"
    );
    assert_eq!(
        terminal, [true; 3],
        "every traveller must cross on an accepted operation"
    );
}

#[test]
fn west_ardougne_exact_move_fence_far_side_finishes_without_reopening() {
    // This is the producer's westward `mournerstewfence` crossing: a
    // directional scripted Door with no reusable open leaf.
    let mut client = scene_client();
    plant_loc(&mut client, 2068, "Mourner fence", "Climb-over", 6, 6);
    let snapshot = snap_at(&mut client, 4, 6);
    let edge = TransportEdge {
        at: WorldTile {
            x: 3206,
            z: 3206,
            level: 0,
        },
        to: WorldTile {
            x: 3205,
            z: 3206,
            level: 0,
        },
        loc_id: 2068,
        open_loc_id: None,
        dir: Some(DoorDir::W),
        ..door_edge()
    };
    let route = web_route(edge);
    let mut traveller = Traveller::new();
    let mut driver = FollowRec {
        route: Some((4, 6)),
        ..FollowRec::default()
    };
    let mut options = TravelOptions {
        close_enough: 0,
        ..TravelOptions::default()
    };

    assert!(traveller
        .follow(&mut driver, &snapshot, route, &mut options)
        .is_none());
    assert_eq!(
        driver.loc_ops, 0,
        "a bot already on the far side must not climb again"
    );
    assert_eq!(
        driver.walked,
        [(5, 6)],
        "continue to the packed west-side destination, not back across the fence"
    );
}
