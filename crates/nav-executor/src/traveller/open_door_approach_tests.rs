use super::*;

fn fixture(open: bool) -> (Client, GameSnapshot, FollowRec, Route) {
    let mut client = scene_client();
    plant_door_at(&mut client, open, 1, 2);
    let snapshot = snap_at(&mut client, 0, 2);
    let edge = TransportEdge {
        at: WorldTile {
            x: 3202,
            z: 3202,
            level: 0,
        },
        to: WorldTile {
            x: 3203,
            z: 3202,
            level: 0,
        },
        open_loc_id: Some(1531),
        ..door_edge()
    };
    let route = Route {
        dest: edge.to,
        ticks: 2.0,
        legs: vec![
            walk_leg(&[(3200, 3202), (3201, 3202)]),
            Leg::Transport {
                edge: Box::new(edge),
            },
        ],
    };
    let driver = FollowRec {
        route: Some((0, 2)),
        reject_to: Some((1, 2)),
        ..FollowRec::default()
    };
    (client, snapshot, driver, route)
}

#[test]
fn open_leaf_on_packed_approach_walks_through_without_waiting_for_door_reset() {
    // Wizards Tower: a prior Open moves the diagonal leaf onto the route's
    // final approach tile. The live path to the far side remains clear.
    let (mut client, mut snapshot, mut driver, route) = fixture(true);
    let mut traveller = Traveller::new();
    let mut options = TravelOptions::default();
    assert!(traveller
        .follow(&mut driver, &snapshot, route.clone(), &mut options)
        .is_none());
    assert_eq!(
        driver.walked,
        vec![(3, 2)],
        "use the next door leg on the first observed tick"
    );
    assert_eq!(driver.loc_ops, 0, "never Close the open leaf");

    plant_player(&mut client, 3, 2);
    bump_rebuild(&mut client, &mut snapshot);
    assert_eq!(
        traveller.follow(&mut driver, &snapshot, route, &mut options),
        Some(TravelOutcome::Arrived {
            at: WorldTile {
                x: 3203,
                z: 3202,
                level: 0
            }
        }),
    );
}

#[test]
fn blocked_approach_requires_the_matching_open_leaf_and_next_door_leg() {
    for case in [
        "closed",
        "unrelated leaf",
        "exact takeoff",
        "ladder",
        "later door",
    ] {
        let (_, snapshot, mut driver, mut route) = fixture(case != "closed");
        let Leg::Transport { edge } = &mut route.legs[1] else {
            unreachable!()
        };
        match case {
            "closed" => {}
            "unrelated leaf" => edge.open_loc_id = Some(999),
            "exact takeoff" => {
                edge.takeoff = Some(WorldTile {
                    x: 3201,
                    z: 3202,
                    level: 0,
                })
            }
            "ladder" => edge.kind = TransportKind::Ladder,
            "later door" => route
                .legs
                .insert(1, walk_leg(&[(3201, 3202), (3202, 3202)])),
            _ => unreachable!(),
        }
        let mut traveller = Traveller::new();
        let mut options = TravelOptions::default();
        for _ in 0..3 {
            assert!(traveller
                .follow(&mut driver, &snapshot, route.clone(), &mut options)
                .is_none());
        }
        assert!(
            driver.walked.is_empty(),
            "{case}: keep the required approach"
        );
        assert_eq!(
            driver.loc_ops, 0,
            "{case}: do not bypass or operate the door"
        );
    }
}

#[test]
fn an_open_leaf_does_not_skip_the_remaining_walk_distance() {
    let (mut client, _, mut driver, mut route) = fixture(true);
    let snapshot = snap_at(&mut client, 0, 2);
    route.legs[0] = walk_leg(&[(3200, 3202), (3201, 3202), (3202, 3202)]);
    client.world.del_wall(0, 1, 2);
    plant_door_at(&mut client, true, 2, 2);
    let mut snapshot = snapshot;
    bump_rebuild(&mut client, &mut snapshot);
    driver.reject_to = Some((2, 2));
    let mut traveller = Traveller::new();
    let mut options = TravelOptions::default();
    assert!(traveller
        .follow(&mut driver, &snapshot, route, &mut options)
        .is_none());
    assert_eq!(
        driver.walked,
        vec![(1, 2)],
        "walk toward the door before its blocked stand"
    );
    assert_eq!(driver.loc_ops, 0);
}

#[test]
fn open_leaf_with_unreachable_far_side_keeps_the_approach_until_it_clears() {
    let (mut client, mut snapshot, mut driver, route) = fixture(true);
    client.collision[0].flags[3][2] |= client::dash3d::CollisionFlag::SQ_BLOCKED;
    bump_rebuild(&mut client, &mut snapshot);
    let mut traveller = Traveller::new();
    let mut options = TravelOptions::default();
    for _ in 0..3 {
        bump_rebuild(&mut client, &mut snapshot);
        assert!(traveller
            .follow(&mut driver, &snapshot, route.clone(), &mut options)
            .is_none());
        assert_eq!(traveller.follow.as_ref().unwrap().leg_index, 0);
    }
    assert!(driver.walked.is_empty(), "do not commit the next door leg");
    assert_eq!(driver.loc_ops, 0, "never Close the leaf to clear a path");

    client.collision[0].flags[3][2] &= !client::dash3d::CollisionFlag::SQ_BLOCKED;
    bump_rebuild(&mut client, &mut snapshot);
    assert!(traveller
        .follow(&mut driver, &snapshot, route, &mut options)
        .is_none());
    assert_eq!(driver.walked, vec![(3, 2)]);
}
