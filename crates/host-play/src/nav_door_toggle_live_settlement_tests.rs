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
                options: script::FindOptions::default(),
                required_after: tick.cx.evidence(),
                evidence: None,
            };
            match tick.actions.begin::<Walk>(request, &mut tick.cx) {
                Ok(handle) => self.handle = Some(handle),
                Err(error) => control.result = Some(Err(error)),
            }
        }
        Ok(ScriptFlow::Continue)
    }
}

fn packed_world() -> (Arc<NavWorld>, Arc<api::gather_methods::GatherCatalog>) {
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

    let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
    let preparation_data = Arc::clone(&data);
    let catalog = api::selected::FamilyPreparation::run(move |worker| {
        preparation_data.prepare_gathering(worker)
    })
    .expect("gathering preparation worker")
    .join()
    .expect("gathering preparation thread")
    .expect("selected gathering catalog");
    world.bind_named_bank_facts(&data).unwrap();
    assert_eq!(world.loc_footprint_at(TARGET), Some((3, 3)));
    (world, catalog)
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
    let (world, _catalog) = packed_world();
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
        api::query::loc_approach::arrived_at(&rig.snapshot, estimated_endpoint, TARGET, 1),
        None,
        "the packed endpoint has no live footprint proof while the target is off-scene"
    );
    let request_id = live_owner_id(&rig.navs);

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

    rig.expose_live_footprint(estimated_endpoint);
    assert_eq!(
        api::query::loc_approach::arrived_at(&rig.snapshot, estimated_endpoint, TARGET, 1),
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
        api::query::loc_approach::arrived_at(&rig.snapshot, refreshed.dest, TARGET, 1),
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
