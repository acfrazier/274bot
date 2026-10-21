//! Exact-ground chooser and Reach integration regressions.

use super::*;
use crate::native::{ledger, ActionHandle, HostEffect, InteractionReceipt};
use crate::quester::families::tests::{def, local_player, with_tick, with_tick_reach};
use api::snapshot::{
    ChatLineView, GameSnapshot, GroundItemView, ItemActionFamily, ItemContainer, ItemView,
    WorldStateView,
};

fn tile(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

fn row(id: i32, at: WorldTile, here: WorldTile) -> GroundItemView {
    GroundItemView {
        def: def(id, "Holiday item"),
        count: 1,
        actions: vec![Some("Take".into())],
        tile: at,
        distance: (at.x - here.x).abs().max((at.z - here.z).abs()),
    }
}

fn snapshot(here: WorldTile, rows: Vec<GroundItemView>) -> GameSnapshot {
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_world(WorldStateView {
        map_base_x: 3168,
        map_base_z: 3168,
        ..Default::default()
    });
    snapshot.seed_local_player(local_player(here));
    snapshot.seed_inventory(vec![], 28);
    snapshot.seed_ground_items(rows);
    snapshot
}

fn flood(here: WorldTile, blocked: &[WorldTile]) -> api::query::ReachQueryView {
    let mut scene = api::snapshot::SceneView {
        available: true,
        base_x: 3180,
        base_z: 3180,
        level: 0,
        width: 72,
        height: 72,
        collision_flags: vec![0; 72 * 72],
    };
    for at in blocked {
        scene.collision_flags
            [((at.x - scene.base_x) * scene.height + at.z - scene.base_z) as usize] |=
            client::dash3d::CollisionFlag::SQ_BLOCKED;
    }
    let flooded = api::query::SceneQuery::new(&scene, Some(here)).flood_reach();
    api::query::pack_reach_query(&scene, flooded.as_ref())
}

fn args(id: i32, target: WorldTile, anchor: WorldTile) -> ReachArgs {
    ReachArgs {
        kind: ReachKind::Ground {
            id,
            obj: "Holiday item".into(),
        },
        op: "Take".into(),
        anchor: Some(anchor),
        radius: 16,
        wait_if_missing: false,
        target_tile: Some(target),
        reachable_only: true,
    }
}

fn dispatch(ledger: &mut Option<Box<ledger::Ledger>>, tick: u64, chat_since: i32) -> u64 {
    let authority = ledger.as_ref().unwrap().outbox.last().unwrap().authority();
    ledger.as_mut().unwrap().complete_interaction(
        &authority,
        InteractionReceipt {
            request_id: authority.request_id().get(),
            evidence: EvidenceStamp {
                run: authority.run(),
                tick,
                sequence: tick,
            },
            accepted: true,
            chat_since,
        },
    );
    authority.request_id().get()
}

fn begin(
    id: i32,
    at: WorldTile,
    snapshot: &GameSnapshot,
    reach: &api::query::ReachQueryView,
    ledger: &mut Option<Box<ledger::Ledger>>,
) -> ActionHandle<GroundTake> {
    with_tick_reach(snapshot, reach, ledger, 1, |tick| {
        tick.actions
            .begin::<GroundTake>(args(id, at, at), &mut tick.cx)
            .unwrap()
    })
}

fn pack(snapshot: &mut GameSnapshot, id: i32, count: i32) {
    snapshot.seed_inventory(
        vec![ItemView {
            def: def(id, "Holiday item"),
            count,
            slot: 0,
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            actions: vec![],
            component_id: 3214,
        }],
        28,
    );
}

fn poll(
    handle: &ActionHandle<GroundTake>,
    snapshot: &GameSnapshot,
    ledger: &mut Option<Box<ledger::Ledger>>,
    tick: u64,
) -> Poll<Result<TakeOutcome, ActionError>> {
    with_tick(snapshot, ledger, tick, |cx| {
        cx.actions.poll(handle, &mut cx.cx)
    })
}

#[test]
fn exact_square_keeps_radius_and_plane_without_changing_loc_npc_floor() {
    let here = tile(3216, 3216);
    let mut other_plane = tile(3216, 3217);
    other_plane.level = 1;
    let snapshot = snapshot(
        here,
        vec![
            row(1053, tile(3218, 3216), here),
            row(1055, other_plane, here),
            row(1057, tile(3217, 3217), here),
        ],
    );
    let reach = flood(here, &[]);
    let mut ledger = None;
    with_tick_reach(&snapshot, &reach, &mut ledger, 1, |tick| {
        let ids = [1053, 1055, 1057];
        let pick = choose_ground(
            &tick.cx,
            GroundIds::Selected {
                ids: &ids,
                excluded: 0,
                failures: None,
            },
            None,
            WorkArea::at_start(here, 1),
            &Avoid::default(),
        )
        .reachable()
        .unwrap();
        assert_eq!(pick.def.id, 1057);
        assert!(Area::new(Some(here), 1).contains(tile(3226, 3216), 10));
        assert!(!WorkArea::at_start(here, 1).contains(tile(3218, 3216)));
        assert!(!WorkArea::at_start(here, 16).contains(other_plane));
    });
}

#[test]
fn selected_ids_rank_by_walk_depth_then_id_plane_x_z() {
    let here = tile(3216, 3216);
    let far = tile(3222, 3216);
    let near = tile(3216, 3218);
    let snapshot = snapshot(
        here,
        vec![
            row(1053, far, here),
            row(1057, near, here),
            row(1055, near, here),
        ],
    );
    let reach = flood(here, &[]);
    let mut ledger = None;
    with_tick_reach(&snapshot, &reach, &mut ledger, 1, |tick| {
        let ids = [1057, 1053, 1055];
        let pick = choose_ground(
            &tick.cx,
            GroundIds::Selected {
                ids: &ids,
                excluded: 0,
                failures: None,
            },
            None,
            WorkArea::at_start(here, 16),
            &Avoid::default(),
        )
        .reachable()
        .unwrap();
        assert_eq!((pick.def.id, pick.tile, pick.steps), (1055, near, Some(1)));
        let mut avoid = Avoid::default();
        avoid.add(
            AvoidKey::Ground(GroundKey {
                id: 1055,
                tile: near,
            }),
            1,
        );
        let pick = choose_ground(
            &tick.cx,
            GroundIds::Selected {
                ids: &ids,
                excluded: 0,
                failures: None,
            },
            None,
            WorkArea::at_start(here, 16),
            &avoid.at(1),
        )
        .reachable()
        .unwrap();
        assert_eq!(
            pick.def.id, 1057,
            "another ID on the same tile remains eligible"
        );
    });
}

#[test]
fn exact_ground_requires_live_access_and_uses_the_table_stand() {
    let here = tile(3216, 3216);
    let table = tile(3219, 3216);
    let snapshot = snapshot(here, vec![row(1959, table, here)]);
    let reach = flood(here, &[table]);
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 1, |tick| {
        assert!(choose_ground(
            &tick.cx,
            1959,
            Some(table),
            WorkArea::at_start(here, 16),
            &Avoid::default()
        )
        .any()
        .is_none());
    });
    let _handle = with_tick_reach(&snapshot, &reach, &mut ledger, 2, |tick| {
        let pick = choose_ground(
            &tick.cx,
            1959,
            Some(table),
            WorkArea::at_start(here, 16),
            &Avoid::default(),
        )
        .reachable()
        .unwrap();
        assert_eq!(pick.stand, Some(tile(3218, 3216)));
        assert_eq!(pick.steps, Some(2));
        tick.actions
            .begin::<GroundTake>(args(1959, table, here), &mut tick.cx)
            .unwrap()
    });
    let HostEffect::Walk(request) = &ledger.as_ref().unwrap().outbox.last().unwrap().effect else {
        panic!("Take before approach")
    };
    assert_eq!((request.target, request.radius), (tile(3218, 3216), 0));
    struct FallbackReach(Reach);
    impl NativeMachine for FallbackReach {
        type Args = ReachArgs;
        type Output = ();
        fn begin(args: ReachArgs, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
            Reach::begin(args, cx).map(Self)
        }
        fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<(), ActionError>> {
            match self.0.walk_to_target(cx) {
                Ok(()) => Poll::Pending,
                Err(error) => Poll::Ready(Err(error)),
            }
        }
        fn cancel(&mut self) {
            self.0.cancel();
        }
    }
    let mut fallback_ledger = None;
    let fallback = with_tick_reach(&snapshot, &reach, &mut fallback_ledger, 3, |tick| {
        tick.actions
            .begin::<FallbackReach>(args(1959, table, here), &mut tick.cx)
            .unwrap()
    });
    with_tick_reach(&snapshot, &reach, &mut fallback_ledger, 4, |tick| {
        assert!(tick.actions.poll(&fallback, &mut tick.cx).is_pending());
        let HostEffect::Walk(request) = &tick
            .cx
            .ledger
            .as_ref()
            .unwrap()
            .outbox
            .last()
            .unwrap()
            .effect
        else {
            panic!("exact fallback did not walk")
        };
        assert_eq!((request.target, request.radius), (tile(3218, 3216), 0));
    });
}

