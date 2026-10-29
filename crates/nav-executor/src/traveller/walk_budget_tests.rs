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
fn winding_walk_budget_uses_planned_steps_not_endpoint_distance() {
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
    assert_eq!(hop.aim, route.dest, "the endpoint is only two tiles away");
    assert!(tiles.len() - 1 > options.budget_ticks_per_hop as usize);
    // A live-looking actor is not progress. Even without movement, this
    // winding path must receive its planned duration, not only 60 ticks.
    client.minimap_flag_x = 12;
    client.minimap_flag_z = 34;
    client.local_player.as_mut().unwrap().entity.route_length = 3;
    for tick in 1..=70 {
        bump_rebuild(&mut client, &mut snapshot);
        let outcome = traveller.follow(&mut driver, &snapshot, route.clone(), &mut options);
        assert!(
            outcome.is_none(),
            "planned-duration tick {tick}: {outcome:?}"
        );
    }
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
    assert_eq!(driver.walked.len(), 1, "movement needs no recovery send");
}

#[test]
fn observed_progress_keeps_a_walk_alive_past_its_planned_allowance() {
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
    // Consume the planned allowance first. New route maxima still extend
    // the hop, but a change of tile alone is not evidence of progress.
    client.minimap_flag_x = 12;
    client.minimap_flag_z = 34;
    client.local_player.as_mut().unwrap().entity.route_length = 3;
    for _ in 0..7 {
        bump_rebuild(&mut client, &mut snapshot);
        assert!(traveller
            .follow(&mut driver, &snapshot, route.clone(), &mut options)
            .is_none());
    }
    let Leg::Walk { tiles: observed } = &route.legs[0] else {
        unreachable!()
    };
    for (index, tile) in observed.iter().enumerate().skip(1) {
        plant_player(&mut client, tile.x - 3200, tile.z - 3200);
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
fn winding_idle_walk_expires_only_after_its_planned_tick_budget() {
    let mut client = scene_client();
    let mut snapshot = snap_at(&mut client, 0, 0);
    let mut driver = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut traveller = Traveller::new();
    let tiles = detour(40, 40, 2);
    let planned_ticks = tiles.len() - 1;
    let route = walk_route(tiles);
    let mut options = TravelOptions::default();
    assert!(traveller
        .follow(&mut driver, &snapshot, route.clone(), &mut options)
        .is_none());
    client.minimap_flag_x = 12;
    client.minimap_flag_z = 34;
    client.local_player.as_mut().unwrap().entity.route_length = 3;
    for tick in 1..=planned_ticks {
        bump_rebuild(&mut client, &mut snapshot);
        let outcome = traveller.follow(&mut driver, &snapshot, route.clone(), &mut options);
        assert!(
            outcome.is_none(),
            "planned-duration tick {tick}: {outcome:?}"
        );
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
fn two_tile_oscillation_ends_within_the_original_budget_not_2000_ticks() {
    non_progressing_movement_expires(|tick| (0, if tick % 2 == 0 { 1 } else { 2 }));
}

#[test]
fn movement_away_from_the_aim_ends_within_the_original_budget() {
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
    for tick in 1..=2000 {
        let (x, z) = position(tick);
        plant_player(&mut client, x, z);
        bump_rebuild(&mut client, &mut snapshot);
        let outcome = traveller.follow(&mut driver, &snapshot, route.clone(), &mut options);
        if tick <= budget {
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
            assert_eq!(tick, budget + 1);
            assert_eq!(driver.walked.len(), 1);
            return;
        }
    }
    panic!("the reviewer's 2000-tick probe must terminate");
}
