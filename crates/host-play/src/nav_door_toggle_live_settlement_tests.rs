use super::*;
use client::dash3d::CollisionFlag;
use script::native::walk::Walk;
use script::native::{
    ActionError, ActionHandle, NativeTick, Script, ScriptFailure, ScriptFlow, WalkReceipt,
    WalkRequest,
};
use std::task::Poll;
use std::time::{Duration, Instant};

const ALICE: &str = "alice";
const TARGET: WorldTile = WorldTile {
    x: 3085,
    z: 3468,
    level: 0,
};
const START: WorldTile = WorldTile {
    x: TARGET.x - 2,
    z: TARGET.z,
    level: TARGET.level,
};
const INITIAL_BASE: WorldTile = WorldTile {
    x: TARGET.x - 104,
    z: TARGET.z - 52,
    level: TARGET.level,
};
const LIVE_BASE: WorldTile = WorldTile {
    x: TARGET.x - 52,
    z: TARGET.z - 52,
    level: TARGET.level,
};

#[derive(Default)]
struct WalkControl {
    started: bool,
    cancel: bool,
    result: Option<Result<WalkReceipt, ActionError>>,
}

struct SettlementWalker {
    control: Arc<parking_lot::Mutex<WalkControl>>,
    handle: Option<ActionHandle<Walk>>,
}

impl Script for SettlementWalker {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        let mut control = self.control.lock();
        if std::mem::take(&mut control.cancel) {
            if let Some(handle) = self.handle.take() {
                tick.actions.cancel(handle);
            }
        }
        let completed =
            self.handle
                .as_ref()
                .and_then(|handle| match tick.actions.poll(handle, &mut tick.cx) {
                    Poll::Ready(result) => Some(result),
                    Poll::Pending => None,
                });
        if let Some(result) = completed {
            control.result = Some(result);
            self.handle = None;
        }
        if self.handle.is_none() && !control.started {
            control.started = true;
            let request = WalkRequest {
                target: TARGET,
                radius: 1,
                loc_id: Some(0),
                options: script::FindOptions::default(),
                required_after: tick.cx.evidence(),
                evidence: None,
                cross: Box::default(),
                protect: false,
                allow: Default::default(),
            };
            match tick.actions.begin::<Walk>(request, &mut tick.cx) {
                Ok(handle) => self.handle = Some(handle),
                Err(error) => control.result = Some(Err(error)),
            }
        }
        Ok(ScriptFlow::Continue)
    }
}

fn packed_world() -> Arc<NavWorld> {
    const SIZE: usize = 160;
    let origin = WorldTile {
        x: TARGET.x - 158,
        z: TARGET.z - 80,
        level: TARGET.level,
    };
    let mut flags = vec![0u32; SIZE * SIZE];
    for tile in [
        TARGET,
        WorldTile {
            x: TARGET.x + 1,
            ..TARGET
        },
        WorldTile {
            x: TARGET.x - 1,
            z: TARGET.z - 1,
            ..TARGET
        },
        WorldTile {
            x: TARGET.x - 1,
            z: TARGET.z + 1,
            ..TARGET
        },
    ] {
        let x = (tile.x - origin.x) as usize;
        let z = (tile.z - origin.z) as usize;
        flags[z * SIZE + x] |= CollisionFlag::SQ_BLOCKED as u32;
    }
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    Arc::new(NavWorld::from_parts(
        nav::collision::WorldCollision {
            origin,
            width: SIZE,
            height: SIZE,
            walk,
            blocked,
            flags: None,
        },
        nav::transport::TransportGraph::default(),
        Vec::new(),
    ))
}

struct SettlementRig {
    client: client::client::Client,
    snapshot: GameSnapshot,
    scripts: ScriptWall,
    cheats: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    navs: Arc<Mutex<HashMap<String, NavBot>>>,
    statuses: Arc<Mutex<Vec<SlotStatus>>>,
    world: Option<Arc<NavWorld>>,
    control: Arc<parking_lot::Mutex<WalkControl>>,
}