#[test]
fn exact_target_is_required_at_emission_availability_and_door_approach() {
    let here = tile(3216, 3216);
    let exact = tile(3217, 3216);
    let other = tile(3216, 3217);
    let snapshot = snapshot(here, vec![row(1053, other, here)]);
    let reach = flood(here, &[]);
    let mut ledger = None;
    with_tick_reach(&snapshot, &reach, &mut ledger, 1, |tick| {
        let request = args(1053, exact, here);
        assert!(!target_available(
            &tick.cx,
            &request.kind,
            "Take",
            16,
            Some(here),
            Some(exact)
        ));
        let _owner = tick
            .actions
            .begin::<GroundTake>(request.clone(), &mut tick.cx)
            .unwrap();
        let mut machine = Reach::begin(request, &mut tick.cx).unwrap();
        assert!(machine.walk_to_target(&mut tick.cx).is_err());
        assert!(tick.cx.ledger.as_ref().unwrap().outbox.is_empty());
        assert_eq!(machine.poll(&mut tick.cx), Poll::Ready(Ok(false)));
    });
}

#[test]
fn active_zone_clip_guarantees_sixteen_tiles_at_all_extreme_origin_offsets() {
    let origin = 400;
    let base = (origin - 6) * 8;
    for ox in [-4, 4] {
        for oz in [-4, 4] {
            for ix in [0, 7] {
                for iz in [0, 7] {
                    let here = tile((origin + ox) * 8 + ix, (origin + oz) * 8 + iz);
                    for dx in [-16, 16] {
                        for dz in [-16, 16] {
                            assert!(ground_zone_live(
                                here,
                                (base, base),
                                tile(here.x + dx, here.z + dz)
                            ));
                        }
                    }
                }
            }
        }
    }
    let edge = tile((origin + 4) * 8 + 7, (origin + 4) * 8 + 7);
    assert!(!ground_zone_live(
        edge,
        (base, base),
        tile(edge.x + 17, edge.z)
    ));
}

