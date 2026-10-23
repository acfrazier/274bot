//! The panel's GPU teardown wiring: `window::release_renderer` (the step
//! `App::drop_window` runs before the renderer drops) calls the hook that
//! `run_panel` installs, and that hook releases the walk map. The textures
//! go through the real `window::Gpu` into a real ImGui WGPU renderer.

use std::cell::RefCell;
use std::rc::Rc;

use api::snapshot::WorldTile;
use dear_imgui_rs::TextureId;
use dear_imgui_wgpu as imgui_wgpu;
use nav_planner::collision::WorldCollision;
use nav_planner::map::spatial::{TileKey, View};
use nav_planner::transport::TransportGraph;
use nav_planner::world::NavWorld;

use super::{panel_gpu_shutdown, PanelState, StartupPreparation};
use crate::test_support::headless_gpu;
use crate::walk_map::{FixtureStore, OverlayColors, OverlayLayers};
use crate::window::{self, Gpu};

/// A `w`×`h` all-walkable world with cells on all four planes.
fn open_world(w: usize, h: usize) -> NavWorld {
    let cells = w * h * 4;
    NavWorld::from_parts(
        WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: w,
            height: h,
            walk: vec![0u8; cells],
            blocked: vec![0u64; cells.div_ceil(64)],
            flags: None,
        },
        TransportGraph::default(),
        Vec::new(),
    )
}

/// One LOD-0 tile at (0, 0) on `plane`, 16 pixels per game tile.
fn tile_view(plane: u8) -> View {
    View {
        west: 0.0,
        south: 0.0,
        east: 64.0,
        north: 64.0,
        pixels_per_tile: 16.0,
        plane,
        max_lod: 0,
    }
}

fn key(plane: u8, x: i32) -> TileKey {
    TileKey {
        plane,
        lod: 0,
        x,
        z: 0,
    }
}

const GRID: OverlayLayers = OverlayLayers {
    grid: true,
    collision_fill: false,
    reach: false,
    nsew: false,
    path: false,
    flood: false,
    special_areas: false,
};

#[test]
fn gpu_teardown_hook_unregisters_every_walk_map_texture_once() {
    let Some((device, queue)) = headless_gpu() else {
        return;
    };
    let _imgui = crate::test_support::imgui_context_guard();
    let mut context = crate::test_support::imgui_context();
    let mut renderer = imgui_wgpu::WgpuRenderer::new(
        imgui_wgpu::WgpuInitInfo::new(
            device.clone(),
            queue.clone(),
            wgpu::TextureFormat::Bgra8UnormSrgb,
        ),
        &mut context,
    )
    .expect("WgpuRenderer init on the headless device");
    let baseline = window::texture_ledger();

    let mut state = PanelState::default();
    state.walk_map.bind_fixtures(
        FixtureStore::with_keys([key(0, 0), key(0, 1), key(1, 0)]).expect("fixtures"),
    );
    state.walk_map.note_open();
    let running = Rc::new(RefCell::new(Some((state, StartupPreparation::new(None)))));

    // Plane 0 loads the visible tile, its ring tile and the grid overlay.
    // Plane 1 sends both plane-0 textures to the pool and takes one back,
    // so the teardown has a slot, a pool and an overlay texture to release.
    let world = open_world(3, 3);
    for plane in [0, 0, 0, 1, 1, 1] {
        let mut gpu = Gpu::new(&device, &queue, &mut renderer);
        let mut running = running.borrow_mut();
        let (state, _) = running.as_mut().expect("panel state");
        state.walk_map.test_sync(
            Some(&mut gpu),
            tile_view(plane),
            &world,
            GRID,
            OverlayColors::default(),
            &[],
            &[],
            None,
        );
    }
    let mut registered = window::texture_ledger().registered[baseline.registered.len()..].to_vec();
    registered.sort_unstable();
    assert_eq!(
        registered.len(),
        3,
        "setup registers two terrain textures and one overlay texture"
    );
    {
        let running = running.borrow();
        let counters = running.as_ref().expect("panel state").0.walk_map.counters();
        assert!(counters.terrain_gpu_bytes > 0, "a slot holds a texture");
        assert!(
            counters.overlay_gpu_bytes > 0,
            "the overlay holds a texture"
        );
    }

    // The same boxed hook type that `App` stores and `drop_window` passes.
    let mut on_gpu_shutdown: Box<dyn FnMut(&mut Gpu)> =
        Box::new(panel_gpu_shutdown(Rc::clone(&running)));
    window::release_renderer(&device, &queue, &mut renderer, &mut *on_gpu_shutdown);

    let ledger = window::texture_ledger();
    let mut unregistered = ledger.unregistered[baseline.unregistered.len()..].to_vec();
    unregistered.sort_unstable();
    let mut unique = unregistered.clone();
    unique.dedup();
    assert_eq!(unique, unregistered, "a texture id was unregistered twice");
    assert_eq!(
        unregistered, registered,
        "teardown must unregister every registered texture exactly once"
    );
    let held: Vec<u64> = registered
        .iter()
        .copied()
        .filter(|&id| {
            renderer
                .texture_manager()
                .contains_texture(TextureId::new(id))
        })
        .collect();
    assert!(
        held.is_empty(),
        "the renderer still holds walk-map textures: {held:?}"
    );
    let running = running.borrow();
    let (state, _) = running.as_ref().expect("panel state");
    assert!(!state.walk_map.is_open(), "teardown closes the walk map");
}
