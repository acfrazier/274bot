//! Typed native walks through the real host pump: `script_observe`'s receipt
//! delivery and native drain, the route-work service and `step_nav_bot`.
use super::*;
use client::dash3d::CollisionFlag;
use nav_planner::bank_fetch::BankRows;
use script::native::walk::Walk;
use script::native::{
    ActionError, ActionHandle, NativeTick, Script, ScriptFailure, ScriptFlow, WalkEnd, WalkEvent,
    WalkEventKind, WalkReceipt, WalkRequest,
};
use std::task::Poll;

#[derive(Default)]
struct Walker {
    /// Return `Blocked` from every tick while set.
    blocked: bool,
    /// On the next tick, cancel the walk and walk the same tile again.
    rewalk: bool,
    begun: usize,
    walks: usize,
    cross_first: Vec<Arc<str>>,
    protect: bool,
    food_guard: bool,
    disallow_prayer: bool,
    target: Option<WorldTile>,
    later_target: Option<WorldTile>,
    radius: u16,
    arrival: nav_planner::arrival::ArrivalKind,
    options: script::native::WalkOptions,
    candidate_goals: Option<script::native::WalkGoals>,
    result: Option<Result<WalkReceipt, ActionError>>,
    results: Vec<Result<WalkReceipt, ActionError>>,
    events: Vec<WalkEvent>,
}

/// One native `Walk` to `(4, 0, 0)`, begun on the first eligible tick and
/// polled on every later one; the terminal lands in the shared cell.
struct WalkerScript {
    shared: Arc<parking_lot::Mutex<Walker>>,
    handle: Option<ActionHandle<Walk>>,
}

impl Script for WalkerScript {
    fn configure(
        &mut self,
        _next: Arc<script::native::PreparedConfig>,
    ) -> Result<script::native::SettingsApply, script::native::ConfigError> {
        Ok(script::native::SettingsApply::Applied)
    }

    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        let mut shared = self.shared.lock();
        if std::mem::take(&mut shared.rewalk) {
            if let Some(handle) = self.handle.take() {
                tick.actions.cancel(handle);
            }
            shared.begun = 0;
        }
        if let Some(handle) = &self.handle {
            while let Some(event) = tick.actions.take_walk_event(handle, &mut tick.cx) {
                shared.events.push(event);
            }
            if let Poll::Ready(result) = tick.actions.poll(handle, &mut tick.cx) {
                shared.result = Some(result.clone());
                shared.results.push(result);
                self.handle = None;
            }
        } else if shared.begun < shared.walks {
            let first_walk = shared.begun == 0;
            let cross = if first_walk {
                shared.cross_first.clone()
            } else {
                Vec::new()
            };
            let target = if first_walk {
                shared.target.unwrap_or(WorldTile {
                    x: 4,
                    z: 0,
                    level: 0,
                })
            } else {
                shared.later_target.unwrap_or(WorldTile {
                    x: 4,
                    z: 0,
                    level: 0,
                })
            };
            shared.begun += 1;
            let request = WalkRequest {
                target,
                candidate_goals: shared.candidate_goals.clone(),
                radius: shared.radius,
                arrival: shared.arrival,
                loc_id: None,
                options: shared.options,
                required_after: tick.cx.evidence(),
                evidence: None,
                cross: cross.into_boxed_slice(),
                protect: shared.protect,
                food_guard: shared.food_guard,
                allow: script::native::WalkAllow {
                    prayer: !shared.disallow_prayer,
                    ..Default::default()
                },
            };
            match tick.actions.begin::<Walk>(request, &mut tick.cx) {
                Ok(handle) => self.handle = Some(handle),
                Err(error) => {
                    let result = Err(error);
                    shared.result = Some(result.clone());
                    shared.results.push(result);
                }
            }
        }
        if shared.blocked {
            return Ok(ScriptFlow::Blocked(ScriptFailure {
                code: "test-blocked".into(),
                message: "blocked by the card".into(),
            }));
        }
        Ok(ScriptFlow::Continue)
    }
}

struct Rig {
    scripts: ScriptWall,
    cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    navs: Arc<Mutex<HashMap<String, NavBot>>>,
    statuses: Arc<Mutex<Vec<SlotStatus>>>,
    world: Option<Arc<NavWorld>>,
    shared: Arc<parking_lot::Mutex<Walker>>,
    driver: NavRec,
    client: client::client::Client,
    snapshot: GameSnapshot,
}

fn rig(world: Option<Arc<NavWorld>>, blocked: bool) -> Rig {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let shared = Arc::new(parking_lot::Mutex::new(Walker {
        blocked,
        walks: 1,
        // These pre-admission geometry/guard fixtures intentionally use a
        // legacy no-zone grid. Only explicit Proceed can arm that grid now.
        options: script::native::WalkOptions {
            allow_danger_zones: if world
                .as_ref()
                .is_some_and(|world| world.graph.zones.is_none())
            {
                script::native::WalkBit::Allow
            } else {
                script::native::WalkBit::Inherit
            },
            ..Default::default()
        },
        ..Walker::default()
    }));
    let slot = script_slot_or_insert(&scripts, "alice");
    {
        let mut slot = slot.lock().unwrap();
        slot.start_test_script(
            Box::new(WalkerScript {
                shared: Arc::clone(&shared),
                handle: None,
            }),
            None,
        )
        .unwrap();
    }
    let work_scope = slot.lock().unwrap().work_scope();
    let bot = NavBot {
        work_scope,
        ..NavBot::default()
    };
    let navs = Arc::new(Mutex::new(HashMap::from([("alice".into(), bot)])));
    let mut client = nav_client();
    let mut snapshot = GameSnapshot::new();
    nav_snapshot_at(&mut client, &mut snapshot, 0, 0);
    Rig {
        scripts,
        cheats: Arc::new(Mutex::new(HashMap::new())),
        navs,
        statuses: Arc::new(Mutex::new(vec![SlotStatus {
            username: "alice".into(),
            ..SlotStatus::default()
        }])),
        world,
        shared,
        driver: NavRec::default(),
        client,
        snapshot,
    }
}

fn open_rig(blocked: bool) -> Rig {
    rig(Some(Arc::new(open_world(40, 1))), blocked)
}
#[test]
fn native_near_walk_after_compat_tile_walk_uses_arrival_endpoint() {
    let mut rig = open_rig(false);
    let target = WorldTile {
        x: 20,
        z: 0,
        level: 0,
    };
    let scope = rig.slot().lock().unwrap().work_scope();
    rig.navs
        .lock()
        .unwrap()
        .entry("alice".into())
        .or_default()
        .work_scope = scope;
    let compat_arm = crate::script_runtime::ScriptWalkArm {
        here: Some((0, 0, 0)),
        world: rig.world.clone(),
        navs: Arc::clone(&rig.navs),
        name: "alice".to_owned(),
        state: None,
        bank: BankRows::default(),
    };
    assert!(compat_arm.queue_route_in_snapshot_avoiding(
        &rig.snapshot,
        target.x,
        target.z,
        target.level,
        nav_planner::router::FindOptions::default(),
        1,
        900,
        true,
        crate::script_runtime::ScriptRouteExclusions::default(),
    ));
    assert!(
        rig.wait_for_route(5_000, || queued(&rig.navs) == Some(target)),
        "the compatibility walk first routes to its requested tile"
    );
    assert!(
        rig.navs.lock().unwrap()["alice"].route_compat_tile_intent,
        "the preceding compatibility walk owns tile intent"
    );
    let compatibility_generation = rig.navs.lock().unwrap()["alice"].route_generation;
    {
        let mut shared = rig.shared.lock();
        shared.target = Some(target);
        shared.radius = 1;
        shared.arrival = nav_planner::arrival::ArrivalKind::Reach;
    }

    rig.observe(1);
    assert!(
        rig.wait_for_route(5_000, || {
            rig.navs.lock().unwrap().get("alice").is_some_and(|bot| {
                bot.route_generation != compatibility_generation
                    && bot.route_request_id == bot.walk_request_id
                    && !bot.route_work_pending()
                    && bot.pending_route.is_none()
                    && bot.has_route()
            })
        }),
        "the native route replaces the preceding compatibility route"
    );
    let navs = rig.navs.lock().unwrap();
    let bot = &navs["alice"];
    let destination = bot.route_destination().expect("the native walk is routed");
    assert!(
        !bot.route_compat_tile_intent,
        "native requests reset the previous compatibility intent"
    );
    assert_ne!(destination, target);
    assert_eq!(
        destination
            .x
            .abs_diff(target.x)
            .max(destination.z.abs_diff(target.z)),
        1,
        "native radius walking still ends on its arrival-side approach tile"
    );
}

#[test]
fn stale_route_terminal_cannot_complete_a_native_walk_with_the_same_request_id() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    {
        let mut navs = rig.navs.lock().unwrap();
        let bot = navs.get_mut("alice").unwrap();
        let request_id = bot.native_walk.as_ref().unwrap().request_id().get();
        let previous_generation = bot.route_generation.wrapping_sub(1);
        // Compatibility walks and native actions have separate request-id
        // sequences. A retained compatibility terminal can reuse this id.
        bot.walk_outcome_seq = bot.walk_outcome_seq.wrapping_add(1);
        bot.walk_outcome_generation = previous_generation;
        bot.walk_outcome_request_id = request_id;
        bot.walk_outcome_failed = false;
        bot.walk_outcome_blocked = false;
    }

    rig.observe(2);
    assert!(
        rig.shared.lock().result.is_none(),
        "a previous route generation must not finish the current native walk"
    );

    let target = WorldTile {
        x: 4,
        z: 0,
        level: 0,
    };
    rig.step_at(target);
    rig.observe_at(3, target);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::Arrived)));
}

#[test]
fn candidate_walk_returns_and_settles_the_chosen_goal() {
    let mut rig = rig(Some(Arc::new(open_world(40, 40))), false);
    // Client scene borders are blocked. Use an actual interior Area stand.
    let tile = |x| WorldTile { x, z: 4, level: 0 };
    {
        let mut shared = rig.shared.lock();
        shared.target = Some(tile(20));
        shared.arrival = nav_planner::contract::ArrivalKind::Area;
        shared.candidate_goals =
            Some(script::native::WalkGoals::new(vec![tile(8), tile(4)]).unwrap());
    }
    rig.observe(1);
    rig.wait_routed();
    assert_eq!(
        rig.navs.lock().unwrap()["alice"].route_destination(),
        Some(tile(4))
    );
    rig.step_at(tile(4));
    rig.observe_at(2, tile(4));
    let shared = rig.shared.lock();
    let receipt = shared.result.as_ref().unwrap().as_ref().unwrap();
    assert_eq!(receipt.end, WalkEnd::Arrived);
    assert_eq!(receipt.goal, Some(tile(4)));
}

#[test]
fn candidate_walk_keeps_preferred_stand_and_reports_its_tier() {
    let tile = |x| WorldTile { x, z: 4, level: 0 };
    for (preferred, goal, tier) in [
        (tile(8), tile(8), "candidate_goal_tier=preferred"),
        (tile(44), tile(4), "candidate_goal_tier=fallback"),
    ] {
        let mut rig = rig(Some(Arc::new(open_world(40, 40))), false);
        {
            let mut shared = rig.shared.lock();
            shared.target = Some(tile(20));
            shared.arrival = nav_planner::contract::ArrivalKind::Area;
            shared.candidate_goals = Some(
                script::native::WalkGoals::with_preferred(vec![preferred, tile(4)], &[preferred])
                    .unwrap(),
            );
        }
        rig.observe(1);
        rig.wait_routed();
        assert_eq!(
            rig.navs.lock().unwrap()["alice"]
                .walk_outcome_detail
                .as_deref(),
            Some(tier),
            "the host publishes the selected tier before follow starts"
        );
        assert_eq!(
            rig.navs.lock().unwrap()["alice"].route_destination(),
            Some(goal),
            "preferred={preferred:?}: a reachable preferred stand outranks the nearer fallback"
        );
        rig.step_at(goal);
        rig.observe_at(2, goal);
        let shared = rig.shared.lock();
        let receipt = shared.result.as_ref().unwrap().as_ref().unwrap();
        assert_eq!(receipt.end, WalkEnd::Arrived);
        assert_eq!(receipt.goal, Some(goal));
        assert_eq!(receipt.detail.as_deref(), Some(tier));
    }
}

#[test]
fn candidate_walk_rejects_a_radius_instead_of_changing_its_meaning() {
    let mut rig = open_rig(false);
    {
        let mut shared = rig.shared.lock();
        shared.radius = 1;
        shared.arrival = nav_planner::contract::ArrivalKind::Area;
        shared.candidate_goals = Some(
            script::native::WalkGoals::new(vec![WorldTile {
                x: 4,
                z: 0,
                level: 0,
            }])
            .unwrap(),
        );
    }
    rig.observe(1);
    assert_eq!(
        rig.shared
            .lock()
            .result
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap_err(),
        &ActionError::WalkGoals(script::native::WalkGoalsError::IncompatibleArrival),
    );
    assert!(!rig.navs.lock().unwrap()["alice"].route_work_pending());
}

#[test]
fn native_follow_failure_preserves_hop_detail_in_receipt() {
    for (why, expected, name) in [
        (
            nav_executor::HopFailure::Expired,
            script::native::WalkStallReason::Expired,
            "Expired",
        ),
        (
            nav_executor::HopFailure::Dropped,
            script::native::WalkStallReason::Dropped,
            "Dropped",
        ),
    ] {
        let mut rig = open_rig(false);
        rig.observe(1);
        rig.wait_routed();
        let failure = nav_executor::TravelOutcome::Stalled {
            at: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            aiming: WorldTile {
                x: 4,
                z: 0,
                level: 0,
            },
            why,
            tries: 3,
            last_refusal: None,
        };
        {
            let mut navs = rig.navs.lock().unwrap();
            super::apply_nav_follow_outcome(navs.get_mut("alice").unwrap(), Some(failure), false);
        }
        rig.observe(2);
        let shared = rig.shared.lock();
        let receipt = shared.result.as_ref().unwrap().as_ref().unwrap();
        assert_eq!(
            receipt.end,
            WalkEnd::Stalled(Arc::new(script::native::WalkStall {
                at: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                aiming: WorldTile {
                    x: 4,
                    z: 0,
                    level: 0,
                },
                why: expected,
                tries: 3,
            }))
        );
        let detail = receipt
            .failure_detail()
            .expect("hop failure detail is owed to the caller");
        assert!(detail.contains(name), "{detail}");
        assert!(detail.contains("tries: 3"), "{detail}");
        assert!(detail.contains("aiming"), "{detail}");
    }
}

#[test]
fn native_follow_failure_names_the_last_crossing_refusal_in_the_receipt() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    let at = WorldTile {
        x: 0,
        z: 0,
        level: 0,
    };
    let aiming = WorldTile {
        x: 4,
        z: 0,
        level: 0,
    };
    {
        let mut navs = rig.navs.lock().unwrap();
        super::apply_nav_follow_outcome(
            navs.get_mut("alice").unwrap(),
            Some(nav_executor::TravelOutcome::Stalled {
                at,
                aiming,
                why: nav_executor::HopFailure::Dropped,
                tries: 7,
                last_refusal: Some(nav_executor::TransportRefusal::Busy(
                    "The ledge is being used",
                )),
            }),
            false,
        );
    }
    rig.observe(2);
    let shared = rig.shared.lock();
    let receipt = shared.result.as_ref().unwrap().as_ref().unwrap();
    assert_eq!(
        receipt.end,
        WalkEnd::Stalled(Arc::new(script::native::WalkStall {
            at,
            aiming,
            why: script::native::WalkStallReason::Dropped,
            tries: 7,
        }))
    );
    let detail = receipt
        .failure_detail()
        .expect("the stall detail is owed to the caller");
    assert!(detail.contains("Dropped"), "{detail}");
    assert!(detail.contains("tries: 7"), "{detail}");
    assert!(detail.contains("The ledge is being used"), "{detail}");
}

#[test]
fn native_end_blocked_follow_preserves_typed_stall_in_receipt() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    let at = WorldTile {
        x: 0,
        z: 0,
        level: 0,
    };
    let aiming = WorldTile {
        x: 4,
        z: 0,
        level: 0,
    };
    {
        let mut navs = rig.navs.lock().unwrap();
        super::apply_nav_follow_outcome(
            navs.get_mut("alice").unwrap(),
            Some(nav_executor::TravelOutcome::Stalled {
                at,
                aiming,
                why: nav_executor::HopFailure::EndBlocked,
                tries: 0,
                last_refusal: None,
            }),
            false,
        );
    }
    rig.observe(2);
    let shared = rig.shared.lock();
    let receipt = shared.result.as_ref().unwrap().as_ref().unwrap();
    assert_eq!(
        receipt.end,
        WalkEnd::Stalled(Arc::new(script::native::WalkStall {
            at,
            aiming,
            why: script::native::WalkStallReason::EndBlocked,
            tries: 0,
        }))
    );
}

#[test]
fn stale_and_legacy_follow_failures_do_not_acquire_native_detail() {
    let failure = || nav_executor::TravelOutcome::Stalled {
        at: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        aiming: WorldTile {
            x: 4,
            z: 0,
            level: 0,
        },
        why: nav_executor::HopFailure::Dropped,
        tries: 3,
        last_refusal: None,
    };

    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    {
        let mut navs = rig.navs.lock().unwrap();
        let bot = navs.get_mut("alice").unwrap();
        bot.walk_outcome_detail = Some(Arc::from("newer correlated outcome"));
        bot.walk_request_id = bot.walk_request_id.wrapping_add(1);
        super::apply_nav_follow_outcome(bot, Some(failure()), false);
        assert_eq!(
            bot.walk_outcome_detail.as_deref(),
            Some("newer correlated outcome")
        );
        assert!(bot.native_walk_failure.is_none());
    }

    let mut legacy = NavBot {
        requested_route: Some((
            WorldTile {
                x: 4,
                z: 0,
                level: 0,
            },
            1,
            false,
            false,
            false,
            Default::default(),
        )),
        ..Default::default()
    };
    super::apply_nav_follow_outcome(&mut legacy, Some(failure()), false);
    assert!(legacy.native_walk_failure.is_none());
    assert!(
        legacy.walk_outcome_detail.is_none(),
        "a legacy route has no native receipt detail"
    );
}

fn zoned_open_world() -> NavWorld {
    let mut world = open_world(40, 1);
    let zones = vec![nav_planner::zones::Zone::npc(
        WorldTile {
            x: 2,
            z: 0,
            level: 0,
        },
        0,
        nav_planner::zones::ZoneClass::Always,
        u16::MAX,
        0,
    )];
    let kinds = vec![nav_planner::zones::ZoneKind::new(
        "test-barrier",
        "Test barrier",
        123,
        0,
        false,
        false,
    )];
    let table = nav_planner::zones::ZoneTable::from_parts(
        zones,
        kinds,
        vec![],
        vec![],
        vec![],
        world.collision.origin,
        world.collision.width as u32,
        world.collision.height as u32,
        &world.graph.wilderness,
    )
    .unwrap();
    world.graph.zones = Some(table);
    world
}
fn catalog_open_world() -> NavWorld {
    let mut world = open_world(40, 10);
    let tile = |x, z| WorldTile { x, z, level: 0 };
    let mut zones = vec![
        nav_planner::zones::Zone::npc(
            tile(2, 0),
            0,
            nav_planner::zones::ZoneClass::Always,
            u16::MAX,
            0,
        ),
        nav_planner::zones::Zone::npc(
            tile(10, 2),
            1,
            nav_planner::zones::ZoneClass::Always,
            u16::MAX,
            0,
        ),
        nav_planner::zones::Zone::npc(
            tile(20, 2),
            1,
            nav_planner::zones::ZoneClass::Always,
            u16::MAX,
            0,
        ),
        nav_planner::zones::Zone::npc(
            tile(10, 6),
            1,
            nav_planner::zones::ZoneClass::Always,
            u16::MAX,
            0,
        ),
        nav_planner::zones::Zone::npc(
            tile(20, 6),
            1,
            nav_planner::zones::ZoneClass::Always,
            u16::MAX,
            0,
        ),
        nav_planner::zones::Zone::npc(
            tile(30, 0),
            0,
            nav_planner::zones::ZoneClass::Always,
            u16::MAX,
            0,
        ),
    ];
    zones[0].group = 0;
    for zone in &mut zones[1..5] {
        zone.group = 1;
    }
    zones[5].group = 2;
    let kinds = vec![nav_planner::zones::ZoneKind::new(
        "test-barrier",
        "Test barrier",
        123,
        0,
        false,
        false,
    )];
    let groups = vec![
        nav_planner::zones::ZoneGroup::new(
            "white-wolf-mountain",
            "White Wolf Mountain",
            nav_planner::router::AvoidRect {
                min_x: 2,
                max_x: 2,
                min_z: 0,
                max_z: 0,
                level: Some(0),
            },
            vec![0].into_boxed_slice(),
        ),
        nav_planner::zones::ZoneGroup::new(
            "draynor-jail-guards",
            "Draynor jail guards",
            nav_planner::router::AvoidRect {
                min_x: 9,
                max_x: 21,
                min_z: 1,
                max_z: 7,
                level: Some(0),
            },
            vec![1, 2, 3, 4].into_boxed_slice(),
        ),
        nav_planner::zones::ZoneGroup::new(
            "death-plateau-throwers",
            "Death Plateau thrower trolls",
            nav_planner::router::AvoidRect {
                min_x: 30,
                max_x: 30,
                min_z: 0,
                max_z: 0,
                level: Some(0),
            },
            vec![5].into_boxed_slice(),
        ),
    ];
    let table = nav_planner::zones::ZoneTable::from_parts(
        zones,
        kinds,
        groups,
        vec![],
        vec![],
        world.collision.origin,
        world.collision.width as u32,
        world.collision.height as u32,
        &world.graph.wilderness,
    )
    .unwrap();
    world.graph.zones = Some(table);
    world
}

fn add_selection_zone(
    world: &mut NavWorld,
    zone: nav_planner::zones::Zone,
    carve: Option<nav_planner::router::AvoidRect>,
) {
    let carves = carve.map_or_else(Vec::new, |carve| vec![(0, carve)]);
    let (npc_id, vislevel) = if zone.class == nav_planner::zones::ZoneClass::LevelRule {
        (0, zone.cap / 2)
    } else {
        (-1, 0)
    };
    let table = nav_planner::zones::ZoneTable::from_parts(
        vec![zone],
        vec![nav_planner::zones::ZoneKind::new(
            "test-danger",
            "Test danger",
            npc_id,
            vislevel,
            false,
            false,
        )],
        vec![],
        carves,
        vec![],
        world.collision.origin,
        u32::try_from(world.collision.width).expect("selection test width fits zone table"),
        u32::try_from(world.collision.height).expect("selection test height fits zone table"),
        &world.graph.wilderness,
    )
    .expect("selection test zone");
    world.graph.zones = Some(table);
}

fn selection_zone_world(
    target: WorldTile,
    zone: nav_planner::zones::Zone,
    carve: Option<nav_planner::router::AvoidRect>,
    wilderness: nav_planner::transport::WildernessRules,
) -> NavWorld {
    let mut world = open_world_at(target.x - 8, target.z - 8, 32, 32);
    world.graph.wilderness = wilderness;
    add_selection_zone(&mut world, zone, carve);
    world
}

fn selection_snapshot(
    target: WorldTile,
    combat_level: i32,
    force_approach: i32,
    blocked: Option<WorldTile>,
) -> GameSnapshot {
    const SCENE_WIDTH: i32 = 10;
    const SCENE_HEIGHT: i32 = 10;
    let base_x = target.x - 4;
    let base_z = target.z - 4;
    let mut snapshot = nav_frame_at(target.x, target.z);
    let mut player = snapshot
        .local_player()
        .expect("nav fixture has a local player")
        .clone();
    player.player.combat_level = combat_level;
    snapshot.seed_local_player(player);

    let mut collision_flags = vec![0; usize::try_from(SCENE_WIDTH * SCENE_HEIGHT).unwrap()];
    if let Some(tile) = blocked {
        let lx = tile.x - base_x;
        let lz = tile.z - base_z;
        let index = usize::try_from(lx * SCENE_HEIGHT + lz).unwrap();
        collision_flags[index] = CollisionFlag::SQ_BLOCKED;
    }
    snapshot.seed_scene(api::snapshot::SceneView {
        available: true,
        base_x,
        base_z,
        level: 0,
        width: SCENE_WIDTH,
        height: SCENE_HEIGHT,
        collision_flags,
    });
    snapshot.seed_locs(vec![api::snapshot::LocView {
        typecode: 10,
        info: 0,
        id: 1276,
        name: Some("Tree".into()),
        description: None,
        actions: vec![Some("Chop down".into())],
        tile: target,
        distance: 0,
        layer: api::snapshot::LocLayer::Ground,
        shape: 10,
        angle: 0,
        width: 2,
        length: 2,
        footprint_width: 2,
        footprint_length: 2,
        block_walk: false,
        block_range: false,
        active: true,
        animation: -1,
        map_function: -1,
        map_scene: -1,
        force_approach,
    }]);
    snapshot
}

fn selection_danger(
    world: &NavWorld,
    snapshot: Option<&GameSnapshot>,
    target: WorldTile,
    loc_id: Option<i32>,
    radius: u16,
    arrival: nav_planner::contract::ArrivalKind,
) -> Result<(), String> {
    use script::native::{WalkDanger, WalkDangerQuery};

    let navs = Arc::new(Mutex::new(HashMap::new()));
    let query =
        crate::walk_permissions::SelectionDangerQuery::new(&navs, "alice", world, snapshot, None);
    match query.check(target, loc_id, radius, arrival) {
        WalkDanger::Allowed => Ok(()),
        WalkDanger::Denied { zone } => Err(zone.to_owned()),
        WalkDanger::Unavailable => panic!("test query has complete navigation authority"),
    }
}

fn avoid_rect(min_x: i32, max_x: i32, min_z: i32, max_z: i32) -> nav_planner::router::AvoidRect {
    nav_planner::router::AvoidRect {
        min_x,
        max_x,
        min_z,
        max_z,
        level: Some(0),
    }
}