#[test]
fn posted_stale_zone_row_is_not_a_candidate_even_inside_anchor_square() {
    let here = tile(3247, 3216);
    let anchor = tile(3231, 3216);
    let stale = tile(3215, 3216);
    let snapshot = snapshot(here, vec![row(1959, stale, here)]);
    let reach = flood(here, &[]);
    let mut ledger = None;
    with_tick_reach(&snapshot, &reach, &mut ledger, 1, |tick| {
        assert!(WorkArea::at_start(anchor, 16).contains(stale));
        assert!(choose_ground(
            &tick.cx,
            1959,
            None,
            WorkArea::at_start(anchor, 16),
            &Avoid::default()
        )
        .any()
        .is_none());
    });
}

#[test]
fn unsent_take_has_no_deadline_and_actual_dispatch_gets_four_later_edges() {
    let here = tile(3216, 3216);
    let snapshot = snapshot(here, vec![row(1959, here, here)]);
    let reach = flood(here, &[]);
    let mut ledger = None;
    let handle = begin(1959, here, &snapshot, &reach, &mut ledger);
    assert!(poll(&handle, &snapshot, &mut ledger, 100).is_pending());
    dispatch(&mut ledger, 100, 0);
    for edge in 100..104 {
        assert!(poll(&handle, &snapshot, &mut ledger, edge).is_pending());
    }
    assert_eq!(
        poll(&handle, &snapshot, &mut ledger, 104),
        Poll::Ready(Ok(TakeOutcome::FailedKey))
    );
    assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
}

#[test]
fn exact_cant_reach_never_enters_the_door_clock_or_resends() {
    let here = tile(3216, 3216);
    let mut snapshot = snapshot(here, vec![row(1959, here, here)]);
    let reach = flood(here, &[]);
    let mut ledger = None;
    let handle = begin(1959, here, &snapshot, &reach, &mut ledger);
    dispatch(&mut ledger, 1, 0);
    snapshot.seed_chat_lines(vec![ChatLineView {
        sequence: 1,
        type_: 0,
        username: None,
        text: "I can't reach that!".into(),
    }]);
    for edge in 2..5 {
        assert!(poll(&handle, &snapshot, &mut ledger, edge).is_pending());
    }
    with_tick(&snapshot, &mut ledger, 4, |tick| {
        tick.cx.active_now = Duration::from_secs(900);
        assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
    });
    assert_eq!(
        poll(&handle, &snapshot, &mut ledger, 5),
        Poll::Ready(Ok(TakeOutcome::FailedKey))
    );
    assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
}

