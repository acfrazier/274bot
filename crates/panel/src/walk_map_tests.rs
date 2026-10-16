use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;
use dear_imgui_rs::TextureId;
use frontend_core::{WalkMapDecodeController, WalkMapDecodeResult};
use nav::collision::WorldCollision;
use nav::map::formats::ImageManifest;
use nav::map::identity::{Digest, ImageIdentity};
use nav::map::spatial::{
    select_lod, GameTile, TileKey, View, TEXTURE_CAP, TILE_PIXELS, TILE_RGBA_BYTES,
};
use nav::transport::{TransportGraph, WildernessZone};
use nav::world::NavWorld;

use super::overlay::{self, OverlayColors, OverlayFit, OverlayLayers, OverlayPaint};
use super::{
    content_priority, decode_tile, dist2_to_centre, encode_tile, kind_rank, overlay_fit,
    parse_coord, phys_footprint_bytes, place_map_labels, rasterize_overlay, special_area_rect,
    view_from_canvas, FixtureStore, LabelRect, MapLabelCandidate, MapPhase, WalkMapRenderer,
    CACHE_UNBOUND, MAX_IDX_DEFAULT, MAX_IDX_LAYERS, MAX_LABELS, MAX_VTX_DEFAULT, MAX_VTX_LAYERS,
    MIN_CELL_PPT, OVERLAY_BYTE_CAP, OVERLAY_MAX_H, OVERLAY_MAX_W,
};
use crate::game_view::FrameGpu;
use crate::test_support::headless_gpu;
use nav::map::poi::PoiKind;

fn decode_service_test_lock() -> std::sync::MutexGuard<'static, ()> {
    crate::work_service_test_lock()
}

fn decode_service_capacity() -> api::work::WorkCapacity {
    script::work_admission::WorkService::shared()
        .expect("shared work service")
        .lock()
        .capacity()
}

fn wait_for_decode_service_capacity_zero() {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let capacity = decode_service_capacity();
        if capacity == api::work::WorkCapacity::EMPTY {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "shared decode capacity did not return to zero: {capacity:?}"
        );
        std::thread::yield_now();
    }
}

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
        special_areas: false,
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
        special_areas: false,
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
        special_areas: false,
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