impl SettlementRig {
    fn new(world: Arc<NavWorld>) -> Self {
        let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
        let control = Arc::new(parking_lot::Mutex::new(WalkControl::default()));
        script_slot_or_insert(&scripts, ALICE)
            .lock()
            .unwrap()
            .start_test_script(
                Box::new(SettlementWalker {
                    control: Arc::clone(&control),
                    handle: None,
                }),
                None,
            )
            .unwrap();

        let mut client = crate::tests::nav_client();
        client.map_build_base_x = INITIAL_BASE.x;
        client.map_build_base_z = INITIAL_BASE.z;
        let mut snapshot = GameSnapshot::new();
        crate::tests::nav_snapshot_at(
            &mut client,
            &mut snapshot,
            START.x - INITIAL_BASE.x,
            START.z - INITIAL_BASE.z,
        );
        let mut player = snapshot.local_player().unwrap().clone();
        player.player.actor.tile = START;
        snapshot.seed_local_player(player);
        Self {
            client,
            snapshot,
            scripts,
            cheats: Arc::new(Mutex::new(HashMap::new())),
            navs: Arc::new(Mutex::new(HashMap::new())),
            statuses: Arc::new(Mutex::new(vec![SlotStatus {
                username: ALICE.into(),
                ..SlotStatus::default()
            }])),
            world: Some(world),
            control,
        }
    }

    fn observe(&mut self, tick: u64, here: WorldTile) {
        script_observe(
            &mut self.client,
            ALICE,
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
    }

    fn pump(&mut self, here: WorldTile) {
        step_nav_bot(
            &mut self.client,
            ALICE,
            Some((here.x, here.z, here.level)),
            &self.snapshot,
            &self.navs,
            &self.statuses,
            self.world.as_ref(),
            false,
            false,
            || Arc::new(api::query::ReachQueryView::unavailable()),
        );
    }

    fn set_position(&mut self, here: WorldTile) {
        let x = here.x - self.client.map_build_base_x;
        let z = here.z - self.client.map_build_base_z;
        crate::tests::nav_snapshot_at(&mut self.client, &mut self.snapshot, x, z);
        let mut player = self.snapshot.local_player().unwrap().clone();
        player.player.actor.tile = here;
        self.snapshot.seed_local_player(player);
    }

    fn expose_live_footprint(&mut self, here: WorldTile) {
        self.client.map_build_base_x = LIVE_BASE.x;
        self.client.map_build_base_z = LIVE_BASE.z;
        crate::tests::plant_nav_footprint_loc(&mut self.client, 52, 52, 1, 1);
        let locs = Arc::get_mut(&mut self.client.cache).expect("test owns client cache");
        locs.locs.last_mut().expect("planted loc").forceapproach = 8;
        crate::tests::plant_nav_footprint_loc(&mut self.client, 53, 52, 1, 1);
        let locs = Arc::get_mut(&mut self.client.cache).expect("test owns client cache");
        locs.locs.last_mut().expect("planted tree").name = "Tree".into();
        self.client.collision[0].flags[52][52] |= CollisionFlag::SQ_BLOCKED;
        self.client.collision[0].flags[53][52] |= CollisionFlag::SQ_BLOCKED;
        self.set_position(here);
    }
}

fn wait_for_route(
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    mut matches: impl FnMut(WorldTile) -> bool,
) -> nav::router::Route {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let route = navs
            .lock()
            .unwrap()
            .get(ALICE)
            .and_then(|bot| bot.route.clone());
        if let Some(route) = route {
            if matches(route.dest) {
                return route;
            }
        }
        assert!(
            Instant::now() < deadline,
            "route worker did not arm the expected endpoint"
        );
        std::thread::yield_now();
    }
}

fn live_owner_id(navs: &Arc<Mutex<HashMap<String, NavBot>>>) -> u64 {
    let all = navs.lock().unwrap();
    let owner = all[ALICE]
        .native_walk
        .as_ref()
        .expect("native walk owner remains attached");
    assert!(owner.live(), "the native walk owner stays live");
    owner.request_id().get()
}