#[test]
fn removal_waits_one_edge_and_pack_delta_wins_the_split_drain() {
    let here = tile(3216, 3216);
    let mut snapshot = snapshot(here, vec![row(1959, here, here)]);
    let reach = flood(here, &[]);
    let mut ledger = None;
    let handle = begin(1959, here, &snapshot, &reach, &mut ledger);
    dispatch(&mut ledger, 1, 0);
    snapshot.seed_ground_items(vec![]);
    assert!(poll(&handle, &snapshot, &mut ledger, 2).is_pending());
    assert!(poll(&handle, &snapshot, &mut ledger, 2).is_pending());
    pack(&mut snapshot, 1959, 1);
    assert_eq!(
        poll(&handle, &snapshot, &mut ledger, 3),
        Poll::Ready(Ok(TakeOutcome::Picked { units: 1 }))
    );
    assert_eq!(
        poll(&handle, &snapshot, &mut ledger, 3),
        Poll::Ready(Err(ActionError::Cancelled))
    );
}

#[test]
fn late_input_table_success_lands_before_fourth_edge_failure() {
    let here = tile(3216, 3216);
    let table = tile(3217, 3216);
    let mut snapshot = snapshot(here, vec![row(1959, table, here)]);
    let reach = flood(here, &[table]);
    let mut ledger = None;
    let handle = with_tick_reach(&snapshot, &reach, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<GroundTake>(args(1959, table, here), &mut tick.cx)
            .unwrap()
    });
    dispatch(&mut ledger, 1, 0);
    for edge in 2..4 {
        assert!(poll(&handle, &snapshot, &mut ledger, edge).is_pending());
    }
    snapshot.seed_ground_items(vec![]);
    assert!(poll(&handle, &snapshot, &mut ledger, 4).is_pending());
    pack(&mut snapshot, 1959, 1);
    assert_eq!(
        poll(&handle, &snapshot, &mut ledger, 5),
        Poll::Ready(Ok(TakeOutcome::Picked { units: 1 }))
    );
}

#[test]
fn identical_bound_copy_loss_needs_a_fresh_take_and_unbound_copy_loss_can_land() {
    let here = tile(3216, 3216);
    for bound_copy_lost in [true, false] {
        let mut snapshot = snapshot(here, vec![row(1959, here, here), row(1959, here, here)]);
        let reach = flood(here, &[]);
        let mut ledger = None;
        let handle = begin(1959, here, &snapshot, &reach, &mut ledger);
        let first_request = dispatch(&mut ledger, 1, 0);
        snapshot.seed_ground_items(vec![row(1959, here, here)]);
        for edge in 2..5 {
            assert!(poll(&handle, &snapshot, &mut ledger, edge).is_pending());
        }
        if !bound_copy_lost {
            pack(&mut snapshot, 1959, 1);
        }
        let expected = if bound_copy_lost {
            TakeOutcome::LostRace
        } else {
            TakeOutcome::Picked { units: 1 }
        };
        assert_eq!(
            poll(&handle, &snapshot, &mut ledger, 5),
            Poll::Ready(Ok(expected))
        );
        assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
        if bound_copy_lost {
            let next = with_tick_reach(&snapshot, &reach, &mut ledger, 6, |tick| {
                tick.actions
                    .begin::<GroundTake>(args(1959, here, here), &mut tick.cx)
                    .unwrap()
            });
            let next_request = dispatch(&mut ledger, 6, 0);
            assert!(
                next_request > first_request,
                "remaining copy requires a fresh Take request"
            );
            pack(&mut snapshot, 1959, 1);
            assert_eq!(
                poll(&next, &snapshot, &mut ledger, 7),
                Poll::Ready(Ok(TakeOutcome::Picked { units: 1 }))
            );
            assert_eq!(
                ledger.as_ref().unwrap().outbox.len(),
                1,
                "retired attempt rows are pruned"
            );
        }
    }
}

#[test]
fn only_new_server_refusals_match_and_yoyo_has_no_duplicate_gate() {
    let here = tile(3216, 3216);
    for (id, text, expected) in [
        (
            1419,
            "You already have a scythe, you don't need another one.",
            GroundRefusal::AlreadyOwned { id: 1419 },
        ),
        (
            1037,
            "You don't need another set of bunny ears.",
            GroundRefusal::AlreadyOwned { id: 1037 },
        ),
        (
            4079,
            "You don't have enough inventory space to hold that item.",
            GroundRefusal::Capacity,
        ),
    ] {
        let mut snapshot = snapshot(here, vec![row(id, here, here)]);
        let reach = flood(here, &[]);
        let mut ledger = None;
        let handle = begin(id, here, &snapshot, &reach, &mut ledger);
        dispatch(&mut ledger, 1, 10);
        snapshot.seed_chat_lines(vec![
            ChatLineView {
                sequence: 10,
                type_: 0,
                username: None,
                text: text.into(),
            },
            ChatLineView {
                sequence: 11,
                type_: 2,
                username: Some("Player".into()),
                text: text.into(),
            },
        ]);
        assert!(poll(&handle, &snapshot, &mut ledger, 2).is_pending());
        snapshot.seed_chat_lines(vec![ChatLineView {
            sequence: 12,
            type_: 0,
            username: None,
            text: text.into(),
        }]);
        assert_eq!(
            poll(&handle, &snapshot, &mut ledger, 2),
            Poll::Ready(Err(ActionError::GroundTake(expected)))
        );
    }
    let mut snapshot = snapshot(here, vec![]);
    snapshot.seed_chat_lines(vec![ChatLineView {
        sequence: 1,
        type_: 0,
        username: None,
        text: "You already have a scythe, you don't need another one.".into(),
    }]);
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 1, |tick| {
        assert_eq!(ground_refusal(&tick.cx, 0, 4079), None)
    });
}