/// F0-4 (§26 §2.2, §2.6): the frozen walk-map-decode family keeps `Job` as
/// one interruptible unit, `Stop` as no unit, and the PNG length check as a
/// size bound rather than a time-bound claim.
#[test]
fn frozen_walk_map_decode_family_keeps_command_contract() {
    use super::{decode_checked, DECODE_LIMIT_BYTES};
    use api::work::{WalkMapDecodeCommand, WorkFamily};
    use nav::map::MapError;

    assert_eq!(
        [
            WalkMapDecodeCommand::Job.is_interruptible_unit(),
            WalkMapDecodeCommand::Stop.is_interruptible_unit(),
        ],
        [true, false]
    );

    let key = lod0(50, 50);
    let png = encode_tile(key).expect("encode");
    let rgba = decode_checked(&png).expect("decode one bounded tile");
    assert_eq!(rgba.len(), TILE_RGBA_BYTES);

    let oversized = vec![0u8; DECODE_LIMIT_BYTES + 1];
    let error = decode_checked(&oversized).expect_err("oversized PNG is refused");
    assert_eq!(
        error.to_string(),
        MapError::Limit("png bytes").to_string(),
        "the length check is a size bound, not a time bound"
    );
    assert_eq!(
        WorkFamily::WalkMapDecode.interruptible_unit(),
        "one decode_checked job (not PNG-length alone)"
    );
    assert_eq!(
        WorkFamily::WalkMapDecode.phases(),
        &["Receive", "Decode", "Publish"]
    );
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
fn special_area_rect_uses_exclusive_content_zone_edges_at_coarse_zoom() {
    let view = View {
        west: 2944.0,
        south: 3519.0,
        east: 3392.0,
        north: 3523.0,
        pixels_per_tile: 2.0,
        plane: 0,
        max_lod: 0,
    };
    let zone = WildernessZone {
        x1: 2944,
        z1: 3520,
        x2: 3391,
        z2: 3521,
        level1: 0,
        level2: 0,
        origin_z: 3520,
    };
    assert_eq!(
        special_area_rect(view, [10.0, 20.0], [896.0, 8.0], zone),
        Some(([10.0, 22.0], [906.0, 26.0])),
        "inclusive zone facts become exclusive screen bounds at 2 px/tile"
    );
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
                special_areas: false,
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
fn close_releases_renderer_state_without_waiting() {
    let world = open_world(3, 3);
    let mut map = WalkMapRenderer::new();
    map.bind_fixtures(FixtureStore::lumbridge().expect("fixtures"));
    map.note_open();
    let view = view_from_canvas((3220, 3220), (0.0, 0.0), 16.0, [200.0, 200.0], 0, 1, 1.0);
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
    let generation = map.generation();
    map.release(None);
    assert!(!map.is_open());
    assert!(map.slot_keys().is_empty());
    assert!(map.overlay_cpu().is_none());
    assert_eq!(map.counters().in_flight, 0);
    assert_eq!(map.counters().decode_staging_bytes, 0);
    map.note_open();
    assert_ne!(map.generation(), generation);
    assert!(map.slot_keys().is_empty());
}

fn wait_for_decode(controller: &mut WalkMapDecodeController) -> WalkMapDecodeResult {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match controller.poll() {
            Some(Ok(result)) => return result,
            Some(Err(error)) => panic!("walk map decode failed: {error}"),
            None => {
                assert!(
                    Instant::now() < deadline,
                    "shared WorkAdmission decode did not settle within five seconds"
                );
                std::thread::yield_now();
            }
        }
    }
}

fn serialize_decode_outcome(
    generation: u64,
    image: ImageIdentity,
    key: TileKey,
    rgba: &[u8],
) -> Vec<u8> {
    let length = u32::try_from(rgba.len()).expect("tile result fits the wire length");
    let mut serialized = Vec::with_capacity(93 + rgba.len());
    serialized.extend_from_slice(b"WMD1");
    serialized.extend_from_slice(&generation.to_be_bytes());
    serialized.extend_from_slice(&image.revision.to_be_bytes());
    serialized.extend_from_slice(&image.content.0);
    serialized.extend_from_slice(&image.policy.0);
    serialized.push(key.plane);
    serialized.push(key.lod);
    serialized.extend_from_slice(&key.x.to_be_bytes());
    serialized.extend_from_slice(&key.z.to_be_bytes());
    serialized.push(0);
    serialized.extend_from_slice(&length.to_be_bytes());
    serialized.extend_from_slice(rgba);
    serialized
}

#[test]
fn walk_map_decode_supersedes_old_generation_identity_and_key() {
    let _work_service_guard = decode_service_test_lock();
    let old_key = lod0(50, 50);
    let new_key = lod0(51, 50);
    let old_identity = super::fixtures::image_identity();
    let new_identity = ImageIdentity {
        revision: old_identity.revision,
        content: Digest([0xe1; 32]),
        policy: Digest([0xe2; 32]),
    };
    let old_png = Arc::<[u8]>::from(encode_tile(old_key).expect("old tile"));
    let new_png_vec = encode_tile(new_key).expect("new tile");
    let old_expected = decode_tile(&new_png_vec).expect("legacy decode oracle");
    let mut controller = WalkMapDecodeController::new();
    controller
        .request(7, old_identity, old_key, old_png)
        .expect("first demand");
    controller
        .request(8, new_identity, new_key, Arc::from(new_png_vec))
        .expect("superseding demand");
    let result = wait_for_decode(&mut controller);
    assert_eq!(result.generation(), 8);
    assert_eq!(result.image_identity(), new_identity);
    assert_eq!(result.key(), new_key);
    let result_rgba = result.into_rgba();
    assert_eq!(
        serialize_decode_outcome(8, new_identity, new_key, &result_rgba),
        serialize_decode_outcome(8, new_identity, new_key, &old_expected),
        "superseding outcome must match the old decoder byte-for-byte"
    );
    assert!(!controller.is_busy());
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
    let mut ctx = crate::test_support::imgui_context();
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
        special_areas: false,
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
fn special_area_layer_avoids_raster_buffers() {
    let world = open_world(8, 8);
    let mut map = WalkMapRenderer::new();
    map.note_open();
    let visible = OverlayLayers {
        special_areas: true,
        ..OverlayLayers::default()
    };
    let zoom_in = map.test_sync(
        None,
        small_view(),
        &world,
        visible,
        OverlayColors::default(),
        &[],
        &[],
        None,
    );
    assert!(!zoom_in, "screen-space area rectangles work at coarse zoom");
    assert_eq!(map.counters().overlay_cpu_bytes, 0);
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
    let mut ctx = crate::test_support::imgui_context();
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

fn lod0(x: i32, z: i32) -> TileKey {
    TileKey {
        plane: 0,
        lod: 0,
        x,
        z,
    }
}

fn tile_view(x: i32, z: i32, scale: f32, size: f32, max_lod: u8) -> View {
    view_from_canvas(
        (x * 64 + 32, z * 64 + 32),
        (0.0, 0.0),
        scale,
        [size, size],
        0,
        max_lod,
        1.0,
    )
}
fn grid_view(min_x: i32, min_z: i32, width: i32, height: i32, plane: u8, lod: u8) -> View {
    let span = (64_i64 << lod) as f64;
    let west = f64::from(min_x) * span;
    let south = f64::from(min_z) * span;
    View {
        west,
        south,
        east: west + f64::from(width) * span,
        north: south + f64::from(height) * span,
        pixels_per_tile: if lod == 0 { 16.0 } else { 1.5 },
        plane,
        max_lod: lod,
    }
}

fn sync_test_view(map: &mut WalkMapRenderer, world: &NavWorld, view: View) {
    map.test_sync(
        None,
        view,
        world,
        layers_none(),
        OverlayColors::default(),
        &[],
        &[],
        None,
    );
}
fn sync_test_view_with_gpu(
    map: &mut WalkMapRenderer,
    world: &NavWorld,
    view: View,
    gpu: &mut Option<RecordingGpu>,
) {
    map.test_sync(
        gpu.as_mut().map(|gpu| gpu as &mut dyn FrameGpu),
        view,
        world,
        layers_none(),
        OverlayColors::default(),
        &[],
        &[],
        None,
    );
}

fn assert_terrain_cap(map: &WalkMapRenderer, gpu: &Option<RecordingGpu>) {
    assert!(map.slots.len() <= TEXTURE_CAP);
    let texture_count =
        map.tex_pool.len() + map.slots.iter().filter(|slot| slot.gpu.is_some()).count();
    assert!(texture_count <= TEXTURE_CAP);
    if let Some(gpu) = gpu {
        assert!(gpu.registered.len() <= TEXTURE_CAP);
    }
}

#[test]
fn renderer_decodes_one_visible_tile_per_sync_turn() {
    let world = open_world(3, 3);
    let keys: Vec<_> = (0..4)
        .flat_map(|x| (0..4).map(move |z| lod0(x, z)))
        .collect();
    let mut map = WalkMapRenderer::new();
    map.bind_fixtures(FixtureStore::with_keys(keys).expect("fixtures"));
    map.note_open();
    let view = tile_view(1, 1, 16.0, 256.0, 0);
    map.test_sync(
        None,
        view,
        &world,
        layers_none(),
        OverlayColors::default(),
        &[],
        &[],
        None,
    );
    assert!(
        map.slot_keys().len() <= 1,
        "one visible sync turn must decode and install at most one tile"
    );
}

#[test]
fn terrain_slots_reuse_when_panning_one_tile_and_back() {
    let _work_service_guard = decode_service_test_lock();
    wait_for_decode_service_capacity_zero();
    let world = open_world(3, 3);
    let near_key = lod0(0, 0);
    let right_key = lod0(1, 0);
    let mut map = WalkMapRenderer::new();
    let mut gpu = headless_gpu().map(|(device, queue)| RecordingGpu::new(device, queue));
    map.test_sync_decode = false;
    map.bind_fixtures(FixtureStore::with_keys([near_key, right_key]).expect("fixtures"));
    map.note_open();
    let near = grid_view(0, 0, 1, 1, 0, 0);
    let right = grid_view(1, 0, 1, 1, 0, 0);
    let near_slots = settle_renderer_with_gpu(&mut map, &world, near, &mut gpu);
    assert!(near_slots.contains(&near_key), "the visible tile must load");
    settle_renderer_with_gpu(&mut map, &world, right, &mut gpu);

    let mut before_return = map.slot_keys();
    before_return.sort();
    let before_counters = map.counters();
    request_async_view(&mut map, near);
    let after_counters = map.counters();
    assert!(!map.decode.is_busy(), "the returning tile needs no decode");
    assert_eq!(after_counters.in_flight, before_counters.in_flight);
    assert_eq!(
        after_counters.pending_upload,
        before_counters.pending_upload
    );
    assert_eq!(
        after_counters.decode_staging_bytes,
        before_counters.decode_staging_bytes
    );
    let after_return = settle_renderer_with_gpu(&mut map, &world, near, &mut gpu);
    let settled_counters = map.counters();
    assert_eq!(settled_counters.in_flight, before_counters.in_flight);
    assert_eq!(
        settled_counters.pending_upload,
        before_counters.pending_upload
    );
    assert_eq!(
        settled_counters.decode_staging_bytes,
        before_counters.decode_staging_bytes
    );
    assert_eq!(
        after_return, before_return,
        "the returning cached tile is not decoded or uploaded again"
    );
    assert!(after_return.contains(&near_key));
    assert!(
        near_slots.contains(&right_key),
        "the one-tile ring must prefetch the next tile"
    );
    wait_for_decode_service_capacity_zero();
}

#[test]
fn terrain_slots_respect_cap_and_evict_least_recent_under_long_pan() {
    let world = open_world(3, 3);
    let keys = (0..34)
        .flat_map(|x| {
            (0..6).map(move |z| TileKey {
                plane: 0,
                lod: 0,
                x,
                z,
            })
        })
        .collect::<Vec<_>>();
    let mut map = WalkMapRenderer::new();
    let mut gpu = headless_gpu().map(|(device, queue)| RecordingGpu::new(device, queue));
    map.bind_fixtures(FixtureStore::with_keys(keys).expect("fixtures"));
    map.note_open();
    let full = grid_view(0, 0, 4, 6, 0, 0);
    for _ in 0..TEXTURE_CAP {
        sync_test_view_with_gpu(&mut map, &world, full, &mut gpu);
        assert_terrain_cap(&map, &gpu);
    }
    assert_eq!(map.slot_keys().len(), TEXTURE_CAP);

    for x in 0..4 {
        for z in 0..6 {
            if (x, z) != (0, 0) {
                sync_test_view_with_gpu(&mut map, &world, grid_view(x, z, 1, 1, 0, 0), &mut gpu);
                assert_terrain_cap(&map, &gpu);
                assert_eq!(
                    map.slot_keys().len(),
                    TEXTURE_CAP,
                    "a one-tile pan must keep the warm terrain slots"
                );
            }
        }
    }

    sync_test_view_with_gpu(&mut map, &world, grid_view(1, 0, 4, 6, 0, 0), &mut gpu);
    assert_terrain_cap(&map, &gpu);
    let after_first_eviction = map.slot_keys();
    assert_eq!(after_first_eviction.len(), TEXTURE_CAP);
    assert!(
        !after_first_eviction.contains(&lod0(0, 0)),
        "the least recently visible slot must leave first"
    );
    assert!(
        after_first_eviction.contains(&lod0(0, 1)),
        "a more recently visible slot must remain"
    );

    for x in 1..=30 {
        let view = grid_view(x, 0, 4, 6, 0, 0);
        for _ in 0..6 {
            sync_test_view_with_gpu(&mut map, &world, view, &mut gpu);
            assert_terrain_cap(&map, &gpu);
        }
    }
}

#[test]
fn terrain_slots_drop_when_plane_or_lod_changes() {
    let world = open_world(3, 3);
    let p0l0 = [
        TileKey {
            plane: 0,
            lod: 0,
            x: 0,
            z: 0,
        },
        TileKey {
            plane: 0,
            lod: 0,
            x: 1,
            z: 0,
        },
    ];
    let p1l0 = TileKey {
        plane: 1,
        lod: 0,
        x: 0,
        z: 0,
    };
    let p1l1 = TileKey {
        plane: 1,
        lod: 1,
        x: 0,
        z: 0,
    };
    let mut map = WalkMapRenderer::new();
    map.bind_fixtures(FixtureStore::with_keys([p0l0[0], p0l0[1], p1l0, p1l1]).expect("fixtures"));
    map.note_open();
    let near = grid_view(0, 0, 1, 1, 0, 0);
    let right = grid_view(1, 0, 1, 1, 0, 0);
    sync_test_view(&mut map, &world, near);
    sync_test_view(&mut map, &world, near);
    sync_test_view(&mut map, &world, right);
    assert!(
        map.slot_keys().contains(&p0l0[0]),
        "panning must retain the earlier plane and LOD"
    );

    sync_test_view(&mut map, &world, grid_view(0, 0, 1, 1, 1, 0));
    let plane_slots = map.slot_keys();
    assert!(plane_slots.iter().all(|key| key.plane == 1 && key.lod == 0));
    assert!(!plane_slots.contains(&p0l0[0]));

    sync_test_view(&mut map, &world, grid_view(0, 0, 1, 1, 1, 1));
    let lod_slots = map.slot_keys();
    assert!(lod_slots.iter().all(|key| key.plane == 1 && key.lod == 1));
    assert!(!lod_slots.contains(&p1l0));
    assert!(lod_slots.contains(&p1l1));
}

#[test]
fn decode_staging_bytes_is_unclamped_sum() {
    let _work_service_guard = decode_service_test_lock();
    wait_for_decode_service_capacity_zero();

    let active_key = lod0(60, 60);
    let pending_key = lod0(61, 60);
    let upload_key = lod0(62, 60);
    let identity = super::fixtures::image_identity();
    let active_png = Arc::<[u8]>::from(encode_tile(active_key).expect("active PNG"));
    let pending_png = Arc::<[u8]>::from(encode_tile(pending_key).expect("pending PNG"));
    let upload_png = encode_tile(upload_key).expect("upload PNG");
    let upload_rgba = decode_tile(&upload_png).expect("decode upload PNG");

    let mut map = WalkMapRenderer::new();
    map.note_open();
    map.decode
        .request(
            map.generation,
            identity,
            active_key,
            Arc::clone(&active_png),
        )
        .expect("submit active decode");
    map.decode
        .request(
            map.generation,
            identity,
            pending_key,
            Arc::clone(&pending_png),
        )
        .expect("retain pending decode");
    assert!(map.decode.has_active());
    assert_eq!(
        map.decode.pending_request(),
        Some((map.generation, identity, pending_key))
    );
    map.pending_upload = Some((map.generation, identity, upload_key, upload_rgba.clone()));

    let expected = active_png.len() + TILE_RGBA_BYTES + pending_png.len() + upload_rgba.len();
    let counters = map.counters();
    assert_eq!(counters.in_flight, 1);
    assert_eq!(counters.pending_upload, 1);
    assert_eq!(counters.decode_staging_bytes, expected);

    drop(map);
    wait_for_decode_service_capacity_zero();
}

#[test]
fn terrain_lod_stops_at_the_bound_bake_max_lod() {
    // At 0.25 px/tile the density rule would coarsen to LOD 3, but this bake
    // only has LOD 0. The 3×3 LOD-0 cover fits the slot cap, so the map must
    // load those tiles instead of asking for levels the bake never wrote.
    let world = open_world(3, 3);
    let keys: Vec<_> = (0..3)
        .flat_map(|x| (0..3).map(move |z| lod0(x, z)))
        .collect();
    let mut map = WalkMapRenderer::new();
    map.bind_fixtures(FixtureStore::with_keys(keys).expect("fixtures"));
    map.note_open();
    let view = tile_view(1, 1, 0.25, 40.0, map.max_lod());
    for _ in 0..24 {
        map.test_sync(
            None,
            view,
            &world,
            layers_none(),
            OverlayColors::default(),
            &[],
            &[],
            None,
        );
    }
    let slots = map.slot_keys();
    assert!(!slots.is_empty(), "LOD-0 terrain must load");
    assert!(
        slots.iter().all(|key| key.lod == 0),
        "terrain asked for a LOD the bake does not have: {slots:?}"
    );
}

fn label_candidate(
    id: usize,
    content_priority: u8,
    kind_rank: u8,
    dist2: u32,
    rect: LabelRect,
) -> MapLabelCandidate {
    MapLabelCandidate {
        id,
        name_off: 0,
        name_len: 0,
        px: rect.x,
        py: rect.y,
        content_priority,
        kind_rank,
        dist2,
        rect,
    }
}

fn placed_ids(candidates: &mut [MapLabelCandidate], cap: usize) -> Vec<usize> {
    let mut out = Vec::new();
    place_map_labels(candidates, cap, &mut out);
    out.iter().map(|&i| candidates[i].id).collect()
}

fn any_overlap(candidates: &[MapLabelCandidate], ids: &[usize]) -> bool {
    ids.iter().enumerate().any(|(i, &a)| {
        ids[i + 1..].iter().any(|&b| {
            let left = candidates.iter().find(|c| c.id == a).unwrap();
            let right = candidates.iter().find(|c| c.id == b).unwrap();
            left.rect.overlaps(right.rect)
        })
    })
}

/// Catalogue-order placement as 0.1.9 `draw_pois` did: first `cap` names, no
/// collision test.
fn place_in_visit_order(candidates: &[MapLabelCandidate], cap: usize) -> Vec<usize> {
    candidates.iter().take(cap).map(|c| c.id).collect()
}

fn varrock_west_bank_row() -> Vec<MapLabelCandidate> {
    // 16 px/tile, "Bank"/"Banker" ~40x13 at the shared (5,-6) offset.
    // Six booths on z=0 and six bankers one tile north; neighbouring labels
    // overlap because the text is wider than a tile.
    let ppt = 16.0;
    let w = 40.0;
    let h = 13.0;
    let mut out = Vec::new();
    for i in 0..6 {
        let x = i as f32 * ppt + 5.0;
        out.push(label_candidate(
            i,
            255,
            kind_rank(PoiKind::Bank),
            dist2_to_centre(i as f64 + 0.5 - 2.5, 0.0),
            LabelRect { x, y: -6.0, w, h },
        ));
    }
    for i in 0..6 {
        let x = i as f32 * ppt + 5.0;
        out.push(label_candidate(
            6 + i,
            255,
            kind_rank(PoiKind::Bank),
            dist2_to_centre(i as f64 + 0.5 - 2.5, 1.5 - 0.5),
            LabelRect {
                x,
                y: ppt - 6.0,
                w,
                h,
            },
        ));
    }
    out
}

#[test]
fn bank_row_labels_skip_overlaps_the_old_visit_order_stacked() {
    let row = varrock_west_bank_row();
    let old = place_in_visit_order(&row, MAX_LABELS);
    assert_eq!(old.len(), 12, "0.1.9 drew every Bank and Banker in the row");
    assert!(
        any_overlap(&row, &old),
        "catalogue-order placement stacks the bank-row labels"
    );

    let mut ranked = row.clone();
    let placed = placed_ids(&mut ranked, MAX_LABELS);
    assert!(
        !any_overlap(&ranked, &placed),
        "placed labels must not overlap: {placed:?}"
    );
    assert!(
        placed.len() < old.len(),
        "overlap filter must drop stacked bank-row labels: {placed:?}"
    );
    assert!(
        placed.contains(&2),
        "the booth nearest the view centre must keep its label: {placed:?}"
    );
}

#[test]
fn content_label_priority_beats_an_overlapping_bank() {
    let bank = label_candidate(
        0,
        content_priority(PoiKind::Bank),
        kind_rank(PoiKind::Bank),
        0,
        LabelRect {
            x: 0.0,
            y: 0.0,
            w: 40.0,
            h: 12.0,
        },
    );
    let place = label_candidate(
        1,
        content_priority(PoiKind::Label { priority: 1 }),
        kind_rank(PoiKind::Label { priority: 1 }),
        80,
        LabelRect {
            x: 8.0,
            y: 0.0,
            w: 80.0,
            h: 12.0,
        },
    );
    let mut candidates = [bank, place];
    let placed = placed_ids(&mut candidates, MAX_LABELS);
    assert_eq!(
        placed,
        vec![1],
        "the content label must win the overlapping bank"
    );
}

#[test]
fn map_label_cap_still_applies_after_overlap_filter() {
    let mut candidates: Vec<_> = (0..40)
        .map(|i| {
            label_candidate(
                i,
                255,
                0,
                i as u32,
                LabelRect {
                    x: i as f32 * 80.0,
                    y: 0.0,
                    w: 40.0,
                    h: 12.0,
                },
            )
        })
        .collect();
    let placed = placed_ids(&mut candidates, MAX_LABELS);
    assert_eq!(placed.len(), MAX_LABELS);
    assert!(
        !any_overlap(&candidates, &placed),
        "non-overlapping labels still respect MAX_LABELS"
    );
}

fn label_collect_view() -> View {
    View {
        west: 0.0,
        south: 0.0,
        east: 100.0,
        north: 100.0,
        pixels_per_tile: 16.0,
        plane: 0,
        max_lod: 0,
    }
}

fn stored_label_text(names: &str, candidate: MapLabelCandidate) -> &str {
    let start = candidate.name_off as usize;
    &names[start..start + candidate.name_len as usize]
}

fn collect_named_poi(names: &mut String, candidates: &mut Vec<MapLabelCandidate>, name: &str) {
    super::push_map_label(
        names,
        candidates,
        label_collect_view(),
        [0.0, 0.0],
        [1600.0, 1600.0],
        50.5,
        50.5,
        PoiKind::Label { priority: 1 },
        name,
    );
}

#[test]
fn map_marker_text_uses_display_name() {
    let mut names = String::new();
    let mut candidates = Vec::new();
    collect_named_poi(&mut names, &mut candidates, "Port/Sarim");
    assert_eq!(candidates.len(), 1);
    assert_eq!(stored_label_text(&names, candidates[0]), "Port Sarim");
    candidates[0].rect = LabelRect {
        x: 0.0,
        y: 0.0,
        w: 40.0,
        h: 12.0,
    };
    let mut placed = Vec::new();
    place_map_labels(&mut candidates, MAX_LABELS, &mut placed);
    assert_eq!(placed.len(), 1);
    assert_eq!(
        stored_label_text(&names, candidates[placed[0]]),
        "Port Sarim",
        "layout must keep the collected display name, not Port/Sarim"
    );
}

#[test]
fn slash_label_collect_reuses_name_buffer_after_warmup() {
    let view = label_collect_view();
    let mut names = String::new();
    let mut candidates = Vec::new();
    let collect = |names: &mut String, candidates: &mut Vec<MapLabelCandidate>| {
        names.clear();
        candidates.clear();
        for i in 0..12 {
            super::push_map_label(
                names,
                candidates,
                view,
                [0.0, 0.0],
                [1600.0, 1600.0],
                10.0 + f64::from(i),
                10.0,
                PoiKind::Label { priority: 1 },
                "Port/Sarim",
            );
        }
    };
    collect(&mut names, &mut candidates);
    assert_eq!(candidates.len(), 12);
    assert_eq!(stored_label_text(&names, candidates[0]), "Port Sarim");
    let name_cap = names.capacity();
    let cand_cap = candidates.capacity();
    collect(&mut names, &mut candidates);
    assert_eq!(
        names.capacity(),
        name_cap,
        "warm name buffer must not grow on the next collect"
    );
    assert_eq!(
        candidates.capacity(),
        cand_cap,
        "warm candidate vec must not grow on the next collect"
    );
    let mut placed = Vec::new();
    place_map_labels(&mut candidates, MAX_LABELS, &mut placed);
    let placed_cap = placed.capacity();
    place_map_labels(&mut candidates, MAX_LABELS, &mut placed);
    assert_eq!(
        placed.capacity(),
        placed_cap,
        "warm placed-id vec must not grow on the next rank"
    );
}
#[test]
#[ignore = "requires BOT_WALK_MAP_DECODE_CORPUS to point at a published real image artifact"]
fn real_walk_map_decode_matches_legacy_outcome_bytes() {
    let _work_service_guard = decode_service_test_lock();
    #[derive(serde::Deserialize)]
    struct DeclaredIdentity {
        identity: ImageIdentity,
    }

    let root = std::path::PathBuf::from(
        std::env::var_os("BOT_WALK_MAP_DECODE_CORPUS")
            .expect("BOT_WALK_MAP_DECODE_CORPUS must name the published image artifact"),
    );
    let manifest_bytes =
        std::fs::read(root.join("manifest.json")).expect("read real image manifest");
    let declared: DeclaredIdentity =
        serde_json::from_slice(&manifest_bytes).expect("read declared image identity");
    let manifest =
        ImageManifest::decode(&manifest_bytes, declared.identity).expect("validate image manifest");
    let tiles = manifest.tiles.as_slice();
    assert!(
        !tiles.is_empty(),
        "published image manifest must contain terrain"
    );
    let mut controller = WalkMapDecodeController::new();

    for receipt in tiles {
        let path = root.join(receipt.key.relative_path().expect("validated tile path"));
        let png = std::fs::read(path).expect("read published terrain PNG");
        receipt
            .verify_png(&png)
            .expect("published terrain PNG matches its receipt");
        let legacy_rgba = decode_tile(&png).expect("legacy decode oracle");
        controller
            .request(1, manifest.identity, receipt.key, Arc::from(png))
            .expect("submit published terrain PNG");
        let result = wait_for_decode(&mut controller);
        assert_eq!(result.generation(), 1);
        assert_eq!(result.image_identity(), manifest.identity);
        assert_eq!(result.key(), receipt.key);
        let result_rgba = result.into_rgba();
        assert_eq!(
            serialize_decode_outcome(1, manifest.identity, receipt.key, &result_rgba),
            serialize_decode_outcome(1, manifest.identity, receipt.key, &legacy_rgba),
            "decoded outcome differs for published tile {:?}",
            receipt.key
        );
        assert!(
            !controller.is_busy(),
            "consuming the real-content result releases its shared ticket"
        );
    }
}

fn encode_png_with_ztxt(rgba: &[u8], text: String) -> Vec<u8> {
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, TILE_PIXELS, TILE_PIXELS);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .add_ztxt_chunk(String::from("description"), text)
            .expect("encode compressed text chunk");
        let mut writer = encoder.write_header().expect("write PNG header");
        writer.write_image_data(rgba).expect("write PNG pixels");
    }
    png
}

fn encode_png_with_iccp(rgba: &[u8], profile: &[u8]) -> Vec<u8> {
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, TILE_PIXELS, TILE_PIXELS);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("write PNG header");
        writer
            .write_chunk(png::chunk::iCCP, profile)
            .expect("encode compressed ICC chunk");
        writer.write_image_data(rgba).expect("write PNG pixels");
    }
    png
}

