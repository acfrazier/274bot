use super::*;

fn detour(width: i32, height: i32, end_z: i32) -> Vec<WorldTile> {
    let tile = |x, z| WorldTile {
        x: 3200 + x,
        z: 3200 + z,
        level: 0,
    };
    let mut tiles = vec![tile(0, 0)];
    tiles.extend((1..=width).map(|x| tile(x, 0)));
    tiles.extend((1..=height).map(|z| tile(width, z)));
    tiles.extend((0..width).rev().map(|x| tile(x, height)));
    tiles.extend((end_z..height).rev().map(|z| tile(0, z)));
    tiles
}

fn walk_route(tiles: Vec<WorldTile>) -> Route {
    Route {
        dest: *tiles.last().unwrap(),
        ticks: tiles.len().saturating_sub(1) as f64,
        legs: vec![Leg::Walk { tiles }],
    }
}

#[test]
fn winding_walk_uses_short_aims_for_a_long_detour() {
    let mut client = scene_client();
    let mut snapshot = snap_at(&mut client, 0, 0);
    let mut driver = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut traveller = Traveller::new();
    let tiles = detour(40, 40, 2);
    let route = walk_route(tiles.clone());
    let mut options = TravelOptions::default();
    assert!(traveller
        .follow(&mut driver, &snapshot, route.clone(), &mut options)
        .is_none());
    let hop = traveller.follow.as_ref().unwrap().walk.as_ref().unwrap();
    assert_eq!(hop.aim, tiles[15], "the first aim is 15 path steps ahead");
    assert!(tiles.len() - 1 > options.budget_ticks_per_hop as usize);
    assert_eq!(hop.budget_ticks, options.budget_ticks_per_hop);
    for (index, tile) in tiles.iter().enumerate().skip(1) {
        plant_player(&mut client, tile.x - 3200, tile.z - 3200);
        bump_rebuild(&mut client, &mut snapshot);
        let outcome = traveller.follow(&mut driver, &snapshot, route.clone(), &mut options);
        if index + 1 == tiles.len() {
            assert!(matches!(outcome, Some(TravelOutcome::Arrived { at }) if at == route.dest));
        } else {
            assert!(outcome.is_none(), "step {index}: {outcome:?}");
        }
    }
    let expected_aims: Vec<_> = [15, 31, 47, 63, 79, 95, 111, 127, 143, 158]
        .into_iter()
        .map(|index| (tiles[index].x - 3200, tiles[index].z - 3200))
        .collect();
    assert_eq!(
        driver.walked, expected_aims,
        "the long detour uses short aims and no recovery sends"
    );
}

#[test]
fn click_ahead_hop_budget_counts_from_player_tile() {
    let mut client = scene_client();
    let mut snapshot = snap_at(&mut client, 0, 0);
    let mut driver = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut traveller = Traveller::new();
    let tiles = detour(40, 40, 2);
    let route = walk_route(tiles.clone());
    let mut options = TravelOptions {
        budget_ticks_per_hop: 4,
        ..TravelOptions::default()
    };

    assert!(traveller
        .follow(&mut driver, &snapshot, route.clone(), &mut options)
        .is_none());
    let first_hop = traveller.follow.as_ref().unwrap().walk.as_ref().unwrap();
    assert_eq!(first_hop.aim, tiles[15]);
    assert_eq!(first_hop.budget_ticks, 15);

    for (index, tile) in tiles.iter().enumerate().take(14).skip(1) {
        plant_player(&mut client, tile.x - 3200, tile.z - 3200);
        bump_rebuild(&mut client, &mut snapshot);
        assert!(
            traveller
                .follow(&mut driver, &snapshot, route.clone(), &mut options)
                .is_none(),
            "route step {index} must keep the walk active"
        );
    }

    let hop = traveller.follow.as_ref().unwrap().walk.as_ref().unwrap();
    assert!(
        hop.cursor > 13,
        "click-ahead advances the cursor past player index 13"
    );
    assert_eq!(hop.aim, tiles[31]);
    assert_eq!(
        hop.budget_ticks, 18,
        "the budget must include all steps from player index 13 to aim index 31"
    );
}