fn packed_collision_scene(world: &NavWorld, target: WorldTile) -> GameSnapshot {
    let mut snapshot = selection_snapshot(target, 3, 0, None);
    let base_x = target.x - 4;
    let base_z = target.z - 4;
    let width = 10;
    let height = 10;
    let mut collision_flags = Vec::with_capacity(usize::try_from(width * height).unwrap());
    for lx in 0..width {
        for lz in 0..height {
            collision_flags.push(
                i32::try_from(world.collision.walkable_word(base_x + lx, base_z + lz, 0))
                    .expect("packed collision word fits scene flags"),
            );
        }
    }
    snapshot.seed_scene(api::snapshot::SceneView {
        available: true,
        base_x,
        base_z,
        level: 0,
        width,
        height,
        collision_flags,
    });
    snapshot
}

impl Rig {
    fn observe(&mut self, tick: u64) {
        self.observe_with_here(
            tick,
            WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
        );
    }

    fn rebuild_snapshot_at(&mut self, here: WorldTile) {
        let x = here.x - self.client.map_build_base_x;
        let z = here.z - self.client.map_build_base_z;
        let mut player = client::dash3d::ClientPlayer::at(x, z);
        player.x = x * 128 + player.size * 64;
        player.z = z * 128 + player.size * 64;
        self.client.local_player = Some(player);
        self.client.bump_gens(client::io::ServerProt::PLAYER_INFO);
        self.client
            .bump_gens(client::io::ServerProt::REBUILD_NORMAL);
        self.snapshot.rebuild(&self.client);
    }

    fn observe_at(&mut self, tick: u64, here: WorldTile) {
        self.rebuild_snapshot_at(here);
        self.observe_with_here(tick, here);
    }

    fn observe_with_here(&mut self, tick: u64, here: WorldTile) {
        script_observe(
            &mut self.driver,
            "alice",
            true,
            true,
            tick,
            Some((here.x, here.z, here.level)),
            None,
            None,
            Some(&self.snapshot),
            None,
            &self.scripts,
            &self.cheats,
            &self.navs,
            &self.world,
            false,
            false,
        );
        crate::script_runtime::pump_route_work(&self.navs, "alice");
    }

    /// Steps from the snapshot's own (network route-head) tile, the same
    /// position authority the host passes in production.
    fn step(&mut self) {
        crate::script_runtime::pump_route_work(&self.navs, "alice");
        step_nav_bot(
            &mut self.driver,
            "alice",
            self.snapshot.tile(),
            &self.snapshot,
            &self.navs,
            &self.statuses,
            self.world.as_ref(),
            false,
            false,
            None,
            no_reach,
        );
    }
    fn wait_for_route(&self, ms: u64, mut condition: impl FnMut() -> bool) -> bool {
        wait_until(ms, || {
            crate::script_runtime::pump_route_work(&self.navs, "alice");
            condition()
        })
    }

    fn step_at(&mut self, here: WorldTile) {
        self.rebuild_snapshot_at(here);
        self.step();
    }

    fn walk_armed(&self) -> bool {
        self.navs
            .lock()
            .unwrap()
            .get("alice")
            .is_some_and(NavBot::script_walk_armed)
    }

    fn wait_routed(&self) {
        assert!(
            self.wait_for_route(5_000, || queued(&self.navs).is_some()),
            "the route service armed the native route; detail: {:?}",
            self.navs
                .lock()
                .unwrap()
                .get("alice")
                .and_then(|bot| bot.walk_outcome_detail.as_deref())
        );
    }

    fn end(&self) -> Option<Result<WalkEnd, ActionError>> {
        self.shared
            .lock()
            .result
            .clone()
            .map(|result| result.map(|receipt| receipt.end))
    }

    fn slot(&self) -> Arc<Mutex<script::SlotScript>> {
        script_slot(&self.scripts, "alice").unwrap()
    }
}

#[test]
fn route_published_after_an_unarmed_check_steps_the_same_player_key() {
    let mut rig = open_rig(false);
    let key = (rig.client.gens.player, rig.snapshot.tile());
    let mut last = None;
    assert!(!crate::play_slots::nav_step_due(
        &mut last,
        key,
        false,
        false,
        || rig.walk_armed(),
    ));
    rig.observe(1);
    rig.wait_routed();
    let identity = {
        let navs = rig.navs.lock().unwrap();
        let bot = navs.get("alice").unwrap();
        (bot.route_generation, bot.walk_request_id)
    };
    assert!(
        crate::play_slots::nav_step_due(&mut last, key, false, false, || rig.walk_armed()),
        "an unarmed check must not delay this route until the next player key"
    );
    rig.step();
    assert_eq!(rig.driver.move_calls, 1);
    assert!(!crate::play_slots::nav_step_due(
        &mut last,
        key,
        false,
        false,
        || rig.walk_armed(),
    ));
    assert_eq!(rig.shared.lock().begun, 1, "no walk was rearmed");
    let navs = rig.navs.lock().unwrap();
    let bot = navs.get("alice").unwrap();
    assert_eq!((bot.route_generation, bot.walk_request_id), identity);
}

#[test]
fn suppressed_unarmed_key_stays_suppressed_after_route_publication() {
    for (hold, exclusive) in [(true, false), (false, true)] {
        let mut last = None;
        let key = (7, Some((0, 0, 0)));
        assert!(!crate::play_slots::nav_step_due(
            &mut last,
            key,
            hold,
            exclusive,
            || false,
        ));
        assert!(!crate::play_slots::nav_step_due(
            &mut last,
            key,
            false,
            false,
            || true,
        ));
        assert!(crate::play_slots::nav_step_due(
            &mut last,
            (8, key.1),
            false,
            false,
            || true,
        ));
        assert!(!crate::play_slots::nav_step_due(
            &mut last,
            (8, key.1),
            false,
            false,
            || true,
        ));
    }
}

#[test]
fn a_blocked_tick_dispatches_no_native_walk() {
    let mut rig = open_rig(true);
    rig.observe(1);
    assert_eq!(rig.shared.lock().begun, 1, "the tick began a walk");
    assert_eq!(
        rig.slot().lock().unwrap().state(),
        script::RunState::Idle,
        "terminal Blocked uses Stop, not a running dispatch hold"
    );
    assert_eq!(
        rig.slot().lock().unwrap().native_status().unwrap().phase,
        script::native::NativePhase::Blocked
    );
    assert!(
        !rig.walk_armed(),
        "Blocked closes dispatch: the tick's walk must not reach the navigator"
    );
    rig.step();
    assert_eq!(rig.driver.walked, None, "no hop is sent while Blocked");
}

#[test]
fn blocking_mid_walk_stops_the_follow() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    rig.step();
    assert_eq!(rig.driver.walked, Some((4, 0)));
    rig.shared.lock().blocked = true;
    rig.observe(2);
    assert_eq!(rig.slot().lock().unwrap().state(), script::RunState::Idle);
    assert!(!rig.slot().lock().unwrap().want_run);
    assert!(rig.slot().lock().unwrap().native_run().is_none());
    assert_eq!(
        rig.slot()
            .lock()
            .unwrap()
            .native_status()
            .unwrap()
            .failure
            .as_ref()
            .unwrap()
            .message
            .as_ref(),
        "blocked by the card"
    );
    rig.driver.walked = None;
    rig.step();
    assert_eq!(queued(&rig.navs), None, "Blocked revokes the follow");
    assert_eq!(rig.driver.walked, None);
    assert!(rig
        .slot()
        .lock()
        .unwrap()
        .native_quiet_read(Instant::now())
        .is_none());
}
#[test]
fn terminal_blocked_resets_all_navigation_once_per_generation() {
    let mut rig = protected_rig();
    raise_owned_missiles(&mut rig);
    rig.shared.lock().blocked = true;
    rig.observe(4);
    assert_eq!(rig.slot().lock().unwrap().state(), script::RunState::Idle);
    let generation = rig
        .slot()
        .lock()
        .unwrap()
        .terminal_lifecycle_generation()
        .expect("Blocked records a Failed lifecycle");
    let (route_generation, walk_outcome_seq) = {
        let mut navs = rig.navs.lock().unwrap();
        let bot = navs.get_mut("alice").unwrap();
        bot.inspect.generation = 9;
        bot.inspect.pending_id = 17;
        bot.bank_pick.posted = script::native::snapshot_source::BankSelectionInput {
            request_id: 23,
            generation: 4,
            bank_index: 2,
            kind: 1,
        };
        bot.duel_offer_partner = Some("Old partner".into());
        bot.walk_outcome_seq = 31;
        bot.walk_outcome_failed = true;
        bot.walk_outcome_blocked = true;
        (bot.route_generation, bot.walk_outcome_seq)
    };

    rig.observe(5);
    let (reset_route_generation, reset_inspect_generation, reset_walk_outcome_seq) = {
        let mut navs = rig.navs.lock().unwrap();
        let bot = navs.get_mut("alice").unwrap();
        assert_eq!(bot.route_generation, route_generation.wrapping_add(1));
        assert_eq!(bot.inspect.generation, 10);
        assert_eq!(bot.inspect.pending_id, 0);
        assert_eq!(
            bot.bank_pick.posted,
            script::native::snapshot_source::BankSelectionInput::default()
        );
        assert_eq!(bot.duel_offer_partner, None);
        assert!(!bot.walk_outcome_failed);
        assert!(!bot.walk_outcome_blocked);
        assert_eq!(bot.walk_outcome_seq, walk_outcome_seq.wrapping_add(1));
        assert!(bot.walk_guard.is_none());
        assert!(bot.walk_guard_off.is_some());
        assert_eq!(bot.terminal_nav_reset_generation, Some(generation));

        let sentinel = script::native::snapshot_source::BankSelectionInput {
            request_id: 29,
            generation: 5,
            bank_index: 3,
            kind: 2,
        };
        bot.inspect.pending_id = 99;
        bot.bank_pick.posted = sentinel;
        bot.duel_offer_partner = Some("Later partner".into());
        (
            bot.route_generation,
            bot.inspect.generation,
            bot.walk_outcome_seq,
        )
    };
    rig.observe(6);
    let navs = rig.navs.lock().unwrap();
    let bot = navs.get("alice").unwrap();
    assert_eq!(bot.route_generation, reset_route_generation);
    assert_eq!(bot.inspect.generation, reset_inspect_generation);
    assert_eq!(bot.inspect.pending_id, 99);
    assert_eq!(
        bot.bank_pick.posted,
        script::native::snapshot_source::BankSelectionInput {
            request_id: 29,
            generation: 5,
            bank_index: 3,
            kind: 2,
        }
    );
    assert_eq!(bot.duel_offer_partner.as_deref(), Some("Later partner"));
    assert_eq!(bot.walk_outcome_seq, reset_walk_outcome_seq);
}

#[test]
fn a_native_walk_refused_for_lack_of_a_world_gets_a_refused_receipt() {
    let mut rig = rig(None, false);
    rig.observe(1);
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::Refused)));
}

fn seed_prayer(snapshot: &mut GameSnapshot, base: i32) {
    snapshot.seed_stats(vec![api::snapshot::StatView {
        index: 5,
        name: "prayer".into(),
        effective: base,
        base,
        xp: 0,
        used: true,
    }]);
}

#[test]
fn a_protected_walk_refuses_when_prayer_cannot_protect() {
    let mut rig = open_rig(false);
    rig.shared.lock().protect = true;
    seed_prayer(&mut rig.snapshot, 36);
    rig.observe(1);
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::Refused)));
    assert!(
        !rig.navs
            .lock()
            .unwrap()
            .get("alice")
            .is_some_and(|bot| bot.walk_guard.is_some()),
        "a refused protect walk must not arm a driver"
    );
}

#[test]
fn a_protected_walk_arms_the_hold_mode_driver() {
    let mut rig = open_rig(false);
    rig.shared.lock().protect = true;
    seed_prayer(&mut rig.snapshot, 43);
    rig.observe(1);
    rig.wait_routed();
    assert!(
        rig.navs
            .lock()
            .unwrap()
            .get("alice")
            .is_some_and(|bot| bot.walk_guard.is_some()),
        "protect walk must arm WalkGuard before follow"
    );
}

#[test]
fn a_protected_walk_refuses_when_prayer_is_disallowed() {
    let mut rig = open_rig(false);
    rig.shared.lock().protect = true;
    rig.shared.lock().disallow_prayer = true;
    seed_prayer(&mut rig.snapshot, 43);
    rig.observe(1);
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::Refused)));
    assert!(
        !rig.navs
            .lock()
            .unwrap()
            .get("alice")
            .is_some_and(|bot| bot.walk_guard.is_some()),
        "a prayer-disallowed protect walk must not arm a driver"
    );
}

#[test]
fn food_only_abort_walk_routes_with_low_or_disallowed_prayer() {
    for (prayer, disallow_prayer) in [(1, false), (43, true)] {
        let mut rig = open_rig(false);
        rig.shared.lock().disallow_prayer = disallow_prayer;
        rig.shared.lock().food_guard = true;
        seed_protect_frame(&mut rig.snapshot, prayer);
        rig.snapshot.seed_tick(1);
        rig.observe(1);
        rig.wait_routed();
        assert!(
            rig.navs.lock().unwrap()["alice"].walk_guard.is_some(),
            "an abort-shaped food-only walk must retain eating without prayer admission"
        );
        rig.step();
        assert!(rig.walk_armed());
        assert!(rig.driver.move_calls > 0, "the escape must actually walk");
        assert!(rig.driver.if_button_components.is_empty());
        assert_eq!(rig.end(), None);
    }
}

#[test]
fn native_host_walk_uses_never_policy_and_still_eats_when_poisoned() {
    let mut rig = open_rig(false);
    rig.shared.lock().food_guard = true;
    seed_protect_frame(&mut rig.snapshot, 43);
    rig.snapshot.seed_tick(1);
    rig.observe(1);
    rig.wait_routed();
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    rig.snapshot.seed_inventory(
        ["lobster", "1doseantipoison"]
            .into_iter()
            .enumerate()
            .map(|(slot, alias)| {
                let fact = data.item_by_alias(alias).unwrap();
                api::snapshot::ItemView {
                    def: api::ItemDefView {
                        id: fact.id,
                        name: fact.name.clone(),
                        stackable: false,
                        members: true,
                        base_value: 0,
                        noted: false,
                        certificate_link: -1,
                        certificate_template: -1,
                    },
                    slot: slot as i32,
                    count: 1,
                    container: api::snapshot::ItemContainer::Inventory,
                    action_family: api::snapshot::ItemActionFamily::Held,
                    actions: vec![Some(if slot == 0 { "Eat" } else { "Drink" }.into())],
                    component_id: 3214,
                }
            })
            .collect(),
        28,
    );
    let mut stats = rig.snapshot.stats().to_vec();
    stats
        .iter_mut()
        .find(|stat| stat.index == 3)
        .unwrap()
        .effective = 1;
    rig.snapshot.seed_stats(stats);
    rig.snapshot
        .seed_inventory_packet_generations(Some(1), None);
    let drain = host::DrainResult {
        player_info: true,
        npc_rows_coherent: true,
        session_changed: false,
        gens: Default::default(),
        dirty: Default::default(),
    };
    {
        let mut navs = rig.navs.lock().unwrap();
        let bot = navs.get_mut("alice").unwrap();
        bot.cure_poison = script::survival::CurePoison::Never;
        bot.condition.session_started(
            1,
            0,
            Some((script::combat::guard::shared_tables().unwrap(), 11)),
        );
        bot.condition.observe_frame(&rig.snapshot, &drain, false);
        rig.snapshot.seed_tick(2);
        rig.snapshot
            .seed_chat_lines(vec![api::snapshot::ChatLineView {
                type_: 0,
                username: None,
                text: "You have been poisoned!".into(),
                sequence: 1,
            }]);
        bot.condition.observe_frame(&rig.snapshot, &drain, false);
        assert!(matches!(
            bot.condition.current().unwrap().poison,
            api::condition::PoisonState::Onset { .. }
        ));
    }
    rig.step();
    assert_eq!(
        rig.driver.held_ops, 1,
        "Never must still admit the due food"
    );
    assert_eq!(
        rig.driver.held_item_ids,
        [379],
        "the host must use NavBot's Never policy, not drink the held cure",
    );
}

#[test]
fn a_default_walk_with_food_allowed_starts_no_host_guard() {
    for prayer in [1, 43] {
        let mut rig = open_rig(false);
        seed_protect_frame(&mut rig.snapshot, prayer);
        rig.snapshot.seed_tick(1);
        rig.observe(1);
        rig.wait_routed();
        assert!(
            rig.navs.lock().unwrap()["alice"].walk_guard.is_none(),
            "default walks (allow.food=true, no food_guard) must not start the food-only guard"
        );
        rig.step();
        assert!(rig.walk_armed());
        assert!(rig.driver.move_calls > 0);
    }
}

fn seed_protect_frame(snapshot: &mut GameSnapshot, prayer_base: i32) {
    snapshot.seed_ingame(2);
    snapshot.seed_inventory(Vec::new(), 28);
    snapshot.seed_equipment(Vec::new());
    snapshot.seed_stats(
        (0..25)
            .map(|index| api::snapshot::StatView {
                index,
                name: String::new(),
                effective: if index == 5 { prayer_base } else { 40 },
                base: if index == 5 { prayer_base } else { 40 },
                xp: 0,
                used: api::snapshot::stat_used(index as usize),
            })
            .collect(),
    );
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    snapshot.seed_varps(
        data.prayers()
            .iter()
            .map(|row| api::snapshot::VarpView {
                index: row.varp,
                value: 0,
            })
            .collect(),
    );
    snapshot.seed_players(Vec::new());
    snapshot.seed_hitmarks(api::snapshot::HitmarksView {
        marks: [api::snapshot::HitmarkView {
            value: 0,
            kind: 0,
            cycle: 0,
        }; 4],
        loop_cycle: 0,
    });
}

fn seed_prayer_widgets(snapshot: &mut GameSnapshot) {
    use api::snapshot::{WidgetKind, WidgetRoot, WidgetView};
    snapshot.seed_main_modal(
        5608,
        [5621, 5622, 5623]
            .into_iter()
            .map(|component_id| WidgetView {
                kind: WidgetKind::Widget,
                component_id,
                layer_id: 5608,
                parent_id: 5608,
                root_component_id: 5608,
                root: WidgetRoot::Main,
                type_: 4,
                button_type: 1,
                client_code: 0,
                x: 0,
                y: 0,
                width: 20,
                height: 20,
                scroll_height: 0,
                scroll_position: 0,
                hidden: false,
                text: None,
                alternate_text: None,
                button_text: None,
                target_verb: None,
                target_base: None,
                target_mask: 0,
                model_type: 0,
                model_id: 0,
                alternate_model_type: 0,
                alternate_model_id: 0,
                scripts: None,
                script_comparators: None,
                script_operands: None,
                varp_bindings: Vec::new(),
                colour: 0,
                actions: Vec::new(),
                items: Vec::new(),
            })
            .collect(),
    );
}

fn seed_missile_launch(snapshot: &mut GameSnapshot) {
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let row = data.npc_by_config("ardougne_archer").unwrap();
    let here = snapshot
        .tile()
        .map(|(x, z, level)| api::snapshot::WorldTile { x, z, level })
        .unwrap_or(api::snapshot::WorldTile {
            x: 0,
            z: 0,
            level: 0,
        });
    let npc_tile = api::snapshot::WorldTile {
        x: here.x + 1,
        z: here.z,
        level: here.level,
    };
    let me = snapshot
        .local_player()
        .map(|player| player.player.index)
        .unwrap_or(1);
    snapshot.seed_npcs(vec![api::snapshot::NpcView {
        index: 7,
        r#type: Some(row.id as usize),
        name: row.display.clone(),
        actions: vec![Some("Attack".into())],
        tile: npc_tile,
        distance: 1,
        animation: -1,
        animation_frame: -1,
        pose_animation: -1,
        orientation: 0,
        target_orientation: 0,
        overhead_text: None,
        spot_animation: -1,
        spot_animation_stamp: -1,
        health: 50,
        total_health: 50,
        face_entity: -1,
        target: Some(api::snapshot::ActorTargetView {
            kind: api::snapshot::ActorKind::Player,
            index: me,
        }),
        moving: false,
        running: false,
        in_combat: true,
        level: 37,
        size: 1,
        network: npc_tile,
        x: 0,
        z: 0,
        yaw: 0,
    }]);
    snapshot.seed_projectiles(vec![api::snapshot::ProjectileView {
        spotanim: 9,
        level: here.level,
        src: npc_tile,
        target: Some(api::snapshot::ActorTargetView {
            kind: api::snapshot::ActorKind::Player,
            index: me,
        }),
        t1: 32,
        t2: 37,
    }]);
}

fn seed_melee_attack(snapshot: &mut GameSnapshot) {
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let melee_npc = data.npc_by_config("khazard_warlord").unwrap();
    let melee_sequence = data
        .style_seqs()
        .iter()
        .find(|row| row.style & 0x01 != 0)
        .unwrap()
        .seq_id;
    seed_missile_launch(snapshot);
    let mut npcs = snapshot.npcs().to_vec();
    let npc = &mut npcs[0];
    npc.r#type = Some(melee_npc.id as usize);
    npc.name = melee_npc.display.clone();
    npc.in_combat = false;
    npc.animation = melee_sequence;
    npc.animation_frame = 0;
    snapshot.seed_npcs(npcs);
    snapshot.seed_projectiles(Vec::new());
}

fn seed_protect_state(snapshot: &mut GameSnapshot, varp: Option<i32>) {
    let mut rows = snapshot.varps().to_vec();
    for row in &mut rows {
        row.value = i32::from(Some(row.index) == varp);
    }
    snapshot.seed_varps(rows);
}

fn protected_rig() -> Rig {
    let mut rig = open_rig(false);
    rig.shared.lock().protect = true;
    seed_protect_frame(&mut rig.snapshot, 43);
    seed_prayer_widgets(&mut rig.snapshot);
    rig.snapshot.seed_tick(1);
    rig.observe(1);
    rig.wait_routed();
    rig
}

fn raise_owned_missiles(rig: &mut Rig) {
    rig.snapshot.seed_tick(2);
    seed_missile_launch(&mut rig.snapshot);
    rig.step();
    assert_eq!(rig.driver.if_button_components, vec![5622]);
    seed_protect_state(&mut rig.snapshot, Some(96));
    rig.snapshot.seed_projectiles(Vec::new());
    rig.snapshot.seed_npcs(Vec::new());
    rig.snapshot.seed_tick(3);
    rig.step();
}

#[derive(Clone, Copy, Debug)]
enum GuardEndSeam {
    Stop,
    Pause,
    Cancel,
    Manual,
    CarryHold,
    OwnerRevoked,
}

fn check_guard_end(seam: GuardEndSeam) {
    for was_on in [true, false] {
        let mut rig = protected_rig();
        raise_owned_missiles(&mut rig);
        match seam {
            GuardEndSeam::Stop => {
                rig.slot().lock().unwrap().stop();
                reset_script_nav(&rig.navs, "alice", None);
            }
            GuardEndSeam::Pause => {
                pause_script(&mut rig.slot().lock().unwrap(), &rig.navs, "alice");
            }
            GuardEndSeam::Cancel => abort_script_walk(&rig.navs, "alice"),
            GuardEndSeam::Manual => {
                rig.navs
                    .lock()
                    .unwrap()
                    .get_mut("alice")
                    .unwrap()
                    .cancel_for_manual_input();
            }
            GuardEndSeam::CarryHold => hold_script_nav(&rig.navs, "alice", None),
            GuardEndSeam::OwnerRevoked => {
                rig.slot().lock().unwrap().stop();
                rig.step();
            }
        }
        assert_eq!(
            rig.driver.if_button_components,
            vec![5622],
            "{seam:?} cannot click without the pump driver"
        );
        seed_protect_state(&mut rig.snapshot, was_on.then_some(96));
        rig.snapshot.seed_tick(4);
        rig.step();
        let expected = if was_on { vec![5622, 5622] } else { vec![5622] };
        assert_eq!(
            rig.driver.if_button_components, expected,
            "{seam:?} must discharge only an observed-on owned protect on the next pump"
        );
        rig.step();
        rig.snapshot.seed_tick(5);
        rig.step();
        assert_eq!(
            rig.driver.if_button_components, expected,
            "{seam:?} must not double-toggle a stale on observation"
        );
    }
}

#[test]
fn stop_owes_exactly_one_protect_off_on_the_next_pump() {
    check_guard_end(GuardEndSeam::Stop);
}

#[test]
fn pause_owes_exactly_one_protect_off_on_the_next_pump() {
    check_guard_end(GuardEndSeam::Pause);
}

#[test]
fn cancel_owes_exactly_one_protect_off_on_the_next_pump() {
    check_guard_end(GuardEndSeam::Cancel);
}

#[test]
fn an_owed_off_is_suppressed_when_the_next_observation_is_already_off() {
    let mut rig = protected_rig();
    raise_owned_missiles(&mut rig);
    rig.slot().lock().unwrap().stop();
    reset_script_nav(&rig.navs, "alice", None);
    seed_protect_state(&mut rig.snapshot, None);
    rig.snapshot.seed_tick(4);
    rig.step();
    assert_eq!(
        rig.driver.if_button_components,
        vec![5622],
        "a conditional off must not turn an already-off protect back on"
    );
}

#[test]
fn manual_takeover_owes_exactly_one_protect_off_on_the_next_pump() {
    check_guard_end(GuardEndSeam::Manual);
}

#[test]
fn carry_hold_owes_exactly_one_protect_off_on_the_next_pump() {
    check_guard_end(GuardEndSeam::CarryHold);
}

#[test]
fn owner_revocation_owes_exactly_one_protect_off_on_the_next_pump() {
    check_guard_end(GuardEndSeam::OwnerRevoked);
}