#[test]
fn unavailable_scene_and_rebuild_cancel_without_lost_race_or_failure() {
    let here = tile(3216, 3216);
    for rebuild in [true, false] {
        let mut snapshot = snapshot(here, vec![row(1959, here, here)]);
        let reach = flood(here, &[]);
        let mut ledger = None;
        let handle = begin(1959, here, &snapshot, &reach, &mut ledger);
        dispatch(&mut ledger, 1, 0);
        if rebuild {
            snapshot.seed_world(WorldStateView {
                map_base_x: 3176,
                map_base_z: 3168,
                ..Default::default()
            });
        } else {
            snapshot.seed_ingame(1);
        }
        assert_eq!(
            poll(&handle, &snapshot, &mut ledger, 2),
            Poll::Ready(Err(ActionError::Stale))
        );
        assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
    }
}

#[test]
fn selected_ids_use_route_steps_instead_of_air_distance() {
    let here = tile(3216, 3216);
    let near = tile(3218, 3216);
    let farther = tile(3216, 3222);
    let wall: Vec<_> = (3200..=3220).map(|z| tile(3217, z)).collect();
    let reach = flood(here, &wall);
    let snapshot = snapshot(here, vec![row(1053, near, here), row(1055, farther, here)]);
    let mut ledger = None;
    with_tick_reach(&snapshot, &reach, &mut ledger, 1, |tick| {
        let ids = [1053, 1055];
        let pick = choose_ground(
            &tick.cx,
            GroundIds::Selected {
                ids: &ids,
                excluded: 0,
                failures: None,
            },
            None,
            WorkArea::at_start(here, 16),
            &Avoid::default(),
        )
        .reachable()
        .unwrap();
        assert_eq!((pick.def.id, pick.tile), (1055, farther));
    });
}

#[test]
fn selected_id_exclusions_cover_the_last_bit_and_reject_oversized_selections() {
    let here = tile(3216, 3216);
    let reach = flood(here, &[]);
    let ids: Vec<_> = (1959..1992).collect();
    for (length, index, excluded, eligible) in [
        (32, 31, 0, true),
        (32, 31, 1u32 << 31, false),
        (33, 0, 0, false),
        (33, 32, 0, false),
    ] {
        let posted = snapshot(here, vec![row(ids[index], here, here)]);
        let mut ledger = None;
        with_tick_reach(&posted, &reach, &mut ledger, 1, |tick| {
            assert_eq!(
                choose_ground(
                    &tick.cx,
                    GroundIds::Selected {
                        ids: &ids[..length],
                        excluded,
                        failures: None,
                    },
                    None,
                    WorkArea::at_start(here, 16),
                    &Avoid::default(),
                )
                .reachable()
                .is_some(),
                eligible,
                "length={length} index={index}",
            );
        });
    }
}

#[cfg(target_pointer_width = "64")]
#[test]
fn reach_retained_size_stays_within_reviewed_per_bot_budget() {
    // Grinch reserves this inline machine for the run. The measured 64-bit
    // Reach/GroundTake layout is 656 B; the 704 B ceiling allows 48 B headroom.
    const RETAINED_BUDGET: usize = 704;
    let reach = std::mem::size_of::<Reach>();
    let take = std::mem::size_of::<GroundTake>();
    assert!(reach <= RETAINED_BUDGET, "Reach is {reach} B");
    assert!(take <= RETAINED_BUDGET, "GroundTake is {take} B");
}