#[test]
fn short_server_path_turn_can_finish_after_scaled_budget_before_dev_bound() {
    let mut client = scene_client();
    let mut snapshot = snap_at(&mut client, 0, 0);
    let mut driver = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut traveller = Traveller::new();
    let route = walk_route(
        (0..=7)
            .map(|z| WorldTile {
                x: 3200,
                z: 3200 + z,
                level: 0,
            })
            .collect(),
    );
    let mut options = TravelOptions {
        budget_ticks_per_hop: 3,
        ..TravelOptions::default()
    };
    assert!(traveller
        .follow(&mut driver, &snapshot, route.clone(), &mut options)
        .is_none());
    let budget = traveller
        .follow
        .as_ref()
        .unwrap()
        .walk
        .as_ref()
        .unwrap()
        .budget_ticks;
    assert_eq!(budget, 7);
    assert_eq!(budget + budget.div_ceil(4), 9);
    assert!(budget + budget.div_ceil(4) < TravelOptions::default().budget_ticks_per_hop);
    // The server turn holds Chebyshev distance at two after tick 5.
    // Its scaled allowance is nine ticks, but the old fixed bound is 60.
    // The walk must remain active long enough to arrive on tick 12.
    let observed = [
        (0, 1),
        (0, 2),
        (0, 3),
        (0, 4),
        (0, 5),
        (1, 5),
        (2, 5),
        (1, 5),
        (2, 5),
        (1, 5),
        (0, 6),
        (0, 7),
    ];
    for (index, (x, z)) in observed.into_iter().enumerate() {
        plant_player(&mut client, x, z);
        bump_rebuild(&mut client, &mut snapshot);
        let outcome = traveller.follow(&mut driver, &snapshot, route.clone(), &mut options);
        if index + 1 == observed.len() {
            assert!(matches!(outcome, Some(TravelOutcome::Arrived { at }) if at == route.dest));
        } else {
            assert!(outcome.is_none(), "moving step {index}: {outcome:?}");
            assert!(
                traveller
                    .follow(&mut driver, &snapshot, route.clone(), &mut options)
                    .is_none(),
                "a repeated moving snapshot cannot expire the walk"
            );
        }
    }
}

#[test]
fn walk_budget_counts_batched_ticks_and_wrap_but_not_duplicate_polls() {
    for initial_ticks in [0, u32::MAX as u64 - 3] {
        let mut client = scene_client();
        let mut snapshot = snap_at(&mut client, 0, 0);
        client.gens.player_info = client.gens.player_info.wrapping_add(initial_ticks);
        client.gens.player = client.gens.player.wrapping_add(1);
        snapshot.rebuild(&client);
        let mut driver = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        let mut traveller = Traveller::new();
        let route = walk_route(
            (0..=4)
                .map(|z| WorldTile {
                    x: 3200,
                    z: 3200 + z,
                    level: 0,
                })
                .collect(),
        );
        let mut options = TravelOptions {
            budget_ticks_per_hop: 3,
            ..TravelOptions::default()
        };
        assert!(traveller
            .follow(&mut driver, &snapshot, route.clone(), &mut options)
            .is_none());
        for _ in 0..100 {
            assert!(traveller
                .follow(&mut driver, &snapshot, route.clone(), &mut options)
                .is_none());
        }
        client.gens.player_info = client.gens.player_info.wrapping_add(4);
        client.gens.player = client.gens.player.wrapping_add(1);
        snapshot.rebuild(&client);
        for _ in 0..100 {
            assert!(traveller
                .follow(&mut driver, &snapshot, route.clone(), &mut options)
                .is_none());
        }
        let hop = traveller.follow.as_ref().unwrap().walk.as_ref().unwrap();
        assert_eq!(hop.ticks_waited.get(), 4);
        bump_rebuild(&mut client, &mut snapshot);
        assert!(matches!(
            traveller.follow(&mut driver, &snapshot, route, &mut options),
            Some(TravelOutcome::Stalled {
                why: HopFailure::Dropped,
                ..
            })
        ));
        assert_eq!(driver.walked.len(), 1);
    }
}