fn check_pending_arrival(previous: Option<i32>) {
    let mut rig = protected_rig();
    rig.snapshot.seed_tick(2);
    seed_protect_state(&mut rig.snapshot, previous);
    seed_missile_launch(&mut rig.snapshot);
    rig.step();
    assert_eq!(rig.driver.if_button_components, vec![5622]);
    // T+1: the route ends before the enable/switch has been acknowledged.
    let mut client = nav_client();
    client.bump_gens(client::io::ServerProt::PLAYER_INFO);
    nav_snapshot_at(&mut client, &mut rig.snapshot, 4, 0);
    seed_protect_frame(&mut rig.snapshot, 43);
    seed_prayer_widgets(&mut rig.snapshot);
    seed_protect_state(&mut rig.snapshot, previous);
    rig.snapshot.seed_tick(3);
    rig.step();
    assert!(
        !rig.navs.lock().unwrap()["alice"].has_route(),
        "T+1 really ends the route before the pending protect is observed"
    );
    assert_eq!(
        rig.driver.if_button_components,
        vec![5622],
        "arrival cannot toggle an old style while the new protect is in flight"
    );
    // T+2: the server applies the in-flight protect after the walk ended.
    seed_protect_state(&mut rig.snapshot, Some(96));
    rig.snapshot.seed_tick(4);
    rig.step();
    assert_eq!(
        rig.driver.if_button_components,
        vec![5622, 5622],
        "the late acknowledged protect must receive its single off-click"
    );
    rig.step();
    rig.snapshot.seed_tick(5);
    seed_protect_state(&mut rig.snapshot, None);
    rig.step();
    assert_eq!(rig.driver.if_button_components, vec![5622, 5622]);
}

#[test]
fn arrival_before_protect_acknowledgement_owes_the_late_off() {
    check_pending_arrival(None);
}

#[test]
fn arrival_during_a_pending_protect_switch_does_not_double_toggle() {
    check_pending_arrival(Some(97));
}

#[test]
fn a_missing_prayer_widget_does_not_admit_or_log_a_guard_click() {
    let mut rig = protected_rig();
    seed_missile_launch(&mut rig.snapshot);
    rig.snapshot.seed_main_modal(-1, Vec::new());
    rig.snapshot.seed_tick(2);
    rig.step();
    assert!(rig.driver.if_button_components.is_empty());
    assert!(!rig.navs.lock().unwrap()["alice"]
        .walk_guard
        .as_ref()
        .unwrap()
        .blocks_follow(2));
    seed_prayer_widgets(&mut rig.snapshot);
    rig.snapshot.seed_tick(3);
    rig.step();
    assert_eq!(
        rig.driver.if_button_components,
        vec![5622],
        "a refused proposal spends no pacing"
    );
}

#[test]
fn relog_discards_temporary_prayer_debt_without_a_click() {
    let mut rig = protected_rig();
    raise_owned_missiles(&mut rig);
    crate::play_slots::reset_slot_session_work(
        "alice",
        &rig.scripts,
        &rig.cheats,
        &Arc::new(Mutex::new(HashMap::new())),
        &rig.navs,
        false,
    );
    rig.snapshot.seed_tick(4);
    rig.step();
    assert_eq!(rig.driver.if_button_components, vec![5622]);
}

#[test]
fn death_discards_protect_without_a_toggle() {
    let mut rig = protected_rig();
    raise_owned_missiles(&mut rig);
    abort_script_walk(&rig.navs, "alice");
    let mut stats = rig.snapshot.stats().to_vec();
    stats
        .iter_mut()
        .find(|stat| stat.index == 3)
        .unwrap()
        .effective = 0;
    rig.snapshot.seed_stats(stats);
    rig.snapshot.seed_tick(4);
    rig.step();
    assert_eq!(rig.driver.if_button_components, vec![5622]);
}

#[test]
fn unprotectable_is_delivered_to_the_walk_owner() {
    let mut rig = open_rig(false);
    rig.shared.lock().protect = true;
    seed_prayer(&mut rig.snapshot, 37);
    seed_protect_frame(&mut rig.snapshot, 37);
    rig.observe(1);
    rig.wait_routed();
    seed_missile_launch(&mut rig.snapshot);
    rig.step();
    assert!(
        rig.walk_armed(),
        "Unprotectable must leave the route following"
    );
    rig.observe(2);
    assert_eq!(rig.end(), None, "the warning must not complete its walk");
    {
        let shared = rig.shared.lock();
        assert_eq!(shared.events.len(), 1);
        assert_eq!(
            shared.events[0].kind,
            WalkEventKind::Unprotectable {
                protect: script::combat::GuardProtect::Missiles
            }
        );
        assert_eq!(
            shared.events[0].detail.as_ref(),
            "Prayer 40 needed for Protect from Missiles"
        );
    }
    for tick in 3..6 {
        rig.snapshot.seed_tick(tick);
        rig.step();
        rig.observe(u64::from(tick));
    }
    assert!(
        rig.walk_armed(),
        "the owner still follows the unprotected crossing"
    );
    assert!(
        rig.driver.move_calls > 0,
        "the route keeps issuing follow work"
    );
    assert_eq!(
        rig.shared.lock().events.len(),
        1,
        "the owner sees the warning once"
    );
    assert!(rig.driver.if_button_components.is_empty());
}

#[test]
fn exhausted_guard_reports_once_and_keeps_the_walk_following() {
    let mut rig = protected_rig();
    let mut stats = rig.snapshot.stats().to_vec();
    stats
        .iter_mut()
        .find(|stat| stat.index == 5)
        .unwrap()
        .effective = 0;
    rig.snapshot.seed_stats(stats);
    seed_missile_launch(&mut rig.snapshot);
    for tick in 2..6 {
        rig.snapshot.seed_tick(tick);
        rig.step();
        rig.observe(u64::from(tick));
    }
    let shared = rig.shared.lock();
    assert!(shared.result.is_none());
    assert_eq!(shared.events.len(), 1);
    assert_eq!(
        shared.events[0].kind,
        WalkEventKind::Unprotectable {
            protect: script::combat::GuardProtect::Missiles,
        }
    );
    assert_eq!(
        shared.events[0].detail.as_ref(),
        "No Prayer points or prayer potion available for Protect from Missiles"
    );
    assert!(rig.walk_armed());
    assert!(rig.driver.move_calls > 0);
    assert!(rig.driver.if_button_components.is_empty());
}

#[test]
fn unprotectable_leaves_a_preexisting_user_style_on_when_the_walk_ends() {
    let mut rig = open_rig(false);
    rig.shared.lock().protect = true;
    seed_protect_frame(&mut rig.snapshot, 37);
    seed_prayer_widgets(&mut rig.snapshot);
    seed_protect_state(&mut rig.snapshot, Some(95));
    rig.snapshot.seed_tick(1);
    rig.observe(1);
    rig.wait_routed();
    seed_missile_launch(&mut rig.snapshot);
    rig.step();
    rig.observe(2);
    assert!(rig.walk_armed());
    assert_eq!(rig.shared.lock().events.len(), 1);
    assert!(rig.driver.if_button_components.is_empty());
    abort_script_walk(&rig.navs, "alice");
    rig.snapshot.seed_tick(3);
    rig.step();
    assert_eq!(
        rig.driver.if_button_components,
        Vec::<i32>::new(),
        "the guard never raised the user's Magic and must leave it on"
    );
}

#[test]
fn dropped_enable_then_end_expires_and_follows_the_next_walk_without_owning_user_prayer() {
    let mut rig = protected_rig();
    rig.shared.lock().walks = 2;
    rig.snapshot.seed_tick(2);
    seed_missile_launch(&mut rig.snapshot);
    rig.step();
    assert_eq!(rig.driver.if_button_components, vec![5622]);
    abort_script_walk(&rig.navs, "alice");
    rig.snapshot.seed_projectiles(Vec::new());
    rig.snapshot.seed_npcs(Vec::new());
    for tick in 3..=4 {
        rig.snapshot.seed_tick(tick);
        rig.step();
        rig.observe(u64::from(tick));
    }
    assert_eq!(rig.shared.lock().begun, 2);
    rig.wait_routed();
    let moves = rig.driver.move_calls;
    rig.snapshot.seed_tick(5);
    rig.step();
    assert!(
        rig.navs.lock().unwrap()["alice"].walk_guard_off.is_none(),
        "the dropped enable debt expires three ticks after its admission"
    );
    assert!(
        rig.driver.move_calls > moves,
        "the second walk follows on the deadline pump"
    );
    seed_protect_state(&mut rig.snapshot, Some(96));
    rig.snapshot.seed_tick(8);
    rig.step();
    abort_script_walk(&rig.navs, "alice");
    rig.snapshot.seed_tick(9);
    rig.step();
    assert_eq!(
        rig.driver.if_button_components,
        vec![5622],
        "neither stale debt nor the second guard may turn off a later user prayer"
    );
}

#[test]
fn a_dropped_switch_retires_the_guard_owned_old_protect() {
    let mut rig = protected_rig();
    raise_owned_missiles(&mut rig);
    seed_melee_attack(&mut rig.snapshot);
    rig.snapshot.seed_tick(4);
    rig.step();
    assert_eq!(
        rig.driver.if_button_components,
        vec![5622, 5623],
        "the guard admits a switch from its owned Missiles style to Melee"
    );

    rig.slot().lock().unwrap().stop();
    reset_script_nav(&rig.navs, "alice", None);
    for tick in 5..=7 {
        rig.snapshot.seed_tick(tick);
        rig.step();
    }
    assert_eq!(
        rig.driver.if_button_components,
        vec![5622, 5623, 5622],
        "when the server drops Melee, cleanup turns off the still-on owned Missiles"
    );
}

#[test]
fn a_dropped_switch_cleanup_does_not_restart_navigation_deferral() {
    let mut rig = protected_rig();
    rig.shared.lock().walks = 2;
    raise_owned_missiles(&mut rig);
    seed_melee_attack(&mut rig.snapshot);
    rig.snapshot.seed_tick(4);
    rig.step();
    assert_eq!(rig.driver.if_button_components, vec![5622, 5623]);
    abort_script_walk(&rig.navs, "alice");
    rig.shared.lock().protect = false;
    rig.snapshot.seed_tick(5);
    rig.step();
    rig.observe(5);
    rig.snapshot.seed_tick(6);
    rig.step();
    rig.observe(6);
    assert_eq!(rig.shared.lock().begun, 2);
    rig.wait_routed();
    let moves = rig.driver.move_calls;
    rig.snapshot.seed_tick(7);
    rig.step();
    assert_eq!(rig.driver.if_button_components, vec![5622, 5623, 5622]);
    assert!(rig.navs.lock().unwrap()["alice"].walk_guard_off.is_some());
    assert!(
        rig.driver.move_calls > moves,
        "retiring the fallback must not start a second navigation deferral window"
    );
}

#[test]
fn a_dropped_switch_does_not_turn_off_a_user_owned_old_protect() {
    let mut rig = protected_rig();
    seed_protect_state(&mut rig.snapshot, Some(96));
    seed_melee_attack(&mut rig.snapshot);
    rig.snapshot.seed_tick(2);
    rig.step();
    assert_eq!(rig.driver.if_button_components, vec![5623]);

    rig.slot().lock().unwrap().stop();
    reset_script_nav(&rig.navs, "alice", None);
    for tick in 3..=5 {
        rig.snapshot.seed_tick(tick);
        rig.step();
    }
    assert_eq!(
        rig.driver.if_button_components,
        vec![5623],
        "the old Missiles style was user-owned, so expiry cannot click it off"
    );
}

#[test]
fn a_real_switch_retires_the_new_protect_without_restoring_the_old_one() {
    let mut rig = protected_rig();
    raise_owned_missiles(&mut rig);
    seed_melee_attack(&mut rig.snapshot);
    rig.snapshot.seed_tick(4);
    rig.step();
    assert_eq!(rig.driver.if_button_components, vec![5622, 5623]);

    rig.slot().lock().unwrap().stop();
    reset_script_nav(&rig.navs, "alice", None);
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let melee_varp = data.prayer_by_name("Protect from Melee").unwrap().varp;
    seed_protect_state(&mut rig.snapshot, Some(melee_varp));
    rig.snapshot.seed_tick(5);
    rig.step();
    assert_eq!(
        rig.driver.if_button_components,
        vec![5622, 5623, 5623],
        "a real switch transfers cleanup to Melee without clicking old Missiles"
    );
}

#[test]
fn a_long_hold_gives_unavailable_owned_on_debt_a_fresh_cleanup_window() {
    let mut rig = protected_rig();
    raise_owned_missiles(&mut rig);
    rig.slot().lock().unwrap().stop();
    reset_script_nav(&rig.navs, "alice", None);

    rig.snapshot.seed_ingame(1);
    rig.snapshot.seed_tick(40);
    rig.step();
    assert!(
        rig.navs.lock().unwrap()["alice"].walk_guard_off.is_some(),
        "the first cleanup pump starts a new clock instead of expiring at stale guard time"
    );
    assert_eq!(rig.driver.if_button_components, vec![5622]);

    seed_protect_frame(&mut rig.snapshot, 43);
    seed_protect_state(&mut rig.snapshot, Some(96));
    rig.snapshot.seed_tick(41);
    rig.step();
    assert_eq!(
        rig.driver.if_button_components,
        vec![5622, 5622],
        "a newly readable on-varp is still retired within the fresh attempt window"
    );
}

#[test]
fn a_long_hold_gives_a_refused_cleanup_click_a_fresh_attempt_window() {
    let mut rig = protected_rig();
    raise_owned_missiles(&mut rig);
    rig.slot().lock().unwrap().stop();
    reset_script_nav(&rig.navs, "alice", None);

    rig.snapshot.seed_main_modal(-1, Vec::new());
    rig.snapshot.seed_tick(40);
    rig.step();
    assert!(
        rig.navs.lock().unwrap()["alice"].walk_guard_off.is_some(),
        "a refused first-pump dispatch keeps the newly started debt alive"
    );
    assert_eq!(rig.driver.if_button_components, vec![5622]);

    seed_prayer_widgets(&mut rig.snapshot);
    rig.snapshot.seed_tick(41);
    rig.step();
    assert_eq!(
        rig.driver.if_button_components,
        vec![5622, 5622],
        "the cleanup click is admitted when its widget resolves on the next pump"
    );
}

#[test]
fn timed_guard_cleanup_drops_report_unavailable_varps_and_refused_clicks() {
    for (missing_varps, reason) in [
        (true, "owned protect varp unavailable within cleanup window"),
        (false, "off click refused within cleanup window"),
    ] {
        let mark = crate::walk_map::test_log::mark();
        let mut rig = protected_rig();
        raise_owned_missiles(&mut rig);
        rig.slot().lock().unwrap().stop();
        reset_script_nav(&rig.navs, "alice", None);
        if missing_varps {
            rig.snapshot.seed_ingame(1);
        } else {
            rig.snapshot.seed_main_modal(-1, Vec::new());
        }
        rig.snapshot.seed_tick(4);
        rig.step();
        assert!(rig.navs.lock().unwrap()["alice"].walk_guard_off.is_some());

        rig.snapshot.seed_tick(7);
        rig.step();
        assert!(
            rig.navs.lock().unwrap()["alice"].walk_guard_off.is_none(),
            "unresolved cleanup remains bounded"
        );
        assert_eq!(rig.driver.if_button_components, vec![5622]);
        assert!(
            crate::walk_map::test_log::records_since(mark)
                .iter()
                .any(|(slot, message)| slot == "alice"
                    && message.starts_with("prayer cleanup drops protect debt component=5622 ")
                    && message.contains(reason)),
            "timed cleanup drop must explain its unresolved cause: {reason}"
        );
    }
}

#[test]
fn dropped_off_click_retains_observation_debt_and_gets_exactly_one_bounded_retry() {
    let mut rig = protected_rig();
    raise_owned_missiles(&mut rig);
    abort_script_walk(&rig.navs, "alice");
    rig.snapshot.seed_tick(4);
    rig.step();
    assert_eq!(rig.driver.if_button_components, vec![5622, 5622]);
    assert!(
        rig.navs.lock().unwrap()["alice"].walk_guard_off.is_some(),
        "sending the off-click does not establish that the server applied it"
    );
    for tick in 5..=6 {
        rig.snapshot.seed_tick(tick);
        rig.step();
    }
    assert_eq!(rig.driver.if_button_components, vec![5622, 5622]);
    rig.snapshot.seed_tick(7);
    rig.step();
    assert_eq!(rig.driver.if_button_components, vec![5622, 5622, 5622]);
    for tick in 8..=10 {
        rig.snapshot.seed_tick(tick);
        rig.step();
    }
    assert!(rig.navs.lock().unwrap()["alice"].walk_guard_off.is_none());
    for tick in 11..=20 {
        rig.snapshot.seed_tick(tick);
        rig.step();
    }
    assert_eq!(
        rig.driver.if_button_components,
        vec![5622, 5622, 5622],
        "a dropped retry must not leave a permanent debt or admit a third off-click"
    );
}

#[test]
fn an_off_retry_does_not_block_a_later_walk_beyond_the_first_pacing_window() {
    let mut rig = protected_rig();
    rig.shared.lock().walks = 2;
    raise_owned_missiles(&mut rig);
    abort_script_walk(&rig.navs, "alice");
    rig.shared.lock().protect = false;
    rig.snapshot.seed_tick(4);
    rig.step();
    rig.observe(4);
    rig.snapshot.seed_tick(5);
    rig.step();
    rig.observe(5);
    assert_eq!(rig.shared.lock().begun, 2);
    rig.wait_routed();
    let moves = rig.driver.move_calls;
    rig.snapshot.seed_tick(7);
    rig.step();
    assert!(rig.navs.lock().unwrap()["alice"].walk_guard_off.is_some());
    assert!(
        rig.driver.move_calls > moves,
        "navigation resumes after three ticks even while the off receipt is outstanding"
    );
}

#[test]
fn another_owners_observed_protect_switch_settles_the_off_without_a_toggle() {
    let mut rig = protected_rig();
    raise_owned_missiles(&mut rig);
    abort_script_walk(&rig.navs, "alice");
    seed_protect_state(&mut rig.snapshot, Some(95));
    rig.snapshot.seed_tick(4);
    rig.step();
    assert!(rig.navs.lock().unwrap()["alice"].walk_guard_off.is_none());
    assert_eq!(
        rig.driver.if_button_components,
        vec![5622],
        "observing another style on means the owed style is already off"
    );
}

#[test]
fn observed_off_settles_cleanup_before_retry_and_leaves_a_later_user_enable_alone() {
    let mut rig = protected_rig();
    raise_owned_missiles(&mut rig);
    abort_script_walk(&rig.navs, "alice");
    rig.snapshot.seed_tick(4);
    rig.step();
    assert!(rig.navs.lock().unwrap()["alice"].walk_guard_off.is_some());
    seed_protect_state(&mut rig.snapshot, None);
    rig.snapshot.seed_tick(5);
    rig.step();
    assert!(rig.navs.lock().unwrap()["alice"].walk_guard_off.is_none());
    seed_protect_state(&mut rig.snapshot, Some(96));
    rig.snapshot.seed_tick(9);
    rig.step();
    assert_eq!(rig.driver.if_button_components, vec![5622, 5622]);
}

#[test]
fn an_unroutable_native_walk_delivers_one_no_path_terminal() {
    let mut rig = rig(Some(Arc::new(open_world(3, 1))), false);
    rig.observe(1);
    assert!(
        rig.wait_for_route(5_000, || !rig.navs.lock().unwrap()["alice"]
            .route_work_pending()),
        "the planner completes the unreachable request"
    );
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::NoPath)));
    for tick in 3..40 {
        rig.observe(tick);
        rig.step();
    }
    assert_eq!(rig.shared.lock().results.len(), 1);
    assert_eq!(rig.shared.lock().begun, 1);
    assert_eq!(rig.driver.walked, None);
}

#[test]
fn native_walk_failure_receipt_names_missing_route_supplies() {
    let world = toll_nav_world();
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    world.bind_named_bank_facts(&selected).unwrap();
    let mut rig = rig(Some(Arc::new(world)), false);
    rig.observe(1);
    assert!(
        rig.wait_for_route(5_000, || !rig.navs.lock().unwrap()["alice"]
            .route_work_pending()),
        "the planner completes the fare-gated request"
    );
    rig.observe(2);

    let shared = rig.shared.lock();
    let receipt = shared.result.as_ref().unwrap().as_ref().unwrap();
    assert_eq!(receipt.end, WalkEnd::NoPath, "{:?}", receipt.detail);
    let detail = receipt
        .detail
        .as_deref()
        .expect("the failed walk receipt carries its navigation diagnosis");
    assert!(
        detail.contains("Insufficient route supplies: 10 more Coins (need 10, carrying 0)"),
        "{detail}"
    );
    assert!(detail.contains("native walk route failed"), "{detail}");
}

fn native_walk_gated_on_fishing(fishing: i32) -> (bool, Option<Result<WalkEnd, ActionError>>) {
    let mut world = toll_nav_world();
    world.graph.edges[0].consumed_req.clear();
    world.graph.edges[0].skill_req = vec![(10, 68)];
    let mut rig = rig(Some(Arc::new(world)), false);
    rig.snapshot.seed_stats(vec![api::snapshot::StatView {
        index: 10,
        name: "fishing".into(),
        base: fishing,
        effective: fishing,
        xp: 0,
        used: true,
    }]);
    // script_observe(..., state: None, Some(&snapshot), ...) — the Starting-frame arm.
    rig.observe(1);
    assert!(
        rig.wait_for_route(5_000, || !rig.navs.lock().unwrap()["alice"]
            .route_work_pending())
    );
    let routed = queued(&rig.navs).is_some();
    rig.observe(2);
    let end = rig.end();
    eprintln!("native-walk fishing={fishing} routed={routed} end={end:?}");
    (routed, end)
}

#[test]
fn native_walk_routes_skill_gate_with_posted_level_and_no_arm_state() {
    let (routed, end) = native_walk_gated_on_fishing(68);
    assert!(
        routed,
        "posted Fishing 68 must route the gate with arm state None; end={end:?}"
    );
    assert_ne!(end, Some(Ok(WalkEnd::NoPath)));
}

#[test]
fn native_walk_refuses_skill_gate_below_level() {
    let (routed, end) = native_walk_gated_on_fishing(67);
    assert!(!routed, "Fishing 67 must not cross a Fishing-68 door");
    assert_eq!(end, Some(Ok(WalkEnd::NoPath)));
}

/// The host's own route arm, as the watchdog's recovery walk and legacy
/// script walks use it: no native authority.
fn host_walk(rig: &Rig, x: i32, retarget: bool) -> bool {
    crate::script_runtime::ScriptWalkArm {
        here: Some((0, 0, 0)),
        world: rig.world.clone(),
        navs: Arc::clone(&rig.navs),
        name: "alice".into(),
        state: None,
        bank: BankRows::default(),
    }
    .queue_route_in_snapshot(
        &rig.snapshot,
        x,
        0,
        0,
        nav_planner::router::FindOptions {
            zones: nav_planner::zones::ZoneExempt::all(),
            ..Default::default()
        },
        0,
        retarget,
        0,
    )
}

#[test]
fn a_native_walk_refused_by_an_in_flight_route_gets_a_refused_receipt() {
    let mut rig = open_rig(false);
    // A host walk to the same destination is already in flight under the
    // legacy request id; the distinct native id is refused, not coalesced.
    assert!(host_walk(&rig, 4, false));
    rig.observe(1);
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::Refused)));
}

#[test]
fn pause_and_resume_end_a_native_walk_with_a_cancelled_receipt() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    {
        let slot = rig.slot();
        let mut slot = slot.lock().unwrap();
        crate::script_runtime::pause_script(&mut slot, &rig.navs, "alice");
        slot.resume();
    }
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::Cancelled)));
    rig.step();
    assert_eq!(rig.driver.walked, None, "no orphaned follow after Resume");
}

#[test]
fn a_watchdog_walk_displacing_a_native_walk_cancels_it() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    // The watchdog's recovery ArmWalk: the host arm without native authority.
    assert!(host_walk(&rig, 8, true));
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::Cancelled)));
}

#[test]
fn an_unproven_crossing_reaches_the_native_owner_typed() {
    use api::selected::{FactKey, QuestGate, Truth};
    let gate = QuestGate::Complete(FactKey(Arc::from("tbwt")));
    let undecided: Arc<[QuestGate]> = Arc::from([gate]);
    for (verdict, unresolved, expected) in [
        (
            Truth::Unknown,
            Arc::clone(&undecided),
            WalkEnd::NeedsEvidence(Arc::clone(&undecided)),
        ),
        (Truth::False, Arc::from([]), WalkEnd::Blocked),
    ] {
        let mut rig = open_rig(false);
        rig.observe(1);
        rig.wait_routed();
        // The follow's pre-send recheck refused the crossing.
        crate::script_runtime::apply_nav_follow_outcome(
            rig.navs.lock().unwrap().get_mut("alice").unwrap(),
            Some(nav_executor::TravelOutcome::EvidenceUnproven {
                at: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                leg: 0,
                verdict,
                unresolved,
            }),
            false,
        );
        rig.observe(2);
        assert_eq!(rig.end(), Some(Ok(expected)));
    }
}

#[test]
fn cancelling_and_rewalking_the_same_destination_arms_the_new_walk() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    // The owner cancels its walk and walks the same destination again
    // before the pump has stepped the abandoned follow away.
    rig.shared.lock().rewalk = true;
    rig.observe(2);
    assert!(
        rig.navs.lock().unwrap()["alice"]
            .native_walk
            .as_ref()
            .is_some_and(|owner| owner.live()),
        "the new walk owns the route, not a refusal beside the abandoned one"
    );
    rig.observe(3);
    assert_eq!(rig.end(), None, "the re-walk is in flight, not refused");
    rig.wait_routed();
}

#[test]
fn native_cross_exemption_is_scoped_to_one_walk() {
    let mut rig = rig(Some(Arc::new(zoned_open_world())), false);
    {
        let mut shared = rig.shared.lock();
        shared.walks = 2;
        shared.cross_first = vec![Arc::from("test-barrier@2,0,0")];
    }

    rig.observe(1);
    rig.wait_routed();
    crate::script_runtime::apply_nav_follow_outcome(
        rig.navs.lock().unwrap().get_mut("alice").unwrap(),
        Some(nav_executor::TravelOutcome::Arrived {
            at: WorldTile {
                x: 4,
                z: 0,
                level: 0,
            },
        }),
        false,
    );
    rig.observe(2);
    assert_eq!(rig.shared.lock().results.len(), 1);
    rig.shared.lock().options.allow_danger_zones = script::native::WalkBit::Forbid;

    rig.observe(3);
    assert!(rig.wait_for_route(5_000, || {
        rig.navs
            .lock()
            .unwrap()
            .get("alice")
            .is_some_and(|bot| bot.native_walk_failure.is_some())
    }));
    rig.observe(4);

    let shared = rig.shared.lock();
    assert_eq!(shared.results.len(), 2);
    assert!(shared.results[0].as_ref().unwrap().blocked.is_none());
    let second = shared.results[1].as_ref().unwrap();
    assert_eq!(second.end, WalkEnd::Refused);
    assert_eq!(
        second.blocked.as_deref(),
        Some(&[nav_planner::zones::ZoneKey::Zone(0)][..])
    );
    assert!(second
        .detail
        .as_deref()
        .is_some_and(|detail| detail.contains("test-barrier@2,0,0")));
}