fn exif_description(size: usize) -> Vec<u8> {
    assert!(size >= 27);
    let mut tiff = vec![0u8; size];
    tiff[0..2].copy_from_slice(b"II");
    tiff[2..4].copy_from_slice(&42u16.to_le_bytes());
    tiff[4..8].copy_from_slice(&8u32.to_le_bytes());
    tiff[8..10].copy_from_slice(&1u16.to_le_bytes());
    tiff[10..12].copy_from_slice(&270u16.to_le_bytes());
    tiff[12..14].copy_from_slice(&2u16.to_le_bytes());
    tiff[14..18].copy_from_slice(&u32::try_from(size - 26).unwrap().to_le_bytes());
    tiff[18..22].copy_from_slice(&26u32.to_le_bytes());
    let text_end = size - 1;
    tiff[26..text_end].fill(b'x');
    tiff
}

fn encode_png_with_exif(rgba: &[u8], exif: &[u8]) -> Vec<u8> {
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, TILE_PIXELS, TILE_PIXELS);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("write PNG header");
        writer
            .write_chunk(png::chunk::eXIf, exif)
            .expect("encode Exif chunk");
        writer.write_image_data(rgba).expect("write PNG pixels");
    }
    png
}

fn find_png_chunk_payload<'a>(png: &'a [u8], wanted: &[u8; 4]) -> &'a [u8] {
    let mut offset = 8usize;
    while offset + 12 <= png.len() {
        let length = u32::from_be_bytes(
            png[offset..offset + 4]
                .try_into()
                .expect("PNG chunk length"),
        ) as usize;
        let end = offset + length + 12;
        assert!(end <= png.len(), "PNG chunk fits the encoded file");
        if &png[offset + 4..offset + 8] == wanted {
            return &png[offset + 8..offset + 8 + length];
        }
        offset = end;
    }
    panic!("requested PNG chunk exists");
}