#[test]
fn borrowed_empty_and_crowded_ground_scans_allocate_nothing() {
    let here = tile(3216, 3216);
    let reach = flood(here, &[]);
    for count in [0, 2048] {
        let snapshot = snapshot(here, (0..count).map(|_| row(1959, here, here)).collect());
        let mut ledger = None;
        with_tick_reach(&snapshot, &reach, &mut ledger, 1, |tick| {
            let ids = [1959];
            let area = WorkArea::at_start(here, 16);
            let avoid = Avoid::default();
            let measured = allocation_counter::measure(|| {
                for _ in 0..100 {
                    let choice = choose_ground(
                        &tick.cx,
                        GroundIds::Selected {
                            ids: &ids,
                            excluded: 0,
                            failures: None,
                        },
                        None,
                        area,
                        &avoid,
                    )
                    .reachable();
                    assert_eq!(choice.is_some(), count > 0);
                    if let Some(choice) = choice {
                        assert!(std::ptr::eq(choice.item, &snapshot.ground_items()[0]));
                    }
                }
            });
            println!("exact-ground rows={count} scans=100 allocations={} retained={}B Reach={}B GroundTake={}B", measured.count_total, measured.bytes_current, std::mem::size_of::<Reach>(), std::mem::size_of::<GroundTake>());
            assert_eq!(measured.count_total, 0);
            assert_eq!(measured.bytes_current, 0);
        });
    }
}

#[test]
fn exact_sixteen_tile_boundary_and_borrowed_run_exclusions_apply() {
    let here = tile(3216, 3216);
    let edge = tile(3232, 3216);
    let outside = tile(3233, 3216);
    let snapshot = snapshot(
        here,
        vec![
            row(1053, outside, here),
            row(1055, edge, here),
            row(1057, here, here),
        ],
    );
    let reach = flood(here, &[]);
    let mut ledger = None;
    let mut failures = FailureLedger::default();
    failures.record_failure(
        GroundKey {
            id: 1057,
            tile: here,
        },
        1,
    );
    with_tick_reach(&snapshot, &reach, &mut ledger, 2, |tick| {
        let ids = [1053, 1055, 1057];
        let filter = GroundIds::Selected {
            ids: &ids,
            excluded: 0,
            failures: Some(&failures),
        };
        let pick = choose_ground(
            &tick.cx,
            filter,
            None,
            WorkArea::at_start(here, 16),
            &Avoid::default(),
        )
        .reachable()
        .unwrap();
        assert_eq!((pick.def.id, pick.tile), (1055, edge));
        let excluded = 1 << 1;
        assert!(choose_ground(
            &tick.cx,
            GroundIds::Selected {
                ids: &ids,
                excluded,
                failures: Some(&failures)
            },
            None,
            WorkArea::at_start(here, 16),
            &Avoid::default()
        )
        .any()
        .is_none());
    });
}

#[test]
fn reach_refusal_with_a_row_decrease_is_a_penalty_free_loss() {
    let here = tile(3216, 3216);
    let mut snapshot = snapshot(here, vec![row(1959, here, here), row(1959, here, here)]);
    let reach = flood(here, &[]);
    let mut ledger = None;
    let handle = begin(1959, here, &snapshot, &reach, &mut ledger);
    dispatch(&mut ledger, 1, 0);
    snapshot.seed_chat_lines(vec![ChatLineView {
        sequence: 1,
        type_: 0,
        username: None,
        text: "I can't reach that!".into(),
    }]);
    snapshot.seed_ground_items(vec![row(1959, here, here)]);
    for edge in 2..5 {
        assert!(poll(&handle, &snapshot, &mut ledger, edge).is_pending());
    }
    assert_eq!(
        poll(&handle, &snapshot, &mut ledger, 5),
        Poll::Ready(Ok(TakeOutcome::LostRace))
    );
    assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
}

#[test]
fn two_yoyo_takes_count_two_pack_deltas_without_duplicate_exclusion() {
    let here = tile(3216, 3216);
    let reach = flood(here, &[]);
    let mut snapshot = snapshot(here, vec![row(4079, here, here)]);
    let mut ledger = None;
    let mut previous_request = None;
    for (send, count) in [(1, 1), (4, 2)] {
        let handle = with_tick_reach(&snapshot, &reach, &mut ledger, send, |tick| {
            tick.actions
                .begin::<GroundTake>(args(4079, here, here), &mut tick.cx)
                .unwrap()
        });
        let request = dispatch(&mut ledger, send, 0);
        assert!(previous_request.is_none_or(|previous| request > previous));
        previous_request = Some(request);
        pack(&mut snapshot, 4079, count);
        assert_eq!(
            poll(&handle, &snapshot, &mut ledger, send + 1),
            Poll::Ready(Ok(TakeOutcome::Picked { units: 1 }))
        );
    }
    assert_eq!(
        ledger.as_ref().unwrap().outbox.len(),
        1,
        "only the current attempt remains queued"
    );
}