#[test]
fn estimated_loc_endpoint_waits_for_live_footprint_then_refreshes_under_same_owner() {
    let world = packed_world();
    let mut rig = SettlementRig::new(world);
    rig.observe(1, START);
    let estimated = wait_for_route(&rig.navs, |_| true);
    let estimated_endpoint = estimated.dest;
    assert_eq!(
        estimated_endpoint,
        WorldTile {
            x: TARGET.x - 1,
            z: TARGET.z,
            level: TARGET.level,
        },
        "the packed estimate picks the short west-side stand"
    );
    assert_eq!(
        api::query::loc_approach::arrived_at(&rig.snapshot, estimated_endpoint, TARGET, 1, 0),
        None,
        "the packed endpoint has no live footprint proof while the target is off-scene"
    );
    let request_id = live_owner_id(&rig.navs);
    let initial_generation = rig.navs.lock().unwrap()[ALICE].route_generation;

    rig.set_position(estimated_endpoint);
    rig.observe(2, estimated_endpoint);
    assert!(
        rig.control.lock().result.is_none(),
        "the native adapter cannot report Arrived before the host pump"
    );
    rig.pump(estimated_endpoint);
    assert!(
        rig.control.lock().result.is_none(),
        "an off-scene estimate cannot settle the native walk"
    );
    assert_eq!(live_owner_id(&rig.navs), request_id);
    assert_eq!(
        rig.navs.lock().unwrap()[ALICE]
            .route
            .as_ref()
            .map(|route| route.dest),
        Some(estimated_endpoint),
        "the estimated route remains armed until live footprint evidence arrives"
    );
    for _ in 0..10 {
        rig.pump(estimated_endpoint);
    }
    let unchanged_generation = rig.navs.lock().unwrap()[ALICE].route_generation;
    assert_eq!(
        unchanged_generation, initial_generation,
        "an unchanged estimate is not refreshed"
    );

    rig.expose_live_footprint(estimated_endpoint);
    // Enter the scene with the footprint already observed.
    assert_eq!(
        api::query::loc_approach::arrived_at(&rig.snapshot, estimated_endpoint, TARGET, 1, 0),
        Some(false),
        "the real footprint rejects its forced-west side"
    );
    rig.observe(3, estimated_endpoint);
    assert!(
        rig.control.lock().result.is_none(),
        "live force-approach also gates native completion before the host pump"
    );
    rig.pump(estimated_endpoint);
    assert_eq!(
        live_owner_id(&rig.navs),
        request_id,
        "refresh preserves native ownership"
    );

    let refreshed = wait_for_route(&rig.navs, |dest| dest != estimated_endpoint);
    let refreshed_generation = rig.navs.lock().unwrap()[ALICE].route_generation;
    assert_eq!(
        refreshed_generation,
        initial_generation + 1,
        "scene entry with a known footprint refreshes once"
    );
    assert!(
        (refreshed.dest
            == WorldTile {
                x: TARGET.x,
                z: TARGET.z - 1,
                level: TARGET.level
            })
            || (refreshed.dest
                == WorldTile {
                    x: TARGET.x,
                    z: TARGET.z + 1,
                    level: TARGET.level
                }),
        "the refreshed route uses a live-valid north/south stand, got {:?}",
        refreshed.dest
    );
    assert_eq!(
        api::query::loc_approach::arrived_at(&rig.snapshot, refreshed.dest, TARGET, 1, 0),
        Some(true),
        "the refreshed endpoint passes the live footprint predicate"
    );
    assert!(
        rig.control.lock().result.is_none(),
        "refresh is not a terminal native receipt"
    );

    rig.control.lock().cancel = true;
    rig.observe(4, estimated_endpoint);
    rig.pump(estimated_endpoint);
    let all = rig.navs.lock().unwrap();
    assert!(
        all[ALICE].route.is_none(),
        "explicit native cancellation clears the refreshed follow"
    );
    assert!(
        all[ALICE]
            .native_walk
            .as_ref()
            .is_none_or(|owner| !owner.live()),
        "explicit cancellation revokes the original owner"
    );
}

