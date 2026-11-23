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

fn line(length: i32) -> Vec<WorldTile> {
    (0..length)
        .map(|x| WorldTile {
            x: 3200 + x,
            z: 3200,
            level: 0,
        })
        .collect()
}

fn web_edge() -> TransportEdge {
    TransportEdge {
        loc_id: 733,
        open_loc_id: Some(734),
        dir: Some(DoorDir::E),
        option: 1,
        ..door_edge()
    }
}

#[test]
fn the_allowance_of_a_long_multi_transport_route_is_the_sum_of_its_legs() {
    use crate::traveller::allowance::{hop_ticks, route_allowance_ticks, transport_clocks};
    // The Traveller's own defaults: a 60-tick hop clock and 60 hops.
    let options = TravelOptions::default();
    assert_eq!((options.budget_ticks_per_hop, options.max_hops), (60, 60));
    // A hop: 60 + ceil(60 / 4) moving budget, plus the poll that finds it
    // lapsed. A phase clock: 60 + 1.
    assert_eq!(hop_ticks(60), 76);
    // Clocks: Npc, Boat and Glider 3 attempts of 3; a slashable web 3 and
    // 16 retries of 2; others 3.
    assert_eq!(transport_clocks(&cart_edge()), 9);
    assert_eq!(transport_clocks(&web_edge()), 35);
    assert_eq!(transport_clocks(&door_edge()), 3);
    let route = Route {
        dest: WorldTile {
            x: 3299,
            z: 3200,
            level: 0,
        },
        ticks: 0.0,
        legs: vec![
            Leg::Walk { tiles: line(100) },
            Leg::Transport {
                edge: Box::new(cart_edge()),
            },
            Leg::Walk { tiles: line(40) },
            Leg::Transport {
                edge: Box::new(door_edge()),
            },
            Leg::Walk { tiles: line(21) },
            Leg::Transport {
                edge: Box::new(web_edge()),
            },
            Leg::Walk { tiles: line(1) },
            Leg::Walk { tiles: line(2) },
        ],
    };
    let shapes = crate::traveller::allowance::leg_shapes(
        &crate::traveller::RouteLegSource::Legacy(route.legs.iter().cloned().collect()),
    );
    // Walk hops: 100 tiles 1 + ceil(80/15) = 7, 40 tiles 3, 21 tiles 2,
    // 2 tiles 1; the one-tile leg sends nothing. 13 hops and one unsent
    // clock for each of the 4 walk legs: (13 + 4) x 76 = 1,292.
    // Transport: (9 + 3 + 35) clocks x 61 = 2,867.
    // Once: settle 3 x 2, stun window 11, one approach clock 61 and one hop
    // clock 76 = 154. Delivery 2.
    let sum = 1_292 + 2_867 + 154 + 2;
    assert_eq!(route_allowance_ticks(shapes, 60, 60), sum);
    assert_eq!(sum, 4_315);
}

#[test]
fn the_hop_cap_bounds_the_walk_hops_of_the_allowance() {
    use crate::traveller::allowance::{hop_ticks, route_allowance_ticks, LegShape};
    // 40 legs of 100 tiles need 280 hops; the follow ends at 60.
    let legs = vec![LegShape::Walk { tiles: 100 }; 40];
    let uncapped = route_allowance_ticks(legs.clone(), 60, u32::MAX);
    let capped = route_allowance_ticks(legs, 60, 60);
    assert_eq!(uncapped - capped, (280 - 60) * hop_ticks(60));
}

#[test]
fn the_follower_reports_the_allowance_of_each_route_it_starts() {
    let mut client = scene_client();
    let snapshot = snap_at(&mut client, 0, 0);
    let mut driver = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut traveller = Traveller::new();
    let mut options = TravelOptions::default();
    assert_eq!(traveller.take_follow_allowance(), None, "no follow yet");
    let first = walk_route(line(100));
    assert!(traveller
        .follow(&mut driver, &snapshot, first.clone(), &mut options)
        .is_none());
    // One walk leg of 100 tiles: (7 hops + 1 unsent clock) x 76, once 154,
    // delivery 2.
    assert_eq!(traveller.take_follow_allowance(), Some(8 * 76 + 154 + 2));
    assert_eq!(traveller.take_follow_allowance(), None, "reported once");
    // Later polls of the same follow report nothing.
    assert!(traveller
        .follow(&mut driver, &snapshot, first, &mut options)
        .is_none());
    assert_eq!(traveller.take_follow_allowance(), None);
    // A replan starts a new follow with its own route and allowance.
    traveller.clear();
    let second = walk_route(line(21));
    assert!(traveller
        .follow(&mut driver, &snapshot, second, &mut options)
        .is_none());
    assert_eq!(traveller.take_follow_allowance(), Some(3 * 76 + 154 + 2));
}
