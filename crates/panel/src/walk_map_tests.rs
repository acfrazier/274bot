use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;
use dear_imgui_rs::TextureId;
use nav::collision::WorldCollision;
use nav::map::spatial::{
    select_lod, GameTile, TileKey, View, TEXTURE_CAP, TILE_PIXELS, TILE_RGBA_BYTES,
};
use nav::transport::TransportGraph;
use nav::world::NavWorld;

use super::overlay::{self, OverlayColors, OverlayFit, OverlayLayers, OverlayPaint};
use super::{
    decode_tile, encode_tile, overlay_fit, parse_coord, phys_footprint_bytes, rasterize_overlay,
    view_from_canvas, FixtureStore, MapPhase, WalkMapRenderer, CACHE_UNBOUND, MAX_IDX_DEFAULT,
    MAX_IDX_LAYERS, MAX_VTX_DEFAULT, MAX_VTX_LAYERS, MIN_CELL_PPT, OVERLAY_BYTE_CAP, OVERLAY_MAX_H,
    OVERLAY_MAX_W,
};
use crate::game_view::FrameGpu;

fn open_world(w: usize, h: usize) -> NavWorld {
    NavWorld::from_parts(
        WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: w,
            height: h,
            walk: vec![0u8; w * h],
            blocked: vec![0u64; (w * h).div_ceil(64)],
            flags: None,
        },
        TransportGraph::default(),
        Vec::new(),
    )
}

fn bake_world(w: usize, h: usize, extras: &[(i32, i32, u32)]) -> NavWorld {
    let mut flags = vec![0u32; w * h];
    for &(x, z, f) in extras {
        flags[z as usize * w + x as usize] |= f;
    }
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    NavWorld::from_parts(
        WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: w,
            height: h,
            walk,
            blocked,
            flags: None,
        },
        TransportGraph::default(),
        Vec::new(),
    )
}

fn layers_grid() -> OverlayLayers {
    OverlayLayers {
        grid: true,
        collision_fill: false,
        reach: false,
        nsew: false,
        path: false,
        flood: false,
    }
}

fn layers_none() -> OverlayLayers {
    OverlayLayers {
        grid: false,
        collision_fill: false,
        reach: false,
        nsew: false,
        path: false,
        flood: false,
    }
}

fn layers_all() -> OverlayLayers {
    OverlayLayers {
        grid: true,
        collision_fill: true,
        reach: true,
        nsew: true,
        path: true,
        flood: true,
    }
}

fn small_view() -> View {
    View {
        west: 0.0,
        south: 0.0,
        east: 8.0,
        north: 8.0,
        pixels_per_tile: 16.0,
        plane: 0,
        max_lod: 1,
    }
}

#[test]
fn fixture_png_is_real_258_rgba8() {
    let key = TileKey {
        plane: 0,
        lod: 0,
        x: 50,
        z: 50,
    };
    let png = encode_tile(key).expect("encode");
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    let rgba = decode_tile(&png).expect("decode");
    assert_eq!(rgba.len(), TILE_RGBA_BYTES);
    assert_eq!(rgba.len(), (TILE_PIXELS * TILE_PIXELS * 4) as usize);
    // Interior north stripe is the high-green marker from the fixture raster.
    let i = ((8 * TILE_PIXELS + 8) * 4) as usize;
    assert_eq!(rgba[i + 1], 220);
    assert_eq!(rgba[i + 3], 255);
}

#[test]
fn overlay_fit_uses_world_span_not_window_scale() {
    let ready = overlay_fit(small_view());
    match ready {
        OverlayFit::Ready { w, h } => {
            assert_eq!(w, (8.0 * MIN_CELL_PPT) as u32);
            assert_eq!(h, (8.0 * MIN_CELL_PPT) as u32);
            assert!(w <= OVERLAY_MAX_W && h <= OVERLAY_MAX_H);
        }
        OverlayFit::ZoomIn => panic!("8-tile view at 4 overlay-px/tile must fit"),
    }
    let coarse = View {
        west: 0.0,
        south: 0.0,
        east: 400.0,
        north: 300.0,
        pixels_per_tile: 2.0,
        plane: 0,
        max_lod: 1,
    };
    assert_eq!(overlay_fit(coarse), OverlayFit::ZoomIn);
}