#[test]
fn native_admission_global_and_walk_danger_permissions_and_forbid() {
    use script::native::WalkBit;
    for (global, bit, permitted) in [
        (false, WalkBit::Inherit, false),
        (false, WalkBit::Allow, true),
        (false, WalkBit::Forbid, false),
        (true, WalkBit::Inherit, true),
        (true, WalkBit::Allow, true),
        (true, WalkBit::Forbid, false),
    ] {
        // With no inherited grant the original router cannot cross this
        // barrier. Holding assessment authority off does not widen routing.
        let mut rig = rig(Some(Arc::new(zoned_open_world())), false);
        let globals = Arc::new(Mutex::new(WalkGlobals {
            allow_danger_zones: global,
            allow_bank_fetch: true,
            ..Default::default()
        }));
        rig.navs
            .lock()
            .unwrap()
            .entry("alice".into())
            .or_default()
            .walk_globals = Some(globals);
        rig.shared.lock().options.allow_danger_zones = bit;
        assert!(
            !crate::walk_permissions::native_admission(
                &rig.navs,
                "alice",
                rig.shared.lock().options,
                None,
            )
            .0
            .allow_bank_fetch,
            "global fetch stays manual-only at the native admission"
        );
        rig.observe(1);
        assert!(rig.wait_for_route(5_000, || {
            rig.navs
                .lock()
                .unwrap()
                .get("alice")
                .is_some_and(|bot| !bot.route_work_pending())
        }));
        {
            let all = rig.navs.lock().unwrap();
            let bot = &all["alice"];
            assert_eq!(bot.has_route(), permitted, "global={global}, walk={bit:?}");
            assert!(
                bot.bank_fetch.is_none(),
                "native walks never use BankBudget"
            );
            if permitted {
                assert!(!bot.requested_route.unwrap().4);
            } else {
                assert!(bot.native_walk_failure.is_some());
                assert!(
                    bot.requested_route.is_none(),
                    "a failed admission retires its route request"
                );
            }
            assert!(
                bot.walk_guard.is_none(),
                "danger crossing does not start protection"
            );
        }
        if !permitted {
            rig.observe(2);
            assert_eq!(rig.end(), Some(Ok(WalkEnd::Refused)));
        }
    }
}

#[test]
fn selection_query_uses_live_loc_access_tiles_and_zone_carves() {
    use nav_planner::contract::ArrivalKind;

    let target = WorldTile {
        x: 20,
        z: 20,
        level: 0,
    };
    let all_approaches = avoid_rect(19, 22, 19, 22);
    let snapshot = selection_snapshot(target, 3, 0, None);
    let fully_zoned = selection_zone_world(
        target,
        nav_planner::zones::Zone::hazard(all_approaches, 0, 0),
        None,
        nav_planner::transport::WildernessRules::default(),
    );
    assert_eq!(
        selection_danger(
            &fully_zoned,
            Some(&snapshot),
            target,
            Some(1276),
            1,
            ArrivalKind::Reach
        ),
        Err("Test danger".into()),
        "every legal stand around the modeled 2x2 loc is denied"
    );

    let center_only = selection_zone_world(
        target,
        nav_planner::zones::Zone::hazard(avoid_rect(20, 20, 20, 20), 0, 0),
        None,
        nav_planner::transport::WildernessRules::default(),
    );
    assert_eq!(
        selection_danger(
            &center_only,
            Some(&snapshot),
            target,
            Some(1276),
            1,
            ArrivalKind::Reach
        ),
        Ok(()),
        "a safe legal stand retains a loc whose anchor lies in a denied zone"
    );

    let north_access = WorldTile {
        x: target.x,
        z: target.z + 2,
        level: target.level,
    };
    let carved = selection_zone_world(
        target,
        nav_planner::zones::Zone::hazard(all_approaches, 0, 0),
        Some(avoid_rect(
            north_access.x,
            north_access.x,
            north_access.z,
            north_access.z,
        )),
        nav_planner::transport::WildernessRules::default(),
    );
    assert_eq!(
        selection_danger(
            &carved,
            Some(&snapshot),
            target,
            Some(1276),
            1,
            ArrivalKind::Reach
        ),
        Ok(()),
        "one carved, legal cardinal approach keeps the resource eligible"
    );

    let force_north = selection_snapshot(target, 3, 0x1, None);
    assert_eq!(
        selection_danger(
            &carved,
            Some(&force_north),
            target,
            Some(1276),
            1,
            ArrivalKind::Reach
        ),
        Err("Test danger".into()),
        "a safe tile rejected by the loc's force-approach mask is not an access alternative"
    );
    let blocked_north = selection_snapshot(target, 3, 0, Some(north_access));
    assert_eq!(
        selection_danger(
            &carved,
            Some(&blocked_north),
            target,
            Some(1276),
            1,
            ArrivalKind::Reach
        ),
        Err("Test danger".into()),
        "a collision-blocked carve is not a legal access tile"
    );

    let diagonal_only = selection_zone_world(
        target,
        nav_planner::zones::Zone::hazard(all_approaches, 0, 0),
        Some(avoid_rect(
            target.x - 1,
            target.x - 1,
            target.z - 1,
            target.z - 1,
        )),
        nav_planner::transport::WildernessRules::default(),
    );
    assert_eq!(
        selection_danger(
            &diagonal_only,
            Some(&snapshot),
            target,
            Some(1276),
            1,
            ArrivalKind::Reach
        ),
        Err("Test danger".into()),
        "a carved diagonal is outside the actual loc operation geometry"
    );
}

#[test]
fn selection_safe_access_goals_restrict_mixed_live_loc_stands() {
    use script::native::{WalkDanger, WalkDangerQuery};

    let at = |x, z| WorldTile { x, z, level: 0 };
    let target = at(20, 20);
    let denied = at(19, 20);
    let world = selection_zone_world(
        target,
        nav_planner::zones::Zone::hazard(avoid_rect(19, 19, 20, 20), 0, 0),
        None,
        nav_planner::transport::WildernessRules::default(),
    );
    let snapshot = selection_snapshot(target, 3, 0, None);
    let navs = Arc::new(Mutex::new(HashMap::new()));
    let query = crate::walk_permissions::SelectionDangerQuery::new(
        &navs,
        "alice",
        &world,
        Some(&snapshot),
        None,
    );

    assert_eq!(
        query.check(
            target,
            Some(1276),
            1,
            nav_planner::contract::ArrivalKind::Reach
        ),
        WalkDanger::Allowed,
        "one safe live loc stand keeps the mixed-stand resource eligible"
    );
    let goals = query
        .safe_access_goals(
            target,
            Some(1276),
            1,
            nav_planner::contract::ArrivalKind::Reach,
            None,
        )
        .unwrap()
        .expect("the partial denied set must restrict the approach");
    let expected = [
        at(19, 21),
        at(20, 19),
        at(20, 20),
        at(20, 21),
        at(20, 22),
        at(21, 19),
        at(21, 20),
        at(21, 21),
        at(21, 22),
        at(22, 20),
        at(22, 21),
    ];
    assert_eq!(goals.as_slice(), expected.as_slice());
    assert!(!goals.as_slice().contains(&denied));
}

#[test]
fn selection_safe_access_goals_preserve_allowed_and_uncovered_stands() {
    use script::native::{WalkDangerQuery, WalkGoals, WalkPermissions};

    let at = |x, z| WorldTile { x, z, level: 0 };
    let target = at(20, 20);
    let denied = at(19, 20);
    let world = selection_zone_world(
        target,
        nav_planner::zones::Zone::hazard(avoid_rect(19, 19, 20, 20), 0, 0),
        None,
        nav_planner::transport::WildernessRules::default(),
    );
    let snapshot = selection_snapshot(target, 3, 0, None);

    for (global, card) in [(false, true), (true, false)] {
        let navs = Arc::new(Mutex::new(HashMap::from([(
            "alice".to_owned(),
            NavBot {
                walk_globals: Some(Arc::new(Mutex::new(WalkGlobals {
                    allow_danger_zones: global,
                    ..Default::default()
                }))),
                ..NavBot::default()
            },
        )])));
        let query = crate::walk_permissions::SelectionDangerQuery::new(
            &navs,
            "alice",
            &world,
            Some(&snapshot),
            Some(WalkPermissions {
                allow_danger_zones: card,
                ..Default::default()
            }),
        );
        assert!(
            matches!(
                query.safe_access_goals(
                    target,
                    Some(1276),
                    1,
                    nav_planner::contract::ArrivalKind::Reach,
                    None
                ),
                Ok(None)
            ),
            "global={global}, card={card}: permitted danger preserves the original approach"
        );
    }

    let navs = Arc::new(Mutex::new(HashMap::new()));
    let query = crate::walk_permissions::SelectionDangerQuery::new(
        &navs,
        "alice",
        &world,
        Some(&snapshot),
        None,
    );
    let safe_in_table = at(20, 20);
    let uncovered = at(50, 50);
    let existing = WalkGoals::with_preferred(vec![safe_in_table, uncovered], &[uncovered]).unwrap();
    assert!(
        matches!(
            query.safe_access_goals(
                target,
                Some(1276),
                1,
                nav_planner::contract::ArrivalKind::Reach,
                Some(&existing)
            ),
            Ok(None)
        ),
        "safe and zone-table-uncovered goals are unchanged when none is denied"
    );
    assert_eq!(existing.preferred(), &[uncovered]);
    assert_eq!(existing.fallback(), &[safe_in_table]);
    assert!(!existing.as_slice().contains(&denied));
}

#[test]
fn selection_safe_access_goals_filter_existing_goal_tiers() {
    use script::native::{WalkDangerQuery, WalkGoals};

    let at = |x, z| WorldTile { x, z, level: 0 };
    let target = at(20, 20);
    let denied_preferred = at(19, 20);
    let safe_preferred = at(20, 20);
    let safe_fallback = at(21, 20);
    let uncovered_fallback = at(50, 50);
    let world = selection_zone_world(
        target,
        nav_planner::zones::Zone::hazard(avoid_rect(19, 19, 20, 20), 0, 0),
        None,
        nav_planner::transport::WildernessRules::default(),
    );
    let navs = Arc::new(Mutex::new(HashMap::new()));
    let query =
        crate::walk_permissions::SelectionDangerQuery::new(&navs, "alice", &world, None, None);
    let existing = WalkGoals::with_preferred(
        vec![
            denied_preferred,
            safe_preferred,
            safe_fallback,
            uncovered_fallback,
        ],
        &[denied_preferred, safe_preferred],
    )
    .unwrap();

    let filtered = query
        .safe_access_goals(
            target,
            Some(1276),
            1,
            nav_planner::contract::ArrivalKind::Reach,
            Some(&existing),
        )
        .unwrap()
        .expect("the denied preferred goal must be removed");
    assert_eq!(filtered.preferred(), &[safe_preferred]);
    assert_eq!(filtered.fallback(), &[safe_fallback, uncovered_fallback]);
    assert!(!filtered.as_slice().contains(&denied_preferred));
    assert_eq!(existing.preferred(), &[denied_preferred, safe_preferred]);
}

#[test]
fn selection_safe_access_goals_reject_empty_and_oversized_restrictions() {
    use script::native::{WalkDangerQuery, WalkGoalsError};

    let at = |x, z| WorldTile { x, z, level: 0 };
    let target = at(20, 20);
    let world = selection_zone_world(
        target,
        nav_planner::zones::Zone::hazard(avoid_rect(19, 22, 19, 22), 0, 0),
        None,
        nav_planner::transport::WildernessRules::default(),
    );
    let snapshot = selection_snapshot(target, 3, 0, None);
    let navs = Arc::new(Mutex::new(HashMap::new()));
    let query = crate::walk_permissions::SelectionDangerQuery::new(
        &navs,
        "alice",
        &world,
        Some(&snapshot),
        None,
    );
    assert!(
        matches!(
            query.safe_access_goals(
                target,
                Some(1276),
                1,
                nav_planner::contract::ArrivalKind::Reach,
                None
            ),
            Err(WalkGoalsError::Empty)
        ),
        "an all-denied generated restriction must fail closed"
    );

    let large_target = at(30, 30);
    let mut large_world = open_world(60, 60);
    add_selection_zone(
        &mut large_world,
        nav_planner::zones::Zone::hazard(avoid_rect(30, 30, 30, 30), 0, 0),
        None,
    );
    let large_query = crate::walk_permissions::SelectionDangerQuery::new(
        &navs,
        "alice",
        &large_world,
        None,
        None,
    );
    assert!(
        matches!(
            large_query.safe_access_goals(
                large_target,
                None,
                40,
                nav_planner::contract::ArrivalKind::Area,
                None
            ),
            Err(WalkGoalsError::TooMany {
                count,
                limit: script::native::WalkGoals::MAX
            }) if count > script::native::WalkGoals::MAX
        ),
        "oversized generated restrictions must not be truncated or dropped"
    );
}

#[test]
fn native_typed_walk_restricts_mixed_live_loc_stands_through_route_and_follow() {
    use script::native::{WalkDanger, WalkDangerQuery};

    let at = |x, z| WorldTile { x, z, level: 0 };
    let start = at(0, 0);
    let target = at(20, 20);
    let denied = at(19, 20);
    let mut world = open_world(40, 40);
    add_selection_zone(
        &mut world,
        nav_planner::zones::Zone::hazard(avoid_rect(19, 19, 20, 20), 0, 0),
        None,
    );
    let world = Arc::new(world);
    let mut rig = rig(Some(Arc::clone(&world)), false);

    let mut snapshot = selection_snapshot(target, 3, 0, None);
    let mut player = snapshot
        .local_player()
        .expect("selection fixture player")
        .clone();
    player.player.actor.tile = start;
    snapshot.seed_local_player(player);
    rig.snapshot = snapshot;

    let query = crate::walk_permissions::SelectionDangerQuery::new(
        &rig.navs,
        "alice",
        &world,
        Some(&rig.snapshot),
        None,
    );
    assert_eq!(
        query.check(
            target,
            Some(1276),
            1,
            nav_planner::contract::ArrivalKind::Reach
        ),
        WalkDanger::Allowed,
        "a safe legal live stand leaves the resource selectable"
    );
    let safe_goals = query
        .safe_access_goals(
            target,
            Some(1276),
            1,
            nav_planner::contract::ArrivalKind::Reach,
            None,
        )
        .unwrap()
        .expect("the native request must receive the safe live stands");
    let expected_goals = safe_goals.as_slice().to_vec();
    assert!(!expected_goals.contains(&denied));

    {
        let mut shared = rig.shared.lock();
        shared.target = Some(target);
        shared.radius = 0;
        shared.arrival = nav_planner::contract::ArrivalKind::Area;
        shared.candidate_goals = Some(safe_goals);
    }
    rig.observe_with_here(1, start);
    rig.wait_routed();

    let (destination, route_positions) = {
        let navs = rig.navs.lock().unwrap();
        let bot = &navs["alice"];
        let route = bot.route_path_for_follow().expect("native route path");
        let mut positions = Vec::new();
        if let Some(view) = route.view() {
            for leg in view.legs() {
                if let Some(tiles) = leg.walk_tiles() {
                    positions.extend_from_slice(tiles);
                }
            }
        } else {
            for leg in &route.legacy_route().expect("legacy route backing").legs {
                if let nav_planner::router::Leg::Walk { tiles } = leg {
                    positions.extend_from_slice(tiles);
                }
            }
        }
        (
            bot.route_destination().expect("native destination"),
            positions,
        )
    };
    assert!(expected_goals.contains(&destination));
    assert_ne!(destination, denied);
    assert!(
        !route_positions.is_empty(),
        "the native walk has a real route"
    );
    assert!(
        route_positions.iter().all(|&position| position != denied),
        "no routed walk position may enter denied access stand {denied:?}"
    );

    for position in route_positions {
        rig.step_at(position);
        let navs = rig.navs.lock().unwrap();
        assert_ne!(
            navs["alice"].traveller.current_aim(),
            Some(denied),
            "the active follow aim must not enter the denied access stand"
        );
        assert_ne!(
            rig.snapshot.tile(),
            Some((denied.x, denied.z, denied.level))
        );
    }
}

#[test]
fn selection_query_uses_area_and_reach_candidates_off_scene() {
    use nav_planner::contract::ArrivalKind;

    let target = WorldTile {
        x: 20,
        z: 20,
        level: 0,
    };
    const WIDTH: usize = 32;
    const HEIGHT: usize = 32;
    let origin = WorldTile {
        x: target.x - 8,
        z: target.z - 8,
        level: 0,
    };
    let mut flags = vec![0u32; 4 * WIDTH * HEIGHT];
    let barrier_left = usize::try_from(target.x - origin.x).unwrap();
    for row in 0..HEIGHT {
        let left = row * WIDTH + barrier_left;
        let right = left + 1;
        flags[left] |= CollisionFlag::W_E as u32;
        flags[right] |= CollisionFlag::W_W as u32;
    }
    let (walk, blocked) = nav_planner::collision::pack_walk(&flags);
    let mut world = NavWorld::from_parts(
        nav_planner::collision::WorldCollision {
            origin,
            width: WIDTH,
            height: HEIGHT,
            walk,
            blocked,
            flags: None,
        },
        nav_planner::transport::TransportGraph::default(),
        Vec::new(),
    );
    let safe_alternative = WorldTile {
        x: target.x + 2,
        z: target.z,
        level: 0,
    };
    add_selection_zone(
        &mut world,
        nav_planner::zones::Zone::hazard(avoid_rect(18, 22, 18, 22), 0, 0),
        Some(avoid_rect(
            safe_alternative.x,
            safe_alternative.x,
            safe_alternative.z,
            safe_alternative.z,
        )),
    );
    assert!(world.collision.standable(safe_alternative));
    assert_eq!(
        selection_danger(&world, None, target, None, 2, ArrivalKind::Area),
        Ok(()),
        "Area accepts the safe stand even though a wall separates it from the target"
    );
    assert_eq!(
        selection_danger(&world, None, target, None, 2, ArrivalKind::Reach),
        Err("Test danger".into()),
        "Reach excludes that unreachable stand; every remaining actual candidate is denied"
    );
}

#[test]
fn selection_query_reuses_combat_and_wilderness_zone_activity() {
    use nav_planner::contract::ArrivalKind;

    let target = WorldTile {
        x: 20,
        z: 20,
        level: 0,
    };
    let rule = || {
        nav_planner::zones::Zone::npc(target, 0, nav_planner::zones::ZoneClass::LevelRule, 10, 0)
    };
    let no_wilderness = selection_zone_world(
        target,
        rule(),
        None,
        nav_planner::transport::WildernessRules::default(),
    );
    let low_combat = selection_snapshot(target, 3, 0, None);
    let high_combat = selection_snapshot(target, 51, 0, None);
    assert_eq!(
        selection_danger(
            &no_wilderness,
            Some(&low_combat),
            target,
            None,
            0,
            ArrivalKind::Area
        ),
        Err("Test danger".into())
    );
    assert_eq!(
        selection_danger(
            &no_wilderness,
            Some(&high_combat),
            target,
            None,
            0,
            ArrivalKind::Area
        ),
        Ok(()),
        "the packed combat cap makes the zone inactive above its threshold"
    );

    let wilderness = nav_planner::transport::WildernessRules {
        zones: vec![nav_planner::transport::WildernessZone {
            x1: target.x,
            z1: target.z,
            x2: target.x,
            z2: target.z,
            level1: 0,
            level2: 0,
            origin_z: target.z,
        }],
        divisor: 1,
        offset: 0,
    };
    let wild_rule = selection_zone_world(target, rule(), None, wilderness);
    assert_eq!(
        selection_danger(
            &wild_rule,
            Some(&high_combat),
            target,
            None,
            0,
            ArrivalKind::Area
        ),
        Err("Test danger".into()),
        "a high combat level does not deactivate an active wilderness zone"
    );
}

#[test]
fn selection_query_and_native_walk_share_effective_card_danger_permissions() {
    use script::native::{WalkDanger, WalkDangerQuery, WalkPermissions};

    for (global, card, permitted) in [
        (false, false, false),
        (false, true, true),
        (true, false, true),
        (true, true, true),
    ] {
        let mut rig = rig(Some(Arc::new(zoned_open_world())), false);
        let current = WalkPermissions {
            allow_danger_zones: card,
            ..Default::default()
        };
        let previous = WalkPermissions {
            allow_danger_zones: !card,
            ..Default::default()
        };
        {
            let mut navs = rig.navs.lock().unwrap();
            let bot = navs.entry("alice".into()).or_default();
            bot.walk_globals = Some(Arc::new(Mutex::new(WalkGlobals {
                allow_danger_zones: global,
                ..Default::default()
            })));
            bot.native_permissions = Some(previous);
        }
        let query_tile = WorldTile {
            x: 2,
            z: 0,
            level: 0,
        };
        {
            let query = crate::walk_permissions::SelectionDangerQuery::new(
                &rig.navs,
                "alice",
                rig.world.as_deref().expect("zoned fixture"),
                Some(&rig.snapshot),
                Some(current),
            );
            assert_eq!(
                matches!(
                    query.check(
                        query_tile,
                        None,
                        0,
                        nav_planner::contract::ArrivalKind::Area
                    ),
                    WalkDanger::Allowed
                ),
                permitted,
                "current card beats stale mirror: global={global}, card={card}"
            );
        }
        let options = crate::walk_permissions::native_admission(
            &rig.navs,
            "alice",
            script::native::WalkOptions::default(),
            Some(current),
        )
        .0;
        assert_eq!(
            options.zones.is_all(),
            permitted,
            "the native Walk route resolver uses the same current callback card"
        );
        let slot = script_slot_or_insert(&rig.scripts, "alice");
        let mut slot = slot.lock().unwrap();
        let run = slot.native_run().expect("running Walker test card");
        let config = script::native::PreparedConfig::for_test_script(
            2,
            Arc::new(
                serde_json::from_value(serde_json::json!({
                    "allow_danger_zones": card,
                }))
                .unwrap(),
            ),
        );
        assert!(matches!(
            slot.configure_compiled(config, run),
            script::CompiledDelivery::Applied
        ));
        assert_eq!(slot.native_walk_permissions(), Some(current));
        drop(slot);
        // Cross the queried barrier rather than end inside the router's
        // deliberate destination-completion exception.
        rig.observe(1);
        assert!(rig.wait_for_route(5_000, || {
            rig.navs
                .lock()
                .unwrap()
                .get("alice")
                .is_some_and(|bot| !bot.route_work_pending())
        }));
        assert_eq!(
            rig.navs.lock().unwrap()["alice"].has_route(),
            permitted,
            "the actual native Walk crosses the same denied zone under global={global}, card={card}"
        );
        if !permitted {
            rig.observe(2);
            assert_eq!(rig.end(), Some(Ok(WalkEnd::Refused)));
        }
    }
}

#[test]
#[ignore = "requires WORLD_NAV_PACK pointing to the real revision-289 nav pack"]
fn selection_query_checks_packed_draynor_jail_tree_against_safe_tiles() {
    use script::native::{WalkDanger, WalkDangerQuery};

    let pack = std::env::var_os("WORLD_NAV_PACK").expect("WORLD_NAV_PACK is required");
    let world = NavWorld::load_pack(std::path::Path::new(&pack)).expect("real revision-289 pack");
    let target = WorldTile {
        x: 3113,
        z: 3235,
        level: 0,
    };
    let snapshot = packed_collision_scene(&world, target);
    let navs = Arc::new(Mutex::new(HashMap::new()));
    let query = crate::walk_permissions::SelectionDangerQuery::new(
        &navs,
        "alice",
        &world,
        Some(&snapshot),
        None,
    );
    assert_eq!(
        query.check(
            target,
            Some(1276),
            1,
            nav_planner::contract::ArrivalKind::Reach
        ),
        WalkDanger::Denied {
            zone: "Draynor jail guards"
        },
        "the normal tree's complete legal access geometry is inside the packed jail zones"
    );

    let table = world.graph.zones.as_ref().expect("packed zone table");
    let filter = nav_planner::zones::ZoneFilter::new(
        table,
        Some(3),
        &[],
        &nav_planner::zones::ZoneExempt::NONE,
    );
    let safe_tile = (target.x - 32..=target.x + 32)
        .flat_map(|x| {
            (target.z - 32..=target.z + 32).map(move |z| WorldTile {
                x,
                z,
                level: target.level,
            })
        })
        .find(|&tile| {
            world.collision.standable(tile)
                && filter
                    .blocking_at(&world.graph.wilderness, tile)
                    .next()
                    .is_none()
        })
        .expect("a safe packed alternative near Draynor");
    assert_eq!(
        query.check(safe_tile, None, 0, nav_planner::contract::ArrivalKind::Area),
        WalkDanger::Allowed,
        "a safe packed alternative remains eligible"
    );
}
#[test]
fn native_admission_teleport_and_wilderness_forbids_override_global_grants() {
    use script::native::WalkBit;
    for bit in [WalkBit::Inherit, WalkBit::Allow, WalkBit::Forbid] {
        let mut rig = open_rig(false);
        rig.navs
            .lock()
            .unwrap()
            .entry("alice".into())
            .or_default()
            .walk_globals = Some(Arc::new(Mutex::new(WalkGlobals {
            allow_teleports: true,
            allow_wilderness: true,
            allow_bank_fetch: true,
            ..Default::default()
        })));
        rig.shared.lock().options = script::native::WalkOptions {
            allow_teleports: bit,
            allow_wilderness: bit,
            allow_danger_zones: WalkBit::Allow,
        };
        rig.observe(1);
        rig.wait_routed();
        let all = rig.navs.lock().unwrap();
        let request = all["alice"].requested_route.unwrap();
        assert_eq!(request.2, bit != WalkBit::Forbid);
        assert_eq!(request.3, bit != WalkBit::Forbid);
        assert!(!request.4);
    }
}

