//! Typed native walks through the real host pump: `script_observe`'s receipt
//! delivery and native drain, the off-pump route worker and `step_nav_bot`.
use super::*;
use script::native::walk::Walk;
use script::native::{
    ActionError, ActionHandle, NativeTick, Script, ScriptFailure, ScriptFlow, WalkEnd, WalkReceipt,
    WalkRequest,
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
    disallow_prayer: bool,
    later_target: Option<WorldTile>,
    result: Option<Result<WalkReceipt, ActionError>>,
    results: Vec<Result<WalkReceipt, ActionError>>,
}

/// One native `Walk` to `(4, 0, 0)`, begun on the first eligible tick and
/// polled on every later one; the terminal lands in the shared cell.
struct WalkerScript {
    shared: Arc<parking_lot::Mutex<Walker>>,
    handle: Option<ActionHandle<Walk>>,
}

impl Script for WalkerScript {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        let mut shared = self.shared.lock();
        if std::mem::take(&mut shared.rewalk) {
            if let Some(handle) = self.handle.take() {
                tick.actions.cancel(handle);
            }
            shared.begun = 0;
        }
        if let Some(handle) = &self.handle {
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
                WorldTile {
                    x: 4,
                    z: 0,
                    level: 0,
                }
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
                radius: 0,
                loc_id: None,
                options: script::FindOptions::default(),
                required_after: tick.cx.evidence(),
                evidence: None,
                cross: cross.into_boxed_slice(),
                protect: shared.protect,
                allow: script::native::WalkAllow {
                    prayer: !shared.disallow_prayer,
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
                retryable: true,
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
    snapshot: GameSnapshot,
}

fn rig(world: Option<Arc<NavWorld>>, blocked: bool) -> Rig {
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let shared = Arc::new(parking_lot::Mutex::new(Walker {
        blocked,
        walks: 1,
        ..Walker::default()
    }));
    script_slot_or_insert(&scripts, "alice")
        .lock()
        .unwrap()
        .start_test_script(
            Box::new(WalkerScript {
                shared: Arc::clone(&shared),
                handle: None,
            }),
            None,
        )
        .unwrap();
    let mut client = nav_client();
    let mut snapshot = GameSnapshot::new();
    nav_snapshot_at(&mut client, &mut snapshot, 0, 0);
    Rig {
        scripts,
        cheats: Arc::new(Mutex::new(HashMap::new())),
        navs: Arc::new(Mutex::new(HashMap::new())),
        statuses: Arc::new(Mutex::new(vec![SlotStatus {
            username: "alice".into(),
            ..SlotStatus::default()
        }])),
        world,
        shared,
        driver: NavRec::default(),
        snapshot,
    }
}

fn open_rig(blocked: bool) -> Rig {
    rig(Some(Arc::new(open_world(40, 1))), blocked)
}
fn zoned_open_world() -> NavWorld {
    let mut world = open_world(40, 1);
    let zones = vec![nav::zones::Zone::npc(
        WorldTile {
            x: 2,
            z: 0,
            level: 0,
        },
        0,
        nav::zones::ZoneClass::Always,
        u16::MAX,
        0,
    )];
    let kinds = vec![nav::zones::ZoneKind::new(
        "test-barrier",
        "Test barrier",
        123,
        0,
        false,
        false,
    )];
    let table = nav::zones::ZoneTable::from_parts(
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
    let mut world = open_world(40, 1);
    let mut zones = vec![
        nav::zones::Zone::npc(
            WorldTile {
                x: 2,
                z: 0,
                level: 0,
            },
            0,
            nav::zones::ZoneClass::Always,
            u16::MAX,
            0,
        ),
        nav::zones::Zone::npc(
            WorldTile {
                x: 3,
                z: 0,
                level: 0,
            },
            0,
            nav::zones::ZoneClass::Always,
            u16::MAX,
            0,
        ),
    ];
    zones[0].group = 0;
    zones[1].group = 1;
    let kinds = vec![nav::zones::ZoneKind::new(
        "test-barrier",
        "Test barrier",
        123,
        0,
        false,
        false,
    )];
    let groups = vec![
        nav::zones::ZoneGroup::new(
            "white-wolf-mountain",
            "White Wolf Mountain",
            nav::router::AvoidRect {
                min_x: 2,
                max_x: 2,
                min_z: 0,
                max_z: 0,
                level: Some(0),
            },
            vec![0].into_boxed_slice(),
        ),
        nav::zones::ZoneGroup::new(
            "draynor-jail-guards",
            "Draynor jail guards",
            nav::router::AvoidRect {
                min_x: 3,
                max_x: 3,
                min_z: 0,
                max_z: 0,
                level: Some(0),
            },
            vec![1].into_boxed_slice(),
        ),
    ];
    let table = nav::zones::ZoneTable::from_parts(
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

impl Rig {
    fn observe(&mut self, tick: u64) {
        script_observe(
            &mut self.driver,
            "alice",
            true,
            true,
            tick,
            Some((0, 0, 0)),
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
    }

    fn step(&mut self) {
        step_nav_bot(
            &mut self.driver,
            "alice",
            Some((0, 0, 0)),
            &self.snapshot,
            &self.navs,
            &self.statuses,
            self.world.as_ref(),
            false,
            false,
            no_reach,
        );
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
            wait_until(5_000, || queued(&self.navs).is_some()),
            "the worker armed the native route"
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
fn a_blocked_tick_dispatches_no_native_walk() {
    let mut rig = open_rig(true);
    rig.observe(1);
    assert_eq!(rig.shared.lock().begun, 1, "the tick began a walk");
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
        t1: 0,
        t2: 30,
    }]);
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
    rig.observe(2);
    let receipt = rig
        .shared
        .lock()
        .result
        .clone()
        .expect("Unprotectable must complete the walk")
        .expect("Unprotectable is a successful host terminal");
    assert_eq!(
        receipt.end,
        WalkEnd::Unprotectable,
        "the host must deliver Unprotectable to the walk owner"
    );
    assert_eq!(
        receipt.detail.as_deref(),
        Some("missiles"),
        "Unprotectable must keep the wanted protection style on the receipt"
    );
    assert!(
        rig.driver.if_button_components.is_empty(),
        "Unprotectable must not send a prayer packet, got {:?}",
        rig.driver.if_button_components
    );
}

#[test]
fn an_unroutable_native_walk_delivers_one_failed_terminal() {
    let mut rig = rig(Some(Arc::new(open_world(3, 1))), false);
    rig.observe(1);
    assert!(
        wait_until(5_000, || rig.navs.lock().unwrap()["alice"]
            .route_worker
            .is_none()),
        "the planner completes the unreachable request"
    );
    rig.observe(2);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::Failed)));
    for tick in 3..40 {
        rig.observe(tick);
        rig.step();
    }
    assert_eq!(rig.shared.lock().results.len(), 1);
    assert_eq!(rig.shared.lock().begun, 1);
    assert_eq!(rig.driver.walked, None);
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
        bank: Vec::new(),
    }
    .queue_route_in_snapshot(
        &rig.snapshot,
        x,
        0,
        0,
        nav::router::FindOptions::default(),
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
            Some(nav::traveller::TravelOutcome::EvidenceUnproven {
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
        Some(nav::traveller::TravelOutcome::Arrived {
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

    rig.observe(3);
    assert!(wait_until(5_000, || {
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
        Some(&[nav::zones::ZoneKey::Zone(0)][..])
    );
    assert!(second
        .detail
        .as_deref()
        .is_some_and(|detail| detail.contains("test-barrier@2,0,0")));
}

#[test]
fn compat_catalog_exclusions_use_frozen_geometry_and_rules() {
    use script::shim::InspectAvoidWire;

    let world = catalog_open_world();
    let outside = WorldTile {
        x: 0,
        z: 0,
        level: 0,
    };
    let inside_jail = WorldTile {
        x: 3_100,
        z: 3_230,
        level: 0,
    };
    let resolve_avoid = |from, state: &WorldState, id: &str| {
        let mut exclusions = crate::script_runtime::ScriptRouteExclusions::default();
        exclusions
            .avoid_wire
            .push(InspectAvoidWire::Catalog(id.to_string()));
        crate::script_runtime::resolve_route_exclusions(
            nav::router::FindOptions::default(),
            &world,
            from,
            outside,
            state,
            exclusions,
        )
    };

    let state = WorldState::empty();
    let (_, white_wolf) = resolve_avoid(outside, &state, "white-wolf-mountain").unwrap();
    assert_eq!(white_wolf.avoid.len(), 1);
    assert_eq!(
        (
            white_wolf.avoid[0].min_x,
            white_wolf.avoid[0].max_x,
            white_wolf.avoid[0].min_z,
            white_wolf.avoid[0].max_z,
            white_wolf.avoid[0].level,
        ),
        (2828, 2878, 3468, 3538, None)
    );

    let (_, jail) = resolve_avoid(outside, &state, "draynor-jail-guards").unwrap();
    assert_eq!(jail.avoid.len(), 4);
    let mut high_combat = WorldState::empty();
    high_combat.combat_level = Some(51);
    let (_, skipped_for_combat) =
        resolve_avoid(outside, &high_combat, "draynor-jail-guards").unwrap();
    assert!(skipped_for_combat.avoid.is_empty());
    let (_, skipped_for_endpoint) =
        resolve_avoid(inside_jail, &state, "draynor-jail-guards").unwrap();
    assert!(skipped_for_endpoint.avoid.is_empty());
}

#[test]
fn compat_named_exclusions_reject_unknown_and_over_limit_names() {
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
            nav::router::FindOptions::default(),
            &world,
            from,
            from,
            &state,
            unknown,
        )
        .unwrap_err(),
        "avoidZones: unknown zone \"no-such-zone\""
    );

    let mut unknown_cross = crate::script_runtime::ScriptRouteExclusions::default();
    unknown_cross.cross.push(Arc::from("no-such-zone"));
    assert_eq!(
        crate::script_runtime::resolve_route_exclusions(
            nav::router::FindOptions::default(),
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
            nav::router::FindOptions::default(),
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

fn drive_second_walk(rig: &mut Rig, first_tick: u64) -> Vec<Result<WalkEnd, ActionError>> {
    for tick in (first_tick..).take(40) {
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
fn review_control_generic_cancel_then_nopath_delivers_failed() {
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
    assert_eq!(ends[1], Ok(WalkEnd::Failed));
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
    assert_eq!(ends[1], Ok(WalkEnd::Failed));
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
        Some(WorldState::empty()),
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
    assert_eq!(ends[1], Ok(WalkEnd::Failed));
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
        bank: Vec::new(),
    };
    let completed = arm
        .queue_route_in_snapshot_synced(
            &rig.snapshot,
            8,
            0,
            0,
            nav::router::FindOptions::default(),
            0,
            true,
            99,
            None,
        )
        .expect("the next walk route arms");
    assert!(
        !rig.navs.lock().unwrap()["alice"].manual_walk_cancelled_detail,
        "the detail clears on the next route arm"
    );
    assert_eq!(
        rig.navs.lock().unwrap()["alice"].walk_outcome_cancel_reason,
        script::isolate_fb::WalkCancelReason::UserInput,
        "the previously published receipt remains intact"
    );
    completed
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("the new route worker completes");
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
            script::isolate_fb::WalkCancelReason::UserInput
        );
    }
}

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
        script::LoadShape::NativeTick,
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
        script::isolate_fb::WalkCancelReason::UserInput
    );
    assert!(play.script_walk_cancelled_by_user("alice"));

    play.script_stop("alice");
    let navs = play.navs.lock().unwrap();
    assert_eq!(
        navs["alice"].walk_outcome_cancel_reason,
        script::isolate_fb::WalkCancelReason::None,
        "Stop clears the prior run's UserInput outcome"
    );
    assert_eq!(navs["alice"].walk_outcome_request_id, 0);
    assert_eq!(navs["alice"].walk_live_refusal_id, 0);
    drop(navs);
    assert!(!play.script_walk_cancelled_by_user("alice"));

    play.script_start_load(
        "alice",
        source.into(),
        script::LoadShape::NativeTick,
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
        manual_move_intent: Some(crate::ManualMoveIntent::Minimap),
        manual_steps: 0,
    }
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
        script::isolate_fb::WalkCancelReason::UserInput
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
        Ok(WalkEnd::Failed),
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
    let route = rig.navs.lock().unwrap()["alice"].route.clone();
    rig.navs.lock().unwrap().insert(
        "bob".into(),
        NavBot {
            route,
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
        Some(nav::traveller::TravelOutcome::Arrived {
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
        assert!(bot.route.is_none());
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
            script::isolate_fb::WalkCancelReason::None
        );
        assert_eq!(bot.manual_takeover_watermark, 0);
        assert_eq!(bot.user_move_intent_seq, 1, "idle intent is still observed");
    }
    rig.observe(3);
    assert_eq!(rig.end(), Some(Ok(WalkEnd::RouteEnded)));
    assert_eq!(rig.shared.lock().results.len(), 1);
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
            .map(|req| script::load::QueuedInteract {
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
            script::isolate_fb::WalkCancelReason::UserInput
        );
    }
    rig.slot()
        .lock()
        .unwrap()
        .restore_host_interacts(vec![script::load::QueuedInteract {
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
    let route = rig.navs.lock().unwrap()["alice"].route.clone();
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
    assert!(alice.lock().unwrap().route.is_none());
    assert_eq!(alice.lock().unwrap().route_generation, 18);
    assert!(bob.lock().unwrap().route.is_some());
    assert_eq!(bob.lock().unwrap().route_generation, 19);
}