#[test]
fn overlay_rasterizes_collision_not_per_tile_quads() {
    let world = bake_world(5, 1, &[(2, 0, CollisionFlag::WR_GRND as u32)]);
    let view = View {
        west: 0.0,
        south: 0.0,
        east: 5.0,
        north: 1.0,
        pixels_per_tile: 16.0,
        plane: 0,
        max_lod: 0,
    };
    let OverlayFit::Ready { w, h } = overlay_fit(view) else {
        panic!("tiny view must fit");
    };
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    let colors = OverlayColors::default();
    rasterize_overlay(
        &mut rgba,
        w,
        h,
        view,
        OverlayPaint {
            world: &world,
            layers: OverlayLayers {
                grid: false,
                collision_fill: true,
                reach: false,
                nsew: false,
                path: false,
                flood: false,
            },
            colors,
            path: &[],
            floods: &[],
            reach: None,
        },
    );
    let sx = w as f64 / 5.0;
    let x = (2.5 * sx) as u32;
    let y = h / 2;
    assert_eq!(overlay::pixel(&rgba, w, x, y), colors.collision);
    let x_open = (0.5 * sx) as u32;
    assert_eq!(overlay::pixel(&rgba, w, x_open, y), [0, 0, 0, 0]);
}

#[test]
fn parse_coord_accepts_plane_and_rejects_junk() {
    assert_eq!(
        parse_coord("3208, 3218, 0"),
        Some(GameTile {
            x: 3208,
            z: 3218,
            plane: 0
        })
    );
    assert_eq!(
        parse_coord("1,2"),
        Some(GameTile {
            x: 1,
            z: 2,
            plane: 0
        })
    );
    assert!(parse_coord("1,2,4").is_none());
    assert!(parse_coord("bank").is_none());
}

#[test]
fn search_filters_fixture_pois() {
    let mut map = WalkMapRenderer::new();
    assert!(map.fixtures().is_empty());
    map.bind_fixtures(FixtureStore::lumbridge().expect("fixtures"));
    map.search = "bank".into();
    let hits = map.search_hits();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].name.as_str(), "Bank booth");
    map.search = "no-such-place".into();
    assert!(map.search_hits().is_empty());
}

#[test]
fn production_map_ships_without_fixture_tiles() {
    let map = WalkMapRenderer::new();
    assert!(map.fixtures().is_empty());
    assert_eq!(map.status_line(), CACHE_UNBOUND);
    assert!(!map
        .search_hits()
        .iter()
        .any(|p| p.name.as_str() == "Bank booth"));
}

#[test]
fn hidpi_zoom_out_falls_back_and_recovers() {
    let world = open_world(3, 3);
    let mut map = WalkMapRenderer::new();
    map.note_open();
    let layers = layers_grid();
    let colors = OverlayColors::default();
    let size = [2560.0, 1440.0];
    let fb = 2.0;
    let far = view_from_canvas((3220, 3220), (0.0, 0.0), 0.25, size, 0, 1, fb);
    let vis = select_lod(
        View {
            max_lod: nav::map::spatial::MAX_LOD,
            ..far
        },
        TEXTURE_CAP,
    );
    assert!(
        vis.is_ok(),
        "physical-pixel view must coarsen instead of error"
    );
    assert!(vis.unwrap().len() <= TEXTURE_CAP as u64);
    for _ in 0..4 {
        map.test_sync(None, far, &world, layers, colors, &[], &[], None);
    }
    assert_ne!(map.phase(), MapPhase::Error);
    assert!(map.slot_keys().len() <= TEXTURE_CAP);
    assert!(map.counters().decode_staging_bytes <= 1024 * 1024);
    let near = view_from_canvas((3220, 3220), (0.0, 0.0), 16.0, size, 0, 1, fb);
    for _ in 0..4 {
        map.test_sync(None, near, &world, layers, colors, &[], &[], None);
    }
    assert_ne!(map.phase(), MapPhase::Error);
    assert_eq!(map.status_line(), CACHE_UNBOUND);
}

#[test]
fn bot_movement_does_not_rerasterize_static_overlay() {
    let world = open_world(8, 8);
    let mut map = WalkMapRenderer::new();
    map.note_open();
    let view = small_view();
    let layers = layers_grid();
    let a = [(
        WorldTile {
            x: 1,
            z: 1,
            level: 0,
        },
        false,
    )];
    let b = [(
        WorldTile {
            x: 4,
            z: 4,
            level: 0,
        },
        false,
    )];
    map.test_sync(
        None,
        view,
        &world,
        layers,
        OverlayColors::default(),
        &a,
        &[],
        None,
    );
    let first = map.overlay_cpu().map(|c| c.to_vec());
    map.test_sync(
        None,
        view,
        &world,
        layers,
        OverlayColors::default(),
        &b,
        &[],
        None,
    );
    let second = map.overlay_cpu().map(|c| c.to_vec());
    assert_eq!(
        first, second,
        "path change must not rebuild the static overlay"
    );
}