#[test]
fn loc_intent_target_gone_settles_instead_of_waiting_for_respawn() {
    for replaced in [true, false] {
        for gone_before_arm in [false, true] {
            let start = WorldTile {
                x: TARGET.x - 4,
                ..TARGET
            };
            let mut rig = SettlementRig::new(packed_world());
            rig.client.map_build_base_x = LIVE_BASE.x;
            rig.client.map_build_base_z = LIVE_BASE.z;
            crate::tests::plant_nav_footprint_loc(&mut rig.client, 52, 52, 1, 1);
            let cache = Arc::get_mut(&mut rig.client.cache).expect("rig owns cache");
            cache.locs.last_mut().unwrap().name = "Yew".into();
            rig.client.collision[0].flags[52][52] |= CollisionFlag::SQ_BLOCKED;
            rig.set_position(start);

            let deplete = |rig: &mut SettlementRig, here| {
                if replaced {
                    if rig
                        .snapshot
                        .locs()
                        .iter()
                        .any(|loc| loc.tile == TARGET && loc.id == 0)
                    {
                        crate::tests::plant_nav_footprint_loc(&mut rig.client, 52, 52, 1, 1);
                        let cache = Arc::get_mut(&mut rig.client.cache).expect("rig owns cache");
                        cache.locs.last_mut().unwrap().name = "Tree stump".into();
                    }
                    rig.set_position(here);
                    assert!(rig
                        .snapshot
                        .locs()
                        .iter()
                        .any(|loc| loc.tile == TARGET && loc.id == 1));
                } else {
                    rig.snapshot.seed_locs(Vec::new());
                }
                assert!(!rig
                    .snapshot
                    .locs()
                    .iter()
                    .any(|loc| loc.tile == TARGET && loc.id == 0));
            };
            if gone_before_arm {
                deplete(&mut rig, start);
            }
            rig.observe(1, start);
            let endpoint = wait_for_route(&rig.navs, |_| true).dest;
            let initial_generation = rig.navs.lock().unwrap()[ALICE].route_generation;
            rig.set_position(endpoint);
            deplete(&mut rig, endpoint);
            for tick in 2..42 {
                rig.observe(tick, endpoint);
                rig.pump(endpoint);
            }
            rig.observe(42, endpoint);
            assert!(
                matches!(rig.control.lock().result.as_ref(), Some(Ok(receipt)) if receipt.end == script::native::WalkEnd::Arrived),
                "replaced={replaced}, gone_before_arm={gone_before_arm}: a gone target must settle through tile arrival"
            );
            let all = rig.navs.lock().unwrap();
            assert!(
                all[ALICE].route.is_none(),
                "the host follow must settle too"
            );
            assert!(all[ALICE]
                .native_walk
                .as_ref()
                .is_none_or(|owner| !owner.live()));
            assert_eq!(
                all[ALICE].route_generation,
                initial_generation + 1,
                "terminal cleanup must not restart the route"
            );
        }
    }
}

#[test]
fn loc_intent_missing_on_scene_entry_settles_through_tile_arrival() {
    let mut rig = SettlementRig::new(packed_world());
    rig.observe(1, START);
    let endpoint = wait_for_route(&rig.navs, |_| true).dest;
    rig.set_position(endpoint);
    rig.pump(endpoint);
    assert!(rig.navs.lock().unwrap()[ALICE].route.is_some());
    rig.client.map_build_base_x = LIVE_BASE.x;
    rig.client.map_build_base_z = LIVE_BASE.z;
    rig.set_position(endpoint);
    rig.pump(endpoint);
    rig.observe(2, endpoint);
    assert!(
        matches!(rig.control.lock().result.as_ref(), Some(Ok(receipt)) if receipt.end == script::native::WalkEnd::Arrived),
        "an observed empty scene is target-gone evidence, not an off-scene estimate"
    );
    assert!(rig.navs.lock().unwrap()[ALICE].route.is_none());
}

#[test]
fn depleted_large_loc_refreshes_to_plain_tile_radius_before_settling() {
    let start = WorldTile {
        x: TARGET.x + 4,
        ..TARGET
    };
    let origin = WorldTile {
        x: TARGET.x - 80,
        z: TARGET.z - 80,
        ..TARGET
    };
    let mut flags = vec![0u32; 160 * 160];
    flags[80 * 160 + 80] = CollisionFlag::SQ_BLOCKED as u32;
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    let world = Arc::new(NavWorld::from_parts(
        nav::collision::WorldCollision {
            origin,
            width: 160,
            height: 160,
            walk,
            blocked,
            flags: None,
        },
        nav::transport::TransportGraph::default(),
        Vec::new(),
    ));
    let mut rig = SettlementRig::new(world);
    rig.client.map_build_base_x = LIVE_BASE.x;
    rig.client.map_build_base_z = LIVE_BASE.z;
    crate::tests::plant_nav_footprint_loc(&mut rig.client, 52, 52, 3, 3);
    let cache = Arc::get_mut(&mut rig.client.cache).expect("rig owns cache");
    cache.locs.last_mut().unwrap().forceapproach = 13; // East side only.
    rig.client.collision[0].flags[52][52] |= CollisionFlag::SQ_BLOCKED;
    rig.set_position(start);
    rig.observe(1, start);
    let old_endpoint = wait_for_route(&rig.navs, |_| true).dest;
    assert!(old_endpoint.x.abs_diff(TARGET.x) > 1);
    let request_id = live_owner_id(&rig.navs);
    let initial_generation = rig.navs.lock().unwrap()[ALICE].route_generation;

    crate::tests::plant_nav_footprint_loc(&mut rig.client, 52, 52, 1, 1);
    rig.set_position(old_endpoint);
    rig.observe(2, old_endpoint);
    rig.pump(old_endpoint);
    let endpoint = wait_for_route(&rig.navs, |dest| dest != old_endpoint).dest;
    assert_eq!(live_owner_id(&rig.navs), request_id);
    assert!(
        endpoint
            .x
            .abs_diff(TARGET.x)
            .max(endpoint.z.abs_diff(TARGET.z))
            <= 1
    );
    assert!(rig.control.lock().result.is_none());
    assert_eq!(
        rig.navs.lock().unwrap()[ALICE].route_generation,
        initial_generation + 1,
        "the vanished footprint refreshes once under the same owner"
    );
    rig.set_position(endpoint);
    rig.pump(endpoint);
    rig.observe(3, endpoint);
    assert!(
        matches!(rig.control.lock().result.as_ref(), Some(Ok(receipt)) if receipt.end == script::native::WalkEnd::Arrived),
        "a depleted large target must not end at its obsolete far perimeter"
    );
    assert!(rig.navs.lock().unwrap()[ALICE].route.is_none());
}