/// Exercise the production R289 packet handlers and snapshot publication, not
/// just seeded row/count changes. Each `deliver` is a separate client drain.
mod packets {
    use super::*;
    use client::client::{Client, ClientConfig, ClientPlayer, ClientRevision};
    use client::config::if_type::{ComponentType, IfType, IfTypeMut};
    use client::config::ObjType;
    use client::io::{ClientStream, ServerProt289, SERVER_PROT_SIZES_289};
    use std::io::Write;
    use std::time::{Duration, Instant};

    const ID: i32 = 1;

    struct WireGround {
        client: Client,
        server: std::net::TcpStream,
        snapshot: GameSnapshot,
        here: WorldTile,
        target: WorldTile,
    }

    impl WireGround {
        fn new(table: bool, identical_rows: usize) -> Self {
            let mut client = Client::new_with_revision(
                ClientConfig {
                    host: "127.0.0.1".into(),
                    port: 1,
                    cache_dir: String::new(),
                    members: true,
                    lowmem: true,
                },
                ClientRevision::R289,
            );
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            client.stream = Some(ClientStream::connect("127.0.0.1", address.port()).unwrap());
            client.random_in = None;
            client.ptype = -1;
            let (server, _) = listener.accept().unwrap();
            server.set_nodelay(true).unwrap();
            client.ingame = true;
            client.scene_state = 2;
            client.map_build_base_x = 3168;
            client.map_build_base_z = 3168;
            client.local_player = Some(ClientPlayer::at(48, 48));
            let cache = std::sync::Arc::get_mut(&mut client.cache).unwrap();
            cache.objs.resize_with(2, ObjType::default);
            cache.objs[1].id = ID;
            cache.objs[1].name = "Holiday item".into();
            cache.objs[1].op[2] = Some("Take".into());
            client.side_icon[3] = 1;
            client.set_iface(
                1,
                IfType {
                    id: 1,
                    layer_id: 1,
                    r#type: ComponentType::TYPE_INV,
                    obj_ops: true,
                    ..Default::default()
                },
            );
            client.set_iface_mut(
                1,
                IfTypeMut {
                    link_obj_type: Some(vec![0; 28]),
                    link_obj_number: Some(vec![0; 28]),
                    ..Default::default()
                },
            );
            let here = tile(3216, 3216);
            let mut wire = Self {
                client,
                server,
                snapshot: GameSnapshot::new(),
                here,
                target: tile(here.x + i32::from(table), here.z),
            };
            wire.deliver(ServerProt289::UPDATE_INV_FULL, vec![0, 1, 0, 0]);
            for _ in 0..identical_rows {
                wire.zone(ServerProt289::ZONE_OBJ_ADD, &[0, ID as u8, 0, 1]);
            }
            wire.edge();
            assert_eq!(wire.tick(), 1);
            assert_eq!(wire.snapshot.ground_items().len(), identical_rows);
            assert_eq!(wire.snapshot.inventory_size(), 28);
            wire
        }

        fn deliver(&mut self, opcode: i32, bytes: Vec<u8>) {
            let mut frame = vec![opcode as u8];
            match SERVER_PROT_SIZES_289[opcode as usize] {
                -2 => frame.extend_from_slice(&u16::try_from(bytes.len()).unwrap().to_be_bytes()),
                -1 => frame.push(u8::try_from(bytes.len()).unwrap()),
                size => assert_eq!(bytes.len(), size as usize),
            }
            frame.extend_from_slice(&bytes);
            self.server.write_all(&frame).unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            while !self.client.tcp_in() {
                assert!(
                    self.client.ingame,
                    "the R289 packet must decode without logout"
                );
                assert!(
                    Instant::now() < deadline,
                    "loopback packet did not reach the client"
                );
                std::thread::yield_now();
            }
            assert!(
                self.client.ingame,
                "the R289 packet must decode without logout"
            );
            self.snapshot.rebuild(&self.client);
        }

        fn zone(&mut self, opcode: i32, payload: &[u8]) {
            let mut bytes = vec![
                48,
                48,
                opcode as u8,
                ((self.target.x - self.here.x) << 4) as u8,
            ];
            bytes.extend_from_slice(payload);
            self.deliver(ServerProt289::UPDATE_ZONE_PARTIAL_ENCLOSED, bytes);
        }

        fn edge(&mut self) {
            // No local movement, zero old visible players, new-player sentinel.
            self.deliver(ServerProt289::PLAYER_INFO, vec![0, 0x7f, 0xf0]);
        }

        fn picked_inventory(&mut self) {
            // Component 1, one slot, stored object ID+1, count one.
            self.deliver(ServerProt289::UPDATE_INV_FULL, vec![0, 1, 0, 1, 0, 2, 1]);
        }

        fn remove_one(&mut self) {
            self.zone(ServerProt289::ZONE_OBJ_DEL, &[0, ID as u8]);
        }

        fn tick(&self) -> u64 {
            u64::from(self.snapshot.tick())
        }