#[test]
fn late_worker_result_cannot_resurrect_closed_map() {
    let world = open_world(3, 3);
    let mut map = WalkMapRenderer::with_sync(false);
    map.bind_fixtures(FixtureStore::lumbridge().expect("fixtures"));
    map.note_open();
    let view = view_from_canvas((3220, 3220), (0.0, 0.0), 16.0, [200.0, 200.0], 0, 1, 1.0);
    let layers = layers_grid();
    let colors = OverlayColors::default();
    map.test_sync(None, view, &world, layers, colors, &[], &[], None);
    let gen = map.generation();
    map.release(None);
    assert!(!map.is_open());
    assert!(map.slot_keys().is_empty());
    assert!(map.overlay_cpu().is_none());
    assert_eq!(map.counters().in_flight, 0);
    std::thread::sleep(Duration::from_millis(150));
    assert!(
        map.slot_keys().is_empty(),
        "a late decode must not restore slots after close"
    );
    map.note_open();
    assert_ne!(map.generation(), gen);
    assert!(map.slot_keys().is_empty());
}

#[test]
fn overlay_cpu_exists_until_gpu_upload() {
    let world = bake_world(4, 4, &[]);
    let mut map = WalkMapRenderer::new();
    map.note_open();
    let view = small_view();
    map.test_sync(
        None,
        view,
        &world,
        layers_grid(),
        OverlayColors::default(),
        &[],
        &[],
        None,
    );
    let (w, h) = map.overlay_size();
    assert!(w > 0 && h > 0);
    assert_eq!(
        map.overlay_cpu().map(|c| c.len()),
        Some((w as usize) * (h as usize) * 4)
    );
    map.release(None);
    assert!(map.overlay_cpu().is_none());
}

#[test]
fn map_draw_geometry_stays_under_budget() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ctx = dear_imgui_rs::Context::create();
    ctx.prepare_frame(
        dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0).renderer_has_textures(),
    );
    let mut map = WalkMapRenderer::new();
    map.note_open();
    let world = open_world(64, 64);
    let view = view_from_canvas((32, 32), (0.0, 0.0), 2.0, [765.0, 503.0], 0, 1, 1.0);
    let mut zoom_in = false;
    {
        let ui = ctx.frame();
        map.present(
            ui,
            None,
            [0.0, 0.0],
            [765.0, 503.0],
            view,
            &world,
            layers_grid(),
            OverlayColors::default(),
            &[],
            &[],
            None,
            None,
            None,
            None,
            &mut zoom_in,
        );
    }
    let draw = ctx.render();
    let vtx = draw.total_vtx_count() as i32;
    let idx = draw.total_idx_count() as i32;
    map.record_geometry(vtx, idx);
    assert!(
        vtx <= MAX_VTX_DEFAULT,
        "default map vtx {vtx} exceeds {MAX_VTX_DEFAULT}"
    );
    assert!(
        idx <= MAX_IDX_DEFAULT,
        "default map idx {idx} exceeds {MAX_IDX_DEFAULT}"
    );
    assert!(vtx <= MAX_VTX_LAYERS && idx <= MAX_IDX_LAYERS);
}

struct RecordingGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    registered: Vec<(u64, wgpu::Texture)>,
    unregistered: Vec<u64>,
}

impl RecordingGpu {
    fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        Self {
            device,
            queue,
            registered: Vec::new(),
            unregistered: Vec::new(),
        }
    }
}

impl FrameGpu for RecordingGpu {
    fn device(&self) -> &wgpu::Device {
        &self.device
    }
    fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }
    fn register_texture(
        &mut self,
        texture: &wgpu::Texture,
        _view: &wgpu::TextureView,
    ) -> TextureId {
        let id = self.registered.len() as u64 + 1;
        self.registered.push((id, texture.clone()));
        TextureId::new(id)
    }
    fn unregister_texture(&mut self, tex_id: TextureId) {
        self.unregistered.push(tex_id.id());
    }
}

fn headless_gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("274 walk-map test"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        experimental_features: wgpu::ExperimentalFeatures::default(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::default(),
    }))
    .ok()
}