#[test]
fn native_protect_alone_does_not_grant_danger_and_invalid_cross_still_refuses() {
    {
        let mut rig = rig(Some(Arc::new(zoned_open_world())), false);
        rig.shared.lock().options.allow_danger_zones = script::native::WalkBit::Forbid;
        rig.shared.lock().protect = true;
        seed_prayer(&mut rig.snapshot, 43);
        rig.observe(1);
        assert!(rig.wait_for_route(5_000, || {
            rig.navs
                .lock()
                .unwrap()
                .get("alice")
                .is_some_and(|bot| bot.native_walk_failure.is_some())
        }));
        rig.observe(2);
        assert_eq!(rig.end(), Some(Ok(WalkEnd::Refused)));
    }
    for names in [
        vec![Arc::from("unknown-danger-zone")],
        vec![Arc::from("test-barrier@2,0,0"); 9],
    ] {
        let mut rig = rig(Some(Arc::new(zoned_open_world())), false);
        rig.shared.lock().options.allow_danger_zones = script::native::WalkBit::Forbid;
        rig.shared.lock().cross_first = names;
        rig.observe(1);
        rig.observe(2);
        assert_eq!(rig.end(), Some(Ok(WalkEnd::Refused)));
        assert!(rig.navs.lock().unwrap()["alice"].walk_guard.is_none());
    }
}

#[cfg(feature = "js")]
#[test]
fn sherlock_bool_walk_inherits_committed_bits_while_isolate_wiring_stays_frozen() {
    script_js::install_cards().expect("install canonical Sherlock card for native walk test");
    let mut slot = script::SlotScript::new();
    slot.bind_incarnation(84);
    slot.start_compiled(
        "alice",
        script::CompiledId("Sherlock"),
        Arc::new(
            serde_json::from_value(serde_json::json!({
                "allow_teleports": true,
                "allow_wilderness": true,
                "allow_danger_zones": true,
            }))
            .unwrap(),
        ),
        api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap(),
        Arc::default(),
    )
    .unwrap();
    assert!(wait_until(5_000, || {
        slot.observe_lifecycle();
        slot.state() == script::RunState::Running
    }));
    let permissions = slot.native_walk_permissions().unwrap();
    for compiled in [true, false] {
        let mut rig = rig(Some(Arc::new(zoned_open_world())), false);
        let mut all = rig.navs.lock().unwrap();
        let bot = all.entry("alice".into()).or_default();
        bot.native_permissions = compiled.then_some(permissions);
        bot.walk_globals = Some(Arc::new(Mutex::new(WalkGlobals {
            allow_bank_fetch: true,
            ..Default::default()
        })));
        drop(all);
        let options = crate::walk_permissions::compiled_options(
            &rig.navs,
            "alice",
            nav_planner::router::FindOptions::default(),
        );
        assert_eq!(options.allow_teleports, compiled);
        assert_eq!(options.allow_wilderness, compiled);
        assert!(!options.allow_bank_fetch);
        assert_eq!(options.zones.is_all(), compiled);
        assert!(dispatch_script_interact(
            &mut rig.client,
            &rig.snapshot,
            None,
            Some((0, 0, 0)),
            &rig.navs,
            &rig.world,
            None,
            "alice",
            vec![script::shim::InteractReq::Walk {
                x: 4,
                z: 0,
                level: 0,
                allow_teleports: false,
                allow_wilderness: false,
                allow_bank_fetch: false,
                request_id: 1,
                avoid: vec![],
                cross: vec![],
            }],
        ));
        assert!(rig.wait_for_route(5_000, || {
            rig.navs
                .lock()
                .unwrap()
                .get("alice")
                .is_some_and(|bot| !bot.route_work_pending())
        }));
        let all = rig.navs.lock().unwrap();
        let bot = &all["alice"];
        assert_eq!(bot.has_route(), compiled);
        if compiled {
            let request = bot.requested_route.unwrap();
            assert!(request.2 && request.3 && request.5.is_all());
            assert!(!request.4);
        } else {
            assert!(bot.requested_route.is_none());
            assert!(bot.walk_outcome_failed);
        }
    }
}

#[test]
fn compat_catalog_exclusions_use_baked_group_geometry_and_rules() {
    use script::shim::InspectAvoidWire;

    let world = catalog_open_world();
    let table = world.graph.zones.as_ref().expect("zone table");
    let group = |id: &str| {
        table
            .groups()
            .iter()
            .find(|group| group.id.as_ref() == id)
            .expect("catalog group")
    };
    let white_wolf_rect = group("white-wolf-mountain").rect;
    let jail_group = group("draynor-jail-guards");
    let jail_rect = jail_group.rect;
    let jail_member_rects: Vec<_> = jail_group
        .members
        .iter()
        .map(|member| {
            let zone = &table.zones()[usize::from(*member)];
            nav_planner::router::AvoidRect {
                min_x: zone.min_x,
                max_x: zone.max_x,
                min_z: zone.min_z,
                max_z: zone.max_z,
                level: Some(i32::from(zone.level)),
            }
        })
        .collect();
    assert_eq!(jail_member_rects.len(), 4);
    let outside = WorldTile {
        x: 0,
        z: 0,
        level: 0,
    };
    let inside_jail = WorldTile {
        x: jail_member_rects[0].min_x,
        z: jail_member_rects[0].min_z,
        level: jail_member_rects[0].level.unwrap(),
    };
    let bbox_gap = WorldTile {
        x: 15,
        z: 4,
        level: jail_rect.level.unwrap(),
    };
    assert!(jail_rect.contains(bbox_gap));
    assert!(
        !jail_member_rects.iter().any(|rect| rect.contains(bbox_gap)),
        "the fixture gap is inside the group bounding box but outside every guard rectangle"
    );
    let resolve_avoid = |from, to, state: &WorldState, id: &str| {
        let mut exclusions = crate::script_runtime::ScriptRouteExclusions::default();
        exclusions
            .avoid_wire
            .push(InspectAvoidWire::Catalog(id.to_string()));
        crate::script_runtime::resolve_route_exclusions(
            nav_planner::router::FindOptions::default(),
            &world,
            from,
            to,
            state,
            exclusions,
        )
    };

    let state = WorldState::empty();
    let (_, white_wolf) = resolve_avoid(outside, outside, &state, "white-wolf-mountain").unwrap();
    assert_eq!(white_wolf.avoid, [white_wolf_rect]);

    let (_, jail) = resolve_avoid(outside, outside, &state, "draynor-jail-guards").unwrap();
    assert_eq!(jail.avoid, jail_member_rects);
    assert!(
        !jail.avoid.iter().any(|rect| rect.contains(bbox_gap)),
        "a tile inside the bounding box but outside every actual rectangle is not avoided"
    );
    let (_, gap_origin) = resolve_avoid(bbox_gap, outside, &state, "draynor-jail-guards").unwrap();
    assert_eq!(
        gap_origin.avoid, jail_member_rects,
        "an endpoint in the bounding-box gap does not exempt the jail"
    );
    let (_, gap_destination) =
        resolve_avoid(outside, bbox_gap, &state, "draynor-jail-guards").unwrap();
    assert_eq!(
        gap_destination.avoid, jail_member_rects,
        "a destination in the bounding-box gap does not exempt the jail"
    );

    let mut high_combat = WorldState::empty();
    high_combat.combat_level = Some(51);
    let (_, skipped_for_combat) =
        resolve_avoid(outside, outside, &high_combat, "draynor-jail-guards").unwrap();
    assert!(skipped_for_combat.avoid.is_empty());
    let (_, skipped_for_source_endpoint) =
        resolve_avoid(inside_jail, outside, &state, "draynor-jail-guards").unwrap();
    assert!(skipped_for_source_endpoint.avoid.is_empty());
    let (_, skipped_for_destination_endpoint) =
        resolve_avoid(outside, inside_jail, &state, "draynor-jail-guards").unwrap();
    assert!(skipped_for_destination_endpoint.avoid.is_empty());
}

#[test]
fn compat_named_exclusions_reject_unknown_and_nonfrozen_catalog_names() {
    use script::shim::InspectAvoidWire;

    let world = catalog_open_world();
    let from = WorldTile {
        x: 0,
        z: 0,
        level: 0,
    };
    let state = WorldState::empty();
    let mut unknown = crate::script_runtime::ScriptRouteExclusions::default();
    unknown
        .avoid_wire
        .push(InspectAvoidWire::Catalog("no-such-zone".to_string()));
    assert_eq!(
        crate::script_runtime::resolve_route_exclusions(
            nav_planner::router::FindOptions::default(),
            &world,
            from,
            from,
            &state,
            unknown,
        )
        .unwrap_err(),
        "avoidZones: unknown zone \"no-such-zone\""
    );
    let mut nonfrozen = crate::script_runtime::ScriptRouteExclusions::default();
    nonfrozen.avoid_wire.push(InspectAvoidWire::Catalog(
        "death-plateau-throwers".to_string(),
    ));
    assert_eq!(
        crate::script_runtime::resolve_route_exclusions(
            nav_planner::router::FindOptions::default(),
            &world,
            from,
            from,
            &state,
            nonfrozen,
        )
        .unwrap_err(),
        "avoidZones: unknown zone \"death-plateau-throwers\""
    );

    let mut unknown_cross = crate::script_runtime::ScriptRouteExclusions::default();
    unknown_cross.cross.push(Arc::from("no-such-zone"));
    assert_eq!(
        crate::script_runtime::resolve_route_exclusions(
            nav_planner::router::FindOptions::default(),
            &world,
            from,
            from,
            &state,
            unknown_cross,
        )
        .unwrap_err(),
        "crossZones: unknown zone \"no-such-zone\""
    );

    let too_many = crate::script_runtime::ScriptRouteExclusions {
        cross: (0..9).map(|_| Arc::from("test-barrier@2,0,0")).collect(),
        ..Default::default()
    };
    assert_eq!(
        crate::script_runtime::resolve_route_exclusions(
            nav_planner::router::FindOptions::default(),
            &world,
            from,
            from,
            &state,
            too_many,
        )
        .unwrap_err(),
        "crossZones: more than 8 zone names"
    );
}

#[test]
fn compat_catalog_exclusions_reject_groups_without_baked_geometry() {
    use script::shim::InspectAvoidWire;

    let world = zoned_open_world();
    let from = WorldTile {
        x: 0,
        z: 0,
        level: 0,
    };
    let mut exclusions = crate::script_runtime::ScriptRouteExclusions::default();
    exclusions
        .avoid_wire
        .push(InspectAvoidWire::Catalog("white-wolf-mountain".to_string()));
    assert_eq!(
        crate::script_runtime::resolve_route_exclusions(
            nav_planner::router::FindOptions::default(),
            &world,
            from,
            from,
            &WorldState::empty(),
            exclusions,
        )
        .unwrap_err(),
        "avoidZones: catalog zone \"white-wolf-mountain\" has no baked group geometry"
    );
}

fn blocked_end_world() -> NavWorld {
    let mut world = open_world(40, 1);
    world.collision.blocked[0] |= 1 << 39;
    world
}

fn two_walk_rig() -> Rig {
    let mut rig = rig(Some(Arc::new(blocked_end_world())), false);
    {
        let mut shared = rig.shared.lock();
        shared.walks = 2;
        shared.later_target = Some(WorldTile {
            x: 39,
            z: 0,
            level: 0,
        });
    }
    rig.observe(1);
    rig.wait_routed();
    rig
}

/// Drive ticks until the worker publishes both walk results, up to a
/// load-proof 60 s: each `observe` advances the simulation, and the 50 ms
/// sleeps only pace the polling.
fn drive_second_walk(rig: &mut Rig, first_tick: u64) -> Vec<Result<WalkEnd, ActionError>> {
    for tick in (first_tick..).take(1_200) {
        rig.observe(tick);
        if rig.shared.lock().results.len() >= 2 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    rig.shared
        .lock()
        .results
        .iter()
        .map(|result| result.clone().map(|receipt| receipt.end))
        .collect()
}

#[test]
fn review_control_generic_cancel_then_nopath_delivers_no_path() {
    let mut rig = two_walk_rig();
    {
        let slot = rig.slot();
        let mut slot = slot.lock().unwrap();
        crate::script_runtime::pause_script(&mut slot, &rig.navs, "alice");
        slot.resume();
    }
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::Cancelled)));
    let ends = drive_second_walk(&mut rig, 3);
    eprintln!(
        "manual-click resident NavBot={}",
        std::mem::size_of::<NavBot>()
    );
    assert_eq!(ends.len(), 2, "control: the second walk gets its terminal");
    assert_eq!(ends[1], Ok(WalkEnd::NoPath));
}

#[test]
fn review_manual_takeover_then_nopath_must_still_deliver_a_terminal() {
    let mut rig = two_walk_rig();
    assert!(crate::script_runtime::take_manual_walk_ownership(
        &rig.scripts,
        &rig.navs,
        "alice",
        manual_frame(false),
        true,
        1,
        false,
    ));
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::UserInput)));
    let first_request = rig.navs.lock().unwrap()["alice"].walk_outcome_request_id;
    let ends = drive_second_walk(&mut rig, 3);
    let bots = rig.navs.lock().unwrap();
    eprintln!(
        "manual-click native probe: ends={ends:?} NavBot={} live_refusal_id={} (cancelled request {first_request}) outcome_request_id={} reason={:?} native_walk_live={} requested_route={:?}",
        std::mem::size_of::<NavBot>(),
        bots["alice"].walk_live_refusal_id,
        bots["alice"].walk_outcome_request_id,
        bots["alice"].walk_outcome_cancel_reason,
        bots["alice"].native_walk.is_some(),
        bots["alice"].requested_route,
    );
    assert_eq!(
        ends.len(),
        2,
        "after a manual takeover the next native walk's NoPath must reach its owner"
    );
    assert_eq!(ends[1], Ok(WalkEnd::NoPath));
}

#[test]
fn review_native_recovery_click_then_nopath_delivers_terminal() {
    let mut rig = two_walk_rig();
    let first_native = rig.navs.lock().unwrap()["alice"].walk_request_id;
    super::apply_watchdog_nav_action(
        script::WatchdogAction::ArmWalk {
            x: 8,
            z: 0,
            level: 0,
        },
        &mut rig.driver,
        Some(&rig.snapshot),
        Some((0, 0, 0)),
        &rig.navs,
        &rig.world,
        Some(&WorldState::empty()),
        None,
        "alice",
    );
    {
        let navs = rig.navs.lock().unwrap();
        let bot = &navs["alice"];
        eprintln!(
            "manual-click recovery probe: native_request={first_native} \
             walk_request_id={} native_walk={} carried_walk={} live_refusal_id={}",
            bot.walk_request_id,
            bot.native_walk.is_some(),
            bot.carried_walk.is_some(),
            bot.walk_live_refusal_id,
        );
    }
    rig.observe(2);
    assert_eq!(
        rig.end(),
        Some(Ok(WalkEnd::Cancelled)),
        "replacing the native follow ends its first owner"
    );
    assert!(crate::script_runtime::take_manual_walk_ownership(
        &rig.scripts,
        &rig.navs,
        "alice",
        manual_frame(false),
        true,
        2,
        false,
    ));
    {
        let navs = rig.navs.lock().unwrap();
        let bot = &navs["alice"];
        eprintln!(
            "manual-click after recovery click: outcome_request_id={} reason={:?} \
             live_refusal_id={} native_walk={} carried_walk={}",
            bot.walk_outcome_request_id,
            bot.walk_outcome_cancel_reason,
            bot.walk_live_refusal_id,
            bot.native_walk.is_some(),
            bot.carried_walk.is_some(),
        );
    }
    let ends = drive_second_walk(&mut rig, 3);
    eprintln!("manual-click native recovery treatment: ends={ends:?}");
    assert_eq!(
        ends.len(),
        2,
        "after native recovery identity and click, the next native NoPath reaches its owner"
    );
    assert_eq!(ends[1], Ok(WalkEnd::NoPath));
}

#[test]
fn manual_cancellation_detail_clears_when_the_next_walk_arms() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    assert!(crate::script_runtime::take_manual_walk_ownership(
        &rig.scripts,
        &rig.navs,
        "alice",
        manual_frame(false),
        true,
        2,
        false,
    ));
    assert!(rig.navs.lock().unwrap()["alice"].manual_walk_cancelled_detail);

    let arm = super::ScriptWalkArm {
        here: Some((0, 0, 0)),
        world: rig.world.clone(),
        navs: Arc::clone(&rig.navs),
        name: "alice".into(),
        state: Some(WorldState::empty()),
        bank: BankRows::default(),
    };
    let completed = arm
        .queue_route_in_snapshot_synced(
            &rig.snapshot,
            8,
            0,
            0,
            nav_planner::router::FindOptions::default(),
            0,
            true,
            99,
            None,
        )
        .expect("the next walk route arms");
    assert!(
        rig.wait_for_route(5_000, || {
            rig.navs.lock().unwrap().get("alice").is_some_and(|bot| {
                bot.route_destination()
                    == Some(WorldTile {
                        x: 8,
                        z: 0,
                        level: 0,
                    })
                    && !bot.route_work_pending()
            })
        }),
        "the new route work did not complete"
    );
    assert!(
        !rig.navs.lock().unwrap()["alice"].manual_walk_cancelled_detail,
        "the detail clears on the next route arm"
    );
    assert_eq!(
        rig.navs.lock().unwrap()["alice"].walk_outcome_cancel_reason,
        script::native::WalkCancelReason::UserInput,
        "the previously published receipt remains intact"
    );
    completed
        .recv_timeout(std::time::Duration::from_secs(60))
        .expect("the route work completion signal was not published");
}

#[test]
fn manual_takeover_of_carried_walk_keeps_its_receipt_identity() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    let (request_id, generation, key) = {
        let mut navs = rig.navs.lock().unwrap();
        let bot = navs.get_mut("alice").unwrap();
        bot.native_walk = None;
        (
            bot.walk_request_id,
            bot.route_generation,
            bot.requested_route
                .expect("the route keeps its request identity"),
        )
    };
    crate::script_runtime::hold_script_nav(&rig.navs, "alice", Some(7));
    let (nav_size, carried_size) = {
        let navs = rig.navs.lock().unwrap();
        let bot = &navs["alice"];
        let carried = bot.carried_walk.as_deref().expect("the route is carried");
        (std::mem::size_of_val(bot), std::mem::size_of_val(carried))
    };
    eprintln!("manual-click resident sizes: NavBot={nav_size} CarriedWalk={carried_size}");
    {
        let mut navs = rig.navs.lock().unwrap();
        let bot = navs.get_mut("alice").unwrap();
        bot.cancel_for_manual_input();
        let (to, radius, teleports, _, _, _) = key;
        assert_eq!(bot.walk_outcome_request_id, request_id);
        assert_eq!(bot.walk_outcome_generation, generation);
        assert_eq!(
            (
                bot.walk_outcome_x,
                bot.walk_outcome_z,
                bot.walk_outcome_level
            ),
            (to.x, to.z, to.level)
        );
        assert_eq!(bot.walk_outcome_radius, radius);
        assert_eq!(bot.walk_outcome_allow_teleports, teleports);
        assert_eq!(
            bot.walk_outcome_cancel_reason,
            script::native::WalkCancelReason::UserInput
        );
    }
}

