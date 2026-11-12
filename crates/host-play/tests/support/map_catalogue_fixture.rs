use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use nav_planner::world::NavWorld;

use super::map_host;

static NEXT: AtomicU64 = AtomicU64::new(0);

pub struct ReadyCatalogueFixture {
    ready: Option<Arc<frontend_core::walk_map::WalkReadyCatalogue>>,
    root: PathBuf,
}

impl ReadyCatalogueFixture {
    pub fn new(profile: &map_host::ServerProfile) -> Self {
        use map_host::map_cache::{
            fixture::{self, CountingMapProducer},
            MapCacheRoot, MapDemand, MapDemandManager, MapJobStatus,
        };

        let root = std::env::temp_dir().join(format!(
            "274bot-ready-catalogue-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let cache = MapCacheRoot::from_root(&root);
        let manager = MapDemandManager::new(cache, Arc::new(CountingMapProducer::default()));
        // The unique root keeps each synthetic ready identity distinct; the producer
        // ignores client assets.
        let (image_policy, catalogue_policy) =
            map_host::map_artifact_policies().expect("map artifact policies");
        let descriptor = map_host::map_cache::MapProfileDescriptor::new(
            profile.revision().as_i32() as u16,
            nav_planner::map::identity::Digest::of(root.to_string_lossy().as_bytes()),
            image_policy,
            catalogue_policy,
        )
        .expect("map profile descriptor");
        let handle = manager
            .request_catalogue(descriptor.clone())
            .expect("catalogue fixture demand");
        fixture::wait_for_status(&handle, MapJobStatus::Ready);
        let ready = manager
            .open_ready(&descriptor, MapDemand::CatalogueOnly)
            .expect("ready catalogue fixture")
            .catalogue;
        Self {
            ready: Some(Arc::new(
                frontend_core::walk_map::WalkReadyCatalogue::from_test_fixture(ready),
            )),
            root,
        }
    }

    pub fn ready(&self) -> Arc<frontend_core::walk_map::WalkReadyCatalogue> {
        Arc::clone(self.ready.as_ref().expect("ready catalogue fixture"))
    }
}

impl Drop for ReadyCatalogueFixture {
    fn drop(&mut self) {
        self.ready.take();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub fn catalogue_overflow_world() -> Arc<NavWorld> {
    catalogue_overflow_world_with_offset(0)
}

pub fn changed_catalogue_overflow_world() -> Arc<NavWorld> {
    catalogue_overflow_world_with_offset(1_000_000)
}

fn catalogue_overflow_world_with_offset(loc_id_offset: i32) -> Arc<NavWorld> {
    let mut world = NavWorld::from_grid(&nav_planner::grid::StepGrid::fixture_open_3x3());
    let tile = api::snapshot::WorldTile {
        x: 0,
        z: 0,
        level: 0,
    };
    let edge = nav_planner::transport::TransportEdge {
        kind: nav_planner::transport::TransportKind::Door,
        at: tile,
        to: tile,
        takeoff: None,
        player_delta: None,
        loc_id: 0,
        option: 1,
        ticks: 1,
        dir: None,
        open_loc_id: None,
        skill_req: Vec::new(),
        item_req: Vec::new(),
        consumed_req: Vec::new(),
        item_returns: Vec::new(),
        quest_req: Vec::new(),
        varp_req: Vec::new(),
        worn_req: Vec::new(),
        worn_all_req: Vec::new(),
        forbidden_held: Vec::new(),
        forbidden_worn: Vec::new(),
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    };
    world.graph.edges.reserve(16_385);
    for index in 0..16_385 {
        let mut edge = edge.clone();
        edge.at.x = index;
        edge.to.x = index + 1;
        edge.loc_id = index + loc_id_offset;
        world.graph.edges.push(edge);
    }
    Arc::new(world)
}