#[test]
fn gpu_close_unregisters_textures_and_drops_cpu() {
    let Some((device, queue)) = headless_gpu() else {
        return;
    };
    let mut gpu = RecordingGpu::new(device, queue);
    let world = open_world(3, 3);
    let mut map = WalkMapRenderer::new();
    map.note_open();
    let view = view_from_canvas((3220, 3220), (0.0, 0.0), 16.0, [200.0, 200.0], 0, 1, 1.0);
    for _ in 0..8 {
        map.test_sync(
            Some(&mut gpu),
            view,
            &world,
            layers_grid(),
            OverlayColors::default(),
            &[],
            &[],
            None,
        );
    }
    assert!(
        !gpu.registered.is_empty(),
        "grid overlay uploads without fixture tiles"
    );
    assert!(
        map.overlay_cpu().is_none(),
        "CPU overlay dropped after GPU upload"
    );
    assert_eq!(map.counters().terrain_cpu_bytes, 0);
    let registered: Vec<u64> = gpu.registered.iter().map(|(id, _)| *id).collect();
    let baseline = phys_footprint_bytes();
    map.release(Some(&mut gpu));
    for id in registered {
        assert!(
            gpu.unregistered.contains(&id),
            "close must unregister texture {id}"
        );
    }
    assert!(map.slot_keys().is_empty());
    assert_eq!(map.counters().overlay_gpu_bytes, 0);
    assert_eq!(map.counters().phase, MapPhase::Pending);
    if let (Some(before), Some(after)) = (baseline, phys_footprint_bytes()) {
        eprintln!(
            "walk-map phys_footprint before-close={before} after-close={after} delta={}",
            after as i64 - before as i64
        );
    }
}

#[test]
fn flood_change_rerasterizes_overlay() {
    let world = open_world(8, 8);
    let mut map = WalkMapRenderer::new();
    map.note_open();
    let view = small_view();
    let layers = OverlayLayers {
        grid: false,
        collision_fill: false,
        reach: false,
        nsew: false,
        path: false,
        flood: true,
    };
    let a: Arc<HashSet<WorldTile>> = Arc::new(
        [WorldTile {
            x: 1,
            z: 1,
            level: 0,
        }]
        .into_iter()
        .collect(),
    );
    map.test_sync(
        None,
        view,
        &world,
        layers,
        OverlayColors::default(),
        &[],
        &[a],
        None,
    );
    let first = map.overlay_cpu().map(|c| c.to_vec());
    let b: Arc<HashSet<WorldTile>> = Arc::new(
        [WorldTile {
            x: 2,
            z: 2,
            level: 0,
        }]
        .into_iter()
        .collect(),
    );
    map.test_sync(
        None,
        view,
        &world,
        layers,
        OverlayColors::default(),
        &[],
        &[b],
        None,
    );
    let second = map.overlay_cpu().map(|c| c.to_vec());
    assert_ne!(
        first, second,
        "flood membership change must rebuild overlay"
    );
}

#[test]
fn overlay_byte_cap_is_1_5_mib_rgba() {
    assert_eq!(OVERLAY_BYTE_CAP, 1_572_864);
    assert_eq!(
        OVERLAY_BYTE_CAP,
        (OVERLAY_MAX_W as usize) * (OVERLAY_MAX_H as usize) * 4
    );
    let max_ready = View {
        west: 0.0,
        south: 0.0,
        east: f64::from(OVERLAY_MAX_W) / MIN_CELL_PPT,
        north: f64::from(OVERLAY_MAX_H) / MIN_CELL_PPT,
        pixels_per_tile: MIN_CELL_PPT,
        plane: 0,
        max_lod: 1,
    };
    match overlay_fit(max_ready) {
        OverlayFit::Ready { w, h } => {
            assert_eq!(w, OVERLAY_MAX_W);
            assert_eq!(h, OVERLAY_MAX_H);
            assert_eq!((w as usize) * (h as usize) * 4, OVERLAY_BYTE_CAP);
        }
        OverlayFit::ZoomIn => panic!("192×128 tiles at 4 overlay-px/tile must fit the cap"),
    }
    let over = View {
        west: 0.0,
        south: 0.0,
        east: f64::from(OVERLAY_MAX_W) / MIN_CELL_PPT + 1.0,
        north: f64::from(OVERLAY_MAX_H) / MIN_CELL_PPT,
        pixels_per_tile: 2.0,
        plane: 0,
        max_lod: 1,
    };
    assert_eq!(overlay_fit(over), OverlayFit::ZoomIn);
}

#[test]
fn last_layer_off_drops_overlay_buffers() {
    let world = bake_world(8, 8, &[]);
    let mut map = WalkMapRenderer::new();
    map.note_open();
    map.test_sync(
        None,
        small_view(),
        &world,
        layers_grid(),
        OverlayColors::default(),
        &[],
        &[],
        None,
    );
    let on = map.counters();
    assert!(on.overlay_cpu_bytes > 0);
    assert!(on.overlay_cpu_bytes <= OVERLAY_BYTE_CAP);
    assert_eq!(on.overlay_gpu_bytes, 0);
    map.test_sync(
        None,
        small_view(),
        &world,
        layers_none(),
        OverlayColors::default(),
        &[],
        &[],
        None,
    );
    let off = map.counters();
    assert_eq!(off.overlay_cpu_bytes, 0);
    assert_eq!(off.overlay_gpu_bytes, 0);
    assert!(map.overlay_cpu().is_none());
    assert_eq!(map.overlay_size(), (0, 0));
}