#[test]
fn ancillary_text_and_icc_preserve_legacy_pixels() {
    let _work_service_guard = decode_service_test_lock();
    let identity = super::fixtures::image_identity();
    let mut controller = WalkMapDecodeController::new();
    let large_text = String::from("x").repeat(400 * 1024);

    for (key, with_icc) in [(lod0(52, 50), false), (lod0(53, 50), true)] {
        let base_png = encode_tile(key).expect("encode source tile");
        let expected_rgba = decode_tile(&base_png).expect("decode source tile");
        let text_png = encode_png_with_ztxt(&expected_rgba, large_text.clone());
        let png = if with_icc {
            let profile = find_png_chunk_payload(&text_png, b"zTXt");
            encode_png_with_iccp(&expected_rgba, profile)
        } else {
            text_png
        };
        assert!(png.len() <= 512 * 1024);

        let legacy_rgba = decode_tile(&png).expect("legacy decoder accepts ancillary PNG");
        assert_eq!(legacy_rgba, expected_rgba);
        controller
            .request(1, identity, key, Arc::from(png))
            .expect("submit ancillary PNG");
        let result = wait_for_decode(&mut controller);
        assert_eq!(result.generation(), 1);
        assert_eq!(result.image_identity(), identity);
        assert_eq!(result.key(), key);
        let result_rgba = result.into_rgba();
        assert_eq!(
            serialize_decode_outcome(1, identity, key, &result_rgba),
            serialize_decode_outcome(1, identity, key, &legacy_rgba),
            "ancillary decode differs from the legacy pixels"
        );
        assert!(!controller.is_busy());
    }
    let key = lod0(54, 50);
    let base_png = encode_tile(key).expect("encode Exif source tile");
    let expected_rgba = decode_tile(&base_png).expect("decode Exif source tile");
    let exif = exif_description(250 * 1024);
    let png = encode_png_with_exif(&expected_rgba, &exif);
    assert!(png.len() <= 512 * 1024);
    let legacy_rgba = decode_tile(&png).expect("legacy decoder accepts Exif PNG");
    assert_eq!(legacy_rgba, expected_rgba);
    controller
        .request(1, identity, key, Arc::from(png))
        .expect("submit Exif PNG");
    let result = wait_for_decode(&mut controller);
    assert_eq!(result.generation(), 1);
    assert_eq!(result.image_identity(), identity);
    assert_eq!(result.key(), key);
    let result_rgba = result.into_rgba();
    assert_eq!(
        serialize_decode_outcome(1, identity, key, &result_rgba),
        serialize_decode_outcome(1, identity, key, &legacy_rgba),
        "Exif decode differs from the legacy pixels"
    );
    assert!(!controller.is_busy());
}