#[cfg(feature = "js")]
#[test]
fn manual_click_terminal_resets_on_stop_before_the_next_load_snapshot() {
    let mut play = run_with_io(
        &PlayOptions {
            host: "127.0.0.1".into(),
            transport: client::Transport::Tcp,
            port: 43594,
            cache_dir: "/tmp".into(),
            lowmem: true,
            mainland: false,
        },
        vec![],
        |_| (None, None),
        |_, _, _| {},
    );
    play.attach_arm("alice", SlotArm::new(7, false));
    let source = "export function tick(api) {}";
    play.script_start_load(
        "alice",
        source.into(),
        script_js::LoadShape::NativeTick,
        None,
        vec![],
    )
    .unwrap();
    wait_script_state(&play, "alice", script::RunState::Running);
    play.navs
        .lock()
        .unwrap()
        .insert("alice".into(), super::following_script_walk());
    assert!(crate::script_runtime::take_manual_walk_ownership(
        &play.scripts,
        &play.navs,
        "alice",
        manual_frame(false),
        true,
        1,
        false,
    ));
    assert_eq!(
        play.navs.lock().unwrap()["alice"].walk_outcome_cancel_reason,
        script::native::WalkCancelReason::UserInput
    );
    assert!(play.script_walk_cancelled_by_user("alice"));

    play.script_stop("alice");
    let navs = play.navs.lock().unwrap();
    assert_eq!(
        navs["alice"].walk_outcome_cancel_reason,
        script::native::WalkCancelReason::None,
        "Stop clears the prior run's UserInput outcome"
    );
    assert_eq!(navs["alice"].walk_outcome_request_id, 0);
    assert_eq!(navs["alice"].walk_live_refusal_id, 0);
    drop(navs);
    assert!(!play.script_walk_cancelled_by_user("alice"));

    play.script_start_load(
        "alice",
        source.into(),
        script_js::LoadShape::NativeTick,
        None,
        vec![],
    )
    .unwrap();
    wait_script_state(&play, "alice", script::RunState::Running);
    let mut client = nav_client();
    let mut snapshot = GameSnapshot::new();
    nav_snapshot_at(&mut client, &mut snapshot, 0, 0);
    script_observe(
        &mut client,
        "alice",
        true,
        true,
        1,
        Some((0, 0, 0)),
        None,
        None,
        Some(&snapshot),
        None,
        &play.scripts,
        &play.cheats,
        &play.navs,
        &play.world,
        false,
        false,
    );
    let reason = script_slot(&play.scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe("globalThis.__rs2b0t_host.snapshot.walk_outcome_cancel_reason")
        .unwrap();
    assert_eq!(
        reason.as_str(),
        Some("none"),
        "the next owner's first Load snapshot starts without the prior cancellation"
    );
    play.script_stop("alice");
}

fn manual_frame(hold: bool) -> crate::SlotFrameInput {
    crate::SlotFrameInput {
        hold,
        host_move_owned: false,
        manual_move_intent: Some(crate::ManualMoveIntent::Minimap),
        manual_steps: 0,
    }
}

/// An admitted machine with no walking request or API session.
struct InPlace {
    command: Arc<std::sync::atomic::AtomicU8>,
}

impl script::native::NativeMachine for InPlace {
    type Args = Arc<std::sync::atomic::AtomicU8>;
    type Output = ();

    fn begin(
        command: Self::Args,
        _: &mut script::native::ActionContext<'_>,
    ) -> Result<Self, ActionError> {
        command.store(4, std::sync::atomic::Ordering::Relaxed);
        Ok(Self { command })
    }

    fn poll(&mut self, _: &mut script::native::ActionContext<'_>) -> Poll<Result<(), ActionError>> {
        if self.command.load(std::sync::atomic::Ordering::Relaxed) == 2 {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }
}

struct InPlaceScript {
    command: Arc<std::sync::atomic::AtomicU8>,
    handle: Option<ActionHandle<InPlace>>,
    begun: bool,
}

impl Script for InPlaceScript {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        let command = self.command.load(std::sync::atomic::Ordering::Relaxed);
        if command == 1 {
            self.handle = None;
        } else if let Some(handle) = &self.handle {
            if tick.actions.poll(handle, &mut tick.cx).is_ready() {
                self.handle = None;
            }
        } else if !self.begun && command != 3 {
            self.handle = Some(
                tick.actions
                    .begin::<InPlace>(Arc::clone(&self.command), &mut tick.cx)
                    .unwrap(),
            );
            self.begun = true;
        }
        Ok(ScriptFlow::Continue)
    }
}

fn in_place_rig(command: u8) -> (Rig, Arc<std::sync::atomic::AtomicU8>) {
    let mut rig = open_rig(false);
    rig.slot().lock().unwrap().stop();
    let command = Arc::new(std::sync::atomic::AtomicU8::new(command));
    rig.slot()
        .lock()
        .unwrap()
        .start_test_script(
            Box::new(InPlaceScript {
                command: Arc::clone(&command),
                handle: None,
                begun: false,
            }),
            None,
        )
        .unwrap();
    rig.observe(1);
    (rig, command)
}

struct TakeoverLog(Mutex<Vec<String>>);

impl api::hostlog::Sink for TakeoverLog {
    fn record(&self, record: &api::hostlog::Record<'_>) {
        if record
            .message
            .contains("walk outcome=cancelled reason=UserInput")
        {
            self.0.lock().unwrap().push(record.message.to_owned());
        }
    }
}

static TAKEOVER_LOG: TakeoverLog = TakeoverLog(Mutex::new(Vec::new()));

#[test]
fn takeover_is_taken_for_a_live_native_owner_without_a_walk() {
    const CHILD: &str = "S2_NATIVE_TAKEOVER_LOG_TEST";
    if std::env::var(CHILD).as_deref() != Ok("1") {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::native_walk::takeover_is_taken_for_a_live_native_owner_without_a_walk",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .expect("run isolated takeover sink test");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("1 passed; 0 failed;"),
            "takeover child failed: {stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    assert!(api::hostlog::install_sink(&TAKEOVER_LOG));
    let (rig, command) = in_place_rig(0);
    assert_eq!(command.load(std::sync::atomic::Ordering::Relaxed), 4);
    let slot = rig.slot();
    {
        let slot = slot.lock().unwrap();
        assert!(!slot.live_walking_operation());
        assert!(!slot.surface_active());
    }
    assert!(!rig.navs.lock().unwrap()["alice"].walk_work_live());
    let before = rig.navs.lock().unwrap()["alice"].walk_outcome_seq;
    assert!(crate::script_runtime::take_manual_walk_ownership(
        &rig.scripts,
        &rig.navs,
        "alice",
        manual_frame(false),
        true,
        41,
        false,
    ));
    let bots = rig.navs.lock().unwrap();
    assert_eq!(bots["alice"].walk_outcome_seq, before + 1);
    assert_eq!(bots["alice"].manual_takeover_watermark, before + 1);
    drop(bots);
    assert!(TAKEOVER_LOG.0.lock().unwrap().iter().any(|line| {
        line.contains("tick=41 live_work=true active_walk=false live_native_owner=true live_walking_operation=false api_session=false")
    }));
}

#[test]
fn takeover_with_pause_on_pauses_a_native_owner_without_a_walk() {
    let (rig, _) = in_place_rig(0);
    let before = rig.navs.lock().unwrap()["alice"].walk_outcome_seq;
    assert!(crate::script_runtime::take_manual_walk_ownership(
        &rig.scripts,
        &rig.navs,
        "alice",
        manual_frame(false),
        true,
        1,
        true,
    ));
    assert_eq!(rig.slot().lock().unwrap().state(), script::RunState::Paused);
    assert_eq!(
        rig.navs.lock().unwrap()["alice"].walk_outcome_seq,
        before + 1
    );
}

#[test]
fn takeover_is_refused_for_an_absent_revoked_or_completed_owner() {
    for command in [3, 1, 2] {
        let (mut rig, control) = in_place_rig(if command == 3 { 3 } else { 0 });
        control.store(command, std::sync::atomic::Ordering::Relaxed);
        rig.observe(2);
        assert!(!rig.slot().lock().unwrap().has_native_actions());
        let before = rig.navs.lock().unwrap()["alice"].walk_outcome_seq;
        assert!(!crate::script_runtime::take_manual_walk_ownership(
            &rig.scripts,
            &rig.navs,
            "alice",
            manual_frame(false),
            true,
            2,
            false,
        ));
        assert_eq!(
            rig.navs.lock().unwrap()["alice"].walk_outcome_seq,
            before,
            "owner command {command}"
        );
    }
}

#[cfg(feature = "js")]
fn in_place_load_rig() -> Rig {
    let mut rig = open_rig(false);
    rig.slot().lock().unwrap().stop();
    let start = crate::script_runtime::js_surface_start(
        crate::script_runtime::js_start_card(
            "export function tick(api) { globalThis.s2Ticks = (globalThis.s2Ticks || 0) + 1; }"
                .into(),
            script_js::LoadShape::NativeTick,
            Vec::new(),
        ),
        None,
        None,
        Arc::new(api::named_banks::NamedBankFacts::empty()),
        Arc::from(Vec::<api::loadout::Loadout>::new()),
    );
    {
        let slot = rig.slot();
        let mut slot = slot.lock().unwrap();
        slot.bind_surface_factory(Arc::new(script_js::surface::JsSurfaceFactory::default()));
        slot.start_surface(start).unwrap();
    }
    rig.slot().lock().unwrap().probe("true").unwrap();
    rig.observe(1);
    rig.slot().lock().unwrap().probe("true").unwrap();
    rig
}

#[cfg(feature = "js")]
#[test]
fn takeover_with_pause_on_pauses_a_load_script_without_a_walk() {
    let mut rig = in_place_load_rig();
    let slot = rig.slot();
    let ticks = slot.lock().unwrap().probe("globalThis.s2Ticks").unwrap();
    assert!(!slot.lock().unwrap().manual_input_owner());
    assert!(crate::script_runtime::take_manual_walk_ownership(
        &rig.scripts,
        &rig.navs,
        "alice",
        manual_frame(false),
        true,
        1,
        true,
    ));
    assert_eq!(slot.lock().unwrap().state(), script::RunState::Paused);
    rig.observe(2);
    assert_eq!(
        slot.lock().unwrap().probe("globalThis.s2Ticks").unwrap(),
        ticks
    );
    // Non-walking compat rows are covered by the live Thiever cell:
    // manual_takeover_compat_live_tests::compat_thiever_pauses_on_viewport_click.
    slot.lock().unwrap().resume();
    rig.observe(3);
    assert_eq!(slot.lock().unwrap().state(), script::RunState::Running);
    assert!(
        slot.lock()
            .unwrap()
            .probe("globalThis.s2Ticks")
            .unwrap()
            .as_u64()
            > ticks.as_u64()
    );
}

#[cfg(feature = "js")]
#[test]
fn takeover_with_pause_off_leaves_a_load_script_without_a_walk_alone() {
    let rig = in_place_load_rig();
    let before = rig.navs.lock().unwrap()["alice"].walk_outcome_seq;
    assert!(!crate::script_runtime::take_manual_walk_ownership(
        &rig.scripts,
        &rig.navs,
        "alice",
        manual_frame(false),
        true,
        1,
        false,
    ));
    assert_eq!(
        rig.slot().lock().unwrap().state(),
        script::RunState::Running
    );
    assert_eq!(rig.navs.lock().unwrap()["alice"].walk_outcome_seq, before);
}

#[test]
fn manual_step_takeover_must_end_active_walk_before_follow() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    let request = rig.navs.lock().unwrap()["alice"].walk_request_id;
    let input = host::SlotInput::new();
    let queues = Mutex::new(HashMap::from([(
        "alice".into(),
        VecDeque::from([crate::WireCmd::Walk {
            x: 0,
            z: 1,
            level: 0,
        }]),
    )]));
    let (frame, human) = crate::play_slots::take_slot_frame_input(&input, "alice", &queues, false);
    assert_eq!(frame.manual_move_count(), 1);
    assert_eq!(human.len(), 1, "the human step is preserved");
    assert!(crate::script_runtime::take_manual_walk_ownership(
        &rig.scripts,
        &rig.navs,
        "alice",
        frame,
        true,
        1,
        false,
    ));
    assert!(
        !rig.walk_armed(),
        "manual movement takes ownership before either follow pump"
    );
    let before = rig.navs.lock().unwrap()["alice"].walk_outcome_seq;
    rig.step();
    rig.observe(2);
    assert_eq!(
        rig.driver.walked, None,
        "the old walk must not send a later hop"
    );
    assert_eq!(rig.end(), Some(Ok(WalkEnd::UserInput)));
    assert_eq!(rig.shared.lock().results.len(), 1, "exactly one terminal");
    assert_eq!(
        rig.slot().lock().unwrap().state(),
        script::RunState::Running,
        "OFF cancels without forcing the owning script to pause"
    );
    let mut bots = rig.navs.lock().unwrap();
    let bot = bots.get_mut("alice").unwrap();
    assert_eq!(bot.walk_outcome_request_id, request);
    assert_eq!(
        bot.walk_outcome_cancel_reason,
        script::native::WalkCancelReason::UserInput
    );
    assert_eq!(bot.user_move_intent_seq, 1);
    assert_eq!(bot.manual_takeover_watermark, before);
    assert!(!bot.walking_decision_is_current(before - 1));
    assert!(
        bot.walking_decision_is_current(before),
        "fresh observed decisions are permitted"
    );
    bot.mark_walk_outcome_posted(before);
    bot.clear_walk_outcome();
    assert_eq!(
        bot.walk_outcome_seq, before,
        "old release cannot erase cancellation"
    );
    drop(bots);
    rig.observe(3);
    rig.step();
    assert_eq!(rig.shared.lock().results.len(), 1);
    assert_eq!(rig.driver.walked, None);
}

#[test]
fn manual_config_on_native_pause_revokes_action_resume_makes_fresh_decision() {
    let mut rig = two_walk_rig();
    let original = rig.navs.lock().unwrap()["alice"].walk_request_id;
    assert!(crate::script_runtime::take_manual_walk_ownership(
        &rig.scripts,
        &rig.navs,
        "alice",
        manual_frame(false),
        true,
        1,
        true,
    ));
    assert_eq!(rig.slot().lock().unwrap().state(), script::RunState::Paused);
    assert!(!rig.slot().lock().unwrap().has_native_actions());
    assert!(rig.navs.lock().unwrap()["alice"].carried_walk.is_none());
    rig.observe(2);
    rig.step();
    assert_eq!(rig.driver.walked, None);
    assert_eq!(rig.shared.lock().begun, 1, "no decision while Paused");
    rig.slot().lock().unwrap().resume();
    let ends = drive_second_walk(&mut rig, 3);
    assert_eq!(ends.len(), 2);
    assert_eq!(
        ends[1],
        Ok(WalkEnd::NoPath),
        "the explicit Resume permits a fresh decision"
    );
    let shared = rig.shared.lock();
    let next = shared.results[1].as_ref().unwrap();
    assert_ne!(
        next.request_id, original,
        "the next walk is not replay of the cancelled request"
    );
}

#[test]
fn held_manual_click_still_cancels_and_other_slot_survives() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    let route = rig.navs.lock().unwrap()["alice"].compact_route.clone();
    rig.navs.lock().unwrap().insert(
        "bob".into(),
        NavBot {
            compact_route: route,
            walk_request_id: 99,
            ..NavBot::default()
        },
    );
    assert!(crate::script_runtime::take_manual_walk_ownership(
        &rig.scripts,
        &rig.navs,
        "alice",
        manual_frame(true),
        true,
        1,
        false,
    ));
    assert!(!rig.walk_armed());
    assert!(rig.navs.lock().unwrap()["bob"].script_walk_armed());
    let seq = rig.navs.lock().unwrap()["alice"].walk_outcome_seq;
    rig.observe(2);
    assert!(
        !crate::script_runtime::take_manual_walk_ownership(
            &rig.scripts,
            &rig.navs,
            "alice",
            manual_frame(true),
            true,
            2,
            false,
        ),
        "an already ended operation cannot produce a second terminal"
    );
    let bots = rig.navs.lock().unwrap();
    assert_eq!(bots["alice"].walk_outcome_seq, seq);
    assert_eq!(bots["alice"].user_move_intent_seq, 2);
}

#[test]
fn manual_round3_click_after_arrival_keeps_the_completed_receipt() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    crate::script_runtime::apply_nav_follow_outcome(
        rig.navs.lock().unwrap().get_mut("alice").unwrap(),
        Some(nav_executor::TravelOutcome::Arrived {
            at: WorldTile {
                x: 4,
                z: 0,
                level: 0,
            },
        }),
        false,
    );
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::RouteEnded)));
    let before = {
        let navs = rig.navs.lock().unwrap();
        let bot = &navs["alice"];
        assert!(!bot.has_route());
        assert!(bot.requested_route.is_some(), "dedupe retains the identity");
        (
            bot.walk_outcome_seq,
            bot.walk_outcome_request_id,
            bot.route_generation,
        )
    };
    let taken = crate::script_runtime::take_manual_walk_ownership(
        &rig.scripts,
        &rig.navs,
        "alice",
        manual_frame(false),
        true,
        2,
        false,
    );
    {
        let navs = rig.navs.lock().unwrap();
        let bot = &navs["alice"];
        eprintln!(
            "click-after-arrival taken={taken} outcome_seq={} request={} failed={} reason={:?} intent={}",
            bot.walk_outcome_seq,
            bot.walk_outcome_request_id,
            bot.walk_outcome_failed,
            bot.walk_outcome_cancel_reason,
            bot.user_move_intent_seq,
        );
        assert!(!taken, "a completed request is not active walking");
        assert_eq!(
            (
                bot.walk_outcome_seq,
                bot.walk_outcome_request_id,
                bot.route_generation
            ),
            before,
            "the click must not replace the completed receipt"
        );
        assert!(!bot.walk_outcome_failed);
        assert_eq!(
            bot.walk_outcome_cancel_reason,
            script::native::WalkCancelReason::None
        );
        assert_eq!(bot.manual_takeover_watermark, 0);
        assert_eq!(bot.user_move_intent_seq, 1, "idle intent is still observed");
    }
    rig.observe(3);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::RouteEnded)));
    assert_eq!(rig.shared.lock().results.len(), 1);
}

/// A compiled fight keeps its walk request registered after the walk arrives
/// (`script/src/slot.rs` `live_walking_operation`), so a later manual step is
/// an eligible takeover. The host then has no correlated request: the posted
/// reason stays `None` and only the sequence and the watermark move. The
/// compiled side still reads that takeover outcome as `UserInput`, and the
/// next ordinary outcome clears it.
#[test]
fn takeover_after_an_arrived_walk_moves_only_the_sequence_and_reads_user_input_compiled() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    crate::script_runtime::apply_nav_follow_outcome(
        rig.navs.lock().unwrap().get_mut("alice").unwrap(),
        Some(nav_executor::TravelOutcome::Arrived {
            at: WorldTile {
                x: 4,
                z: 0,
                level: 0,
            },
        }),
        false,
    );
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::RouteEnded)));
    let mut navs = rig.navs.lock().unwrap();
    let bot = navs.get_mut("alice").unwrap();
    let (seq, request, generation) = (
        bot.walk_outcome_seq,
        bot.walk_outcome_request_id,
        bot.walk_outcome_generation,
    );
    assert_eq!(
        bot.compiled_walk_cancel_reason(),
        script::native::WalkCancelReason::None
    );

    bot.cancel_for_manual_input();
    assert_eq!(
        bot.walk_outcome_cancel_reason,
        script::native::WalkCancelReason::None,
        "no correlated request: the posted Load reason stays None"
    );
    assert_eq!(bot.walk_outcome_request_id, request);
    assert_eq!(bot.walk_outcome_generation, generation);
    assert!(!bot.walk_outcome_failed);
    assert_eq!(bot.walk_outcome_seq, seq + 1, "the sequence moves");
    assert_eq!(
        bot.manual_takeover_watermark,
        seq + 1,
        "the watermark moves"
    );
    assert_eq!(
        bot.compiled_walk_cancel_reason(),
        script::native::WalkCancelReason::UserInput
    );

    bot.clear_walk_outcome();
    assert_eq!(bot.walk_outcome_seq, seq + 2);
    assert_eq!(
        bot.compiled_walk_cancel_reason(),
        script::native::WalkCancelReason::None,
        "a later ordinary outcome is not a takeover"
    );
}

/// Review probe F1 (M0ReviewSol), with the gate expression replaced by the
/// live cell's gate: an admitted compact route (`route` is None) is walking
/// work, so the M0 cell must not inject.
#[test]
fn m0_host_gate_must_reject_a_live_compact_route() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    let bots = rig.navs.lock().unwrap();
    let bot = &bots["alice"];
    assert!(
        bot.compact_route.is_some() && bot.route.is_none(),
        "exercise the actual admitted compact route"
    );
    assert!(bot.has_route(), "the native walk has not arrived");
    assert!(bot.native_walk.as_ref().is_some_and(|owner| owner.live()));
    let host = crate::combat_live_proof_tests::m0_host_state(bot);
    assert_eq!(host["compact_route"], true);
    assert_eq!(host["walk_work_live"], true);
    assert!(
        !crate::combat_live_proof_tests::m0_host_gate_open(&host),
        "M0 must not inject on a live compact route"
    );
}

/// The allowed case: the route arrived and the native walk owner is still
/// registered and live.
#[test]
fn m0_host_gate_accepts_an_arrived_walk_with_a_live_owner() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    let mut bots = rig.navs.lock().unwrap();
    let bot = bots.get_mut("alice").unwrap();
    crate::script_runtime::apply_nav_follow_outcome(
        bot,
        Some(nav_executor::TravelOutcome::Arrived {
            at: WorldTile {
                x: 4,
                z: 0,
                level: 0,
            },
        }),
        false,
    );
    assert!(!bot.walk_work_live(), "the route arrived");
    assert!(bot.native_walk.as_ref().is_some_and(|owner| owner.live()));
    let host = crate::combat_live_proof_tests::m0_host_state(bot);
    assert_eq!(host["native_walk_live"], true);
    assert!(crate::combat_live_proof_tests::m0_host_gate_open(&host));
}

/// After a manual takeover, later ordinary outcomes (an arrival, a failure
/// and a navigation reset) move the sequence past the watermark and leave
/// the watermark unchanged, so the compiled side never reuses the old
/// `UserInput` reason.
#[test]
fn later_ordinary_outcomes_do_not_reuse_the_manual_takeover_watermark() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    let mut bots = rig.navs.lock().unwrap();
    let bot = bots.get_mut("alice").unwrap();
    bot.cancel_for_manual_input();
    let watermark = bot.manual_takeover_watermark;
    assert_ne!(watermark, 0);
    assert_eq!(
        bot.compiled_walk_cancel_reason(),
        script::native::WalkCancelReason::UserInput
    );
    let goal = WorldTile {
        x: 4,
        z: 0,
        level: 0,
    };
    bot.walk_request_id = 997;
    bot.route_request_id = 997;
    bot.requested_route = Some(native_requested(goal, 0, false));
    crate::script_runtime::apply_nav_follow_outcome(
        bot,
        Some(nav_executor::TravelOutcome::Arrived { at: goal }),
        false,
    );
    assert_eq!(bot.manual_takeover_watermark, watermark);
    assert!(bot.walk_outcome_seq > watermark);
    assert_eq!(
        bot.compiled_walk_cancel_reason(),
        script::native::WalkCancelReason::None,
        "an arrival after the takeover is not a takeover"
    );
    bot.note_failure(bot.route_generation, 998, goal, 0, false);
    assert!(bot.walk_outcome_seq > watermark);
    assert_eq!(
        bot.compiled_walk_cancel_reason(),
        script::native::WalkCancelReason::None,
        "a failure after the takeover is not a takeover"
    );
    drop(bots);
    crate::script_runtime::reset_script_nav(&rig.navs, "alice", None);
    let bots = rig.navs.lock().unwrap();
    let bot = &bots["alice"];
    assert_eq!(bot.manual_takeover_watermark, watermark);
    assert!(bot.walk_outcome_seq > watermark);
    assert_eq!(
        bot.compiled_walk_cancel_reason(),
        script::native::WalkCancelReason::None,
        "a navigation reset after the takeover is not a takeover"
    );
}

/// A reconnect holds navigation and later resets it. Neither step is a
/// manual takeover: the watermark stays 0 and the compiled reason stays
/// `None`.
#[test]
fn reconnect_hold_and_reset_do_not_create_a_manual_cancel_reason() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    let seq = rig.navs.lock().unwrap()["alice"].walk_outcome_seq;
    let generation = rig.slot().lock().unwrap().runtime_generation();
    crate::script_runtime::hold_script_nav(&rig.navs, "alice", Some(generation));
    let bots = rig.navs.lock().unwrap();
    let bot = &bots["alice"];
    assert_eq!(bot.walk_outcome_seq, seq);
    assert_eq!(bot.manual_takeover_watermark, 0);
    assert_eq!(
        bot.compiled_walk_cancel_reason(),
        script::native::WalkCancelReason::None
    );
    drop(bots);
    crate::script_runtime::reset_script_nav(&rig.navs, "alice", None);
    let bots = rig.navs.lock().unwrap();
    let bot = &bots["alice"];
    assert!(bot.walk_outcome_seq > seq);
    assert_eq!(bot.manual_takeover_watermark, 0);
    assert_eq!(
        bot.compiled_walk_cancel_reason(),
        script::native::WalkCancelReason::None
    );
}

#[test]
fn operator_paused_walk_is_not_cancelled_by_manual_intent() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    {
        let cell = rig.slot();
        let mut slot = cell.lock().unwrap();
        crate::script_runtime::pause_script(&mut slot, &rig.navs, "alice");
    }
    let old_seq = rig.navs.lock().unwrap()["alice"].walk_outcome_seq;
    assert!(!crate::script_runtime::take_manual_walk_ownership(
        &rig.scripts,
        &rig.navs,
        "alice",
        manual_frame(false),
        true,
        1,
        false,
    ));
    let bots = rig.navs.lock().unwrap();
    assert_eq!(bots["alice"].walk_outcome_seq, old_seq);
    assert_eq!(bots["alice"].manual_takeover_watermark, 0);
    assert_eq!(bots["alice"].user_move_intent_seq, 1);
}

#[test]
fn reconnect_gated_owner_is_not_taken_over_before_its_ready_observation() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    {
        let cell = rig.slot();
        let mut slot = cell.lock().unwrap();
        slot.on_is_up(false);
        assert!(
            slot.want_run,
            "reconnect gating does not change operator intent"
        );
        assert_eq!(slot.state(), script::RunState::Paused);
    }
    let original = {
        let bots = rig.navs.lock().unwrap();
        (
            bots["alice"].walk_request_id,
            bots["alice"].walk_outcome_seq,
        )
    };
    assert!(!crate::script_runtime::take_manual_walk_ownership(
        &rig.scripts,
        &rig.navs,
        "alice",
        manual_frame(false),
        true,
        1,
        false,
    ));
    let bots = rig.navs.lock().unwrap();
    assert_eq!(
        (
            bots["alice"].walk_request_id,
            bots["alice"].walk_outcome_seq
        ),
        original
    );
    assert_eq!(bots["alice"].manual_takeover_watermark, 0);
    assert_eq!(bots["alice"].user_move_intent_seq, 1);
}

#[test]
fn takeover_during_pending_worker_cannot_reinstall_follow() {
    let mut rig = open_rig(false);
    rig.observe(1);
    let (generation, request_id) = {
        let bots = rig.navs.lock().unwrap();
        (
            bots["alice"].route_generation,
            bots["alice"].walk_request_id,
        )
    };
    assert!(crate::script_runtime::take_manual_walk_ownership(
        &rig.scripts,
        &rig.navs,
        "alice",
        manual_frame(false),
        true,
        1,
        false,
    ));
    let seq = rig.navs.lock().unwrap()["alice"].walk_outcome_seq;
    rig.navs
        .lock()
        .unwrap()
        .get_mut("alice")
        .unwrap()
        .publish_route(
            generation,
            request_id,
            false,
            crate::walk_plan::RouteOutcome::Routed(Route {
                legs: vec![],
                dest: WorldTile {
                    x: 4,
                    z: 0,
                    level: 0,
                },
                ticks: 4.0,
            }),
        );
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::UserInput)));
    rig.step();
    assert!(!rig.walk_armed());
    assert_eq!(rig.driver.walked, None);
    assert_eq!(rig.navs.lock().unwrap()["alice"].walk_outcome_seq, seq);
}

#[test]
fn queued_walking_decisions_are_fenced_but_observed_fresh_walk_is_allowed() {
    use script::shim::InteractReq;
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    let stale = [
        InteractReq::Walk {
            x: 11,
            z: 0,
            level: 0,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            request_id: 90,
            avoid: vec![],
            cross: vec![],
        },
        InteractReq::WalkNear {
            x: 12,
            z: 0,
            level: 0,
            radius: 1,
            compat_tile_intent: false,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            request_id: 91,
            avoid: vec![],
            cross: vec![],
        },
        InteractReq::WalkNearestBank,
        InteractReq::WalkTo {
            x: 13,
            z: 0,
            level: 0,
        },
    ];
    rig.slot().lock().unwrap().restore_host_interacts(
        stale
            .into_iter()
            .map(|req| script::interact::QueuedInteract {
                req,
                observed_walk_outcome_seq: 0,
            })
            .collect(),
    );
    assert!(crate::script_runtime::take_manual_walk_ownership(
        &rig.scripts,
        &rig.navs,
        "alice",
        manual_frame(false),
        true,
        1,
        false,
    ));
    let seq = rig.navs.lock().unwrap()["alice"].walk_outcome_seq;
    let original = rig.navs.lock().unwrap()["alice"].walk_outcome_request_id;
    rig.observe(2);
    assert!(!rig.walk_armed());
    assert_eq!(rig.driver.walked, None);
    {
        let bots = rig.navs.lock().unwrap();
        assert_eq!(bots["alice"].walk_outcome_seq, seq);
        assert_eq!(bots["alice"].walk_outcome_request_id, original);
        assert_eq!(
            bots["alice"].walk_outcome_cancel_reason,
            script::native::WalkCancelReason::UserInput
        );
    }
    rig.slot()
        .lock()
        .unwrap()
        .restore_host_interacts(vec![script::interact::QueuedInteract {
            req: InteractReq::WalkTo {
                x: 14,
                z: 0,
                level: 0,
            },
            observed_walk_outcome_seq: seq,
        }]);
    rig.observe(3);
    assert_eq!(
        rig.driver.walked,
        Some((14, 0)),
        "an explicit decision based on the takeover snapshot remains legal with pause OFF"
    );
}

#[test]
fn held_manual_input_cancels_only_its_operator_walk_once() {
    let mut rig = open_rig(false);
    rig.observe(1);
    rig.wait_routed();
    let route = Some(Arc::new(Route {
        legs: vec![],
        dest: WorldTile {
            x: 4,
            z: 0,
            level: 0,
        },
        ticks: 4.0,
    }));
    let alice = Arc::new(Mutex::new(crate::WalkArm {
        route: route.clone(),
        route_generation: 17,
        ..crate::WalkArm::default()
    }));
    let bob = Arc::new(Mutex::new(crate::WalkArm {
        route,
        route_generation: 19,
        ..crate::WalkArm::default()
    }));
    let arms = Arc::new(Mutex::new(HashMap::from([
        ("alice".into(), Arc::clone(&alice)),
        ("bob".into(), Arc::clone(&bob)),
    ])));
    assert!(crate::cancel_walk_arm_on_manual_input(
        "alice",
        &arms,
        Some(WorldTile {
            x: 0,
            z: 0,
            level: 0
        }),
        manual_frame(true),
    ));
    assert!(!crate::cancel_walk_arm_on_manual_input(
        "alice",
        &arms,
        Some(WorldTile {
            x: 0,
            z: 0,
            level: 0
        }),
        manual_frame(true),
    ));
    assert!(alice.lock().unwrap().queued_tile().is_none());
    assert_eq!(alice.lock().unwrap().route_generation, 18);
    assert!(bob.lock().unwrap().queued_tile().is_some());
    assert_eq!(bob.lock().unwrap().route_generation, 19);
}
const WATER_CENTER: WorldTile = WorldTile {
    x: 60,
    z: 60,
    level: 0,
};
const WATER_SHORE: WorldTile = WorldTile {
    x: 20,
    z: 60,
    level: 0,
};
const ISOLATED_SHORE: WorldTile = WorldTile {
    x: 65,
    z: 65,
    level: 0,
};

fn water_area_world(separating_wall: bool) -> NavWorld {
    const SIZE: usize = 128;
    let mut flags = vec![0u32; SIZE * SIZE];
    for x in 20..=100 {
        for z in 20..=100 {
            if (x, z) != (WATER_SHORE.x as usize, WATER_SHORE.z as usize)
                && (x, z) != (ISOLATED_SHORE.x as usize, ISOLATED_SHORE.z as usize)
            {
                flags[z * SIZE + x] |= CollisionFlag::SQ_BLOCKED as u32;
            }
        }
    }
    if separating_wall {
        for z in 0..SIZE {
            flags[z * SIZE + 10] |= CollisionFlag::SQ_BLOCKED as u32;
        }
    }
    let (walk, blocked) = nav_planner::collision::pack_walk(&flags);
    NavWorld::from_parts(
        nav_planner::collision::WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: SIZE,
            height: SIZE,
            walk,
            blocked,
            flags: None,
        },
        nav_planner::transport::TransportGraph::default(),
        Vec::new(),
    )
}

