use super::*;

#[test]
fn follow_sends_the_last_tile_after_a_penultimate_fallback() {
    for route_index_limit in [None, Some(2)] {
        let mut client = scene_client();
        let mut snapshot = snap_at(&mut client, 0, 0);
        let mut driver = FollowRec {
            route: Some((0, 0)),
            reject_far: true,
            ..FollowRec::default()
        };
        let mut traveller = Traveller::new();
        let route = Route {
            legs: vec![walk_leg(&[(3200, 3200), (3200, 3201), (3200, 3202)])],
            dest: WorldTile {
                x: 3200,
                z: 3202,
                level: 0,
            },
            ticks: 2.0,
        };
        let mut options = TravelOptions {
            close_enough: 0,
            route_index_limit,
            ..TravelOptions::default()
        };

        assert_eq!(
            traveller.follow(&mut driver, &snapshot, route.clone(), &mut options),
            None
        );
        assert_eq!(driver.walked, [(0, 1)], "the far aim was refused");

        plant_player(&mut client, 0, 1);
        driver.route = Some((0, 1));
        bump_rebuild(&mut client, &mut snapshot);
        assert_eq!(
            traveller.follow(&mut driver, &snapshot, route.clone(), &mut options),
            None
        );
        assert_eq!(
            driver.walked,
            [(0, 1), (0, 2)],
            "the next cursor is the last tile, not a cleared tile"
        );

        plant_player(&mut client, 0, 2);
        bump_rebuild(&mut client, &mut snapshot);
        assert_eq!(
            traveller.follow(&mut driver, &snapshot, route.clone(), &mut options),
            Some(TravelOutcome::Arrived { at: route.dest })
        );
    }
}

#[test]
fn follow_bounds_an_unsent_walk_by_game_ticks_not_polls() {
    for (first_tick, budget) in [(1, 3), (u32::MAX - 10, 60)] {
        let mut client = scene_client();
        let mut snapshot = snap_at(&mut client, 0, 0);
        snapshot.seed_tick(first_tick);
        let mut driver = FollowRec {
            route: Some((0, 0)),
            reject_far: true,
            reject_to: Some((0, 1)),
            ..FollowRec::default()
        };
        let mut traveller = Traveller::new();
        let route = Route {
            legs: vec![walk_leg(&[
                (3200, 3200),
                (3200, 3201),
                (3200, 3202),
                (3200, 3203),
            ])],
            dest: WorldTile {
                x: 3200,
                z: 3203,
                level: 0,
            },
            ticks: 3.0,
        };
        let mut options = TravelOptions {
            budget_ticks_per_hop: budget,
            ..TravelOptions::default()
        };
        assert_eq!(
            traveller.follow(&mut driver, &snapshot, route.clone(), &mut options),
            None
        );
        for _ in 0..100 {
            assert_eq!(
                traveller.follow(&mut driver, &snapshot, route.clone(), &mut options),
                None,
                "polls without PLAYER_INFO cannot spend the send allowance"
            );
        }
        // Unsent hops use the same game-tick allowance as sent hops.
        snapshot.seed_tick(first_tick.wrapping_add(budget - 1));
        assert_eq!(
            traveller.follow(&mut driver, &snapshot, route.clone(), &mut options),
            None
        );
        snapshot.seed_tick(first_tick.wrapping_add(budget));
        assert!(matches!(
            traveller.follow(&mut driver, &snapshot, route.clone(), &mut options),
            Some(TravelOutcome::Stalled {
                why: HopFailure::Dropped,
                tries: 0,
                last_refusal: None,
                ..
            })
        ));
        assert!(driver.walked.is_empty(), "no refused click was sent");
    }
}

#[test]
fn follow_does_not_spend_the_send_budget_while_a_route_limit_holds_it() {
    let mut client = scene_client();
    let mut snapshot = snap_at(&mut client, 0, 0);
    snapshot.seed_tick(1);
    let mut driver = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut traveller = Traveller::new();
    let route = Route {
        legs: vec![walk_leg(&[(3200, 3200), (3200, 3201), (3200, 3202)])],
        dest: WorldTile {
            x: 3200,
            z: 3202,
            level: 0,
        },
        ticks: 2.0,
    };
    let mut options = TravelOptions {
        budget_ticks_per_hop: 3,
        route_index_limit: Some(0),
        ..TravelOptions::default()
    };
    assert_eq!(
        traveller.follow(&mut driver, &snapshot, route.clone(), &mut options),
        None
    );
    snapshot.seed_tick(100);
    assert_eq!(
        traveller.follow(&mut driver, &snapshot, route.clone(), &mut options),
        None
    );
    assert!(driver.walked.is_empty());
    options.route_index_limit = None;
    assert_eq!(
        traveller.follow(&mut driver, &snapshot, route, &mut options),
        None
    );
    assert_eq!(driver.walked, [(0, 2)]);
}