fn async_test_turn(map: &mut WalkMapRenderer, world: &NavWorld, view: View) {
    let mut gpu = None;
    async_test_turn_with_gpu(map, world, view, &mut gpu);
}

fn async_test_turn_with_gpu(
    map: &mut WalkMapRenderer,
    world: &NavWorld,
    view: View,
    gpu: &mut Option<RecordingGpu>,
) {
    map.test_sync(
        gpu.as_mut().map(|gpu| gpu as &mut dyn FrameGpu),
        view,
        world,
        layers_none(),
        OverlayColors::default(),
        &[],
        &[],
        None,
    );
}

fn request_async_view(map: &mut WalkMapRenderer, view: View) {
    let mut gpu: Option<&mut dyn FrameGpu> = None;
    map.sync_terrain(view, &mut gpu);
}

fn settle_renderer(map: &mut WalkMapRenderer, world: &NavWorld, view: View) -> Vec<TileKey> {
    let mut gpu = None;
    settle_renderer_with_gpu(map, world, view, &mut gpu)
}

fn settle_renderer_with_gpu(
    map: &mut WalkMapRenderer,
    world: &NavWorld,
    view: View,
    gpu: &mut Option<RecordingGpu>,
) -> Vec<TileKey> {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut quiet = 0;
    let mut previous = Vec::new();
    while quiet < 16 {
        async_test_turn_with_gpu(map, world, view, gpu);
        let mut keys = map.slot_keys();
        keys.sort();
        if !map.decode.is_busy() && map.pending_upload.is_none() && keys == previous {
            quiet += 1;
        } else {
            quiet = 0;
        }
        previous = keys;
        assert!(
            Instant::now() < deadline,
            "async renderer did not settle; slots: {previous:?}"
        );
        std::thread::yield_now();
    }
    previous
}