#[test]
fn offscene_solid_strip_walk_makes_progress_without_unchanged_refreshes() {
    const SIZE: usize = 160;
    let origin = WorldTile {
        x: TARGET.x - 140,
        z: TARGET.z - 80,
        ..TARGET
    };
    let mut flags = vec![0u32; SIZE * SIZE];
    for x in TARGET.x - 100..=TARGET.x + 1 {
        for z in TARGET.z - 1..=TARGET.z + 1 {
            flags[(z - origin.z) as usize * SIZE + (x - origin.x) as usize] |=
                CollisionFlag::SQ_BLOCKED as u32;
        }
    }
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    let world = Arc::new(NavWorld::from_parts(
        nav::collision::WorldCollision {
            origin,
            width: SIZE,
            height: SIZE,
            walk,
            blocked,
            flags: None,
        },
        nav::transport::TransportGraph::default(),
        Vec::new(),
    ));
    let start = WorldTile {
        x: TARGET.x - 101,
        ..TARGET
    };
    let mut rig = SettlementRig::new(world);
    rig.client.map_build_base_x = start.x - 52;
    rig.set_position(start);
    rig.observe(1, start);
    let deadline = Instant::now() + Duration::from_secs(3);
    while rig.navs.lock().unwrap()[ALICE].route_worker.is_some() {
        assert!(Instant::now() < deadline, "initial worker did not finish");
        std::thread::yield_now();
    }
    rig.observe(2, start);
    rig.pump(start);
    assert!(
        matches!(rig.control.lock().result.as_ref(), Some(Ok(receipt)) if receipt.end == script::native::WalkEnd::Failed),
        "the impossible anchor estimate must fail promptly rather than livelock"
    );
    let generation = rig.navs.lock().unwrap()[ALICE].route_generation;
    for tick in 3..13 {
        rig.observe(tick, start);
        rig.pump(start);
    }
    let (after, route) = {
        let all = rig.navs.lock().unwrap();
        (all[ALICE].route_generation, all[ALICE].route.clone())
    };
    assert_eq!(
        after, generation,
        "unchanged inputs cannot restart the worker"
    );
    assert!(
        route.is_none(),
        "no anchor-radius stand exists inside this strip; never route to its remote perimeter"
    );
}

#[test]
fn plain_radius_walk_ignores_ground_decoration_on_target() {
    let target = WorldTile {
        x: 10,
        z: 10,
        level: 0,
    };
    let from = WorldTile {
        x: 13,
        z: 11,
        ..target
    };
    let mut client = crate::tests::nav_client();
    crate::tests::plant_nav_footprint_loc(&mut client, target.x, target.z, 1, 1);
    let mut snapshot = GameSnapshot::new();
    crate::tests::nav_snapshot_at(&mut client, &mut snapshot, from.x, from.z);
    let mut locs = snapshot.locs().to_vec();
    let loc = locs
        .iter_mut()
        .find(|loc| loc.tile == target)
        .expect("planted loc");
    loc.shape = 22;
    loc.name = Some("Daisies".into());
    loc.actions.clear();
    loc.block_walk = false;
    snapshot.seed_locs(locs);
    let stamp = api::quest_progress::EvidenceStamp {
        run: api::selected::RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        tick: 1,
        sequence: 1,
    };
    let view = api::snapshot::SnapshotView::new(Some(&snapshot), stamp);
    assert!(
        view.walk_arrived(from, target, 5),
        "a decoration cannot shrink a plain tile radius"
    );
}
