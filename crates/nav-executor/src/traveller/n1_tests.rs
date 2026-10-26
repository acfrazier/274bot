use super::*;

type StallLog = (String, api::hostlog::Source, api::hostlog::Level, String);

struct StallLogSink(std::sync::Mutex<Vec<StallLog>>);

static STALL_LOGS: StallLogSink = StallLogSink(std::sync::Mutex::new(Vec::new()));

impl api::hostlog::Sink for StallLogSink {
    fn record(&self, record: &api::hostlog::Record<'_>) {
        if let Some(slot) = record
            .slot
            .filter(|slot| slot.starts_with("walk-stall-test-"))
        {
            self.0.lock().unwrap().push((
                slot.to_owned(),
                record.source,
                record.level,
                record.message.to_owned(),
            ));
        }
    }
}

fn stall_logs(slot: &str) -> Vec<StallLog> {
    STALL_LOGS
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|record| record.0 == slot)
        .cloned()
        .collect()
}

#[test]
fn walk_stall_writes_one_info_nav_line_with_hop_facts() {
    assert!(api::hostlog::install_sink(&STALL_LOGS));
    std::thread::spawn(|| {
        for moved in [false, true] {
            for disconnected in [false, true] {
                let slot = format!("walk-stall-test-{moved}-{disconnected}");
                api::hostlog::bind_slot(&slot);
                let mut c = scene_client();
                c.runenergy = 17;
                Arc::get_mut(&mut c.cache)
                    .unwrap()
                    .varps
                    .resize_with(174, Default::default);
                c.var = vec![0; 174];
                c.var[173] = i32::from(moved);
                let mut snapshot = snap_at(&mut c, 0, 0);
                let start = WorldTile { x: 3200, z: 3200, level: 0 };
                let aim = WorldTile { x: 3210, ..start };
                let route = Route {
                    legs: vec![walk_leg(&[(3200, 3200), (3210, 3200)])],
                    dest: aim,
                    ticks: 10.0,
                };
                let mut traveller = Traveller::new();
                let mut driver = FollowRec {
                    route: Some((0, 0)),
                    ..FollowRec::default()
                };
                let mut options = TravelOptions {
                    budget_ticks_per_hop: 8,
                    ..TravelOptions::default()
                };
                assert!(traveller
                    .follow(&mut driver, &snapshot, route.clone(), &mut options)
                    .is_none());
                for tick in 2..=10 {
                    if moved && (tick == 2 || tick == 7) {
                        let x = if tick == 2 { 1 } else { 2 };
                        plant_player(&mut c, x, 0);
                        driver.route = Some((x, 0));
                    }
                    if tick == 4 {
                        c.scene_state = 1;
                        c.bump_gens(ServerProt::REBUILD_NORMAL);
                    } else if tick == 5 {
                        c.scene_state = 2;
                    } else if tick == 8 {
                        // A loc refresh is not another scene rebuild.
                        c.gens.scene += 1;
                    }
                    if disconnected && tick == 10 {
                        c.stream = None;
                    }
                    c.bump_gens(ServerProt::PLAYER_INFO);
                    snapshot.rebuild(&c);
                    api::hostlog::set_tick(snapshot.tick());
                    let outcome =
                        traveller.follow(&mut driver, &snapshot, route.clone(), &mut options);
                    if tick < 10 {
                        assert_eq!(outcome, None);
                        assert!(stall_logs(&slot).is_empty(), "no per-tick stall logging");
                    } else {
                        let why = if disconnected || !moved {
                            HopFailure::Dropped
                        } else {
                            HopFailure::Expired
                        };
                        let end = WorldTile { x: 3200 + if moved { 2 } else { 0 }, ..start };
                        assert_eq!(outcome, Some(TravelOutcome::Stalled { at: end, aiming: aim, why, tries: 2 }));
                        let logs = stall_logs(&slot);
                        assert_eq!(logs.len(), 1);
                        assert_eq!(logs[0].1, api::hostlog::Source::Nav);
                        assert_eq!(logs[0].2, api::hostlog::Level::Info);
                        assert_eq!(logs[0].3, format!(
                            "walk-stall why={why:?} start={start:?} end={end:?} aim={aim:?} first_send_tick=1 last_send_tick=6 tries=2 waited_calls={} idle_ticks=4 last_scene_rebuild_tick=Some(4) run_energy=17 run_mode={moved} last_server_movement_tick={:?}",
                            if disconnected { 8 } else { 9 },
                            moved.then_some(7),
                        ));
                        assert_eq!(traveller.current_aim(), None);
                        let settled = Route { legs: Vec::new(), dest: end, ticks: 0.0 };
                        assert_eq!(traveller.follow(&mut driver, &snapshot, settled, &mut options), Some(TravelOutcome::Arrived { at: end }));
                        assert_eq!(stall_logs(&slot).len(), 1, "the completed stall is not logged again");
                    }
                }
                assert_eq!(driver.walked.len(), 2, "one idle-window resend");
            }
        }
    })
    .join()
    .unwrap();
}

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
            matches!(traveller.follow(&mut driver, &snapshot, route, &mut options), Some(TravelOutcome::Blocked { why, .. }) if why.to_string().contains("30") && why.to_string().contains("29"))
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
        matches!(traveller.follow(&mut driver, &snapshot, web_route(glory_edge()), &mut TravelOptions::default()), Some(TravelOutcome::Blocked { why, .. }) if why.to_string().contains("1712"))
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
        matches!(outcome, Some(TravelOutcome::Blocked { why, .. }) if why.to_string().contains("2552") && why.to_string().contains("inventory control") && !why.to_string().contains("loaded scene"))
    );
    assert_eq!(driver.held_ops, 0);
}