fn water_area_rig(separating_wall: bool, radius: u16) -> Rig {
    let mut rig = rig(Some(Arc::new(water_area_world(separating_wall))), false);
    {
        let mut shared = rig.shared.lock();
        shared.target = Some(WATER_CENTER);
        shared.radius = radius;
        shared.arrival = nav_planner::arrival::ArrivalKind::Area;
    }
    for tile in [
        WATER_CENTER,
        WorldTile {
            x: WATER_CENTER.x - 1,
            ..WATER_CENTER
        },
        WorldTile {
            x: WATER_CENTER.x + 1,
            ..WATER_CENTER
        },
        WorldTile {
            z: WATER_CENTER.z - 1,
            ..WATER_CENTER
        },
        WorldTile {
            z: WATER_CENTER.z + 1,
            ..WATER_CENTER
        },
    ] {
        let x = (tile.x - rig.client.map_build_base_x) as usize;
        let z = (tile.z - rig.client.map_build_base_z) as usize;
        rig.client.collision[0].flags[x][z] |= CollisionFlag::SQ_BLOCKED;
    }
    rig.rebuild_snapshot_at(WorldTile {
        x: 0,
        z: 0,
        level: 0,
    });
    rig
}

fn wait_for_native_route_outcome(rig: &Rig) {
    assert!(
        rig.wait_for_route(5_000, || {
            rig.navs
                .lock()
                .unwrap()
                .get("alice")
                .is_some_and(|bot| bot.walk_outcome_seq != 0 && !bot.route_work_pending())
        }),
        "the native route service published a terminal outcome"
    );
}

#[test]
fn area_walk_routes_to_reachable_shore_and_native_receipt_settles_there() {
    let mut rig = water_area_rig(false, 40);
    rig.observe(1);
    rig.wait_routed();

    let goal = rig.navs.lock().unwrap()["alice"]
        .route_destination()
        .expect("reachable shore route");
    assert_eq!(
        goal, WATER_SHORE,
        "the connected shoreline is the reachable area goal"
    );
    assert!(
        goal.x
            .abs_diff(WATER_CENTER.x)
            .max(goal.z.abs_diff(WATER_CENTER.z))
            <= 40,
        "goal {goal:?} is within the area's Chebyshev radius"
    );
    assert!(rig.world.as_ref().unwrap().collision.standable(goal));
    assert_ne!(
        goal, WATER_CENTER,
        "area routing does not require the water center"
    );
    assert_ne!(
        goal, ISOLATED_SHORE,
        "a disconnected stand is not a route goal"
    );

    rig.step_at(goal);
    rig.observe_at(2, goal);
    let shared = rig.shared.lock();
    let receipt = shared.result.as_ref().unwrap().as_ref().unwrap();
    assert_eq!(receipt.end, WalkEnd::Arrived);
}

#[test]
fn area_walk_does_not_arrive_on_a_disconnected_candidate() {
    let mut rig = water_area_rig(true, 40);
    rig.observe(1);
    wait_for_native_route_outcome(&rig);
    rig.observe(2);

    assert_eq!(rig.end(), Some(Ok(WalkEnd::NoPath)));
    assert!(
        !rig.navs.lock().unwrap()["alice"].has_route(),
        "a standable candidate across a full collision wall is not reachable"
    );
}

#[test]
fn zero_radius_area_refuses_a_nonstandable_target() {
    let mut rig = water_area_rig(false, 0);
    rig.observe(1);
    wait_for_native_route_outcome(&rig);
    rig.observe(2);

    let shared = rig.shared.lock();
    let receipt = shared.result.as_ref().unwrap().as_ref().unwrap();
    assert_eq!(receipt.end, WalkEnd::NoPath);
    let detail = receipt
        .detail
        .as_deref()
        .expect("NoPath diagnostic is owed");
    assert!(detail.contains("Area arrival"), "{detail}");
    assert!(detail.contains(&format!("to {WATER_CENTER:?}")), "{detail}");
    assert!(detail.contains("within radius 0"), "{detail}");
}

#[test]
fn reach_walk_keeps_adjacent_solid_target_arrival() {
    let target = WorldTile {
        x: 4,
        z: 4,
        level: 0,
    };
    let mut world = open_world(40, 40);
    let index = target.z as usize * world.collision.width + target.x as usize;
    world.collision.blocked[index / 64] |= 1 << (index % 64);
    let mut rig = rig(Some(Arc::new(world)), false);
    {
        let mut shared = rig.shared.lock();
        shared.target = Some(target);
        shared.radius = 1;
        shared.arrival = nav_planner::arrival::ArrivalKind::Reach;
    }
    let x = (target.x - rig.client.map_build_base_x) as usize;
    let z = (target.z - rig.client.map_build_base_z) as usize;
    rig.client.collision[0].flags[x][z] |= CollisionFlag::SQ_BLOCKED;
    rig.rebuild_snapshot_at(WorldTile {
        x: 0,
        z: 0,
        level: 0,
    });
    rig.observe(1);
    rig.wait_routed();

    let goal = rig.navs.lock().unwrap()["alice"]
        .route_destination()
        .expect("reach route to the solid target's approach");
    assert_eq!(
        goal.x.abs_diff(target.x).max(goal.z.abs_diff(target.z)),
        1,
        "Reach retains the old adjacent-solid target behavior"
    );
    rig.step_at(goal);
    rig.observe_at(2, goal);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::Arrived)));
}

#[test]
fn area_route_refresh_keeps_mode_through_native_receipt() {
    let mut rig = water_area_rig(false, 40);
    rig.observe(1);
    rig.wait_routed();
    assert!(
        rig.wait_for_route(5_000, || {
            rig.navs
                .lock()
                .unwrap()
                .get("alice")
                .is_some_and(|bot| !bot.route_work_pending() && bot.has_route())
        }),
        "the initial area route completed"
    );

    let (request_id, generation, authority, basis) = {
        let navs = rig.navs.lock().unwrap();
        let bot = navs.get("alice").unwrap();
        (
            bot.walk_request_id,
            bot.route_generation,
            bot.native_walk.clone().expect("native walk owner"),
            Arc::clone(bot.route_basis.as_ref().expect("original route basis")),
        )
    };
    // Live geometry refresh abandons the old follow before re-finding under
    // the same authority; it must not be mistaken for a retransmission.
    {
        let mut bots = rig.navs.lock().unwrap();
        let bot = bots.get_mut("alice").unwrap();
        bot.route = None;
        bot.compact_route = None;
        bot.traveller.clear();
    }
    let arm = crate::script_runtime::ScriptWalkArm {
        here: Some((0, 0, 0)),
        world: rig.world.clone(),
        navs: Arc::clone(&rig.navs),
        name: "alice".to_owned(),
        state: None,
        bank: BankRows::default(),
    };
    assert!(arm.refresh_route_in_snapshot(
        &rig.snapshot,
        WATER_CENTER,
        40,
        nav_planner::router::FindOptions {
            allow_teleports: !basis.options.allow_teleports,
            allow_wilderness: !basis.options.allow_wilderness,
            allow_bank_fetch: !basis.options.allow_bank_fetch,
            ..nav_planner::router::FindOptions::default()
        },
        request_id,
        crate::script_runtime::ScriptRouteExclusions::default(),
        Some(authority),
        None,
        nav_planner::arrival::ArrivalKind::Area,
        false,
    ));
    assert!(
        rig.wait_for_route(5_000, || {
            rig.navs.lock().unwrap().get("alice").is_some_and(|bot| {
                bot.route_generation != generation
                    && !bot.route_work_pending()
                    && bot.pending_route.is_none()
                    && bot.has_route()
            })
        }),
        "the refreshed area route completed"
    );

    let (goal, arrival) = {
        let navs = rig.navs.lock().unwrap();
        let bot = navs.get("alice").unwrap();
        assert!(Arc::ptr_eq(
            bot.route_basis.as_ref().expect("retained original basis"),
            &basis
        ));
        let (_, _, teleports, wilderness, bank_fetch, grants) =
            bot.requested_route.expect("refreshed route options");
        assert_eq!(teleports, basis.options.allow_teleports);
        assert_eq!(wilderness, basis.options.allow_wilderness);
        assert_eq!(bank_fetch, basis.options.allow_bank_fetch);
        assert_eq!(grants, basis.options.zones);
        (
            bot.route_destination().expect("refreshed route"),
            bot.route_arrival,
        )
    };
    assert_eq!(arrival, nav_planner::arrival::ArrivalKind::Area);
    assert_eq!(goal, WATER_SHORE);
    rig.step_at(goal);
    rig.observe_at(2, goal);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::Arrived)));
}

#[test]
#[ignore = "requires WORLD_NAV_PACK pointing to the real 289 nav pack"]
fn real_catherby_water_centroid_accepts_area_shore_but_preserves_reach_refusal() {
    use crate::ScriptRouteRequest;
    use nav_planner::arrival::ArrivalKind;

    let pack = std::env::var_os("WORLD_NAV_PACK").expect("WORLD_NAV_PACK is required");
    let world = Arc::new(NavWorld::load_pack(std::path::Path::new(&pack)).unwrap());
    let centre = WorldTile {
        x: 2848,
        z: 3426,
        level: 0,
    };
    let shore = WorldTile {
        x: 2840,
        z: 3436,
        level: 0,
    };
    assert!(!world.collision.standable(centre));
    assert!(world.collision.standable(shore));
    let request = |arrival, radius| ScriptRouteRequest {
        candidate_goals: None,
        admission: crate::admission::Admission {
            enforce: true,
            ..crate::admission::Admission::manual(
                FindOptions::default(),
                Default::default(),
                0,
                crate::WalkGlobals::default(),
            )
        },
        generation: 1,
        request_id: 1,
        world: Arc::clone(&world),
        from: shore,
        to: centre,
        radius,
        loc_id: None,
        arrival,
        opts: FindOptions::default(),
        state: None,
        bank_origin: api::bank_memory::Origin::Unknown,
        bank: Vec::new(),
        live_geometry: None,
        exclusions: None,
        compat_tile_intent: false,
        completion: Default::default(),
    };
    for radius in [40, 104] {
        let (area, _) = request(ArrivalKind::Area, radius).drive_route_job_for_test();
        let RouteOutcome::Routed(route) = area else {
            panic!("Area arrival must accept the real standable Catherby shoreline");
        };
        assert_eq!(route.dest, shore);
        assert!(world.collision.standable(route.dest));
        let (reach, _) = request(ArrivalKind::Reach, radius).drive_route_job_for_test();
        assert!(matches!(reach, RouteOutcome::NoPath));
    }
}

#[test]
fn lifecycle_followups_stop_clears_combat_raise_but_preserves_user_prayer() {
    let mut rig = open_rig(false);
    rig.slot().lock().unwrap().stop();
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let quests =
        Arc::new(api::quest_facts::QuestCatalog::from_identity(selected.quest_identity()).unwrap());
    let mut document: serde_json::Value = serde_json::from_str(include_str!(
        "../../script/paths/289/fixtures/combat_melee_upkeep.json"
    ))
    .unwrap();
    let args = &mut document["roles"][0]["sequences"][0]["steps"][0]["args"];
    args["target"]["npc"] = serde_json::json!("ardougne_archer");
    let path = script::quester::compile::prepare_for_test({
        let selected = Arc::clone(&selected);
        let quests = Arc::clone(&quests);
        move |cap| {
            script::quester::compile::compile_path(
                &serde_json::to_vec(&document).unwrap(),
                &selected,
                &quests,
                cap,
                &script::local_state::OperatorLoadouts::absent(),
            )
        }
    })
    .unwrap();
    let quester = script::quester::runner::Quester::new(
        api::selected::RunKey {
            slot: 0,
            run: 0,
            session: 0,
        },
        path,
        Arc::clone(&selected),
        quests,
        Arc::new(api::named_banks::NamedBankFacts::empty()),
    );
    rig.slot()
        .lock()
        .unwrap()
        .start_test_script(Box::new(quester), Some(Arc::clone(&selected)))
        .unwrap();
    let here = WorldTile {
        x: 2457,
        z: 3302,
        level: 0,
    };
    rig.rebuild_snapshot_at(here);
    seed_protect_frame(&mut rig.snapshot, 43);
    seed_prayer_widgets(&mut rig.snapshot);
    seed_missile_launch(&mut rig.snapshot);
    rig.snapshot.seed_chat_lines(Vec::new());
    rig.snapshot.seed_quest_statuses(
        vec![api::snapshot::QuestStatusView {
            name: "Imp Catcher".into(),
            component_id: 42,
            colour: 0xf8f800,
        }],
        true,
    );
    let skin = selected.prayer_by_name("Thick Skin").unwrap();
    let protect = selected.prayer_by_name("Protect from Missiles").unwrap();
    let mut varps = rig.snapshot.varps().to_vec();
    varps
        .iter_mut()
        .find(|row| row.index == skin.varp)
        .unwrap()
        .value = 1;
    rig.snapshot.seed_varps(varps);
    let raised_tick = (1..=20)
        .find(|tick| {
            rig.snapshot.seed_tick(*tick);
            rig.observe_with_here(u64::from(*tick), here);
            rig.driver
                .if_button_components
                .contains(&protect.button_com)
        })
        .expect("the real Combat must admit a protect raise");
    // Stop before Combat polls that accepted receipt: ownership must include
    // the host-accepted raise, not only the machine's last polled mask.
    let before_stop = rig.driver.if_button_components.len();
    let mut varps = rig.snapshot.varps().to_vec();
    varps
        .iter_mut()
        .find(|row| row.index == protect.varp)
        .unwrap()
        .value = 1;
    rig.snapshot.seed_varps(varps);
    rig.slot().lock().unwrap().stop();
    rig.snapshot.seed_tick(raised_tick + 1);
    rig.observe_with_here(u64::from(raised_tick + 1), here);
    assert_eq!(
        &rig.driver.if_button_components[before_stop..],
        &[protect.button_com],
        "Stop owes only Combat's accepted protect, never the user's Thick Skin"
    );
    rig.observe_with_here(u64::from(raised_tick + 1), here);
    assert_eq!(rig.driver.if_button_components.len(), before_stop + 1);
    assert_eq!(
        rig.snapshot
            .varps()
            .iter()
            .find(|row| row.index == skin.varp)
            .unwrap()
            .value,
        1
    );
    let mut varps = rig.snapshot.varps().to_vec();
    varps
        .iter_mut()
        .find(|row| row.index == protect.varp)
        .unwrap()
        .value = 0;
    rig.snapshot.seed_varps(varps);
    rig.snapshot.seed_tick(raised_tick + 2);
    rig.observe_with_here(u64::from(raised_tick + 2), here);
    assert_eq!(rig.slot().lock().unwrap().state(), script::RunState::Idle);
    assert_eq!(rig.driver.if_button_components.len(), before_stop + 1);
}

fn combat_prayer(name: &str) -> (i32, i32) {
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289)
        .expect("R289 prayer facts");
    let row = data
        .prayer_by_name(name)
        .unwrap_or_else(|| panic!("R289 lacks prayer {name}"));
    (row.varp, row.button_com)
}

/// Slot-less bot with an attached, ingame scene and prayer widgets: only the
/// combat-prayer debt arm acts on `step`.
fn combat_debt_rig() -> Rig {
    let mut rig = open_rig(false);
    seed_protect_frame(&mut rig.snapshot, 43);
    seed_prayer_widgets(&mut rig.snapshot);
    rig.navs
        .lock()
        .unwrap()
        .insert("alice".to_string(), NavBot::default());
    rig
}

fn owe_combat_debt(rig: &Rig, varps: &[i32], tick: u16) {
    let owned = script::combat::RaisedPrayers::from_test_varps(varps);
    let mut navs = rig.navs.lock().unwrap();
    let bot = navs.get_mut("alice").expect("debt bot");
    owe_combat_prayers_off(bot, owned, tick);
}

fn set_combat_varp(snapshot: &mut GameSnapshot, varp: i32, value: i32) {
    let mut rows = snapshot.varps().to_vec();
    for row in &mut rows {
        if row.index == varp {
            row.value = value;
        }
    }
    snapshot.seed_varps(rows);
}

#[test]
fn combat_prayer_off_sends_exactly_one_bounded_retry() {
    let (varp, component) = combat_prayer("Protect from Missiles");
    let mut rig = combat_debt_rig();
    set_combat_varp(&mut rig.snapshot, varp, 1);
    owe_combat_debt(&rig, &[varp], 0);
    assert!(rig.navs.lock().unwrap()["alice"]
        .combat_prayer_off
        .is_some());
    for tick in 0u32..=12 {
        rig.snapshot.seed_tick(tick);
        rig.step();
        if tick == 0 {
            assert_eq!(
                rig.driver.if_button_components,
                vec![component],
                "Stop debt opens with one off-click"
            );
        } else if tick <= 2 {
            assert_eq!(
                rig.driver.if_button_components.len(),
                1,
                "no re-send while the off receipt is outstanding"
            );
        } else if tick == 3 {
            assert_eq!(
                rig.driver.if_button_components,
                vec![component, component],
                "exactly one bounded retry"
            );
        }
    }
    assert_eq!(
        rig.driver.if_button_components,
        vec![component, component],
        "a dropped retry must not admit a third off-click"
    );
    assert!(
        rig.navs.lock().unwrap()["alice"]
            .combat_prayer_off
            .is_none(),
        "the expired retry drops the debt"
    );
}

#[test]
fn combat_prayer_off_refused_retry_spends_the_retry_without_a_loop() {
    let (varp, component) = combat_prayer("Protect from Missiles");
    let mut rig = combat_debt_rig();
    set_combat_varp(&mut rig.snapshot, varp, 1);
    owe_combat_debt(&rig, &[varp], 0);
    for tick in 0u32..=10 {
        rig.snapshot.seed_tick(tick);
        if tick == 1 {
            // The retry's target vanishes, so the retry is refused at the
            // send gate after the first off-click was accepted.
            rig.snapshot.seed_main_modal(5608, Vec::new());
        }
        if tick == 4 {
            // Restore the retry target so an unbounded retry becomes visible.
            seed_prayer_widgets(&mut rig.snapshot);
        }

        rig.step();
        if tick == 0 {
            assert_eq!(
                rig.driver.if_button_components,
                vec![component],
                "the first off-click is accepted"
            );
        }
    }
    assert_eq!(
        rig.driver.if_button_components,
        vec![component],
        "a refused retry is never re-sent"
    );
    assert!(
        rig.navs.lock().unwrap()["alice"]
            .combat_prayer_off
            .is_none(),
        "the refused retry still expires the debt on its deadline"
    );
}

#[test]
fn combat_prayer_off_clears_two_prayers_serially_in_table_order() {
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289)
        .expect("R289 prayer facts");
    let missiles = data
        .prayer_by_name("Protect from Missiles")
        .expect("R289 missiles");
    let melee = data
        .prayer_by_name("Protect from Melee")
        .expect("R289 melee");
    let (missiles_varp, missiles_com) = (missiles.varp, missiles.button_com);
    let (melee_varp, melee_com) = (melee.varp, melee.button_com);
    let first_varp = data
        .prayers()
        .iter()
        .map(|row| row.varp)
        .find(|varp| *varp == missiles_varp || *varp == melee_varp)
        .expect("an owned prayer in table order");
    let (first_varp, first_com, second_varp, second_com) = if first_varp == missiles_varp {
        (missiles_varp, missiles_com, melee_varp, melee_com)
    } else {
        (melee_varp, melee_com, missiles_varp, missiles_com)
    };
    let mut rig = combat_debt_rig();
    set_combat_varp(&mut rig.snapshot, missiles_varp, 1);
    set_combat_varp(&mut rig.snapshot, melee_varp, 1);
    owe_combat_debt(&rig, &[missiles_varp, melee_varp], 0);
    rig.snapshot.seed_tick(0);
    rig.step();
    assert_eq!(
        rig.driver.if_button_components,
        vec![first_com],
        "one off-click in flight at a time"
    );
    set_combat_varp(&mut rig.snapshot, first_varp, 0);
    rig.snapshot.seed_tick(1);
    rig.step();
    assert_eq!(
        rig.driver.if_button_components,
        vec![first_com, second_com],
        "the next owned prayer is serviced after the first settles"
    );
    set_combat_varp(&mut rig.snapshot, second_varp, 0);
    rig.snapshot.seed_tick(2);
    rig.step();
    assert!(
        rig.navs.lock().unwrap()["alice"]
            .combat_prayer_off
            .is_none(),
        "both owned prayers retired"
    );
    assert_eq!(
        rig.driver.if_button_components,
        vec![first_com, second_com],
        "settled prayers are never re-clicked"
    );
}

#[test]
fn idle_nav_bot_does_not_allocate_request_payloads() {
    let measured = allocation_counter::measure(|| {
        std::hint::black_box(NavBot::default());
    });
    assert_eq!(measured.count_total, 0, "{measured:?}");
    assert_eq!(measured.bytes_current, 0, "{measured:?}");
}

#[test]
fn s2b_admission_per_bot_layouts() {
    eprintln!(
        "S2b admission layout NavBot={} SlotScript={} Combat={} WalkGuard={}",
        std::mem::size_of::<NavBot>(),
        std::mem::size_of::<script::SlotScript>(),
        std::mem::size_of::<script::combat::Combat>(),
        std::mem::size_of::<script::combat::WalkGuard>()
    );
    let bot = NavBot::default();
    eprintln!(
        "NavBot fields Traveller={} InspectNav={} BankPickState={} BankFetch={} CompactRoute={}",
        std::mem::size_of_val(&bot.traveller),
        std::mem::size_of_val(&bot.inspect),
        std::mem::size_of_val(&bot.bank_pick),
        std::mem::size_of_val(&bot.bank_fetch),
        std::mem::size_of_val(&bot.compact_route),
    );
    // FIX-025: request-only inspection work and unpublished bank results are
    // boxed, not reserved inline in every idle bot. Their owners do not change.
    // The remaining growth since the S2b pin is intentional: admitted-work
    // identities/cancellation, compact route leases, and the shared script
    // seat's bounded delivery/hold queues. See FIX-025/mac-layout/report.md
    // for the field/commit measurements. Exact pins have zero growth headroom.
    // Grinch's shared native progress and evidence-wait run fences add two
    // inline Option<RunKey> values (64 B): macOS SlotScript measures 4608 B.
    // Neither fence retains a scene or allocates a per-bot heap payload.
    // EVENT-RUNTIME-S2 adds one inline u64 consumed-takeover sequence:
    // macOS SlotScript measures 4616 B (+8 B), with no new heap payload.
    // Entrance evidence adds a 48 B RouteQuestContext and an 8 B provider-byte
    // account to NavBot's retained QuestEvidence.
    // Removing the legacy-grid fields shrank Traveller by 88 B (904 B to
    // 816 B), so the measured NavBot exact pin falls from 3760 B to 3672 B.
    // Nav split S3's typed NPC hop reason shrank NavBot by 16 B (3656 B).
    // SURVIVAL-1.1 adds the 104 B inline SlotCondition owner, and the walk
    // stall diagnostic adds 16 B to Traveller (original send tile and tick
    // across resends): NavBot measures 3776 B, with no new heap payload and
    // no SlotScript growth.
    // SURVIVAL-1.4 adds one byte of cure_poison, padded to 8 by alignment: NavBot is 3784 B.
    // Rust 1.98.1 is pinned for the workspace and CI. These are lib-test
    // layouts: test-only inspection budgets and bank fixtures are included.
    // S0c moves the platform-specific isolate mutexes into the surface run.
    // The shared 64-bit slot layout no longer contains those mutexes.
    // S0d adds a 16 B per-slot generation cache for lock-free compiled-card reads.
    // NAV-BUSY-TRANSPORT adds 32 B to Traveller (busy-refusal retry state and
    // the distinct-tick transport clock): NavBot is 3816 B. Lab memory A/B on
    // the branch head passed (NAV-BUSY-TRANSPORT/mem-ab-r3).
    #[cfg(all(
        target_os = "macos",
        target_arch = "aarch64",
        not(feature = "memory-profile")
    ))]
    assert_eq!(
        (
            std::mem::size_of::<NavBot>(),
            std::mem::size_of::<script::SlotScript>(),
            std::mem::size_of::<script::combat::Combat>(),
            std::mem::size_of::<script::combat::WalkGuard>()
        ),
        (3816, 1624, 512, 264)
    );
    #[cfg(all(
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu",
        not(feature = "memory-profile")
    ))]
    assert_eq!(
        (
            std::mem::size_of::<NavBot>(),
            std::mem::size_of::<script::SlotScript>(),
            std::mem::size_of::<script::combat::Combat>(),
            std::mem::size_of::<script::combat::WalkGuard>()
        ),
        (3816, 1624, 512, 264)
    );
    assert!(std::mem::size_of::<script::combat::Combat>() <= 512);
    assert!(std::mem::size_of::<script::combat::WalkGuard>() <= 264);
}

mod s2b_admission_contract {
    use super::*;
    use api::condition::PoisonState;
    use script::combat::guard::{Escape, EscapeAction, EscapeOwner, EscapeState};
    use script::combat::risk::{UnknownWhy, Verdict};
    use script::native::{AssessReceipt, WalkBit};

    fn tile(x: i32) -> WorldTile {
        WorldTile { x, z: 0, level: 0 }
    }

    fn ice_world() -> NavWorld {
        let mut world = open_world(256, 1);
        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let npc = data.npc_by_config("icewarrior").unwrap();
        world.graph.zones = Some(
            nav_planner::zones::ZoneTable::from_parts(
                vec![nav_planner::zones::Zone::npc(
                    tile(120),
                    1,
                    nav_planner::zones::ZoneClass::Always,
                    u16::MAX,
                    0,
                )],
                vec![nav_planner::zones::ZoneKind::new(
                    "ice",
                    "Ice warrior",
                    npc.id,
                    57,
                    false,
                    false,
                )],
                vec![],
                vec![],
                vec![],
                tile(0),
                256,
                1,
                &world.graph.wilderness,
            )
            .unwrap(),
        );
        world
    }