fn async_renderer_keys() -> Vec<TileKey> {
    let mut keys: Vec<_> = (0..8)
        .flat_map(|x| (0..8).map(move |z| lod0(x, z)))
        .collect();
    keys.extend((20..28).flat_map(|x| (20..28).map(move |z| lod0(x, z))));
    keys
}

#[test]
fn renderer_background_decode_pans_refills_and_releases_every_owner() {
    let _work_service_guard = decode_service_test_lock();
    wait_for_decode_service_capacity_zero();
    let world = open_world(3, 3);
    let near = tile_view(3, 3, 4.0, 640.0, 0);
    let far = tile_view(23, 23, 4.0, 640.0, 0);
    let fixtures = FixtureStore::with_keys(async_renderer_keys()).expect("fixtures");

    let near_visible = select_lod(near, TEXTURE_CAP).expect("near visible tiles");
    let far_visible = select_lod(far, TEXTURE_CAP).expect("far visible tiles");
    let mut close_map = WalkMapRenderer::new();
    close_map.test_sync_decode = false;
    close_map.bind_fixtures(fixtures.clone());
    close_map.note_open();
    request_async_view(&mut close_map, far);
    assert!(
        close_map.decode.has_active(),
        "close setup has an active decode"
    );
    close_map.release(None);
    assert_eq!(close_map.counters().in_flight, 0);
    assert_eq!(close_map.counters().decode_staging_bytes, 0);
    wait_for_decode_service_capacity_zero();

    let mut dropped = WalkMapRenderer::new();
    dropped.test_sync_decode = false;
    dropped.bind_fixtures(fixtures.clone());
    dropped.note_open();
    request_async_view(&mut dropped, near);
    assert!(
        dropped.decode.has_active(),
        "drop setup has an active decode"
    );
    drop(dropped);
    wait_for_decode_service_capacity_zero();

    let mut map = WalkMapRenderer::new();
    map.test_sync_decode = false;
    map.bind_fixtures(fixtures.clone());
    map.note_open();
    request_async_view(&mut map, near);
    assert!(
        map.decode.has_active(),
        "renderer turns must submit through the real background controller"
    );
    let near_slots = settle_renderer(&mut map, &world, near);
    assert!(
        near_visible.keys().all(|key| near_slots.contains(&key)),
        "the renderer must load every near visible tile"
    );
    wait_for_decode_service_capacity_zero();

    let mut max_held_tickets = 0;
    for turn in 0..400 {
        async_test_turn(&mut map, &world, if turn % 2 == 0 { near } else { far });
        max_held_tickets = max_held_tickets.max(map.decode.held_ticket_count_for_test());
    }
    assert!(
        max_held_tickets <= 1,
        "one renderer retains at most one accepted ticket, saw {max_held_tickets}"
    );
    let far_slots = settle_renderer(&mut map, &world, far);
    assert!(
        far_visible.keys().all(|key| far_slots.contains(&key)),
        "the renderer must load every far visible tile"
    );
    assert!(far_slots.len() <= TEXTURE_CAP);
    wait_for_decode_service_capacity_zero();

    map.release(None);
    map.bind_fixtures(fixtures.clone());
    map.note_open();
    request_async_view(&mut map, near);
    assert!(
        map.decode.has_active(),
        "pack-change setup has an active decode"
    );
    let old_generation = map.generation();
    map.sync_identity(Digest([0x5a; 32]), Some((0, 0, 3, 3)), None);
    assert_ne!(map.generation(), old_generation);
    assert!(map.pending_upload.is_none());
    assert!(!map.decode.is_busy());
    let near_slots = settle_renderer(&mut map, &world, near);
    assert!(
        near_visible.keys().all(|key| near_slots.contains(&key)),
        "the renderer must reload every visible tile after identity invalidation"
    );
    wait_for_decode_service_capacity_zero();

    for cycle in 0..200 {
        let mut churn = WalkMapRenderer::new();
        churn.test_sync_decode = false;
        churn.bind_fixtures(fixtures.clone());
        churn.note_open();
        request_async_view(&mut churn, if cycle % 2 == 0 { near } else { far });
        if cycle % 3 == 0 {
            request_async_view(&mut churn, if cycle % 2 == 0 { far } else { near });
        }
        drop(churn);
    }
    wait_for_decode_service_capacity_zero();
}