#[test]
fn close_then_reopen_releases_overlay_and_starts_empty() {
    let world = bake_world(8, 8, &[]);
    let mut map = WalkMapRenderer::new();
    map.note_open();
    let gen = map.generation();
    map.test_sync(
        None,
        small_view(),
        &world,
        layers_grid(),
        OverlayColors::default(),
        &[],
        &[],
        None,
    );
    assert!(map.counters().overlay_cpu_bytes > 0);
    map.release(None);
    let closed = map.counters();
    assert!(!map.is_open());
    assert_ne!(map.generation(), gen);
    assert_eq!(closed.overlay_cpu_bytes, 0);
    assert_eq!(closed.overlay_gpu_bytes, 0);
    assert_eq!(closed.terrain_slots, 0);
    assert_eq!(closed.terrain_cpu_bytes, 0);
    assert_eq!(closed.pending_upload, 0);
    assert_eq!(closed.decode_staging_bytes, 0);
    map.note_open();
    let reopened = map.counters();
    assert!(map.is_open());
    assert_eq!(reopened.overlay_cpu_bytes, 0);
    assert_eq!(reopened.overlay_gpu_bytes, 0);
    assert_eq!(reopened.terrain_slots, 0);
}

#[test]
fn all_tile_layers_geometry_stays_under_layer_budget() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ctx = dear_imgui_rs::Context::create();
    ctx.prepare_frame(
        dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0).renderer_has_textures(),
    );
    let mut map = WalkMapRenderer::new();
    map.note_open();
    let world = open_world(256, 256);
    let view = view_from_canvas((96, 64), (0.0, 0.0), 4.0, [765.0, 503.0], 0, 1, 1.0);
    let path = [(
        WorldTile {
            x: 96,
            z: 64,
            level: 0,
        },
        false,
    )];
    let mut zoom_in = false;
    {
        let ui = ctx.frame();
        map.present(
            ui,
            None,
            [0.0, 0.0],
            [765.0, 503.0],
            view,
            &world,
            layers_all(),
            OverlayColors::default(),
            &path,
            &[],
            None,
            None,
            None,
            None,
            &mut zoom_in,
        );
    }
    assert!(!zoom_in, "4 px/tile on 765×503 must keep the overlay ready");
    let draw = ctx.render();
    let vtx = draw.total_vtx_count() as i32;
    let idx = draw.total_idx_count() as i32;
    map.record_geometry(vtx, idx);
    assert!(
        vtx <= MAX_VTX_LAYERS,
        "layered map vtx {vtx} exceeds {MAX_VTX_LAYERS}"
    );
    assert!(
        idx <= MAX_IDX_LAYERS,
        "layered map idx {idx} exceeds {MAX_IDX_LAYERS}"
    );
    let c = map.counters();
    assert!(c.overlay_cpu_bytes > 0);
    assert!(c.overlay_cpu_bytes <= OVERLAY_BYTE_CAP);
}

#[test]
fn last_layer_off_unregisters_overlay_gpu() {
    let Some((device, queue)) = headless_gpu() else {
        return;
    };
    let mut gpu = RecordingGpu::new(device, queue);
    let world = bake_world(8, 8, &[]);
    let mut map = WalkMapRenderer::new();
    map.note_open();
    map.test_sync(
        Some(&mut gpu),
        small_view(),
        &world,
        layers_grid(),
        OverlayColors::default(),
        &[],
        &[],
        None,
    );
    assert!(
        !gpu.registered.is_empty(),
        "overlay GPU upload on first layer"
    );
    assert!(map.counters().overlay_gpu_bytes > 0);
    assert!(map.counters().overlay_gpu_bytes <= OVERLAY_BYTE_CAP);
    assert_eq!(map.counters().overlay_cpu_bytes, 0);
    let unreg_before = gpu.unregistered.len();
    map.test_sync(
        Some(&mut gpu),
        small_view(),
        &world,
        layers_none(),
        OverlayColors::default(),
        &[],
        &[],
        None,
    );
    assert_eq!(map.counters().overlay_gpu_bytes, 0);
    assert_eq!(map.counters().overlay_cpu_bytes, 0);
    assert!(
        gpu.unregistered.len() > unreg_before,
        "turning the last layer off must unregister the overlay texture"
    );
}