    fn observed_loadout(snapshot: &mut GameSnapshot) {
        snapshot.seed_inventory(Vec::new(), 28);
        snapshot.seed_equipment(Vec::new());
        snapshot.seed_stats(vec![
            api::snapshot::StatView {
                index: 3,
                name: "hitpoints".into(),
                base: 50,
                effective: 50,
                xp: 0,
                used: true,
            },
            api::snapshot::StatView {
                index: 5,
                name: "prayer".into(),
                base: 1,
                effective: 1,
                xp: 0,
                used: true,
            },
        ]);
        let mut player = snapshot.local_player().unwrap().clone();
        player.player.combat_level = 3;
        snapshot.seed_local_player(player);
        snapshot.seed_varps(vec![api::snapshot::VarpView {
            index: 173,
            value: 0,
        }]);
    }

    #[test]
    fn first_native_crossing_with_explicit_forbid_refuses_poison_unknown_without_arming_or_waiting_it_clear(
    ) {
        let mut rig = rig(Some(Arc::new(ice_world())), false);
        rig.shared.lock().options.allow_danger_zones = WalkBit::Forbid;
        observed_loadout(&mut rig.snapshot);
        rig.shared.lock().target = Some(tile(120));
        rig.observe(1);
        assert!(
            rig.wait_for_route(5_000, || rig.navs.lock().unwrap()["alice"]
                .risk_refusal
                .is_some())
        );
        {
            let bots = rig.navs.lock().unwrap();
            let bot = &bots["alice"];
            assert!(!bot.has_route() && bot.bank_fetch.is_none());
            assert!(!bot.route_work_pending() && bot.pending_route.is_none());
        }
        rig.step();
        assert!(rig.driver.walked.is_none());
        rig.observe(2);
        let first = rig
            .shared
            .lock()
            .result
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()
            .clone();
        assert_eq!(first.end, WalkEnd::Refused);
        assert_eq!(
            first.refusal,
            Some(script::native::WalkRefusal::Unknown(UnknownWhy::Poison))
        );
        assert_eq!(
            first.assessment.as_ref().unwrap().verdict,
            Verdict::Unknown(UnknownWhy::Poison)
        );
        let reason = first.detail.as_deref().unwrap();
        assert!(
            reason.contains("Poison status is unknown until the bot can track it"),
            "{reason}"
        );
        assert!(!reason.contains("S2c"), "{reason}");
        let request = rig.navs.lock().unwrap()["alice"].walk_outcome_request_id;
        rig.snapshot.seed_tick(32);
        rig.shared.lock().rewalk = true;
        rig.observe(32);
        assert!(
            rig.wait_for_route(5_000, || rig.navs.lock().unwrap()["alice"]
                .walk_outcome_request_id
                != request)
        );
        rig.observe(33);
        let shared = rig.shared.lock();
        let second = shared.result.as_ref().unwrap().as_ref().unwrap();
        assert_eq!(second.end, WalkEnd::Refused);
        assert_eq!(second.refusal, first.refusal);
        assert!(matches!(
            second.assessment.as_ref().unwrap().input.poison,
            PoisonState::Unknown { .. }
        ));
    }

    #[test]
    fn first_native_safe_only_walk_keeps_the_poison_exemption_and_assessment() {
        let mut rig = rig(Some(Arc::new(ice_world())), false);
        observed_loadout(&mut rig.snapshot);
        rig.observe(1);
        rig.wait_routed();
        let bots = rig.navs.lock().unwrap();
        let assessment = bots["alice"].assessment.as_ref().unwrap();
        assert_eq!(assessment.verdict, Verdict::Survivable);
        assert!(assessment.plan.crossings.is_empty());
        assert!(matches!(
            assessment.input.poison,
            PoisonState::Unknown { .. }
        ));
        assert_eq!(
            bots["alice"].admission.as_ref().unwrap().policy,
            script::native::RiskPolicy::Avoid
        );
    }

    #[test]
    fn explicit_native_opt_in_proceeds_and_returns_its_honest_unknown_assessment() {
        let mut rig = rig(Some(Arc::new(ice_world())), false);
        observed_loadout(&mut rig.snapshot);
        {
            let mut shared = rig.shared.lock();
            shared.target = Some(tile(120));
            shared.options.allow_danger_zones = WalkBit::Allow;
        }
        rig.observe(1);
        rig.wait_routed();
        rig.step();
        assert!(rig.driver.walked.is_some());
        abort_script_walk(&rig.navs, "alice");
        rig.observe(2);
        let shared = rig.shared.lock();
        let receipt = shared.result.as_ref().unwrap().as_ref().unwrap();
        assert_eq!(receipt.end, WalkEnd::Cancelled);
        assert!(receipt.refusal.is_none());
        assert_eq!(
            receipt.assessment.as_ref().unwrap().verdict,
            Verdict::Unknown(UnknownWhy::Poison)
        );
    }

    #[cfg(feature = "js")]
    #[test]
    fn real_v1_metadata_routes_across_derived_zones_with_boolean_only_and_no_host_guard() {
        for hp in [14, 45] {
            let source = once_src(
                "import { Traversal } from '../../api/walking/Traversal.js';",
                "Traversal.walkTo({ x: 255, z: 0, level: 0 }, { radius: 0, timeoutMs: 300000 })",
            );
            let mut rig = ReconnectRig::with_source(source, ice_world(), (0, 0, 0));
            nav_snapshot_at(&mut rig.client, &mut rig.snap, 0, 0);
            observed_loadout(&mut rig.snap);
            rig.snap.seed_stats(vec![
                api::snapshot::StatView {
                    index: 3,
                    name: "hitpoints".into(),
                    base: hp,
                    effective: hp,
                    xp: 0,
                    used: true,
                },
                api::snapshot::StatView {
                    index: 5,
                    name: "prayer".into(),
                    base: 1,
                    effective: 1,
                    xp: 0,
                    used: true,
                },
            ]);
            assert_eq!(
                rig.slot().lock().unwrap().surface_family(),
                Some(script::surface::SurfaceFamily::JsV1)
            );
            rig.frames(1);
            assert!(rig.wait_for_route_work(5_000, || rig
                .navs
                .lock()
                .unwrap()
                .get("alice")
                .is_some_and(|bot| bot.assessment.is_some())));
            {
                let navs = rig.navs.lock().unwrap();
                let bot = &navs["alice"];
                let admission = bot.admission.as_ref().unwrap();
                assert!(admission.compat_v1);
                assert_eq!(admission.policy, script::native::RiskPolicy::Proceed);
                assert!(
                    !admission.allow.food && !admission.allow.prayer && !admission.allow.escape
                );
                assert!(bot.has_route() && bot.risk_refusal.is_none());
                assert!(bot.walk_guard.is_none() && bot.slot_escape().is_none());
                assert_eq!(
                    bot.assessment.as_ref().unwrap().verdict,
                    Verdict::Unknown(UnknownWhy::Poison)
                );
                assert!(!bot.route_basis.as_ref().unwrap().options.allow_teleports);
            }
            rig.here = (255, 0, 0);
            nav_snapshot_at(&mut rig.client, &mut rig.snap, 255, 0);
            rig.frames(2);
            assert_eq!(
                rig.slot().lock().unwrap().probe("__rs_ok").unwrap(),
                serde_json::Value::Bool(true),
                "v1 still receives its legacy boolean at HP {hp}"
            );
            rig.slot().lock().unwrap().stop();
        }
    }

    #[cfg(feature = "js")]
    /// FlourCollector (v1) walked out of Sinclair Mansion past the
    /// Highwayman and Wolf zones on its compiled Reach.locOp walk, then its
    /// Banking.open stand walk, a machine-owned native walk, was refused by
    /// the same zones. A v1 card's native walks keep the frozen zone-exempt
    /// routing of its compiled walks.
    #[test]
    fn real_v1_banking_stand_walk_routes_across_zones_like_its_compiled_walks() {
        let source = once_src(
            "import { Banking } from '../../api/bank/Banking.js';",
            "Banking.open({ stand: { x: 255, z: 0, level: 0 } })",
        );
        let mut rig = ReconnectRig::t23_bank_wake_with_source(source, ice_world(), (0, 0, 0));
        // Closed bank and no scene booth: Banking.open walks to its stand.
        rig.client.world.del_wall(0, 5, 6);
        rig.client.main_modal_id = -1;
        rig.client.gens.iface = rig.client.gens.iface.wrapping_add(1);
        nav_snapshot_at(&mut rig.client, &mut rig.snap, 0, 0);
        observed_loadout(&mut rig.snap);
        assert_eq!(
            rig.slot().lock().unwrap().surface_family(),
            Some(script::surface::SurfaceFamily::JsV1)
        );
        rig.t23_bank_frame(1, true);
        assert!(rig.wait_for_route_work(5_000, || rig
            .navs
            .lock()
            .unwrap()
            .get("alice")
            .is_some_and(|bot| bot.admission.is_some() && !bot.route_work_pending())));
        let navs = rig.navs.lock().unwrap();
        let bot = &navs["alice"];
        let admission = bot.admission.as_ref().unwrap();
        assert!(admission.compat_v1, "the stand walk keeps the v1 rule");
        assert!(bot.has_route(), "the stand walk crosses the ice zone");
        assert!(bot.native_walk_failure.is_none());
        drop(navs);
        rig.slot().lock().unwrap().stop();
    }

    #[derive(Default)]
    struct Advice {
        requests: Vec<u64>,
        receipt: Option<AssessReceipt>,
        count: usize,
        also_walk: bool,
    }

    struct Adviser {
        shared: Arc<parking_lot::Mutex<Advice>>,
        walk: Option<ActionHandle<Walk>>,
    }

    impl Script for Adviser {
        fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
            let mut shared = self.shared.lock();
            if let Some(&id) = shared.requests.last() {
                if let Some(receipt) = tick.actions.assess_receipt(&tick.cx, id) {
                    shared.receipt = Some(receipt.clone());
                }
            }
            let required_after = tick.cx.evidence();
            let request = || WalkRequest {
                target: tile(4),
                candidate_goals: None,
                radius: 0,
                arrival: Default::default(),
                loc_id: None,
                options: script::native::WalkOptions {
                    allow_danger_zones: WalkBit::Allow,
                    ..Default::default()
                },
                required_after,
                evidence: None,
                cross: Box::new([]),
                protect: false,
                food_guard: false,
                allow: Default::default(),
            };
            while shared.requests.len() < shared.count {
                let id = tick
                    .actions
                    .assess_walk(request(), &mut tick.cx)
                    .map_err(|error| ScriptFailure {
                        code: "assess-test".into(),
                        message: format!("{error:?}").into(),
                    })?;
                shared.requests.push(id);
            }
            if shared.also_walk && self.walk.is_none() {
                self.walk = Some(
                    tick.actions
                        .begin::<Walk>(request(), &mut tick.cx)
                        .map_err(|error| ScriptFailure {
                            code: "walk-test".into(),
                            message: format!("{error:?}").into(),
                        })?,
                );
            }
            Ok(ScriptFlow::Continue)
        }
    }

    fn advising_rig(count: usize, also_walk: bool) -> (Rig, Arc<parking_lot::Mutex<Advice>>) {
        let rig = open_rig(false);
        let shared = Arc::new(parking_lot::Mutex::new(Advice {
            count,
            also_walk,
            ..Default::default()
        }));
        {
            let slot = rig.slot();
            let mut slot = slot.lock().unwrap();
            slot.stop();
            slot.start_test_script(
                Box::new(Adviser {
                    shared: Arc::clone(&shared),
                    walk: None,
                }),
                None,
            )
            .unwrap();
        }
        (rig, shared)
    }

    fn await_advice(rig: &mut Rig, shared: &Arc<parking_lot::Mutex<Advice>>) {
        assert!(
            rig.wait_for_route(5_000, || rig.navs.lock().unwrap()["alice"]
                .assess_result
                .is_some())
        );
        rig.observe(2);
        assert!(shared.lock().receipt.is_some());
    }

    fn oversized_route_refusal() -> nav_planner::producer::OwnerResultRefusal {
        let maximum = nav_planner::producer::OWNER_RESULT_MAX_BYTES;
        let ledger = nav_planner::producer::OwnerResultLedger::with_limits(maximum * 2, maximum);
        match ledger.try_reserve_bytes(maximum + 1) {
            Err(error) => error,
            Ok(_) => panic!("the oversized route result must refuse owner custody"),
        }
    }

    fn expected_oversized_route_refusal() -> script::native::WalkRefusal {
        script::native::WalkRefusal::RouteTooLarge {
            required_bytes: nav_planner::producer::OWNER_RESULT_MAX_BYTES + 1,
            maximum_bytes: nav_planner::producer::OWNER_RESULT_MAX_BYTES,
        }
    }

    #[test]
    fn oversized_route_result_publishes_a_typed_walk_refusal_not_no_path() {
        let mut rig = open_rig(false);
        rig.observe(1);
        let (generation, request_id, authority) = {
            let mut bots = rig.navs.lock().unwrap();
            let bot = bots.get_mut("alice").unwrap();
            let authority = bot.native_walk.as_ref().unwrap().clone();
            // Isolate the owner-result publication boundary from a concurrent
            // search completion. The actual request and action owner stay live.
            bot.supersede_route_work();
            (bot.route_generation, bot.walk_request_id, authority)
        };
        crate::script_runtime::publish_route_owner_refusal(
            &rig.navs,
            "alice",
            generation,
            request_id,
            crate::script_runtime::RouteDemandKind::Walk {
                authority: Some(authority),
            },
            oversized_route_refusal(),
        );
        rig.observe(2);
        let shared = rig.shared.lock();
        let receipt = shared.result.as_ref().unwrap().as_ref().unwrap();
        assert_eq!(receipt.end, WalkEnd::Refused);
        assert_eq!(receipt.refusal, Some(expected_oversized_route_refusal()));
        assert_eq!(
            receipt.detail.as_deref(),
            Some(expected_oversized_route_refusal().to_string().as_str())
        );
        assert!(rig.driver.walked.is_none());
    }

    #[test]
    fn oversized_route_result_publishes_a_typed_advisory_refusal_not_no_path() {
        let (mut rig, shared) = advising_rig(1, false);
        rig.observe(1);
        assert!(
            rig.wait_for_route(5_000, || rig.navs.lock().unwrap()["alice"]
                .assess_result
                .is_some())
        );
        let (authority, evidence, generation) = {
            let bots = rig.navs.lock().unwrap();
            let bot = &bots["alice"];
            let (authority, receipt) = bot.assess_result.as_deref().unwrap();
            (authority.clone(), receipt.evidence, bot.route_generation)
        };
        crate::script_runtime::publish_route_owner_refusal(
            &rig.navs,
            "alice",
            generation,
            authority.request_id().get(),
            crate::script_runtime::RouteDemandKind::Advisory {
                authority,
                evidence,
            },
            oversized_route_refusal(),
        );
        rig.observe(2);
        let shared = shared.lock();
        let receipt = shared.receipt.as_ref().unwrap();
        assert_eq!(receipt.refusal, Some(expected_oversized_route_refusal()));
        assert_eq!(
            receipt.detail.as_deref(),
            Some(expected_oversized_route_refusal().to_string().as_str())
        );
        assert!(receipt.assessment.is_none());
        assert!(rig.driver.walked.is_none());
    }

    #[test]
    fn advisory_only_worker_returns_a_correlated_receipt_without_becoming_a_mover() {
        let (mut rig, shared) = advising_rig(1, false);
        rig.observe(1);
        await_advice(&mut rig, &shared);
        let bots = rig.navs.lock().unwrap();
        let bot = &bots["alice"];
        assert!(!bot.ordinary_movement_owned(0));
        assert!(!bot.has_route() && bot.native_walk.is_none() && bot.walk_guard.is_none());
        assert_eq!(bot.route_generation, 0);
        assert_eq!(bot.walk_request_id, 0);
        let shared = shared.lock();
        let receipt = shared.receipt.as_ref().unwrap();
        assert_eq!(receipt.request_id, shared.requests[0]);
        assert_eq!(receipt.route_ticks, 8);
        assert!(receipt.assessment.is_some());
        assert!(rig.driver.walked.is_none());
    }

    #[test]
    fn newer_advice_replaces_the_old_request_without_spending_the_walk_budget() {
        let (mut rig, shared) = advising_rig(2, true);
        rig.observe(1);
        rig.wait_routed();
        await_advice(&mut rig, &shared);
        let shared = shared.lock();
        assert_eq!(shared.requests.len(), 2);
        assert_eq!(
            shared.receipt.as_ref().unwrap().request_id,
            shared.requests[1]
        );
        let bots = rig.navs.lock().unwrap();
        assert!(bots["alice"].has_route());
        assert!(bots["alice"].native_walk.as_ref().unwrap().live());
    }

    #[test]
    fn stopped_advisory_owner_cannot_publish_or_restore_a_receipt() {
        let (mut rig, shared) = advising_rig(1, false);
        rig.observe(1);
        assert!(
            rig.wait_for_route(5_000, || rig.navs.lock().unwrap()["alice"]
                .assess_result
                .is_some())
        );
        rig.slot().lock().unwrap().stop();
        rig.observe(2);
        assert!(shared.lock().receipt.is_none());
        let bots = rig.navs.lock().unwrap();
        assert!(bots["alice"].assess_result.is_none());
        assert!(!bots["alice"].ordinary_movement_owned(0));
    }

    fn unresolved_escape() -> Escape {
        Escape {
            action: EscapeAction::Stand,
            state: EscapeState::Unresolved,
            owner: EscapeOwner::Host,
            allow: Default::default(),
            from: tile(0),
            to: tile(0),
            hp: 10,
            floor: 18,
            volley: 6,
            tick: 1,
            generation: 1,
            switches: 0,
            last_hit: 1,
            retired: None,
        }
    }

    #[test]
    fn advisory_computation_is_allowed_during_a_shared_manual_escape_without_replacing_it() {
        let (mut rig, shared) = advising_rig(1, false);
        let manual = Arc::new(Mutex::new(WalkArm {
            admission: Some(Box::new(crate::admission::Admission {
                enforce: true,
                escape: Some(unresolved_escape()),
                ..crate::admission::Admission::manual(
                    Default::default(),
                    Default::default(),
                    1,
                    WalkGlobals::default(),
                )
            })),
            ..Default::default()
        }));
        rig.navs
            .lock()
            .unwrap()
            .entry("alice".into())
            .or_default()
            .manual_arm = Some(Arc::downgrade(&manual));
        rig.observe(1);
        await_advice(&mut rig, &shared);
        let bots = rig.navs.lock().unwrap();
        assert_eq!(bots["alice"].slot_escape(), Some(unresolved_escape()));
        assert!(bots["alice"].native_walk.is_none() && !bots["alice"].has_route());
        assert_eq!(
            manual.lock().unwrap().admission.as_ref().unwrap().escape,
            Some(unresolved_escape())
        );
    }

    #[test]
    #[cfg(feature = "js")]
    fn frame_projection_freezes_an_armed_walk_through_script_start_pause_and_stop() {
        let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
        let slot = crate::script_runtime::script_slot_or_insert(&scripts, "alice");
        let assert_frozen = |expected| {
            let state = crate::script_runtime::script_frame(&scripts, "alice").state;
            assert_eq!(state, Some(expected));
            assert!(crate::play_slots::project_slot_frame_host_move_owned(
                state, false
            ));
        };

        let start = crate::script_runtime::js_surface_start(
            crate::script_runtime::js_start_card(
                "export function tick(api) {}".into(),
                script_js::LoadShape::NativeTick,
                Vec::new(),
            ),
            None,
            None,
            Arc::new(api::named_banks::NamedBankFacts::empty()),
            Arc::from(Vec::<api::loadout::Loadout>::new()),
        );
        slot.lock().unwrap().start_surface(start).unwrap();
        assert_frozen(script::RunState::Starting);
        assert!(wait_until(5_000, || {
            let mut slot = slot.lock().unwrap();
            slot.observe_lifecycle();
            slot.state() == script::RunState::Running
        }));
        assert_frozen(script::RunState::Running);

        slot.lock().unwrap().pause();
        assert_frozen(script::RunState::Paused);
        slot.lock().unwrap().stop();
        assert_frozen(script::RunState::Stopping);
        assert!(wait_until(5_000, || {
            let mut slot = slot.lock().unwrap();
            slot.observe_lifecycle();
            slot.state() == script::RunState::Idle
        }));
        let state = crate::script_runtime::script_frame(&scripts, "alice").state;
        assert_eq!(state, Some(script::RunState::Idle));
        assert!(
            !crate::play_slots::project_slot_frame_host_move_owned(state, false),
            "Stop->Idle releases frontend follow"
        );
        assert!(crate::play_slots::project_slot_frame_host_move_owned(
            None, true
        ));
        assert!(!crate::play_slots::project_slot_frame_host_move_owned(
            None, false
        ));
        for state in [
            None,
            Some(script::RunState::Idle),
            Some(script::RunState::Error),
        ] {
            assert!(!crate::script_runtime::script_movement_owned(state));
        }
        assert!(!NavBot::default().ordinary_movement_owned(0));
    }
}

/// One full observation, keeping the result the test wrappers drop.
fn observe_full(rig: &mut Rig, tick: u64) -> crate::script_runtime::ScriptObservation {
    crate::script_runtime::script_observe_cached_with_channels(
        &mut rig.driver,
        "alice",
        true,
        true,
        false,
        tick,
        Some((0, 0, 0)),
        None,
        None,
        Some(&rig.snapshot),
        None,
        #[cfg(feature = "js")]
        None,
        None,
        &rig.scripts,
        &rig.cheats,
        &rig.navs,
        &rig.world,
        false,
        false,
        false,
        None,
        None,
        None,
        None,
        #[cfg(feature = "js")]
        None,
        #[cfg(feature = "js")]
        crate::script_channels::BrokerWorld::Unavailable,
        None,
        None,
        None,
        Some(&rig.statuses),
    )
}

/// PANEL-STATE-GAP-1023: a run that ends on its own resets its navigation in
/// the observer, and the same observation ends the run's published walk
/// destination, once. The nav step runs only while a route or cleanup is
/// armed, so nothing else would clear the row. Here the lifecycle reaps a
/// Stop with its walk still armed.
#[test]
fn a_run_that_ends_in_the_pump_retires_its_published_walk_once() {
    let mut rig = open_rig(false);
    {
        let mut shared = rig.shared.lock();
        shared.target = Some(WorldTile {
            x: 20,
            z: 0,
            level: 0,
        });
    }
    rig.observe(1);
    rig.wait_routed();
    rig.step();
    assert!(
        rig.statuses.lock().unwrap()[0].script_walk().is_some(),
        "the nav step published the armed destination"
    );

    rig.slot().lock().unwrap().stop();
    let mut tick = 2;
    assert!(wait_until(5_000, || {
        tick += 1;
        observe_full(&mut rig, tick);
        rig.statuses.lock().unwrap()[0].script_walk().is_none()
    }));
    assert_eq!(queued(&rig.navs), None, "the run's route ended with it");

    // A later publication stands for the next run's walk: the ended run does
    // not retire it again.
    rig.statuses.lock().unwrap()[0].publish_script_walk(Some(WorldTile {
        x: 7,
        z: 7,
        level: 0,
    }));
    for _ in 0..3 {
        tick += 1;
        observe_full(&mut rig, tick);
    }
    assert!(
        rig.statuses.lock().unwrap()[0].script_walk().is_some(),
        "an ended run is retired once"
    );
}

/// PANEL-STATE-GAP-1023: a run that fails ends its published rows the same
/// way (the lifecycle receipt names its generation), once per run.
#[test]
fn a_failed_run_retires_its_published_rows_once() {
    let mut rig = open_rig(true);
    rig.statuses.lock().unwrap()[0].publish_script_walk(Some(WorldTile {
        x: 20,
        z: 0,
        level: 0,
    }));
    let mut tick = 1;
    assert!(wait_until(5_000, || {
        tick += 1;
        observe_full(&mut rig, tick);
        rig.slot()
            .lock()
            .unwrap()
            .terminal_lifecycle_generation()
            .is_some()
            && rig.statuses.lock().unwrap()[0].script_walk().is_none()
    }));

    rig.statuses.lock().unwrap()[0].publish_script_walk(Some(WorldTile {
        x: 7,
        z: 7,
        level: 0,
    }));
    for _ in 0..3 {
        tick += 1;
        observe_full(&mut rig, tick);
    }
    assert!(
        rig.statuses.lock().unwrap()[0].script_walk().is_some(),
        "a failed run is retired once"
    );
}

/// PANEL-STATE-GAP-1023: a Stop that lands between the nav step's read of the
/// armed destination and its status write must not be overwritten. The nav
/// step parks at that point; a Stop thread runs `Play::script_stop`'s own
/// sequence (stop the run, reset navigation, release the slot, retire the
/// published rows); the step then writes. The destination read belongs to the
/// retired run, so it must not be published after the retirement.
#[test]
fn a_stop_between_the_walk_read_and_its_publication_is_not_overwritten() {
    use std::sync::mpsc::channel;
    use std::time::Duration;

    let mut rig = open_rig(false);
    {
        let mut shared = rig.shared.lock();
        shared.target = Some(WorldTile {
            x: 20,
            z: 0,
            level: 0,
        });
    }
    rig.observe(1);
    rig.wait_routed();
    rig.step();
    assert!(
        rig.statuses.lock().unwrap()[0].script_walk().is_some(),
        "the nav step published the armed destination"
    );

    let (read_tx, read_rx) = channel::<()>();
    let (stopped_tx, stopped_rx) = channel::<()>();
    let slot = rig.slot();
    let navs = Arc::clone(&rig.navs);
    let statuses = Arc::clone(&rig.statuses);
    let stopper = std::thread::spawn(move || {
        read_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let mut guard = slot.lock().unwrap();
        guard.stop();
        crate::script_runtime::reset_script_nav(&navs, "alice", None);
        drop(guard);
        crate::play_scripts::retire_script_run_status(&statuses, "alice");
        // The publisher may have stopped waiting by now.
        let _ = stopped_tx.send(());
    });
    crate::script_runtime::BEFORE_PUBLISH.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            read_tx.send(()).unwrap();
            // Where the destination is read under the navigation lock, the
            // Stop waits for that lock; where it is not, the Stop finishes here.
            let _ = stopped_rx.recv_timeout(Duration::from_secs(1));
        }));
    });
    rig.step();
    stopper.join().unwrap();

    assert_eq!(queued(&rig.navs), None, "Stop ended the run's route");
    assert_eq!(
        rig.statuses.lock().unwrap()[0].script_walk(),
        None,
        "an in-flight publication restored the retired run's destination"
    );
}