        fn begin(&self, ledger: &mut Option<Box<ledger::Ledger>>) -> ActionHandle<GroundTake> {
            let reach = flood(
                self.here,
                if self.target == self.here {
                    &[]
                } else {
                    std::slice::from_ref(&self.target)
                },
            );
            with_tick_reach(&self.snapshot, &reach, ledger, self.tick(), |tick| {
                tick.actions
                    .begin::<GroundTake>(args(ID, self.target, self.here), &mut tick.cx)
                    .unwrap()
            })
        }

        fn poll(
            &self,
            handle: &ActionHandle<GroundTake>,
            ledger: &mut Option<Box<ledger::Ledger>>,
        ) -> Poll<Result<TakeOutcome, ActionError>> {
            super::poll(handle, &self.snapshot, ledger, self.tick())
        }
    }

    #[test]
    fn floor_table_and_late_table_pickups_follow_real_split_packet_drains() {
        // Immediate floor; one-delay table; table delivery after one missed
        // server input phase. The final success batch precedes its PLAYER_INFO.
        for (table, unchanged_edges) in [(false, 0), (true, 1), (true, 3)] {
            let mut wire = WireGround::new(table, 1);
            let mut ledger = None;
            let handle = wire.begin(&mut ledger);
            dispatch(&mut ledger, wire.tick(), 0);
            for _ in 0..unchanged_edges {
                wire.edge();
                assert!(matches!(wire.poll(&handle, &mut ledger), Poll::Pending));
            }
            let before = wire.tick();
            wire.picked_inventory();
            assert_eq!(
                wire.tick(),
                before,
                "inventory-only drains do not invent a game edge"
            );
            assert_eq!(
                wire.snapshot.ground_items().len(),
                1,
                "inventory arrives before OBJ_DEL"
            );
            assert!(matches!(
                wire.poll(&handle, &mut ledger),
                Poll::Ready(Ok(TakeOutcome::Picked { units: 1 }))
            ));
            wire.remove_one();
            wire.edge();
            assert!(wire.snapshot.ground_items().is_empty());
            assert!(!matches!(
                wire.poll(&handle, &mut ledger),
                Poll::Ready(Ok(TakeOutcome::Picked { .. }))
            ));
            assert_eq!(
                ledger.as_ref().unwrap().outbox.len(),
                1,
                "no resend or double count"
            );
        }
    }

    #[test]
    fn unchanged_real_packet_edges_fail_only_at_the_fourth_dispatch_fence() {
        let mut wire = WireGround::new(false, 1);
        let mut ledger = None;
        let handle = wire.begin(&mut ledger);
        dispatch(&mut ledger, wire.tick(), 0);
        for later in 1..=4 {
            wire.edge();
            let result = wire.poll(&handle, &mut ledger);
            if later < 4 {
                assert!(matches!(result, Poll::Pending));
            } else {
                assert!(matches!(result, Poll::Ready(Ok(TakeOutcome::FailedKey))));
            }
            assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
        }
    }

    #[test]
    fn identical_copy_loss_uses_decoded_removals_and_requires_a_fresh_take() {
        for bound_copy_lost in [true, false] {
            let mut wire = WireGround::new(false, 2);
            let mut ledger = None;
            let handle = wire.begin(&mut ledger);
            let first_request = dispatch(&mut ledger, wire.tick(), 0);
            wire.remove_one();
            assert_eq!(wire.snapshot.ground_items().len(), 1);
            assert!(matches!(wire.poll(&handle, &mut ledger), Poll::Pending));
            if bound_copy_lost {
                for later in 1..=4 {
                    wire.edge();
                    let result = wire.poll(&handle, &mut ledger);
                    if later < 4 {
                        assert!(matches!(result, Poll::Pending));
                    } else {
                        assert!(matches!(result, Poll::Ready(Ok(TakeOutcome::LostRace))));
                    }
                }
                let fresh = wire.begin(&mut ledger);
                let fresh_request = dispatch(&mut ledger, wire.tick(), 0);
                assert!(
                    fresh_request > first_request,
                    "remaining copy requires a fresh Take request"
                );
                wire.picked_inventory();
                assert!(matches!(
                    wire.poll(&fresh, &mut ledger),
                    Poll::Ready(Ok(TakeOutcome::Picked { units: 1 }))
                ));
            } else {
                wire.edge();
                wire.picked_inventory();
                assert!(matches!(
                    wire.poll(&handle, &mut ledger),
                    Poll::Ready(Ok(TakeOutcome::Picked { units: 1 }))
                ));
                assert_eq!(
                    ledger.as_ref().unwrap().outbox.len(),
                    1,
                    "original bound copy lands without a second Take"
                );
            }
        }
    }
}