fn png_chunk(kind: &[u8; 4], payload: &[u8], valid_crc: bool) -> Vec<u8> {
    let mut chunk = (payload.len() as u32).to_be_bytes().to_vec();
    chunk.extend_from_slice(kind);
    chunk.extend_from_slice(payload);
    let crc = if valid_crc {
        let mut crc = 0xffff_ffffu32;
        for byte in kind.iter().chain(payload) {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    } else {
        0
    };
    chunk.extend_from_slice(&crc.to_be_bytes());
    chunk
}

#[test]
fn trailing_data_matches_legacy_decoder_acceptance() {
    let _work_service_guard = decode_service_test_lock();
    wait_for_decode_service_capacity_zero();
    let key = lod0(63, 60);
    let identity = super::fixtures::image_identity();
    let base = encode_tile(key).expect("valid producer tile");
    let iend = base.len() - 12;
    assert_eq!(&base[iend + 4..iend + 8], b"IEND");
    let with_before_iend = |extra: Vec<u8>| {
        let mut png = base.clone();
        png.splice(iend..iend, extra);
        png
    };
    let mut truncated_trailing_chunk = base[..iend].to_vec();
    truncated_trailing_chunk.extend_from_slice(&100u32.to_be_bytes());
    truncated_trailing_chunk.extend_from_slice(b"tEXt");
    truncated_trailing_chunk.extend_from_slice(b"partial");
    let cases = [
        (
            "damaged tEXt CRC",
            with_before_iend(png_chunk(b"tEXt", b"x", false)),
        ),
        (
            "damaged ancillary CRC",
            with_before_iend(png_chunk(b"prIv", b"x", false)),
        ),
        (
            "unknown critical chunk after IDAT",
            with_before_iend(png_chunk(b"QQQQ", b"x", true)),
        ),
        (
            "trailing IDAT after another chunk",
            with_before_iend(
                [
                    png_chunk(b"prIv", b"x", true),
                    png_chunk(b"IDAT", b"x", true),
                ]
                .concat(),
            ),
        ),
        (
            "truncated trailing chunk without IEND",
            truncated_trailing_chunk,
        ),
        ("no IEND", base[..iend].to_vec()),
    ];
    let mut mismatches = Vec::new();
    for (index, (label, png)) in cases.into_iter().enumerate() {
        let legacy = decode_tile(&png);
        let mut controller = WalkMapDecodeController::new();
        controller
            .request(12 + index as u64, identity, key, Arc::from(png))
            .expect("submit trailing-data variant");
        let deadline = Instant::now() + Duration::from_secs(5);
        let decoded = loop {
            match controller.poll() {
                Some(result) => break result,
                None => {
                    assert!(
                        Instant::now() < deadline,
                        "{label}: background decode did not settle"
                    );
                    std::thread::yield_now();
                }
            }
        };
        let legacy_accepted = legacy.is_ok();
        let new_accepted = decoded.is_ok();
        if legacy_accepted != new_accepted {
            mismatches.push(format!(
                "{label}: legacy={legacy_accepted}, new={new_accepted}"
            ));
        }
        if let (Ok(expected), Ok(result)) = (legacy, decoded) {
            assert_eq!(
                result.into_rgba(),
                expected,
                "{label}: decoded pixels differ"
            );
        }
        assert!(!controller.is_busy());
    }
    wait_for_decode_service_capacity_zero();
    assert!(
        mismatches.is_empty(),
        "new decoder accept/reject behavior differs from legacy: {mismatches:?}"
    );
}