#[test]
fn winding_idle_walk_expires_after_its_short_hop_budget() {
    let mut client = scene_client();
    let mut snapshot = snap_at(&mut client, 0, 0);
    let mut driver = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut traveller = Traveller::new();
    let tiles = detour(40, 40, 2);
    let route = walk_route(tiles.clone());
    let mut options = TravelOptions::default();
    assert!(traveller
        .follow(&mut driver, &snapshot, route.clone(), &mut options)
        .is_none());
    let hop = traveller.follow.as_ref().unwrap().walk.as_ref().unwrap();
    assert_eq!(hop.aim, tiles[15], "the first aim is 15 path steps ahead");
    assert_eq!(hop.budget_ticks, options.budget_ticks_per_hop);
    client.minimap_flag_x = 12;
    client.minimap_flag_z = 34;
    client.local_player.as_mut().unwrap().entity.route_length = 3;
    for tick in 1..=options.budget_ticks_per_hop {
        bump_rebuild(&mut client, &mut snapshot);
        let outcome = traveller.follow(&mut driver, &snapshot, route.clone(), &mut options);
        assert!(outcome.is_none(), "hop-budget tick {tick}: {outcome:?}");
    }
    bump_rebuild(&mut client, &mut snapshot);
    assert!(matches!(
        traveller.follow(&mut driver, &snapshot, route, &mut options),
        Some(TravelOutcome::Stalled {
            why: HopFailure::Dropped,
            ..
        })
    ));
    assert_eq!(driver.walked.len(), 1, "a stale live actor is not progress");
}

#[test]
fn two_tile_oscillation_ends_within_the_moving_budget_not_2000_ticks() {
    non_progressing_movement_expires(|tick| (0, if tick % 2 == 0 { 1 } else { 2 }));
}

#[test]
fn movement_away_from_the_aim_ends_within_the_moving_budget() {
    non_progressing_movement_expires(|tick| (tick as i32, 0));
}

fn non_progressing_movement_expires(position: impl Fn(u32) -> (i32, i32)) {
    let mut client = scene_client();
    let mut snapshot = snap_at(&mut client, 0, 0);
    let mut driver = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let route = walk_route(
        (0..=7)
            .map(|z| WorldTile {
                x: 3200,
                z: 3200 + z,
                level: 0,
            })
            .collect(),
    );
    let mut traveller = Traveller::new();
    let mut options = TravelOptions {
        budget_ticks_per_hop: 3,
        ..TravelOptions::default()
    };
    assert!(traveller
        .follow(&mut driver, &snapshot, route.clone(), &mut options)
        .is_none());
    let budget = traveller
        .follow
        .as_ref()
        .unwrap()
        .walk
        .as_ref()
        .unwrap()
        .budget_ticks;
    assert_eq!(budget, 7);
    let scaled_budget = budget + budget.div_ceil(4);
    assert_eq!(scaled_budget, 9);
    let moving_budget = scaled_budget.max(TravelOptions::default().budget_ticks_per_hop);
    assert_eq!(moving_budget, 60);
    for tick in 1..=2000 {
        let (x, z) = position(tick);
        plant_player(&mut client, x, z);
        bump_rebuild(&mut client, &mut snapshot);
        let outcome = traveller.follow(&mut driver, &snapshot, route.clone(), &mut options);
        if tick <= moving_budget {
            assert!(outcome.is_none(), "tick {tick}: {outcome:?}");
        } else {
            assert!(
                matches!(
                    outcome,
                    Some(TravelOutcome::Stalled {
                        why: HopFailure::Expired,
                        ..
                    })
                ),
                "tick {tick}: {outcome:?}"
            );
            assert_eq!(tick, moving_budget + 1);
            assert_eq!(driver.walked.len(), 1);
            return;
        }
    }
    panic!("the reviewer's 2000-tick probe must terminate");
}
